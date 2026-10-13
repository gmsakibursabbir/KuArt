//! Design → code (Dev Mode, `docs/dev-mode.md`). Builds a small code IR from any node of the
//! Universal Design Model, then emits HTML, CSS, Tailwind, React, Vue or design tokens.
//!
//! Layout: auto layout becomes flexbox; children stacked on one axis with an even gap are
//! inferred as flexbox too; anything else falls back to absolute positioning (reported in
//! [`Output::notes`]). Output is deterministic for the same input.

mod emit;
mod tailwind;

use std::collections::HashMap;

use ude_core::layout::{ChildSizing, CrossAlign, LayoutDirection, MainAlign};
use ude_core::styles::{Color, Effect, GradientKind, Paint, PaintKind, StrokeAlign};
use ude_core::typography::{
    LetterSpacing, LineHeight, TextAlignH, TextCase, TextDecoration, TextSizing, TextStyle,
};
use ude_core::{Document, Node, NodeFlags, NodeId, NodeKind};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    /// Standalone HTML page with an embedded stylesheet.
    Html,
    Css,
    /// HTML markup with Tailwind utility classes.
    Tailwind,
    /// React function component (JSX) plus its CSS file.
    React,
    /// Vue single-file component with scoped CSS.
    Vue,
    /// Design tokens as CSS custom properties and JSON.
    Tokens,
}

impl Target {
    pub const ALL: [Target; 6] = [
        Target::Html,
        Target::Css,
        Target::Tailwind,
        Target::React,
        Target::Vue,
        Target::Tokens,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Target::Html => "html",
            Target::Css => "css",
            Target::Tailwind => "tailwind",
            Target::React => "react",
            Target::Vue => "vue",
            Target::Tokens => "tokens",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|t| t.name().eq_ignore_ascii_case(s))
    }
}

pub struct Output {
    pub code: String,
    /// Things a developer should know (absolute positioning, unsupported effects, …).
    pub notes: Vec<String>,
}

/// Generates code for `id` (a page, frame, group or any layer).
pub fn generate(doc: &Document, id: NodeId, target: Target) -> Result<Output, String> {
    let node = doc.node(id).ok_or("node not found")?;
    let mut b = Builder {
        doc,
        classes: HashMap::new(),
        notes: Vec::new(),
        absolute: 0,
    };
    let root = if matches!(node.kind, NodeKind::Page | NodeKind::Document) {
        // A page has no box of its own: wrap its top-level layers.
        let children: Vec<El> = node
            .children
            .iter()
            .filter_map(|&c| b.element(c, Parent::Flow))
            .collect();
        let mut el = El::new("main", b.class("page"));
        el.decls.push(("display", "flex".into()));
        el.decls.push(("flex-direction", "column".into()));
        el.decls.push(("gap", "40px".into()));
        el.children = children;
        el
    } else {
        b.element(id, Parent::Root).ok_or("the layer is hidden")?
    };
    if b.absolute > 0 {
        b.notes.push(format!(
            "{} element(s) positioned absolutely: overlapping or irregularly placed layers",
            b.absolute
        ));
    }
    let component = pascal(doc.strings.resolve(node.name));
    let code = match target {
        Target::Html => emit::html(&root),
        Target::Css => emit::css(&root),
        Target::Tailwind => emit::tailwind(&root),
        Target::React => emit::react(&root, &component),
        Target::Vue => emit::vue(&root),
        Target::Tokens => emit::tokens(&root),
    };
    // Each note once, in first-seen order.
    let mut notes: Vec<String> = Vec::new();
    for n in b.notes {
        if !notes.contains(&n) {
            notes.push(n);
        }
    }
    Ok(Output { code, notes })
}

