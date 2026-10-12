use super::*;
use ude_core::styles::Paints;
use ude_core::typography::{TextData, TextStyle};
use ude_core::{Affine, Symbol};

fn text_node(doc: &mut Document, s: &str, size: f32, weight: u16) -> Node {
    let style = TextStyle {
        family: doc.strings.intern("Inter"),
        weight,
        italic: false,
        size,
        line_height: LineHeight::Auto,
        letter_spacing: LetterSpacing::Normal,
        decoration: TextDecoration::None,
        case: TextCase::Original,
        fills: Paints::new(vec![Paint::solid(Color::from_rgba8(0x11, 0x18, 0x27, 255))]),
    };
    let mut t = TextData::plain(s, style);
    t.sizing = TextSizing::AutoWidth;
    Node::new(NodeKind::Text(Box::new(t)), Symbol::EMPTY)
}

/// Card 320×200: title, body and a button stacked with a 12px gap at 24px padding.
fn card() -> (Document, NodeId) {
    let mut doc = Document::new();
    let root = doc.root();
    let page = doc
        .build_child(root, Node::new(NodeKind::Page, Symbol::EMPTY))
        .unwrap();
    let name = doc.strings.intern("Product Card");
    let card = doc
        .build_child(
            page,
            Node::new(NodeKind::Frame, name)
                .with_size(320.0, 200.0)
                .with_fills(Paints::new(vec![Paint::solid(Color::WHITE)])),
        )
        .unwrap();
    doc.apply(Op::Set {
        id: card,
        value: Prop::CornerRadius(12.0),
    })
    .unwrap();
    let title = text_node(&mut doc, "Hello <World>", 32.0, 700)
        .with_transform(Affine::translate(24.0, 24.0))
        .with_size(200.0, 40.0);
    doc.build_child(card, title).unwrap();
    let body = text_node(&mut doc, "Body", 16.0, 400)
        .with_transform(Affine::translate(24.0, 76.0))
        .with_size(40.0, 20.0);
    doc.build_child(card, body).unwrap();
    let bname = doc.strings.intern("Buy Button");
    let button = doc
        .build_child(
            card,
            Node::new(NodeKind::Frame, bname)
                .with_transform(Affine::translate(24.0, 108.0))
                .with_size(120.0, 40.0)
                .with_fills(Paints::new(vec![Paint::solid(Color::from_rgba8(
                    0x25, 0x63, 0xeb, 255,
                ))])),
        )
        .unwrap();
    let label = text_node(&mut doc, "Buy", 16.0, 600)
        .with_transform(Affine::translate(44.0, 10.0))
        .with_size(32.0, 20.0);
    doc.build_child(button, label).unwrap();
    (doc, card)
}

use ude_core::{Op, Prop};

#[test]
fn infers_flex_column_and_semantics() {
    let (doc, card) = card();
    let out = generate(&doc, card, Target::Html).unwrap();
    let c = &out.code;
    assert!(c.contains(".product-card {"), "{c}");
    assert!(c.contains("flex-direction: column;"), "{c}");
    assert!(c.contains("gap: 12px;"), "{c}");
    assert!(c.contains("padding: 24px 0 0 24px;"), "{c}");
    assert!(c.contains("border-radius: 12px;"), "{c}");
    assert!(
        c.contains("<h1 class=\"title\">Hello &lt;World&gt;</h1>"),
        "{c}"
    );
    assert!(
        c.contains("<button class=\"buy-button\" type=\"button\">"),
        "{c}"
    );
    assert!(c.contains("background: #2563eb;"), "{c}");
    assert!(c.contains("justify-content: center;"), "{c}");
    // The button's single label child is placed absolutely (one child, offset inside).
    assert!(out.notes.is_empty() || out.notes[0].contains("absolutely"));
}

#[test]
fn all_targets_are_deterministic() {
    let (doc, card) = card();
    for t in Target::ALL {
        let a = generate(&doc, card, t).unwrap().code;
        let b = generate(&doc, card, t).unwrap().code;
        assert_eq!(a, b, "{}", t.name());
        assert!(!a.is_empty());
    }
    let react = generate(&doc, card, Target::React).unwrap().code;
    assert!(
        react.contains("export default function ProductCard()"),
        "{react}"
    );
    assert!(react.contains("className=\"product-card\""), "{react}");
    let tw = generate(&doc, card, Target::Tailwind).unwrap().code;
    assert!(tw.contains("w-80 h-[200px]"), "{tw}");
    assert!(tw.contains("bg-[#2563eb]"), "{tw}");
    assert!(tw.contains("gap-3"), "{tw}");
    let tokens = generate(&doc, card, Target::Tokens).unwrap().code;
    assert!(tokens.contains("--color-1:"), "{tokens}");
    assert!(tokens.contains("\"$type\": \"color\""), "{tokens}");
}

#[test]
fn helpers() {
    assert_eq!(class_name("Frame 12", "container"), "container");
    assert_eq!(class_name("Hero Section", "x"), "hero-section");
    assert_eq!(class_name("primaryCTA", "x"), "primary-cta");
    assert_eq!(pascal("product card"), "ProductCard");
    assert_eq!(padding([8.0, 16.0, 8.0, 16.0]).unwrap(), "8px 16px");
    assert_eq!(num(1.004), "1");
    assert_eq!(num(12.5), "12.5");
    assert_eq!(
        emit_jsx("<path fill-rule=\"evenodd\" stroke-width=\"2\"/>"),
        "<path fillRule=\"evenodd\" strokeWidth=\"2\"/>"
    );
}

fn emit_jsx(s: &str) -> String {
    emit::jsx_svg(s)
}

#[test]
#[ignore = "prints sample output"]
fn print_samples() {
    let (doc, card) = card();
    for t in Target::ALL {
        println!(
            "===== {} =====\n{}",
            t.name(),
            generate(&doc, card, t).unwrap().code
        );
    }
}
