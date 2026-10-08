//! Drawing tools: drag on the canvas to create a frame, shape, line or text box.
//! Creation and sizing share a merge key, so one drag is one undo step.

use ude_core::scene::NodeFlags;
use ude_core::styles::{Color, Paint, Stroke};
use ude_core::typography::{
    LetterSpacing, LineHeight, TextCase, TextData, TextDecoration, TextSizing, TextStyle,
};
use ude_core::{
    DocError, Document, History, Node, NodeId, NodeKind, Op, Paints, Prop, Size, Transaction,
};
use ude_spatial::kurbo::{Affine, Point, Rect, Vec2};
use ude_spatial::{SpatialIndex, contains_point, from_kurbo, to_kurbo};

use crate::transform::Modifiers;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Tool {
    #[default]
    Move,
    Frame,
    Rectangle,
    Ellipse,
    Polygon,
    Star,
    Line,
    Text,
}

impl Tool {
    pub const ALL: [Tool; 8] = [
        Tool::Move,
        Tool::Frame,
        Tool::Rectangle,
        Tool::Ellipse,
        Tool::Polygon,
        Tool::Star,
        Tool::Line,
        Tool::Text,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Tool::Move => "Move",
            Tool::Frame => "Frame",
            Tool::Rectangle => "Rectangle",
            Tool::Ellipse => "Ellipse",
            Tool::Polygon => "Polygon",
            Tool::Star => "Star",
            Tool::Line => "Line",
            Tool::Text => "Text",
        }
    }

    /// Single-key shortcut, as in mainstream design tools.
    pub fn shortcut(self) -> char {
        match self {
            Tool::Move => 'V',
            Tool::Frame => 'F',
            Tool::Rectangle => 'R',
            Tool::Ellipse => 'O',
            Tool::Polygon => 'P',
            Tool::Star => 'S',
            Tool::Line => 'L',
            Tool::Text => 'T',
        }
    }
}

/// Size given to a shape created by a click without a drag.
const CLICK_SIZE: f64 = 100.0;
/// Below this drag distance (document units) a press counts as a click.
const CLICK_SLOP: f64 = 2.0;

pub struct Create {
    pub id: NodeId,
    tool: Tool,
    start: Point,
    parent_world: Affine,
    merge_key: u64,
}

impl Create {
    /// Creates the node at `start` (document units) inside the artboard under it, or the page.
    pub fn begin(
        doc: &mut Document,
        history: &mut History,
        index: &SpatialIndex,
        tool: Tool,
        start: Point,
        merge_key: u64,
    ) -> Result<Self, DocError> {
        let parent = artboard_at(doc, index, start).unwrap_or(index.page());
        let parent_world = to_kurbo(&doc.world_transform(parent));
        let mut node = default_node(doc, tool);
        node.transform = from_kurbo(parent_world.inverse() * Affine::translate(start.to_vec2()));
        let id = doc.create(node);
        let index_in_parent = doc.children(parent).len() as u32;
        let mut tx = Transaction::new(tool.label()).with_merge_key(merge_key);
        tx.apply(
            doc,
            Op::Attach {
                id,
                parent,
                index: index_in_parent,
            },
        )?;
        history.commit(doc, tx);
        Ok(Self {
            id,
            tool,
            start,
            parent_world,
            merge_key,
        })
    }

    /// Resizes the new node to the drag from `start` to `pointer`.
    pub fn update(
        &self,
        doc: &mut Document,
        history: &mut History,
        pointer: Point,
        mods: Modifiers,
    ) -> Result<(), DocError> {
        let (world, size) = if self.tool == Tool::Line {
            let mut d = pointer - self.start;
            if mods.shift {
                // 45° steps.
                let step = std::f64::consts::FRAC_PI_4;
                let a = (d.atan2() / step).round() * step;
                d = Vec2::from_angle(a) * d.hypot();
            }
            (
                Affine::translate(self.start.to_vec2()) * Affine::rotate(d.atan2()),
                (d.hypot(), 0.0),
            )
        } else {
            let r = drag_rect(self.start, pointer, mods);
            (Affine::translate((r.x0, r.y0)), (r.width(), r.height()))
        };
        self.set(doc, history, world, size)
    }