/// One element of the code IR.
pub(crate) struct El {
    pub tag: &'static str,
    pub class: String,
    /// Ordered CSS declarations.
    pub decls: Vec<(&'static str, String)>,
    pub attrs: Vec<(&'static str, String)>,
    pub text: Option<String>,
    /// Inline markup (SVG for vector layers).
    pub raw: Option<String>,
    pub children: Vec<El>,
}

impl El {
    fn new(tag: &'static str, class: String) -> Self {
        Self {
            tag,
            class,
            decls: Vec::new(),
            attrs: Vec::new(),
            text: None,
            raw: None,
            children: Vec::new(),
        }
    }
}

/// How the parent lays out this element.
#[derive(Clone, Copy)]
enum Parent {
    Root,
    /// In flow (flexbox parent); `margin` offsets the cross axis start.
    Flow,
    FlowWith {
        direction: LayoutDirection,
        h: ChildSizing,
        v: ChildSizing,
    },
    Absolute,
}

struct Builder<'a> {
    doc: &'a Document,
    classes: HashMap<String, u32>,
    notes: Vec<String>,
    absolute: usize,
}

impl Builder<'_> {
    /// Unique class name.
    fn class(&mut self, base: &str) -> String {
        let n = self.classes.entry(base.to_owned()).or_insert(0);
        *n += 1;
        if *n == 1 {
            base.to_owned()
        } else {
            format!("{base}-{n}")
        }
    }

    fn element(&mut self, id: NodeId, parent: Parent) -> Option<El> {
        let doc = self.doc;
        let n = doc.node(id)?;
        if !n.flags.contains(NodeFlags::VISIBLE) {
            return None;
        }
        let name = doc.strings.resolve(n.name);
        let (tag, role) = self.semantics(n, name);
        let class = self.class(&class_name(name, role));
        let mut el = El::new(tag, class);

        // Box and position.
        let m = n.transform.0;
        let rotated = m[1].abs() > 1e-3 || m[2].abs() > 1e-3;
        match parent {
            Parent::Root => {}
            Parent::Absolute => {
                el.decls.push(("position", "absolute".into()));
                el.decls.push(("left", px(m[4])));
                el.decls.push(("top", px(m[5])));
            }
            Parent::Flow | Parent::FlowWith { .. } => {}
        }
        if rotated {
            let deg = m[1].atan2(m[0]).to_degrees();
            el.decls
                .push(("transform", format!("rotate({}deg)", num(deg))));
            el.decls.push(("transform-origin", "top left".into()));
        }
        let (fill_w, fill_h, hug_w, hug_h) = match parent {
            Parent::FlowWith { direction, h, v } => {
                let row = direction != LayoutDirection::Vertical;
                let (main, cross) = if row { (h, v) } else { (v, h) };
                if main == ChildSizing::Fill {
                    el.decls.push(("flex", "1 1 0".into()));
                }
                if cross == ChildSizing::Fill {
                    el.decls.push(("align-self", "stretch".into()));
                }
                (
                    h == ChildSizing::Fill,
                    v == ChildSizing::Fill,
                    h == ChildSizing::Hug,
                    v == ChildSizing::Hug,
                )
            }
            _ => (false, false, false, false),
        };
        let text_sizing = match &n.kind {
            NodeKind::Text(t) => Some(t.sizing),
            _ => None,
        };
        let auto_layout = n.ext.as_ref().and_then(|e| e.auto_layout.as_deref());
        let hug =
            |c: ude_core::layout::ContainerSizing| c == ude_core::layout::ContainerSizing::Hug;
        let (al_hug_w, al_hug_h) = auto_layout.map_or((false, false), |al| {
            let row = al.direction != LayoutDirection::Vertical;
            if row {
                (hug(al.primary_sizing), hug(al.counter_sizing))
            } else {
                (hug(al.counter_sizing), hug(al.primary_sizing))
            }
        });
        let skip_w = fill_w || hug_w || al_hug_w || text_sizing == Some(TextSizing::AutoWidth);
        let skip_h = fill_h
            || hug_h
            || al_hug_h
            || matches!(
                text_sizing,
                Some(TextSizing::AutoWidth | TextSizing::AutoHeight)
            );
        if !skip_w {
            el.decls.push(("width", px(n.size.w)));
        }
        if !skip_h && !matches!(n.kind, NodeKind::Line) {
            el.decls.push(("height", px(n.size.h)));
        }

        // Visual properties.
        match &n.kind {
            NodeKind::Text(t) => self.text(&mut el, t),
            NodeKind::Raster { .. } => {
                el.attrs.push(("src", format!("assets/{}.png", el.class)));
                el.attrs.push(("alt", name.to_owned()));
                el.decls.push(("object-fit", "cover".into()));
            }
            NodeKind::Vector(_)
            | NodeKind::Star { .. }
            | NodeKind::Polygon { .. }
            | NodeKind::BooleanOp(_) => {
                el.raw = ude_format_svg::export(doc, id)
                    .ok()
                    .map(|s| strip_xml_decl(&s));
                el.decls.push(("display", "block".into()));
            }
            NodeKind::Line => {
                if let Some(s) = n.ext.as_ref().and_then(|e| e.strokes.as_deref())
                    && let Some(c) = first_solid(s.paints.as_slice())
                {
                    el.decls.push((
                        "border-top",
                        format!("{}px solid {}", num(s.width), css_color(c)),
                    ));
                }
            }
            _ => self.box_style(&mut el, n),
        }
        if n.opacity < 0.999 {
            el.decls.push(("opacity", num(n.opacity)));
        }
        if n.flags.contains(NodeFlags::CLIPS_CONTENT) && !n.children.is_empty() {
            el.decls.push(("overflow", "hidden".into()));
        }

        // Children and their layout.
        if !n.children.is_empty() && el.raw.is_none() {
            self.children(&mut el, n);
        }
        Some(el)
    }

