//! Pen tool and vector point editing on the canvas. A session edits an [`EditPath`] in
//! document coordinates and writes it back after every change; the whole session is one undo step.

use ude_core::{NodeId, NodeKind};
use ude_editor::path_edit::{self, AnchorRef, EditPath, Side, SubPath};
use ude_editor::{Tool, create::artboard_at};
use ude_renderer::vello::kurbo::{Affine, BezPath, Point, Vec2};
use winit::event::KeyEvent;
use winit::keyboard::{Key, NamedKey};

use crate::App;

/// Pointer tolerance for points and handles, in device pixels.
const GRAB_PX: f64 = 7.0;

pub(crate) struct PathEdit {
    /// The layer; `None` until the pen has placed two points.
    id: Option<NodeId>,
    parent: NodeId,
    path: EditPath,
    key: u64,
    /// Drawing with the pen (adding points) rather than editing existing ones.
    pen: bool,
    selected: Option<AnchorRef>,
    drag: Option<PathDrag>,
}

#[derive(Clone, Copy)]
enum PathDrag {
    /// Pen: pulling the handles out of the point just placed.
    NewHandles(AnchorRef),
    /// A point, with the pointer's offset from it at the press (so it does not jump).
    Anchor(AnchorRef, Vec2),
    Handle(AnchorRef, Side),
}

/// What the overlay shows for a path session (device space).
#[derive(Default)]
pub(crate) struct PathOverlay {
    pub path: Option<BezPath>,
    pub anchors: Vec<(Point, bool)>,
    pub handles: Vec<(Point, Point)>,
    /// Pen preview from the last point to the pointer.
    pub rubber: Option<BezPath>,
    /// The pointer is over the first point: a click closes the path.
    pub closing: Option<Point>,
}

impl App {
    fn grab_tol(&self) -> f64 {
        GRAB_PX / (self.camera.zoom * self.scale_factor())
    }

    pub(crate) fn path_editing(&self) -> bool {
        self.path_edit.is_some()
    }

    /// Starts point editing on a vector layer.
    pub(crate) fn begin_path_edit(&mut self, id: NodeId) -> bool {
        let Some(path) = EditPath::of_node(&self.doc, id) else {
            return false;
        };
        let Some(parent) = self.doc.node(id).and_then(|n| n.parent) else {
            return false;
        };
        let key = self.next_merge_key();
        self.selection.set([id]);
        self.path_edit = Some(PathEdit {
            id: Some(id),
            parent,
            path,
            key,
            pen: false,
            selected: None,
            drag: None,
        });
        self.redraw();
        true
    }

    pub(crate) fn end_path_edit(&mut self) {
        let Some(e) = self.path_edit.take() else {
            return;
        };
        if e.pen {
            self.tool = Tool::Move;
        }
        match e.id {
            Some(id) if self.doc.contains(id) => self.selection.set([id]),
            _ => self.selection.clear(),
        }
        self.redraw();
    }

    fn write_path(&mut self) {
        let Some(e) = &mut self.path_edit else { return };
        if e.path.anchor_count() < 2 {
            return;
        }
        let r = path_edit::commit(
            &mut self.doc,
            &mut self.history,
            &mut e.id,
            e.parent,
            &e.path,
            e.key,
        );
        if let Err(err) = r {
            eprintln!("path edit failed: {err}");
            self.path_edit = None;
        } else if let Some(id) = e.id {
            self.selection.set([id]);
        }
        self.redraw();
    }

    /// Mouse press with the pen tool or during point editing. Returns true when used.
    pub(crate) fn path_press(&mut self, double: bool) -> bool {
        let p = self.doc_point(self.cursor);
        let tol = self.grab_tol();
        if self.tool == Tool::Pen && self.path_edit.as_ref().is_none_or(|e| !e.pen) {
            self.end_path_edit();
            let parent = self
                .index
                .as_ref()
                .and_then(|i| artboard_at(&self.doc, i, p))
                .or(self.page);
            let Some(parent) = parent else { return false };
            let key = self.next_merge_key();
            self.path_edit = Some(PathEdit {
                id: None,
                parent,
                path: EditPath {
                    subs: vec![SubPath::default()],
                },
                key,
                pen: true,
                selected: None,
                drag: None,
            });
        }
        let Some(e) = &mut self.path_edit else {
            return false;
        };
        let grab = |path: &EditPath, r: AnchorRef| path.anchor(r).map_or(Vec2::ZERO, |a| a.p - p);
        if e.pen {
            let sub = e.path.subs.len() - 1;
            let n = e.path.subs[sub].anchors.len();
            // Clicking the first point closes the shape and finishes.
            if n >= 2 && e.path.hit_anchor(p, tol * 1.5) == Some((sub, 0)) {
                e.path.subs[sub].closed = true;
                e.drag = Some(PathDrag::NewHandles((sub, 0)));
                self.write_path();
                return true;
            }
            e.path.subs[sub].anchors.push(path_edit::Anchor::corner(p));
            e.selected = Some((sub, n));
            e.drag = Some(PathDrag::NewHandles((sub, n)));
            self.write_path();
            return true;
        }
        // Point editing: handles first (they sit on top), then points, then segments.
        if let Some((r, side)) = e.path.hit_handle(p, tol) {
            e.drag = Some(PathDrag::Handle(r, side));
            return true;
        }
        if let Some(r) = e.path.hit_anchor(p, tol) {
            e.selected = Some(r);
            if double {
                e.path.toggle_smooth(r);
                self.write_path();
            } else {
                e.drag = Some(PathDrag::Anchor(r, grab(&e.path, r)));
            }
            self.redraw();
            return true;
        }
        if let Some((s, seg, t)) = e.path.hit_segment(p, tol) {
            let r = e.path.insert(s, seg, t);
            e.selected = Some(r);
            e.drag = Some(PathDrag::Anchor(r, grab(&e.path, r)));
            self.write_path();
            return true;
        }
        // Pressing elsewhere leaves point editing; the press then selects as usual.
        self.end_path_edit();
        false
    }

