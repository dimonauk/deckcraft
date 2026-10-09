//! HoloCore Spatial Daemon for Deckcraft.
//!
//! Listens on `ws://127.0.0.1:8095` and serves JSON-RPC 2.0 requests from
//! HoloCore XR clients (@holo-core/craft-kit).

use std::net::TcpListener;
use std::thread;

use deckcraft_bridge::Bridge;
use deckcraft_mcp::{Headless, Server};
use serde_json::{Value, json};
use tungstenite::accept;

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

            let mut bridge = Bridge::new();
            let headless = Box::new(Headless::new());
            let mut mcp_server = Server::new(headless);

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
                                match bridge.spatial_render_tree() {
                                    Ok(tree) => json!({ "jsonrpc": "2.0", "id": id, "result": tree }),
                                    Err(err) => json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32000, "message": err.to_string() } }),
                                }
                            }
                            "doc.ast" => {
                                match bridge.ast_value() {
                                    Ok(ast) => json!({ "jsonrpc": "2.0", "id": id, "result": ast }),
                                    Err(err) => json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32000, "message": err.to_string() } }),
                                }
                            }
                            "bridge.dispatch" => {
                                let action_str = serde_json::to_string(&params).unwrap_or_default();
                                let res_str = bridge.dispatch(&action_str);
                                let res_val: Value = serde_json::from_str(&res_str).unwrap_or_else(|_| json!(res_str));
                                json!({ "jsonrpc": "2.0", "id": id, "result": res_val })
                            }
                            _ => {
                                // Delegate to MCP server
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