    /// Element tag and the role used for generic layer names.
    fn semantics(&self, n: &Node, name: &str) -> (&'static str, &'static str) {
        let lower = name.to_ascii_lowercase();
        match &n.kind {
            NodeKind::Text(t) => {
                let size = t.styles.iter().map(|s| s.size).fold(0.0, f32::max);
                let bold = t.styles.iter().any(|s| s.weight >= 600);
                match size {
                    s if s >= 32.0 => ("h1", "title"),
                    s if s >= 24.0 => ("h2", "heading"),
                    s if s >= 18.0 && bold => ("h3", "subheading"),
                    _ if n
                        .parent
                        .and_then(|p| self.doc.node(p))
                        .is_some_and(|p| is_button(self.doc, p)) =>
                    {
                        ("span", "label")
                    }
                    _ => ("p", "text"),
                }
            }
            NodeKind::Raster { .. } => ("img", "image"),
            NodeKind::Vector(_)
            | NodeKind::Star { .. }
            | NodeKind::Polygon { .. }
            | NodeKind::BooleanOp(_) => ("div", "icon"),
            NodeKind::Line => ("hr", "divider"),
            _ if is_button(self.doc, n) => {
                if lower.contains("link") {
                    ("a", "link")
                } else {
                    ("button", "button")
                }
            }
            _ => {
                for (word, tag) in [
                    ("header", "header"),
                    ("nav", "nav"),
                    ("footer", "footer"),
                    ("section", "section"),
                    ("aside", "aside"),
                    ("main", "main"),
                    ("card", "article"),
                ] {
                    if lower
                        .split(|c: char| !c.is_ascii_alphanumeric())
                        .any(|w| w == word)
                    {
                        return (
                            if tag == "article" { "div" } else { tag },
                            if word == "card" { "card" } else { tag },
                        );
                    }
                }
                (
                    "div",
                    if matches!(
                        n.kind,
                        NodeKind::Frame | NodeKind::Component(_) | NodeKind::Instance(_)
                    ) {
                        "container"
                    } else {
                        "box"
                    },
                )
            }
        }
    }