    /// Pointer moved: drags a point or handle. Returns true when used.
    pub(crate) fn path_drag(&mut self) -> bool {
        let p = self.doc_point(self.cursor);
        let independent = self.modifiers.alt_key();
        let tol = self.grab_tol();
        let Some(e) = &mut self.path_edit else {
            return false;
        };
        match e.drag {
            Some(PathDrag::NewHandles(r)) => {
                // Dragging from the new point pulls out mirrored handles (a smooth point).
                let Some(a) = e.path.anchor(r) else {
                    return true;
                };
                if a.p.distance(p) > tol * 0.5 || !a.is_sharp() {
                    let at = a.p;
                    if let Some(a) = e.path.anchor_mut(r) {
                        a.h_out = p;
                        a.h_in = at - (p - at);
                    }
                    self.write_path();
                }
            }
            Some(PathDrag::Anchor(r, offset)) => {
                e.path.move_anchor(r, p + offset);
                self.write_path();
            }
            Some(PathDrag::Handle(r, side)) => {
                e.path.set_handle(r, side, p, independent);
                self.write_path();
            }
            None => {
                // Pen preview follows the pointer.
                let pen = e.pen;
                if pen {
                    self.redraw();
                }
                return pen;
            }
        }
        true
    }

    pub(crate) fn path_release(&mut self) {
        let Some(e) = &mut self.path_edit else { return };
        e.drag = None;
        let closed = e.pen && e.path.subs.last().is_some_and(|s| s.closed);
        if closed {
            self.end_path_edit();
        }
    }

    /// Keys during a session; true when handled.
    pub(crate) fn path_key(&mut self, event: &KeyEvent) -> bool {
        if event.state != winit::event::ElementState::Pressed {
            return false;
        }
        let ctrl = self.modifiers.control_key() || self.modifiers.super_key();
        match &event.logical_key {
            Key::Named(NamedKey::Escape | NamedKey::Enter) => self.end_path_edit(),
            Key::Named(NamedKey::Backspace | NamedKey::Delete) => {
                let Some(e) = &mut self.path_edit else {
                    return false;
                };
                if e.pen {
                    // Removes the last point placed.
                    if let Some(sub) = e.path.subs.last_mut() {
                        sub.anchors.pop();
                    }
                    e.selected = None;
                } else if let Some(r) = e.selected.take() {
                    e.path.remove(r);
                } else {
                    return true;
                }
                if e.path.anchor_count() >= 2 {
                    self.write_path();
                } else {
                    self.redraw();
                }
            }
            Key::Character(c) if ctrl && matches!(c.as_str(), "z" | "Z" | "y" | "Y") => {
                // Undo/redo apply to the document; leave the session first.
                self.end_path_edit();
                return false;
            }
            _ => return false,
        }
        true
    }

    pub(crate) fn path_overlay(&self, view: Affine) -> PathOverlay {
        let Some(e) = &self.path_edit else {
            return PathOverlay::default();
        };
        let mut o = PathOverlay {
            path: Some(e.path.bez(view)),
            ..Default::default()
        };
        for (s, sub) in e.path.subs.iter().enumerate() {
            for (i, a) in sub.anchors.iter().enumerate() {
                let selected = e.selected == Some((s, i));
                o.anchors.push((view * a.p, selected));
                // Handles of the selected point and, while drawing, of the last one.
                let show = !e.pen || selected || e.drag.is_some();
                if show && (selected || !e.pen) {
                    for h in [a.h_in, a.h_out] {
                        if h != a.p {
                            o.handles.push((view * a.p, view * h));
                        }
                    }
                }
            }
        }
        if e.pen && e.drag.is_none() {
            let p = self.doc_point(self.cursor);
            if let Some(sub) = e.path.subs.last()
                && let Some(last) = sub.anchors.last()
                && !sub.closed
            {
                let first = sub.anchors[0].p;
                let closing = sub.anchors.len() >= 2 && first.distance(p) <= self.grab_tol() * 1.5;
                let end = if closing { first } else { p };
                let mut b = BezPath::new();
                b.move_to(view * last.p);
                b.curve_to(view * last.h_out, view * end, view * end);
                o.rubber = Some(b);
                o.closing = closing.then(|| view * first);
            }
        }
        o
    }

