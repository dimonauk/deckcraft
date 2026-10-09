//! HoloCore Spatial Daemon for Deckcraft.
//!
//! Listens on `ws://127.0.0.1:8095` and serves JSON-RPC 2.0 requests from
//! HoloCore XR clients (@holo-core/craft-kit).
//!
//! Shares the exact same Presentation Session between the spatial render tree
//! bridge (`render.spatial_tree`, `bridge.dispatch`, `doc.ast`) and the
//! standard MCP tools (`command_run`, `command_batch`, `doc_inspect`, etc.).

use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::thread;

use deckcraft_bridge::Bridge;
use deckcraft_mcp::{Backend, Server};
use serde_json::{Value, json};
use tungstenite::accept;

/// A shared backend that directs MCP tool execution to the bridge's live Session.
struct SharedBridgeBackend {
    bridge: Arc<Mutex<Bridge>>,
}

impl SharedBridgeBackend {
    fn new(bridge: Arc<Mutex<Bridge>>) -> Self {
        Self { bridge }
    }
}

impl Backend for SharedBridgeBackend {
    fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let mut b = self.bridge.lock().map_err(|e| e.to_string())?;
        let s = b.session_mut();
        match method {
            "engine.execute" | "command" | "ui.menu.invoke" => {
                let id = params
                    .get("command")
                    .or_else(|| params.get("id"))
                    .and_then(Value::as_str)
                    .ok_or("missing `command`")?;
                let cp = params.get("params").cloned().filter(|v| !v.is_null()).unwrap_or(json!({}));
                let r = s.execute(id, &cp).map_err(|e| e.to_string())?;
                s.ui_requests.clear();
                Ok(r)
            }
            "engine.commands" => Ok(serde_json::to_value(s.commands()).unwrap_or_default()),
            "document.inspect" => {
                let r = s.execute("document.inspect", &json!({})).map_err(|e| e.to_string())?;
                s.ui_requests.clear();
                Ok(r)
            }
            "app.open" => {
                let r = s.execute("file.open", &params).map_err(|e| e.to_string())?;
                s.ui_requests.clear();
                Ok(r)
            }
            "app.save" => {
                let r = s.execute("file.save", &params).map_err(|e| e.to_string())?;
                s.ui_requests.clear();
                Ok(r)
            }
            "app.export" => {
                let r = s.execute("file.export", &params).map_err(|e| e.to_string())?;
                s.ui_requests.clear();
                Ok(r)
            }
            "ui.render" | "app.renderSlide" => {
                let st = s.active().ok_or("no presentation open")?;
                let i = params.get("slide").and_then(Value::as_u64).map(|v| v as usize).unwrap_or(st.selection.slide);
                if i >= st.doc.slides.len() {
                    return Err(format!("no slide {i}"));
                }
                let scale = params.get("scale").and_then(Value::as_f64).unwrap_or(1.0).clamp(0.05, 8.0);
                let img = deckcraft_engine::render::render_slide(
                    &st.doc,
                    i,
                    &deckcraft_engine::render::RenderOpts {
                        scale,
                        edit: params.get("edit").and_then(Value::as_bool).unwrap_or(false),
                        ..Default::default()
                    },
                );
                let png = img.to_png();
                Ok(json!({
                    "png": deckcraft_engine::cmd::base64_encode(&png),
                    "width": img.width,
                    "height": img.height,
                    "slide": i
                }))
            }
            "ui.inspect" => Ok(json!({
                "headless": true,
                "tool": serde_json::to_value(&s.tool.kind).unwrap_or_default()
            })),
            other => Err(format!("unsupported method `{other}` in shared bridge backend")),
        }
    }

    fn has_ui(&self) -> bool {
        false
    }

    fn describe(&self) -> String {
        "shared-spatial-bridge".into()
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let port = std::env::var("PORT").unwrap_or_else(|_| "8095".into());
    let addr = format!("127.0.0.1:{}", port);
    let server = TcpListener::bind(&addr)?;
    println!("[deckcraft-spatial-daemon] Listening on ws://{}", addr);

    for stream in server.incoming() {
        let stream = match stream {
            Ok(s) => s,
            Err(e) => {
                eprintln!("[deckcraft-spatial-daemon] Connection accept error: {}", e);
                continue;
            }
        };

        thread::spawn(move || {
            let mut websocket = match accept(stream) {
                Ok(ws) => ws,
                Err(e) => {
                    eprintln!("[deckcraft-spatial-daemon] WebSocket handshake error: {}", e);
                    return;
                }
            };

            let bridge = Arc::new(Mutex::new(Bridge::new()));
            let backend = Box::new(SharedBridgeBackend::new(bridge.clone()));
            let mut mcp_server = Server::new(backend);

            println!("[deckcraft-spatial-daemon] Client connected");

            loop {
                let msg = match websocket.read() {
                    Ok(m) => m,
                    Err(e) => {
                        println!("[deckcraft-spatial-daemon] Client disconnected: {}", e);
                        break;
                    }
                };

                if msg.is_text() {
                    let text = msg.to_text().unwrap_or("");
                    if let Ok(req) = serde_json::from_str::<Value>(text) {
                        let id = req.get("id").cloned().unwrap_or(Value::Null);
                        let method = req.get("method").and_then(Value::as_str).unwrap_or("");
                        let params = req.get("params").cloned().unwrap_or(Value::Null);

                        let reply: Value = match method {
                            "render.spatial_tree" => {
                                match bridge.lock().unwrap().spatial_render_tree() {
                                    Ok(tree) => json!({ "jsonrpc": "2.0", "id": id, "result": tree }),
                                    Err(err) => json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32000, "message": err.to_string() } }),
                                }
                            }
                            "doc.ast" => {
                                match bridge.lock().unwrap().ast_value() {
                                    Ok(ast) => json!({ "jsonrpc": "2.0", "id": id, "result": ast }),
                                    Err(err) => json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32000, "message": err.to_string() } }),
                                }
                            }
                            "bridge.dispatch" => {
                                let action_str = serde_json::to_string(&params).unwrap_or_default();
                                let res_str = bridge.lock().unwrap().dispatch(&action_str);
                                let res_val: Value = serde_json::from_str(&res_str).unwrap_or_else(|_| json!(res_str));
                                json!({ "jsonrpc": "2.0", "id": id, "result": res_val })
                            }
                            _ => {
                                // Delegate to MCP server (handles inspect_document, run_command, list_commands, etc.)
                                if let Some(resp_line) = mcp_server.handle_line(text) {
                                    serde_json::from_str(&resp_line).unwrap_or_else(|_| {
                                        json!({ "jsonrpc": "2.0", "id": id, "result": resp_line })
                                    })
                                } else {
                                    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32601, "message": format!("Method not found: {}", method) } })
                                }
                            }
                        };

                        let reply_text = reply.to_string();
                        if let Err(e) = websocket.send(tungstenite::Message::Text(reply_text.into())) {
                            eprintln!("[deckcraft-spatial-daemon] Send error: {}", e);
                            break;
                        }
                    }
                } else if msg.is_close() {
                    break;
                }
            }
        });
    }

    Ok(())
}