    fn box_style(&mut self, el: &mut El, n: &Node) {
        let paints: Vec<&Paint> = n.fills.as_slice().iter().filter(|p| p.visible).collect();
        if let [p] = paints.as_slice()
            && let PaintKind::Solid(c) = p.kind
        {
            el.decls
                .push(("background", css_color(with_alpha(c, p.opacity))));
        } else if !paints.is_empty() {
            // CSS lists the top layer first; the model lists bottom first.
            let layers: Vec<String> = paints
                .iter()
                .rev()
                .filter_map(|p| self.paint(p, n))
                .collect();
            if !layers.is_empty() {
                el.decls.push(("background", layers.join(", ")));
            }
        }
        let ext = n.ext.as_deref();
        if let Some(s) = ext.and_then(|e| e.strokes.as_deref())
            && let Some(c) = first_solid(s.paints.as_slice())
            && s.width > 0.0
        {
            let line = format!(
                "{}px {} {}",
                num(s.width),
                if s.dashes.is_empty() {
                    "solid"
                } else {
                    "dashed"
                },
                css_color(c)
            );
            match s.align {
                // Inside strokes keep the box size with box-sizing: border-box.
                StrokeAlign::Inside | StrokeAlign::Center => el.decls.push(("border", line)),
                StrokeAlign::Outside => el.decls.push(("outline", line)),
            }
        }
        if matches!(n.kind, NodeKind::Ellipse(_)) {
            el.decls.push(("border-radius", "50%".into()));
        } else if let Some(c) = ext.and_then(|e| e.corners.as_ref()) {
            let r = c.radii;
            if r.iter().any(|&v| v > 0.0) {
                el.decls.push((
                    "border-radius",
                    r.iter().map(|&v| px(v)).collect::<Vec<_>>().join(" "),
                ));
            }
        } else if n.corner_radius > 0.0 {
            el.decls.push(("border-radius", px(n.corner_radius)));
        }
        let mut shadows = Vec::new();
        for e in ext.map_or(&[][..], |e| &e.effects) {
            match e {
                Effect::DropShadow(s) | Effect::InnerShadow(s) if s.visible => {
                    shadows.push(format!(
                        "{}{} {} {} {} {}",
                        if matches!(e, Effect::InnerShadow(_)) {
                            "inset "
                        } else {
                            ""
                        },
                        px(s.offset.x),
                        px(s.offset.y),
                        px(s.blur),
                        px(s.spread),
                        css_color(s.color)
                    ))
                }
                Effect::LayerBlur {
                    radius,
                    visible: true,
                } => {
                    el.decls
                        .push(("filter", format!("blur({})", px(radius / 2.0))));
                }
                Effect::BackgroundBlur {
                    radius,
                    visible: true,
                } => {
                    el.decls
                        .push(("backdrop-filter", format!("blur({})", px(radius / 2.0))));
                }
                _ => {}
            }
        }
        if !shadows.is_empty() {
            el.decls.push(("box-shadow", shadows.join(", ")));
        }
    }