    /// Pencil: starts a stroke at the pointer.
    pub(crate) fn pencil_press(&mut self) {
        let p = self.doc_point(self.cursor);
        self.pencil = Some(vec![p]);
        self.redraw();
    }

    /// Pencil: adds the pointer to the stroke (skipping sub-pixel moves).
    pub(crate) fn pencil_drag(&mut self) -> bool {
        let p = self.doc_point(self.cursor);
        let min = 0.75 / (self.camera.zoom * self.scale_factor());
        let Some(stroke) = &mut self.pencil else {
            return false;
        };
        if stroke.last().is_none_or(|l| l.distance(p) >= min) {
            stroke.push(p);
            self.redraw();
        }
        true
    }

    /// Pencil: smooths the stroke into a vector layer (one undo step).
    pub(crate) fn pencil_release(&mut self) {
        let Some(stroke) = self.pencil.take() else {
            return;
        };
        if stroke.len() < 2 {
            return self.redraw();
        }
        let tol = 1.5 / (self.camera.zoom * self.scale_factor());
        let path = EditPath {
            subs: vec![path_edit::freehand(&stroke, tol)],
        };
        let parent = self
            .index
            .as_ref()
            .and_then(|i| artboard_at(&self.doc, i, stroke[0]))
            .or(self.page);
        let Some(parent) = parent else { return };
        let key = self.next_merge_key();
        let mut id = None;
        match path_edit::commit(
            &mut self.doc,
            &mut self.history,
            &mut id,
            parent,
            &path,
            key,
        ) {
            Ok(()) => {
                if let Some(id) = id {
                    // Named after the tool that drew it, in the same undo step.
                    let name = self.doc.strings.intern("Pencil");
                    let mut tx = ude_core::Transaction::new("Pencil").with_merge_key(key);
                    let set = ude_core::Op::Set {
                        id,
                        value: ude_core::Prop::Name(name),
                    };
                    if tx.apply(&mut self.doc, set).is_ok() {
                        self.history.commit(&mut self.doc, tx);
                    }
                    self.selection.set([id]);
                }
            }
            Err(e) => eprintln!("pencil failed: {e}"),
        }
        self.redraw();
    }

    /// The stroke being drawn, in device space (for the overlay).
    pub(crate) fn pencil_overlay(&self, view: Affine) -> Option<BezPath> {
        let stroke = self.pencil.as_ref()?;
        let mut b = BezPath::new();
        b.move_to(view * stroke[0]);
        for &p in &stroke[1..] {
            b.line_to(view * p);
        }
        Some(b)
    }

    /// Status bar hint for the current tool or session.
    pub(crate) fn hint(&self) -> Option<&'static str> {
        if let Some(e) = &self.path_edit {
            return Some(if e.pen {
                "Click to add a point · drag to curve · click the first point to close · Backspace removes the last point · Enter finishes"
            } else {
                "Drag points and handles (Alt: break handles) · click a segment to add a point · double-click a point: smooth/sharp · Delete removes · Enter finishes"
            });
        }
        if self.editing.is_some() {
            return Some("Typing · Ctrl+A selects all · Esc finishes");
        }
        match self.tool {
            Tool::Move => match self.selection.ids() {
                [id] if matches!(
                    self.doc.node(*id).map(|n| &n.kind),
                    Some(NodeKind::Vector(_))
                ) =>
                {
                    Some("Enter or double-click to edit points")
                }
                [id] if matches!(self.doc.node(*id).map(|n| &n.kind), Some(NodeKind::Text(_))) => {
                    Some("Enter or double-click to edit text")
                }
                [_, _, ..] => Some("Ctrl+G groups · Ctrl+Alt+U/S/I/X combines shapes"),
                _ => None,
            },
            Tool::Pen => Some("Click to start a path · drag to curve"),
            Tool::Pencil => Some("Drag to draw freehand · the stroke is smoothed when you let go"),
            Tool::Text => Some("Click to add text"),
            _ => Some(
                "Drag to draw · Shift keeps proportions · Alt draws from the centre · click for 100×100",
            ),
        }
    }
}
