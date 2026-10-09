//! Headless bridge between the DeckCraft engine and HoloCore XR front ends.
//!
//! A [`Bridge`] owns an engine [`Session`] and exposes exactly two surfaces:
//!
//! * **AST out**: [`Bridge::ast_json`] serialises the active [`Presentation`] (slides, shapes,
//!   text runs, themes…) as JSON, and [`Bridge::snapshot_json`] wraps it with the revision,
//!   selection and undo/redo state a renderer needs to know when to redraw.
//! * **Actions in**: [`Bridge::dispatch`] takes one JSON [`Action`] (`InsertShape`,
//!   `UpdateTransform`, `ReorderSlide`, `Undo`, `Redo`) and routes it through the engine's command
//!   registry, so every change is journaled and undoable like any other frontend's.
//!
//! The crate has no UI, DOM or windowing dependency. Strings in, strings out: the same API serves
//! a `wasm-bindgen` wrapper (browser / WebXR) and a WebSocket or JSON-RPC daemon (INAIR Pod, PCVR).
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod spatial;

pub use spatial::SCHEMA as SPATIAL_SCHEMA;

use std::fmt;

use deckcraft_engine::{EngineError, Session};
use deckcraft_model::Presentation;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// A mutation of the active presentation. The wire form is internally tagged:
/// `{"type":"ReorderSlide","from":0,"to":2}`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all_fields = "camelCase")]
pub enum Action {
    /// Insert a preset shape on the current slide. `rect` is `[x, y, w, h]` in points.
    /// `preset` is an engine preset name (`rect`, `roundRect`, `ellipse`, `star5`, …).
    InsertShape {
        preset: String,
        rect: [f64; 4],
        #[serde(default, skip_serializing_if = "Option::is_none")]
        text: Option<String>,
    },
    /// Set any of a shape's absolute position (`x`, `y`), size (`w`, `h`) and rotation
    /// (`rotation`, degrees). Omitted fields are left alone.
    UpdateTransform {
        id: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        x: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        y: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        w: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        h: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        rotation: Option<f64>,
    },
    /// Move the slide at index `from` so it ends up at index `to`.
    ReorderSlide {
        from: usize,
        to: usize,
    },
    Undo,
    Redo,
}

#[derive(Debug)]
pub enum BridgeError {
    /// The action JSON could not be parsed, or the AST could not be serialised.
    Json(serde_json::Error),
    /// The action was well-formed but its values are unusable (NaN, no fields to change…).
    InvalidAction(String),
    /// The engine rejected or failed the command (the presentation is left as it was).
    Engine(EngineError),
    /// No presentation is open.
    NoDocument,
}

impl fmt::Display for BridgeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BridgeError::Json(e) => write!(f, "json: {e}"),
            BridgeError::InvalidAction(m) => write!(f, "invalid action: {m}"),
            BridgeError::Engine(e) => write!(f, "{e}"),
            BridgeError::NoDocument => write!(f, "no active presentation"),
        }
    }
}

impl std::error::Error for BridgeError {}

impl From<serde_json::Error> for BridgeError {
    fn from(e: serde_json::Error) -> Self {
        BridgeError::Json(e)
    }
}

impl From<EngineError> for BridgeError {
    fn from(e: EngineError) -> Self {
        match e {
            EngineError::NoDocument => BridgeError::NoDocument,
            e => BridgeError::Engine(e),
        }
    }
}

pub type Result<T> = std::result::Result<T, BridgeError>;

/// What a successful action reports back.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Outcome {
    /// The last engine command's result (e.g. the new shape's id for `InsertShape`).
    pub result: Value,
    /// Bumps on every change to the document or selection; renderers redraw when it moves.
    pub revision: u64,
    pub can_undo: bool,
    pub can_redo: bool,
    pub slide_count: usize,
}