    /// One CSS background layer.
    fn paint(&mut self, p: &Paint, n: &Node) -> Option<String> {
        match &p.kind {
            PaintKind::Solid(c) => {
                let c = css_color(with_alpha(*c, p.opacity));
                Some(format!("linear-gradient({c}, {c})"))
            }
            PaintKind::Gradient(g) => {
                let (w, h) = (n.size.w.max(1e-3), n.size.h.max(1e-3));
                let t = g.transform.0;
                // Gradient space → box pixels (see renderer::convert::brush).
                let map = |x: f32, y: f32| {
                    (
                        (t[0] * x + t[2] * y + t[4]) * w,
                        (t[1] * x + t[3] * y + t[5]) * h,
                    )
                };
                let stops = |pos: &dyn Fn(f32) -> f32| {
                    g.stops
                        .iter()
                        .map(|s| {
                            format!(
                                "{} {}%",
                                css_color(with_alpha(s.color, p.opacity)),
                                num(pos(s.offset) * 100.0)
                            )
                        })
                        .collect::<Vec<_>>()
                        .join(", ")
                };
                Some(match g.kind {
                    GradientKind::Linear => {
                        let (p0, p1) = (map(0.0, 0.5), map(1.0, 0.5));
                        let (dx, dy) = (p1.0 - p0.0, p1.1 - p0.1);
                        let angle = dx.atan2(-dy);
                        // CSS gradient line: through the centre, length |w sin a| + |h cos a|.
                        let (ux, uy) = (angle.sin(), -angle.cos());
                        let len = (w * ux).abs() + (h * uy).abs();
                        let proj = |q: (f32, f32)| {
                            ((q.0 - w / 2.0) * ux + (q.1 - h / 2.0) * uy) / len + 0.5
                        };
                        let (a, b) = (proj(p0), proj(p1));
                        format!(
                            "linear-gradient({}deg, {})",
                            num(angle.to_degrees().rem_euclid(360.0)),
                            stops(&|s| a + s * (b - a))
                        )
                    }
                    GradientKind::Radial | GradientKind::Diamond => {
                        let c = map(0.5, 0.5);
                        let rx = (t[0] * w).hypot(t[1] * h) * 0.5;
                        let ry = (t[2] * w).hypot(t[3] * h) * 0.5;
                        format!(
                            "radial-gradient(ellipse {} {} at {} {}, {})",
                            px(rx),
                            px(ry),
                            px(c.0),
                            px(c.1),
                            stops(&|s| s)
                        )
                    }
                    GradientKind::Angular => {
                        let c = map(0.5, 0.5);
                        let from = t[1].atan2(t[0]).to_degrees() + 90.0;
                        format!(
                            "conic-gradient(from {}deg at {} {}, {})",
                            num(from),
                            px(c.0),
                            px(c.1),
                            stops(&|s| s)
                        )
                    }
                })
            }
            PaintKind::Image { .. } => {
                self.notes
                    .push("Image fills reference assets/*.png placeholders".into());
                Some("url(assets/image.png) center / cover no-repeat".into())
            }
        }
    }

    fn text(&mut self, el: &mut El, t: &ude_core::typography::TextData) {
        let strings = &self.doc.strings;
        let Some(base) = t.styles.first() else { return };
        text_decls(&mut el.decls, base, strings);
        match t.align_h {
            TextAlignH::Left => {}
            TextAlignH::Center => el.decls.push(("text-align", "center".into())),
            TextAlignH::Right => el.decls.push(("text-align", "right".into())),
            TextAlignH::Justify => el.decls.push(("text-align", "justify".into())),
        }
        if t.sizing == TextSizing::AutoWidth {
            el.decls.push(("white-space", "nowrap".into()));
        }
        if t.runs.iter().all(|r| r.style == 0) || t.runs.len() <= 1 {
            el.text = Some(t.content.to_string());
            return;
        }
        // Rich text: one span per run whose style differs from the first.
        for run in t.runs.iter() {
            let s = t
                .content
                .get(run.range.start as usize..run.range.end as usize)
                .unwrap_or("");
            let mut span = El::new("span", String::new());
            span.text = Some(s.to_owned());
            if let Some(style) = t.styles.get(run.style as usize)
                && run.style != 0
            {
                span.class = self.class(&format!("{}-span", el.class));
                let mut all = Vec::new();
                text_decls(&mut all, style, strings);
                let mut base_decls = Vec::new();
                text_decls(&mut base_decls, base, strings);
                span.decls = all
                    .into_iter()
                    .filter(|d| !base_decls.contains(d))
                    .collect();
            }
            el.children.push(span);
        }
    }

