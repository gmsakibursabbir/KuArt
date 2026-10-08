//! Every model feature survives save → load → save unchanged.
#![allow(clippy::unwrap_used)]

use std::sync::Arc;

use ude_core::assets::{AssetEntry, AssetKind, AssetSource, Codec, ContentHash};
use ude_core::geometry::{FillRule, PathBuilder};
use ude_core::layout::{
    AutoLayout, ChildSizing, Constraint, Constraints, GridKind, LayoutChild, LayoutDirection,
    LayoutGrid, SizeLimits,
};
use ude_core::scene::{
    ArcData, BoolOpKind, ComponentInfo, ComponentRef, CornerRadii, InstanceData, LayerMask,
    Override, Preserved, VectorData, VectorPath,
};
use ude_core::styles::{
    BlendMode, Color, Effect, Gradient, GradientKind, GradientStop, ImageScaleMode, LineCap,
    LineJoin, Paint, PaintKind, Shadow, StrokeAlign,
};
use ude_core::typography::{
    LetterSpacing, LineHeight, TextCase, TextData, TextDecoration, TextRun, TextStyle,
};
use ude_core::{Affine, Document, Node, NodeKind, Op, Paints, Point, Prop, SourceFormat, Stroke};
use ude_format_native::{Limits, NativeError, load, save};

fn kitchen_sink() -> Document {
    let mut doc = Document::new();
    doc.meta.title = "Kitchen sink".into();
    doc.meta.source = Some(SourceFormat::Psd);
    doc.meta
        .preserved_sections
        .push((*b"FUTR", Arc::from(&b"from a newer version"[..])));
    let s = |doc: &mut Document, t: &str| doc.strings.intern(t);

    let img = doc.assets.insert(AssetEntry {
        hash: ContentHash([1; 32]),
        kind: AssetKind::Image {
            width: 640,
            height: 480,
        },
        source: AssetSource::Memory {
            bytes: Arc::from(&b"\x89PNG fake"[..]),
            codec: Codec::Png,
        },
    });
    let raw = doc.assets.insert(AssetEntry {
        hash: ContentHash([2; 32]),
        kind: AssetKind::Blob,
        source: AssetSource::Memory {
            bytes: Arc::from(vec![0u8; 5000]),
            codec: Codec::Raw,
        },
    });

    let name = s(&mut doc, "Page");
    let page = doc
        .build_child(doc.root(), Node::new(NodeKind::Page, name))
        .unwrap();

    let name = s(&mut doc, "Button");
    let comp_info = ComponentInfo {
        key: s(&mut doc, "btn-key"),
        description: s(&mut doc, "Primary button"),
        variant_props: Box::new([(s(&mut doc, "State"), s(&mut doc, "Hover"))]),
    };
    let mut frame =
        Node::new(NodeKind::Component(Box::new(comp_info)), name).with_size(160.0, 48.0);
    frame.fills = Paints::new(vec![
        Paint::solid(Color::from_rgba8(37, 99, 235, 255)),
        Paint {
            kind: PaintKind::Gradient(Arc::new(Gradient {
                kind: GradientKind::Radial,
                stops: Box::new([
                    GradientStop {
                        offset: 0.0,
                        color: Color::WHITE,
                    },
                    GradientStop {
                        offset: 1.0,
                        color: Color::BLACK,
                    },
                ]),
                transform: Affine::scale(0.5, 0.5),
            })),
            opacity: 0.5,
            blend: BlendMode::Overlay,
            visible: false,
        },
        Paint {
            kind: PaintKind::Image {
                asset: img,
                mode: ImageScaleMode::Crop,
                transform: Some(Box::new(Affine::rotate(0.3))),
            },
            opacity: 1.0,
            blend: BlendMode::Normal,
            visible: true,
        },
    ]);
    frame.corner_radius = 6.0;
    let ext = frame.ext_mut();
    ext.strokes = Some(Box::new(Stroke {
        paints: Paints::new(vec![Paint::solid(Color::BLACK)]),
        width: 2.0,
        align: StrokeAlign::Inside,
        cap: LineCap::Round,
        join: LineJoin::Bevel,
        miter_limit: 4.0,
        dashes: Box::new([4.0, 2.0]),
    }));
    ext.effects = Box::new([
        Effect::DropShadow(Shadow {
            color: Color::from_rgba8(0, 0, 0, 64),
            offset: Point::new(0.0, 4.0),
            blur: 8.0,
            spread: -1.0,
            blend: BlendMode::Multiply,
            visible: true,
        }),
        Effect::BackgroundBlur {
            radius: 20.0,
            visible: true,
        },
    ]);
    ext.corners = Some(CornerRadii {
        radii: [8.0, 8.0, 0.0, 4.0],
        smoothing: 0.6,
    });
    ext.auto_layout = Some(Box::new(AutoLayout {
        direction: LayoutDirection::Vertical,
        gap: 8.0,
        padding: [12.0, 20.0, 12.0, 20.0],
        ..Default::default()
    }));
    ext.layout_child = LayoutChild {
        h_sizing: ChildSizing::Fill,
        absolute: true,
        limits: Some(Box::new(SizeLimits {
            min: [Some(10.0), None],
            max: [None, Some(400.0)],
        })),
        ..Default::default()
    };
    ext.constraints = Constraints {
        horizontal: Constraint::Stretch,
        vertical: Constraint::Scale,
    };
    ext.grids = Box::new([LayoutGrid {
        kind: GridKind::Columns,
        count: 12,
        gutter: 24.0,
        margin: 80.0,
        size: None,
    }]);
    ext.layer_mask = Some(Box::new(LayerMask::Raster {
        asset: raw,
        default_color: 255,
        disabled: false,
    }));
    let key = s(&mut doc, "lfx2");
    let src = s(&mut doc, "1:23");
    let ext = frame.ext_mut();
    ext.source_id = src;
    ext.preserved.push(Preserved {
        source: SourceFormat::Psd,
        key,
        data: raw,
    });
    let component = doc.build_child(page, frame).unwrap();

    let family = s(&mut doc, "Inter");
    let style = TextStyle {
        family,
        weight: 600,
        italic: true,
        size: 16.0,
        line_height: LineHeight::Percent(150.0),
        letter_spacing: LetterSpacing::Px(-0.2),
        decoration: TextDecoration::Underline,
        case: TextCase::Upper,
        fills: Paints::new(vec![Paint::solid(Color::WHITE)]),
    };
    let mut text = TextData::plain("Héllo wörld", style.clone());
    text.styles = Arc::from([
        style.clone(),
        TextStyle {
            weight: 400,
            ..style
        },
    ]);
    text.runs = Box::new([
        TextRun {
            range: 0..6,
            style: 0,
        },
        TextRun {
            range: 6..13,
            style: 1,
        },
    ]);
    assert!(text.validate());
    let name = s(&mut doc, "Label");
    doc.build_child(component, Node::new(NodeKind::Text(Box::new(text)), name))
        .unwrap();

    let path = PathBuilder::default()
        .move_to(0.0, 0.0)
        .quad_to(5.0, 10.0, 10.0, 0.0)
        .line_to(10.0, 10.0)
        .close()
        .build();
    let vector = Arc::new(VectorData {
        paths: Box::new([VectorPath {
            path,
            fill_rule: FillRule::EvenOdd,
        }]),
    });
    let name = s(&mut doc, "Icon");
    doc.build_child(component, Node::new(NodeKind::Vector(vector.clone()), name))
        .unwrap();
    // Shared vector must still decode (and dedupe) when referenced twice.
    doc.build_child(component, Node::new(NodeKind::Vector(vector), name))
        .unwrap();

    let label = s(&mut doc, "Label");
    let instance = InstanceData {
        component: ComponentRef::Local(component),
        overrides: Box::new([
            Override {
                path: Box::new([label]),
                prop: Prop::Opacity(0.5),
            },
            Override {
                path: Box::new([label]),
                prop: Prop::CornerRadius(12.0),
            },
            Override {
                path: Box::new([label]),
                prop: Prop::Kind(NodeKind::Star {
                    points: 5,
                    inner_ratio: 0.4,
                }),
            },
        ]),
    };
    let name = s(&mut doc, "Button instance");
    // Placed on a page *before* the component's page: forward references must resolve.
    let page2 = doc.create(Node::new(NodeKind::Page, name));
    let root = doc.root();
    doc.apply(Op::Attach {
        id: page2,
        parent: root,
        index: 0,
    })
    .unwrap();
    doc.build_child(
        page2,
        Node::new(NodeKind::Instance(Box::new(instance)), name),
    )
    .unwrap();

    for kind in [
        NodeKind::Ellipse(Some(Box::new(ArcData {
            start: 0.0,
            sweep: 2.5,
            inner_ratio: 0.5,
        }))),
        NodeKind::Polygon { sides: 6 },
        NodeKind::Line,
        NodeKind::Raster { asset: img },
        NodeKind::BooleanOp(BoolOpKind::Exclude),
        NodeKind::ComponentSet,
        NodeKind::Slice,
        NodeKind::Preserved {
            source: SourceFormat::Xd,
        },
        NodeKind::Group,
        NodeKind::Frame,
        NodeKind::Rectangle,
    ] {
        doc.build_child(
            page2,
            Node::new(kind, name).with_transform(Affine::rotate(1.0)),
        )
        .unwrap();
    }
    doc
}