/// One engine command: its registry id and JSON parameters.
type Step = (&'static str, Value);

pub struct Bridge {
    session: Session,
}

impl Default for Bridge {
    fn default() -> Self {
        Self::new()
    }
}

impl Bridge {
    /// A bridge over a session with one new, empty presentation open.
    pub fn new() -> Self {
        Bridge { session: Session::with_new() }
    }

    /// A bridge over an existing session (e.g. one that already loaded a deck).
    pub fn from_session(session: Session) -> Self {
        Bridge { session }
    }

    pub fn session(&self) -> &Session {
        &self.session
    }

    /// Escape hatch for host code that needs engine commands the bridge does not wrap.
    pub fn session_mut(&mut self) -> &mut Session {
        &mut self.session
    }

    /// The active presentation as a JSON value.
    pub fn ast_value(&self) -> Result<Value> {
        let st = self.session.doc()?;
        let p: &Presentation = &st.doc;
        Ok(serde_json::to_value(p)?)
    }

    /// The active presentation as a JSON string (the document AST only).
    pub fn ast_json(&self) -> Result<String> {
        let st = self.session.doc()?;
        let p: &Presentation = &st.doc;
        Ok(serde_json::to_string(p)?)
    }

    /// The AST plus the state a renderer needs: `{revision, canUndo, canRedo, selection,
    /// presentation}`.
    pub fn snapshot_json(&self) -> Result<String> {
        let st = self.session.doc()?;
        let snap = json!({
            "revision": st.revision,
            "canUndo": !st.history.undo.is_empty(),
            "canRedo": !st.history.redo.is_empty(),
            "selection": serde_json::to_value(&st.selection)?,
            "presentation": serde_json::to_value(&*st.doc)?,
        });
        Ok(serde_json::to_string(&snap)?)
    }

    /// The active presentation as a spatial render tree (see [`SPATIAL_SCHEMA`]): slide cards with
    /// resolved shape boxes, z-order and text, ready for a 3D front end to lay out.
    pub fn spatial_render_tree(&self) -> Result<Value> {
        Ok(spatial::render_tree(self.session.doc()?))
    }

    /// [`Bridge::spatial_render_tree`] as a JSON string.
    pub fn spatial_render_tree_json(&self) -> Result<String> {
        Ok(serde_json::to_string(&self.spatial_render_tree()?)?)
    }

    /// Apply a typed action.
    pub fn dispatch_action(&mut self, action: &Action) -> Result<Outcome> {
        let steps = plan(action)?;
        let mut result = Value::Null;
        for (id, params) in steps {
            result = self.session.execute(id, &params)?;
        }
        let st = self.session.doc()?;
        Ok(Outcome {
            result,
            revision: st.revision,
            can_undo: !st.history.undo.is_empty(),
            can_redo: !st.history.redo.is_empty(),
            slide_count: st.doc.slides.len(),
        })
    }

    /// Apply one JSON action and return a JSON reply: `{"ok":true, …Outcome}` or
    /// `{"ok":false,"error":"…"}`. Never panics and never returns a non-JSON string.
    pub fn dispatch(&mut self, action_json: &str) -> String {
        let reply = match serde_json::from_str::<Action>(action_json).map_err(BridgeError::from).and_then(|a| self.dispatch_action(&a)) {
            Ok(o) => match serde_json::to_value(&o) {
                Ok(Value::Object(mut m)) => {
                    m.insert("ok".into(), Value::Bool(true));
                    Value::Object(m)
                }
                _ => json!({ "ok": true }),
            },
            Err(e) => json!({ "ok": false, "error": e.to_string() }),
        };
        reply.to_string()
    }
}

fn finite(v: f64, what: &str) -> Result<f64> {
    if v.is_finite() { Ok(v) } else { Err(BridgeError::InvalidAction(format!("`{what}` must be a finite number"))) }
}

/// Translate an action into the engine commands that implement it.
fn plan(action: &Action) -> Result<Vec<Step>> {
    match action {
        Action::InsertShape { preset, rect, text } => {
            for (v, n) in rect.iter().zip(["rect[0]", "rect[1]", "rect[2]", "rect[3]"]) {
                finite(*v, n)?;
            }
            let mut p = json!({ "preset": preset, "rect": rect });
            if let (Some(t), Some(m)) = (text, p.as_object_mut()) {
                m.insert("text".into(), Value::String(t.clone()));
            }
            Ok(vec![("shape.insert", p)])
        }
        Action::UpdateTransform { id, x, y, w, h, rotation } => {
            let ids = json!([id]);
            let mut steps: Vec<Step> = Vec::new();
            let fin = |o: &Option<f64>, n: &str| -> Result<Option<f64>> { o.map(|v| finite(v, n)).transpose() };
            let (x, y, w, h, rot) = (fin(x, "x")?, fin(y, "y")?, fin(w, "w")?, fin(h, "h")?, fin(rotation, "rotation")?);
            if let (Some(x), Some(y), Some(w), Some(h)) = (x, y, w, h) {
                steps.push(("shape.setBounds", json!({ "x": x, "y": y, "w": w, "h": h, "ids": ids })));
            } else {
                if w.is_some() || h.is_some() {
                    let mut p = json!({ "ids": ids });
                    if let Some(m) = p.as_object_mut() {
                        if let Some(w) = w {
                            m.insert("w".into(), json!(w));
                        }
                        if let Some(h) = h {
                            m.insert("h".into(), json!(h));
                        }
                    }
                    steps.push(("shape.resize", p));
                }
                if x.is_some() || y.is_some() {
                    let mut p = json!({ "ids": ids });
                    if let Some(m) = p.as_object_mut() {
                        if let Some(x) = x {
                            m.insert("x".into(), json!(x));
                        }
                        if let Some(y) = y {
                            m.insert("y".into(), json!(y));
                        }
                    }
                    steps.push(("shape.move", p));
                }
            }
            if let Some(deg) = rot {
                steps.push(("shape.rotate", json!({ "deg": deg, "ids": ids })));
            }
            if steps.is_empty() {
                return Err(BridgeError::InvalidAction("UpdateTransform needs at least one of x, y, w, h, rotation".into()));
            }
            Ok(steps)
        }
        Action::ReorderSlide { from, to } => Ok(vec![("slide.move", json!({ "from": from, "to": to }))]),
        Action::Undo => Ok(vec![("edit.undo", json!({}))]),
        Action::Redo => Ok(vec![("edit.redo", json!({}))]),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn slide_ids(b: &Bridge) -> Vec<u64> {
        let v = b.ast_value().unwrap();
        v["slides"].as_array().unwrap().iter().map(|s| s["id"].as_u64().unwrap()).collect()
    }

    #[test]
    fn action_wire_format_round_trips() {
        let a: Action = serde_json::from_str(r#"{"type":"ReorderSlide","from":0,"to":2}"#).unwrap();
        assert_eq!(a, Action::ReorderSlide { from: 0, to: 2 });
        let a: Action = serde_json::from_str(r#"{"type":"Undo"}"#).unwrap();
        assert_eq!(a, Action::Undo);
        let a: Action = serde_json::from_str(r#"{"type":"UpdateTransform","id":4,"x":10,"rotation":15}"#).unwrap();
        assert_eq!(a, Action::UpdateTransform { id: 4, x: Some(10.0), y: None, w: None, h: None, rotation: Some(15.0) });
    }

    #[test]
    fn ast_is_json_with_slides() {
        let b = Bridge::new();
        let v: Value = serde_json::from_str(&b.ast_json().unwrap()).unwrap();
        assert!(v.get("slides").is_some_and(Value::is_array));
        let snap: Value = serde_json::from_str(&b.snapshot_json().unwrap()).unwrap();
        assert!(snap["presentation"].is_object());
        assert_eq!(snap["canUndo"], false);
    }

    #[test]
    fn insert_transform_undo_redo() {
        let mut b = Bridge::new();
        let before = b.ast_json().unwrap();
        let out = b.dispatch_action(&Action::InsertShape { preset: "rect".into(), rect: [10.0, 20.0, 100.0, 50.0], text: None }).unwrap();
        assert!(out.can_undo);
        let after_insert = b.ast_json().unwrap();
        assert_ne!(before, after_insert);

        let id = out.result["id"].as_u64().expect("shape.insert returns {id}") as u32;
        b.dispatch_action(&Action::UpdateTransform { id, x: Some(200.0), y: Some(120.0), w: Some(80.0), h: Some(40.0), rotation: None }).unwrap();
        assert_ne!(after_insert, b.ast_json().unwrap());

        b.dispatch_action(&Action::Undo).unwrap();
        assert_eq!(after_insert, b.ast_json().unwrap());
        let r = b.dispatch_action(&Action::Redo).unwrap();
        assert!(!r.can_redo);
    }

    #[test]
    fn reorder_slides() {
        let mut b = Bridge::new();
        for _ in 0..2 {
            b.session_mut().execute("slide.new", &Value::Null).unwrap();
        }
        let ids = slide_ids(&b);
        assert!(ids.len() >= 3);
        b.dispatch_action(&Action::ReorderSlide { from: 0, to: 2 }).unwrap();
        let moved = slide_ids(&b);
        assert_eq!(moved[2], ids[0]);
    }

    #[test]
    fn spatial_tree_tracks_edits() {
        let mut b = Bridge::new();
        let t0 = b.spatial_render_tree().unwrap();
        assert_eq!(t0["schema"], SPATIAL_SCHEMA);
        let slide0 = t0["slides"].as_array().unwrap().len();
        assert!(slide0 >= 1);

        b.dispatch_action(&Action::InsertShape { preset: "ellipse".into(), rect: [30.0, 40.0, 120.0, 60.0], text: Some("Hello XR".into()) }).unwrap();
        let t1: Value = serde_json::from_str(&b.spatial_render_tree_json().unwrap()).unwrap();
        assert!(t1["revision"].as_u64() > t0["revision"].as_u64());
        let cur = t1["currentSlide"].as_u64().unwrap() as usize;
        let shapes = t1["slides"][cur]["shapes"].as_array().unwrap();
        let s = shapes.last().unwrap();
        assert_eq!(s["preset"], "ellipse");
        assert_eq!(s["text"], "Hello XR");
        assert_eq!(s["w"].as_f64(), Some(120.0));
        assert_eq!(s["z"].as_u64(), Some(shapes.len() as u64 - 1));
    }

    #[test]
    fn bad_input_gets_a_json_error() {
        let mut b = Bridge::new();
        for bad in ["not json", r#"{"type":"Nope"}"#, r#"{"type":"UpdateTransform","id":1}"#, r#"{"type":"Undo"}"#] {
            let v: Value = serde_json::from_str(&b.dispatch(bad)).unwrap();
            assert_eq!(v["ok"], false, "{bad}");
            assert!(v["error"].is_string());
        }
    }
}