    fn children(&mut self, el: &mut El, n: &Node) {
        let doc = self.doc;
        let visible: Vec<NodeId> = n
            .children
            .iter()
            .copied()
            .filter(|&c| {
                doc.node(c)
                    .is_some_and(|c| c.flags.contains(NodeFlags::VISIBLE))
            })
            .collect();
        if let Some(al) = n.ext.as_ref().and_then(|e| e.auto_layout.as_deref()) {
            el.decls.push(("display", "flex".into()));
            match al.direction {
                LayoutDirection::Horizontal => {}
                LayoutDirection::Vertical => el.decls.push(("flex-direction", "column".into())),
                LayoutDirection::Wrap => el.decls.push(("flex-wrap", "wrap".into())),
            }
            if al.gap > 0.0 {
                if al.direction == LayoutDirection::Wrap && al.cross_gap != al.gap {
                    el.decls
                        .push(("gap", format!("{} {}", px(al.cross_gap), px(al.gap))));
                } else {
                    el.decls.push(("gap", px(al.gap)));
                }
            }
            match al.main_align {
                MainAlign::Start => {}
                MainAlign::Center => el.decls.push(("justify-content", "center".into())),
                MainAlign::End => el.decls.push(("justify-content", "flex-end".into())),
                MainAlign::SpaceBetween => {
                    el.decls.push(("justify-content", "space-between".into()))
                }
            }
            el.decls.push((
                "align-items",
                match al.cross_align {
                    CrossAlign::Start => "flex-start",
                    CrossAlign::Center => "center",
                    CrossAlign::End => "flex-end",
                    CrossAlign::Baseline => "baseline",
                    CrossAlign::Stretch => "stretch",
                }
                .into(),
            ));
            if let Some(p) = padding(al.padding) {
                el.decls.push(("padding", p));
            }
            let mut any_absolute = false;
            for c in visible {
                let lc = doc
                    .node(c)
                    .and_then(|c| c.ext.as_ref())
                    .map(|e| e.layout_child.clone())
                    .unwrap_or_default();
                let parent = if lc.absolute {
                    any_absolute = true;
                    self.absolute += 1;
                    Parent::Absolute
                } else {
                    Parent::FlowWith {
                        direction: al.direction,
                        h: lc.h_sizing,
                        v: lc.v_sizing,
                    }
                };
                el.children.extend(self.element(c, parent));
            }
            if any_absolute {
                el.decls.push(("position", "relative".into()));
            }
            return;
        }
        // A single child centred in its parent (button labels, icons in badges).
        if let [only] = visible.as_slice()
            && let Some(c) = doc.node(*only)
            && c.transform.0[1].abs() < 1e-3
            && c.transform.0[2].abs() < 1e-3
        {
            let m = c.transform.0;
            let (w, h) = match &c.kind {
                // Auto-width text has a measured size that may not be stored yet.
                NodeKind::Text(_) if c.size.w <= 0.0 => {
                    (n.size.w - 2.0 * m[4], n.size.h - 2.0 * m[5])
                }
                _ => (c.size.w, c.size.h),
            };
            let centred_x = ((m[4] + w / 2.0) - n.size.w / 2.0).abs() <= 1.0;
            let centred_y = ((m[5] + h / 2.0) - n.size.h / 2.0).abs() <= 1.0;
            if centred_x && centred_y {
                el.decls.push(("display", "flex".into()));
                el.decls.push(("justify-content", "center".into()));
                el.decls.push(("align-items", "center".into()));
                el.children.extend(self.element(*only, Parent::Flow));
                return;
            }
        }
        if let Some(stack) = infer_stack(doc, &visible) {
            el.decls.push(("display", "flex".into()));
            if stack.column {
                el.decls.push(("flex-direction", "column".into()));
            }
            if stack.gap > 0.5 {
                el.decls.push(("gap", px(stack.gap)));
            }
            el.decls.push(("align-items", "flex-start".into()));
            let (top, left) = if stack.column {
                (stack.start, stack.cross_start)
            } else {
                (stack.cross_start, stack.start)
            };
            if let Some(p) = padding([top, 0.0, 0.0, left]) {
                el.decls.push(("padding", p));
            }
            for (c, offset) in visible.iter().zip(stack.cross_offsets) {
                if let Some(mut child) = self.element(*c, Parent::Flow) {
                    if offset > 0.5 {
                        child.decls.push((
                            if stack.column {
                                "margin-left"
                            } else {
                                "margin-top"
                            },
                            px(offset),
                        ));
                    }
                    el.children.push(child);
                }
            }
            return;
        }
        el.decls.push(("position", "relative".into()));
        for c in visible {
            self.absolute += 1;
            el.children.extend(self.element(c, Parent::Absolute));
        }
    }
}