    /// Ends the gesture. A click without a drag gives the shape a default size.
    pub fn finish(
        &self,
        doc: &mut Document,
        history: &mut History,
        pointer: Point,
    ) -> Result<(), DocError> {
        let dragged = (pointer - self.start).hypot() > CLICK_SLOP;
        if dragged || self.tool == Tool::Text {
            return Ok(());
        }
        let size = if self.tool == Tool::Line {
            (CLICK_SIZE, 0.0)
        } else {
            (CLICK_SIZE, CLICK_SIZE)
        };
        self.set(doc, history, Affine::translate(self.start.to_vec2()), size)
    }

    fn set(
        &self,
        doc: &mut Document,
        history: &mut History,
        world: Affine,
        size: (f64, f64),
    ) -> Result<(), DocError> {
        let mut tx = Transaction::new(self.tool.label()).with_merge_key(self.merge_key);
        let local = from_kurbo(self.parent_world.inverse() * world);
        tx.apply(
            doc,
            Op::Set {
                id: self.id,
                value: Prop::Transform(local),
            },
        )?;
        tx.apply(
            doc,
            Op::Set {
                id: self.id,
                value: Prop::Size(Size::new(size.0 as f32, size.1 as f32)),
            },
        )?;
        history.commit(doc, tx);
        Ok(())
    }
}

/// Rectangle from a drag: Shift = square, Alt = centred on the start point.
fn drag_rect(start: Point, p: Point, mods: Modifiers) -> Rect {
    let mut d = p - start;
    if mods.shift {
        let m = d.x.abs().max(d.y.abs());
        d = Vec2::new(m.copysign(d.x), m.copysign(d.y));
    }
    if mods.alt {
        Rect::from_points(start - d, start + d)
    } else {
        Rect::from_points(start, start + d)
    }
}

/// Topmost artboard containing `p`: new layers go inside it.
fn artboard_at(doc: &Document, index: &SpatialIndex, p: Point) -> Option<NodeId> {
    index
        .query_point(p.x as f32, p.y as f32, 0.0)
        .filter(|&id| doc.is_artboard(id) && contains_point(doc, id, p.x, p.y, 0.0))
        .max_by(|&a, &b| ude_spatial::paint_order_cmp(doc, a, b))
}

