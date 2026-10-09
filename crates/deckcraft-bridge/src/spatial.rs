//! The spatial render tree: a renderer-neutral JSON description of the active presentation that a
//! 3D front end (`tsl-kit`) can lay out as a light-table of slide cards.
//!
//! Everything is in slide points, origin top-left, y down, exactly as the engine stores it; the
//! front end owns the mapping to metres and room space. Placeholders that inherit their box from a
//! layout or master are resolved, so every shape node carries a concrete `x, y, w, h`.
//!
//! v1 scope: top-level shapes only (group children are not expanded, `childCount` says how many
//! there are), plain text per shape, no fills or strokes yet.

use deckcraft_engine::cmd::xfrm_of;
use deckcraft_engine::{DocState, Selection};
use deckcraft_model::{Geom, Presentation, Shape, ShapeKind};
use serde_json::{Value, json};

/// Identifies the shape of the tree so a front end can reject trees it doesn't understand.
pub const SCHEMA: &str = "holocore.spatial/deckcraft/1";

pub(crate) fn render_tree(st: &DocState) -> Value {
    let doc: &Presentation = &st.doc;
    let slides: Vec<Value> = doc
        .slides
        .iter()
        .enumerate()
        .map(|(i, slide)| {
            // Resolve inherited boxes in the context of *this* slide, not the current one.
            let sel = Selection { slide: i, ..Selection::default() };
            let shapes: Vec<Value> = slide.shapes.iter().enumerate().map(|(z, sh)| shape_node(doc, &sel, sh, z)).collect();
            json!({
                "index": i,
                "id": slide.id.0,
                "name": slide.name,
                "hidden": slide.hidden,
                "current": i == st.selection.slide,
                "selected": st.selection.slides.contains(&slide.id),
                "shapes": shapes,
            })
        })
        .collect();
    json!({
        "schema": SCHEMA,
        "revision": st.revision,
        "slideSize": doc.slide_size,
        "currentSlide": st.selection.slide,
        "selectedShapes": st.selection.shapes.iter().map(|s| s.0).collect::<Vec<_>>(),
        "slides": slides,
    })
}

fn shape_node(doc: &Presentation, sel: &Selection, sh: &Shape, z: usize) -> Value {
    let x = xfrm_of(doc, sel, sh);
    let mut node = json!({
        "id": sh.id.0,
        "name": sh.name,
        "kind": kind_name(sh),
        "z": z,
        "x": x.x,
        "y": x.y,
        "w": x.w,
        "h": x.h,
        "rotation": x.rot,
        "flipH": x.flip_h,
        "flipV": x.flip_v,
        "hidden": sh.hidden,
        "locked": sh.locked,
        "placeholder": sh.ph.is_some(),
    });
    if let Some(m) = node.as_object_mut() {
        if let Geom::Preset { name, .. } = &sh.geom {
            m.insert("preset".into(), Value::String(name.clone()));
        }
        if let Some(t) = &sh.text {
            let text = t.text();
            if !text.is_empty() {
                m.insert("text".into(), Value::String(text));
            }
        }
        if let ShapeKind::Group { children, .. } = &sh.kind {
            m.insert("childCount".into(), json!(children.len()));
        }
    }
    node
}

fn kind_name(sh: &Shape) -> &'static str {
    match &sh.kind {
        ShapeKind::Shape if sh.text_box => "textBox",
        ShapeKind::Shape => "shape",
        ShapeKind::Picture { .. } => "picture",
        ShapeKind::Group { .. } => "group",
        ShapeKind::Connector { .. } => "connector",
        ShapeKind::Table(_) => "table",
        ShapeKind::Chart(_) => "chart",
        ShapeKind::Media(_) => "media",
        ShapeKind::Ink { .. } => "ink",
        _ => "other",
    }
}
