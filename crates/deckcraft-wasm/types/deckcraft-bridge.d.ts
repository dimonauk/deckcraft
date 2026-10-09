/**
 * TypeScript types for the deckcraft-bridge JSON contract.
 * Source of truth: crates/deckcraft-bridge (Rust). Drafted from
 * HANDOFF_2026-10-09_deckcraft-bridge.md; verify against the Rust types
 * before relying on any field marked "unverified".
 */

/** Schema id carried by every spatial render tree (SPATIAL_SCHEMA). */
export type SpatialSchemaId = "holocore.spatial/deckcraft/1";

// ---------------------------------------------------------------------------
// Actions in (dispatch_action input). Internally tagged by `type`, camelCase.
// ---------------------------------------------------------------------------

export interface InsertShapeAction {
  type: "InsertShape";
  preset: string; // e.g. "rect", "ellipse"
  /** [x, y, w, h] in slide points, origin top-left, y down. */
  rect: [number, number, number, number];
  text?: string;
}

/** All fields optional, but at least one must be present. */
export interface UpdateTransformAction {
  type: "UpdateTransform";
  id: number;
  x?: number;
  y?: number;
  w?: number;
  h?: number;
  rotation?: number;
}

export interface ReorderSlideAction {
  type: "ReorderSlide";
  from: number;
  to: number;
}

export interface UndoAction {
  type: "Undo";
}

export interface RedoAction {
  type: "Redo";
}

export type Action =
  | InsertShapeAction
  | UpdateTransformAction
  | ReorderSlideAction
  | UndoAction
  | RedoAction;

// ---------------------------------------------------------------------------
// Replies out (dispatch_action output). Never thrown; errors are JSON.
// Key order is alphabetical on the wire; do not rely on `ok` being first.
// ---------------------------------------------------------------------------

export interface DispatchOk {
  ok: true;
  /** Command result payload; shape varies per action (unverified). */
  result: unknown;
  revision: number;
  canUndo: boolean;
  canRedo: boolean;
  slideCount: number;
}

export interface DispatchErr {
  ok: false;
  error: string;
}

export type DispatchReply = DispatchOk | DispatchErr;

// ---------------------------------------------------------------------------
// Spatial render tree (get_spatial_render_tree output), v1.
// Units: slide points, origin top-left, y down.
// v1 limits: top-level shapes only (groups not expanded), no fills/strokes,
// no picture pixels.
// ---------------------------------------------------------------------------

export interface SpatialShape {
  id: number;
  name: string;
  kind: string;
  z: number;
  x: number;
  y: number;
  w: number;
  h: number;
  rotation: number;
  flipH: boolean;
  flipV: boolean;
  hidden: boolean;
  locked: boolean;
  placeholder: boolean;
  preset?: string;
  text?: string;
  childCount?: number;
}

export interface SpatialSlide {
  index: number;
  id: number;
  name: string;
  hidden: boolean;
  current: boolean;
  selected: boolean;
  shapes: SpatialShape[];
}

export interface SpatialRenderTree {
  schema: SpatialSchemaId;
  revision: number;
  /** Shape unverified: likely { w, h } or [w, h]; confirm in src/spatial.rs. */
  slideSize: unknown;
  currentSlide: number;
  selectedShapes: number[];
  slides: SpatialSlide[];
}

// ---------------------------------------------------------------------------
// wasm-bindgen surface (deckcraft-wasm). All string-in / string-out JSON.
// ---------------------------------------------------------------------------

export interface PresentationHandle {
  free(): void;
}

export declare function create_presentation(): PresentationHandle;
export declare function dispatch_action(
  handle: PresentationHandle,
  actionJson: string
): string; // JSON-encoded DispatchReply
export declare function get_spatial_render_tree(
  handle: PresentationHandle
): string; // JSON-encoded SpatialRenderTree
export declare function get_document_ast(handle: PresentationHandle): string;