fn default_node(doc: &mut Document, tool: Tool) -> Node {
    let name = doc.strings.intern(tool.label());
    // Fills follow the mainstream defaults: light grey shapes, white frames, black text and lines.
    let grey = Paints::new(vec![Paint::solid(Color::from_rgba8(
        0xD9, 0xD9, 0xD9, 0xFF,
    ))]);
    let black = Paints::new(vec![Paint::solid(Color::BLACK)]);
    match tool {
        Tool::Frame => {
            let mut n = Node::new(NodeKind::Frame, name)
                .with_fills(Paints::new(vec![Paint::solid(Color::WHITE)]));
            n.flags.set(NodeFlags::CLIPS_CONTENT, true);
            n
        }
        Tool::Move | Tool::Rectangle => Node::new(NodeKind::Rectangle, name).with_fills(grey),
        Tool::Ellipse => Node::new(NodeKind::Ellipse(None), name).with_fills(grey),
        Tool::Polygon => Node::new(NodeKind::Polygon { sides: 3 }, name).with_fills(grey),
        Tool::Star => Node::new(
            NodeKind::Star {
                points: 5,
                inner_ratio: 0.38,
            },
            name,
        )
        .with_fills(grey),
        Tool::Line => {
            let mut n = Node::new(NodeKind::Line, name);
            n.ext_mut().strokes = Some(Box::new(Stroke {
                paints: black,
                width: 1.0,
                miter_limit: 4.0,
                ..Default::default()
            }));
            n
        }
        Tool::Text => {
            let family = doc.strings.intern("Inter");
            let style = TextStyle {
                family,
                weight: 400,
                italic: false,
                size: 16.0,
                line_height: LineHeight::Auto,
                letter_spacing: LetterSpacing::Normal,
                decoration: TextDecoration::None,
                case: TextCase::Original,
                fills: black,
            };
            let mut text = TextData::plain("Text", style);
            text.sizing = TextSizing::AutoWidth;
            Node::new(NodeKind::Text(Box::new(text)), name).with_size(40.0, 20.0)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ude_core::Symbol;

    fn setup() -> (Document, SpatialIndex, NodeId) {
        let mut doc = Document::new();
        let root = doc.root();
        let page = doc
            .build_child(root, Node::new(NodeKind::Page, Symbol::EMPTY))
            .unwrap();
        let artboard = doc
            .build_child(
                page,
                Node::new(NodeKind::Frame, Symbol::EMPTY)
                    .with_transform(ude_core::Affine::translate(100.0, 100.0))
                    .with_size(400.0, 400.0),
            )
            .unwrap();
        let index = SpatialIndex::build(&doc, page);
        (doc, index, artboard)
    }

    #[test]
    fn drag_creates_inside_artboard_as_one_undo_step() {
        let (mut doc, index, artboard) = setup();
        let mut h = History::default();
        let c = Create::begin(
            &mut doc,
            &mut h,
            &index,
            Tool::Rectangle,
            Point::new(150.0, 150.0),
            7,
        )
        .unwrap();
        for i in 1..=20 {
            c.update(
                &mut doc,
                &mut h,
                Point::new(150.0 + f64::from(i) * 5.0, 150.0 + f64::from(i) * 2.0),
                Modifiers::default(),
            )
            .unwrap();
        }
        c.finish(&mut doc, &mut h, Point::new(250.0, 190.0))
            .unwrap();
        let n = doc.node(c.id).unwrap();
        assert_eq!(
            n.parent,
            Some(artboard),
            "created inside the artboard under the pointer"
        );
        assert_eq!(
            n.transform.translation(),
            ude_core::Point::new(50.0, 50.0),
            "local to the artboard"
        );
        assert_eq!((n.size.w, n.size.h), (100.0, 40.0));
        assert_eq!(h.len(), 1, "creation + 20 drag updates = one undo step");
        h.undo(&mut doc).unwrap();
        assert!(doc.children(artboard).is_empty());
    }

    #[test]
    fn modifiers_and_click() {
        let (mut doc, index, _) = setup();
        let mut h = History::default();
        let shift = Modifiers {
            shift: true,
            alt: false,
        };
        let c = Create::begin(
            &mut doc,
            &mut h,
            &index,
            Tool::Ellipse,
            Point::new(600.0, 0.0),
            1,
        )
        .unwrap();
        c.update(&mut doc, &mut h, Point::new(650.0, 20.0), shift)
            .unwrap();
        assert_eq!(
            doc.node(c.id).unwrap().size,
            Size::new(50.0, 50.0),
            "Shift = circle"
        );

        let c = Create::begin(
            &mut doc,
            &mut h,
            &index,
            Tool::Rectangle,
            Point::new(700.0, 0.0),
            2,
        )
        .unwrap();
        c.finish(&mut doc, &mut h, Point::new(700.5, 0.0)).unwrap();
        assert_eq!(
            doc.node(c.id).unwrap().size,
            Size::new(100.0, 100.0),
            "click = default size"
        );

        let c = Create::begin(
            &mut doc,
            &mut h,
            &index,
            Tool::Line,
            Point::new(0.0, 600.0),
            3,
        )
        .unwrap();
        c.update(&mut doc, &mut h, Point::new(100.0, 590.0), shift)
            .unwrap();
        let t = doc.node(c.id).unwrap().transform.0;
        assert!(
            t[1].abs() < 1e-6,
            "Shift snaps a near-horizontal line to 0°"
        );
    }
}