#[test]
fn every_feature_round_trips() {
    let doc = kitchen_sink();
    let a = save(&doc).unwrap();
    let loaded = load(&a, &Limits::default()).unwrap();
    assert_eq!(save(&loaded).unwrap(), a, "re-save must be byte-identical");

    assert_eq!(loaded.meta.title, "Kitchen sink");
    assert_eq!(loaded.meta.source, Some(SourceFormat::Psd));
    assert_eq!(loaded.meta.preserved_sections, doc.meta.preserved_sections);
    assert_eq!(
        loaded.descendants(loaded.root()).count(),
        doc.descendants(doc.root()).count()
    );

    // The instance's local reference points at the reloaded component.
    let pages = loaded.children(loaded.root());
    let instance = loaded.node(loaded.children(pages[0])[0]).unwrap();
    let component = loaded.children(pages[1])[0];
    let NodeKind::Instance(data) = &instance.kind else {
        panic!("expected instance")
    };
    assert_eq!(data.component, ComponentRef::Local(component));

    // Shared vector data is shared again after load.
    let kids = loaded.children(component);
    let (NodeKind::Vector(a), NodeKind::Vector(b)) = (
        &loaded.node(kids[1]).unwrap().kind,
        &loaded.node(kids[2]).unwrap().kind,
    ) else {
        panic!("expected vectors")
    };
    assert!(Arc::ptr_eq(a, b));
}

#[test]
fn rejects_wrong_magic_and_truncation() {
    let a = save(&kitchen_sink()).unwrap();
    assert!(matches!(
        load(b"hello", &Limits::default()),
        Err(NativeError::Truncated)
    ));
    assert!(matches!(
        load(&a[..a.len() - 1], &Limits::default()),
        Err(NativeError::Truncated)
    ));
    let tight = Limits {
        max_nodes: 3,
        ..Limits::default()
    };
    assert!(matches!(
        load(&a, &tight),
        Err(NativeError::LimitExceeded(_))
    ));
}