fn is_button(doc: &Document, n: &Node) -> bool {
    let name = doc.strings.resolve(n.name).to_ascii_lowercase();
    let named = ["button", "btn", "cta"].iter().any(|w| name.contains(w));
    named
        && matches!(
            n.kind,
            NodeKind::Frame | NodeKind::Group | NodeKind::Instance(_) | NodeKind::Component(_)
        )
        && n.children.iter().any(|&c| {
            doc.node(c)
                .is_some_and(|c| matches!(c.kind, NodeKind::Text(_)))
        })
}

/// Children laid out on one axis without overlap and with an even gap.
struct Stack {
    column: bool,
    gap: f32,
    /// Main-axis offset of the first child.
    start: f32,
    /// Smallest cross-axis offset (becomes padding).
    cross_start: f32,
    /// Each child's cross offset beyond `cross_start` (becomes a margin).
    cross_offsets: Vec<f32>,
}

fn infer_stack(doc: &Document, ids: &[NodeId]) -> Option<Stack> {
    let boxes: Vec<(f32, f32, f32, f32)> = ids
        .iter()
        .map(|&id| {
            let n = doc.node(id)?;
            let m = n.transform.0;
            // Only translated boxes can flow.
            if m[1].abs() > 1e-3 || m[2].abs() > 1e-3 || m[0] < 0.0 || m[3] < 0.0 {
                return None;
            }
            Some((m[4], m[5], n.size.w, n.size.h))
        })
        .collect::<Option<_>>()?;
    if boxes.is_empty() {
        return None;
    }
    for column in [true, false] {
        // Children must already be in paint order along the axis.
        let main = |b: &(f32, f32, f32, f32)| if column { (b.1, b.3) } else { (b.0, b.2) };
        let cross = |b: &(f32, f32, f32, f32)| if column { b.0 } else { b.1 };
        let gaps: Vec<f32> = boxes
            .windows(2)
            .map(|w| main(&w[1]).0 - (main(&w[0]).0 + main(&w[0]).1))
            .collect();
        let (lo, hi) = gaps
            .iter()
            .fold((f32::MAX, f32::MIN), |(lo, hi), &g| (lo.min(g), hi.max(g)));
        if gaps.is_empty() || (lo >= -0.5 && hi - lo <= 1.0) {
            let cross_start = boxes.iter().map(cross).fold(f32::MAX, f32::min).max(0.0);
            return Some(Stack {
                column,
                gap: if gaps.is_empty() {
                    0.0
                } else {
                    (lo + hi) / 2.0
                },
                start: main(&boxes[0]).0.max(0.0),
                cross_start,
                cross_offsets: boxes.iter().map(|b| cross(b) - cross_start).collect(),
            });
        }
    }
    None
}

