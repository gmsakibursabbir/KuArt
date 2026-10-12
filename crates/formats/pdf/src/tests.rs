use lopdf::content::{Content, Operation};
use lopdf::{Object, Stream, dictionary};

use super::*;

fn op(name: &str, operands: Vec<Object>) -> Operation {
    Operation::new(name, operands)
}

fn n(v: f32) -> Object {
    Object::Real(v)
}

/// One 200×100 page: red rectangle, CMYK stroked line at 50% opacity, a form XObject drawing
/// a blue square through a translate matrix, and a line of Helvetica-Bold text.
fn sample_pdf() -> Vec<u8> {
    let mut doc = lopdf::Document::with_version("1.7");
    let pages_id = doc.new_object_id();
    let font_id = doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "ABCDEF+Helvetica-Bold",
    });
    let gs_id = doc.add_object(dictionary! { "Type" => "ExtGState", "CA" => 0.5 });
    let form_content = Content {
        operations: vec![
            op("rg", vec![n(0.0), n(0.0), n(1.0)]),
            op("re", vec![n(0.0), n(0.0), n(10.0), n(10.0)]),
            op("f", vec![]),
        ],
    };
    let form_id = doc.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Form",
            "BBox" => vec![0.into(), 0.into(), 10.into(), 10.into()],
            "Matrix" => vec![1.into(), 0.into(), 0.into(), 1.into(), 150.into(), 10.into()],
        },
        form_content.encode().unwrap(),
    ));
    let content = Content {
        operations: vec![
            op("rg", vec![n(1.0), n(0.0), n(0.0)]),
            op("re", vec![n(10.0), n(60.0), n(50.0), n(30.0)]),
            op("f", vec![]),
            op("q", vec![]),
            op("gs", vec![Object::Name(b"G0".to_vec())]),
            op("K", vec![n(0.0), n(0.0), n(0.0), n(1.0)]),
            op("w", vec![n(2.0)]),
            op("m", vec![n(10.0), n(10.0)]),
            op("l", vec![n(100.0), n(10.0)]),
            op("S", vec![]),
            op("Q", vec![]),
            op("Do", vec![Object::Name(b"X0".to_vec())]),
            op("BT", vec![]),
            op("Tf", vec![Object::Name(b"F1".to_vec()), n(12.0)]),
            op("Td", vec![n(20.0), n(40.0)]),
            op("Tj", vec![Object::string_literal("Hello PDF")]),
            op("ET", vec![]),
            op("W", vec![]),
            op("n", vec![]),
        ],
    };
    let content_id = doc.add_object(Stream::new(dictionary! {}, content.encode().unwrap()));
    let resources = dictionary! {
        "Font" => dictionary! { "F1" => font_id },
        "ExtGState" => dictionary! { "G0" => gs_id },
        "XObject" => dictionary! { "X0" => form_id },
    };
    let page_id = doc.add_object(dictionary! {
        "Type" => "Page", "Parent" => pages_id, "Contents" => content_id, "Resources" => resources,
    });
    doc.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages", "Kids" => vec![page_id.into()], "Count" => 1,
            "MediaBox" => vec![0.into(), 0.into(), 200.into(), 100.into()],
        }),
    );
    let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    doc.trailer.set("Root", catalog);
    let mut out = Vec::new();
    doc.save_to(&mut out).unwrap();
    out
}

#[test]
fn imports_paths_colours_forms_and_text() {
    let im = import(&sample_pdf(), "doc.pdf", false).unwrap();
    let doc = &im.doc;
    let page = doc.children(doc.root())[0];
    let frame = doc.children(page)[0];
    assert_eq!(
        doc.node(frame).unwrap().size,
        ude_core::Size::new(200.0, 100.0),
        "inherited MediaBox"
    );
    let kids = doc.children(frame);
    assert_eq!(kids.len(), 4, "rect, line, form square, text");

    // y-up → y-down: the rect at y 60..90 lands at 10..40 from the top.
    let rect = doc.node(kids[0]).unwrap();
    assert_eq!(rect.transform.translation(), Point::new(10.0, 10.0));
    assert_eq!((rect.size.w, rect.size.h), (50.0, 30.0));
    assert_eq!(
        rect.fills.as_slice()[0].kind,
        ude_core::styles::PaintKind::Solid(Color {
            r: 1.0,
            g: 0.0,
            b: 0.0,
            a: 1.0
        })
    );

    let line = doc.node(kids[1]).unwrap();
    let stroke = line.ext.as_ref().unwrap().strokes.as_ref().unwrap();
    assert_eq!(stroke.width, 2.0);
    let ude_core::styles::PaintKind::Solid(c) = stroke.paints.as_slice()[0].kind else {
        panic!()
    };
    assert_eq!((c.r, c.a), (0.0, 0.5), "CMYK black at 50% stroke opacity");
    assert!(line.fills.is_empty());

    let square = doc.node(kids[2]).unwrap();
    assert_eq!(
        square.transform.translation(),
        Point::new(150.0, 80.0),
        "form matrix applied"
    );

    let NodeKind::Text(t) = &doc.node(kids[3]).unwrap().kind else {
        panic!("text")
    };
    assert_eq!(&*t.content, "Hello PDF");
    assert_eq!(doc.strings.resolve(t.styles[0].family), "Helvetica");
    assert_eq!((t.styles[0].weight, t.styles[0].size), (700, 12.0));
    let o = doc.node(kids[3]).unwrap().transform.translation();
    assert!(
        (o.x - 20.0).abs() < 1e-3 && (o.y - (60.0 - 12.0 * ASCENT)).abs() < 1e-3,
        "{o:?}"
    );

    assert!(
        im.warnings.iter().any(|w| w.contains("clipping")),
        "{:?}",
        im.warnings
    );
}

#[test]
fn illustrator_files() {
    let im = import(&sample_pdf(), "logo.ai", true).unwrap();
    let page = im.doc.children(im.doc.root())[0];
    assert_eq!(
        im.doc.strings.resolve(im.doc.node(page).unwrap().name),
        "Artboard 1"
    );
    assert_eq!(im.doc.meta.source, Some(SourceFormat::Illustrator));
    let err = import(
        b"%!PS-Adobe-3.0\n%%Creator: Adobe Illustrator",
        "old.ai",
        true,
    )
    .err()
    .unwrap();
    assert!(err.contains("PDF Compatible"), "{err}");
    assert!(import(b"hello", "x.pdf", false).is_err());
}

#[test]
fn fonts_and_cmaps() {
    assert_eq!(
        text::split_base_font("ABCDEF+MyriadPro-BoldIt"),
        ("Myriad Pro".into(), "BoldItalic".into())
    );
    assert_eq!(
        text::split_base_font("Arial,Bold"),
        ("Arial".into(), "Bold".into())
    );
    let map = text::parse_cmap(
        b"begincmap 2 beginbfchar <0003> <0020> <0024> <0041> endbfchar\n1 beginbfrange <0044> <0046> <0061> endbfrange\n1 beginbfrange <0050> <0051> [<00E9> <D83DDC4B>] endbfrange endcmap",
    );
    assert_eq!(map[&0x24], "A");
    assert_eq!(map[&0x46], "c");
    assert_eq!(map[&0x51], "\u{1f44b}");
}
