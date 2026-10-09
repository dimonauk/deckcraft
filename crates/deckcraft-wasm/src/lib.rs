//! DeckCraft in the browser: the headless engine behind a three-function JS API.
//!
//! ```text
//! const h = create_presentation();
//! const reply = dispatch_action(h, '{"type":"InsertShape","preset":"rect","rect":[40,40,200,100]}');
//! const tree  = get_spatial_render_tree(h);   // JSON string, schema "holocore.spatial/deckcraft/1"
//! ```
//!
//! Every function takes and returns plain strings, so the same shapes work over a WebSocket or
//! JSON-RPC daemon (INAIR Pod, PCVR). Errors never throw: they come back as
//! `{"ok":false,"error":"…"}`.

use deckcraft_bridge::Bridge;
use wasm_bindgen::prelude::*;

/// Opaque handle to one open presentation (owned by Rust, referenced from JS).
#[wasm_bindgen]
pub struct PresentationHandle {
    bridge: Bridge,
}

/// Open a new, empty presentation.
#[wasm_bindgen]
pub fn create_presentation() -> PresentationHandle {
    PresentationHandle { bridge: Bridge::new() }
}

/// Apply one JSON action (`InsertShape`, `UpdateTransform`, `ReorderSlide`, `Undo`, `Redo`).
/// Returns `{"ok":true,"result":…,"revision":…,"canUndo":…,"canRedo":…,"slideCount":…}` or
/// `{"ok":false,"error":"…"}`.
#[wasm_bindgen]
pub fn dispatch_action(handle: &mut PresentationHandle, action_json: &str) -> String {
    handle.bridge.dispatch(action_json)
}

/// The spatial render tree as a JSON string (slide cards, resolved shape boxes, z-order, text).
#[wasm_bindgen]
pub fn get_spatial_render_tree(handle: &PresentationHandle) -> String {
    match handle.bridge.spatial_render_tree_json() {
        Ok(s) => s,
        Err(e) => error_json(&e.to_string()),
    }
}

/// The full document AST as a JSON string.
#[wasm_bindgen]
pub fn get_document_ast(handle: &PresentationHandle) -> String {
    match handle.bridge.ast_json() {
        Ok(s) => s,
        Err(e) => error_json(&e.to_string()),
    }
}

fn error_json(msg: &str) -> String {
    // Escape through serde-free means: only `"` and `\` and control characters need care.
    let mut out = String::from("{\"ok\":false,\"error\":\"");
    for c in msg.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 => out.push(' '),
            c => out.push(c),
        }
    }
    out.push_str("\"}");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_dispatch_render() {
        let mut h = create_presentation();
        let reply = dispatch_action(&mut h, r#"{"type":"InsertShape","preset":"rect","rect":[40,40,200,100],"text":"Hi"}"#);
        assert!(reply.contains("\"ok\":true"), "{reply}");
        let tree = get_spatial_render_tree(&h);
        assert!(tree.contains("holocore.spatial/deckcraft/1"));
        assert!(tree.contains("\"text\":\"Hi\""), "{tree}");
        assert!(get_document_ast(&h).contains("\"slides\""));
    }

    #[test]
    fn errors_are_json() {
        let mut h = create_presentation();
        let reply = dispatch_action(&mut h, "nonsense");
        assert!(reply.contains("\"ok\":false") && reply.contains("\"error\":"), "{reply}");
        assert_eq!(error_json("a \"b\"\n"), "{\"ok\":false,\"error\":\"a \\\"b\\\" \"}");
    }
}