fn text_decls(out: &mut Vec<(&'static str, String)>, s: &TextStyle, strings: &ude_core::Interner) {
    if let Some(c) = first_solid(s.fills.as_slice()) {
        out.push(("color", css_color(c)));
    }
    let family = strings.resolve(s.family);
    if !family.is_empty() {
        out.push(("font-family", format!("\"{family}\", sans-serif")));
    }
    out.push(("font-size", px(s.size)));
    if s.weight != 400 {
        out.push(("font-weight", s.weight.to_string()));
    }
    if s.italic {
        out.push(("font-style", "italic".into()));
    }
    match s.line_height {
        LineHeight::Auto => {}
        LineHeight::Px(v) => out.push(("line-height", px(v))),
        LineHeight::Percent(p) => out.push(("line-height", num(p / 100.0))),
    }
    match s.letter_spacing {
        LetterSpacing::Normal => {}
        LetterSpacing::Px(v) if v != 0.0 => out.push(("letter-spacing", px(v))),
        LetterSpacing::Percent(p) if p != 0.0 => {
            out.push(("letter-spacing", format!("{}em", num(p / 100.0))))
        }
        _ => {}
    }
    match s.decoration {
        TextDecoration::None => {}
        TextDecoration::Underline => out.push(("text-decoration", "underline".into())),
        TextDecoration::Strikethrough => out.push(("text-decoration", "line-through".into())),
    }
    match s.case {
        TextCase::Original => {}
        TextCase::Upper => out.push(("text-transform", "uppercase".into())),
        TextCase::Lower => out.push(("text-transform", "lowercase".into())),
        TextCase::Title => out.push(("text-transform", "capitalize".into())),
        TextCase::SmallCaps => out.push(("font-variant", "small-caps".into())),
    }
}

fn first_solid(paints: &[Paint]) -> Option<Color> {
    paints.iter().find_map(|p| match p.kind {
        PaintKind::Solid(c) if p.visible => Some(with_alpha(c, p.opacity)),
        _ => None,
    })
}

fn with_alpha(mut c: Color, a: f32) -> Color {
    c.a *= a;
    c
}

/// `#rrggbb`, or `rgba()` when translucent.
pub(crate) fn css_color(c: Color) -> String {
    let [r, g, b, a] = c.to_rgba8();
    if a == 255 {
        format!("#{r:02x}{g:02x}{b:02x}")
    } else {
        format!("rgba({r}, {g}, {b}, {})", num(f32::from(a) / 255.0))
    }
}

/// Shortest CSS padding shorthand, or `None` when all zero. Order: top, right, bottom, left.
fn padding(p: [f32; 4]) -> Option<String> {
    if p.iter().all(|&v| v.abs() < 0.01) {
        return None;
    }
    let [t, r, b, l] = p.map(px);
    Some(if t == b && r == l {
        if t == r { t } else { format!("{t} {r}") }
    } else if r == l {
        format!("{t} {r} {b}")
    } else {
        format!("{t} {r} {b} {l}")
    })
}

/// Number with at most 2 decimals and no trailing zeros.
pub(crate) fn num(v: f32) -> String {
    let s = format!("{:.2}", (v * 100.0).round() / 100.0);
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".into() } else { s.into() }
}

pub(crate) fn px(v: f32) -> String {
    let n = num(v);
    if n == "0" { n } else { format!("{n}px") }
}

/// Layer name → kebab-case class; generic names ("Frame 12", "Rectangle") use `role`.
fn class_name(name: &str, role: &str) -> String {
    const GENERIC: [&str; 11] = [
        "frame",
        "rectangle",
        "ellipse",
        "group",
        "text",
        "vector",
        "polygon",
        "star",
        "line",
        "image",
        "page",
    ];
    let mut out = String::new();
    let mut prev = ' ';
    for c in name.chars() {
        let after = std::mem::replace(&mut prev, c);
        if c.is_ascii_alphanumeric() {
            if c.is_ascii_uppercase() && after.is_ascii_lowercase() {
                out.push('-');
            }
            out.push(c.to_ascii_lowercase());
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    let out = out.trim_end_matches('-').to_owned();
    let stem = out.trim_end_matches(|c: char| c.is_ascii_digit() || c == '-');
    if out.is_empty() || GENERIC.contains(&stem) || out.starts_with(|c: char| c.is_ascii_digit()) {
        role.to_owned()
    } else {
        out
    }
}

/// Layer name → PascalCase component name.
fn pascal(name: &str) -> String {
    let mut out = String::new();
    for word in name
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
    {
        let mut cs = word.chars();
        if let Some(f) = cs.next() {
            out.push(f.to_ascii_uppercase());
            out.extend(cs);
        }
    }
    if out.is_empty() || out.starts_with(|c: char| c.is_ascii_digit()) {
        format!("Design{out}")
    } else {
        out
    }
}

fn strip_xml_decl(svg: &str) -> String {
    let s = svg.trim();
    let s = if s.starts_with("<?xml") {
        s.split_once("?>").map_or(s, |(_, r)| r).trim()
    } else {
        s
    };
    s.to_owned()
}

#[cfg(test)]
mod tests;
