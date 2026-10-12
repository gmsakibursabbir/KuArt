//! Builds `.fig` files in the real wire format (Kiwi schema + message, deflate/zstd chunks,
//! optional ZIP container) and checks the imported tree.

use std::io::Write;

use super::*;
use crate::kiwi::write::{float, int, string, uint};

const BOOL: i32 = -1;
const BYTE: i32 = -2;
const UINT: i32 = -4;
const FLOAT: i32 = -5;
const STRING: i32 = -6;

/// (name, kind, fields: (name, type, is_array)). Kinds: 0 enum, 1 struct, 2 message.
type DefSpec = (&'static str, u8, &'static [(&'static str, i32, bool)]);

const DEFS: &[DefSpec] = &[
    (
        "GUID",
        1,
        &[("sessionID", UINT, false), ("localID", UINT, false)],
    ),
    (
        "ParentIndex",
        1,
        &[("guid", 0, false), ("position", STRING, false)],
    ),
    ("Vector", 1, &[("x", FLOAT, false), ("y", FLOAT, false)]),
    (
        "Matrix",
        1,
        &[
            ("m00", FLOAT, false),
            ("m01", FLOAT, false),
            ("m02", FLOAT, false),
            ("m10", FLOAT, false),
            ("m11", FLOAT, false),
            ("m12", FLOAT, false),
        ],
    ),
    (
        "Color",
        1,
        &[
            ("r", FLOAT, false),
            ("g", FLOAT, false),
            ("b", FLOAT, false),
            ("a", FLOAT, false),
        ],
    ),
    (
        "NodeType",
        0,
        &[
            ("DOCUMENT", 0, false),
            ("CANVAS", 0, false),
            ("FRAME", 0, false),
            ("TEXT", 0, false),
            ("VECTOR", 0, false),
            ("INSTANCE", 0, false),
            ("SYMBOL", 0, false),
            ("RECTANGLE", 0, false),
        ],
    ),
    (
        "PaintType",
        0,
        &[("SOLID", 0, false), ("GRADIENT_LINEAR", 0, false)],
    ),
    (
        "Paint",
        2,
        &[
            ("type", 6, false),
            ("color", 4, false),
            ("opacity", FLOAT, false),
            ("visible", BOOL, false),
        ],
    ),
    (
        "FontName",
        1,
        &[("family", STRING, false), ("style", STRING, false)],
    ),
    ("TextData", 2, &[("characters", STRING, false)]),
    (
        "WindingRule",
        0,
        &[("NONZERO", 0, false), ("ODD", 0, false)],
    ),
    (
        "Path",
        2,
        &[("windingRule", 10, false), ("commandsBlob", UINT, false)],
    ),
    ("SymbolData", 2, &[("symbolID", 0, false)]),
    (
        "NodeChange",
        2,
        &[
            ("guid", 0, false),
            ("parentIndex", 1, false),
            ("type", 5, false),
            ("name", STRING, false),
            ("visible", BOOL, false),
            ("opacity", FLOAT, false),
            ("size", 2, false),
            ("transform", 3, false),
            ("fillPaints", 7, true),
            ("textData", 9, false),
            ("fontSize", FLOAT, false),
            ("fontName", 8, false),
            ("fillGeometry", 11, true),
            ("cornerRadius", FLOAT, false),
            ("symbolData", 12, false),
        ],
    ),
    ("Blob", 2, &[("bytes", BYTE, true)]),
    (
        "Message",
        2,
        &[("nodeChanges", 13, true), ("blobs", 14, true)],
    ),
];

fn schema() -> Vec<u8> {
    let mut out = Vec::new();
    uint(&mut out, DEFS.len() as u32);
    for (name, kind, fields) in DEFS {
        string(&mut out, name);
        out.push(*kind);
        uint(&mut out, fields.len() as u32);
        for (i, (f, ty, array)) in fields.iter().enumerate() {
            string(&mut out, f);
            int(&mut out, *ty);
            out.push(u8::from(*array));
            // Enum values and struct order are 0-based; message field ids start at 1.
            uint(&mut out, if *kind == 2 { i as u32 + 1 } else { i as u32 });
        }
    }
    out
}

/// Message fields: (field id, encoded value).
fn message(fields: Vec<(u32, Vec<u8>)>) -> Vec<u8> {
    let mut out = Vec::new();
    for (id, v) in fields {
        uint(&mut out, id);
        out.extend(v);
    }
    out.push(0);
    out
}

fn u(v: u32) -> Vec<u8> {
    let mut o = Vec::new();
    uint(&mut o, v);
    o
}
fn f(vals: &[f32]) -> Vec<u8> {
    let mut o = Vec::new();
    for &v in vals {
        float(&mut o, v);
    }
    o
}
fn s(v: &str) -> Vec<u8> {
    let mut o = Vec::new();
    string(&mut o, v);
    o
}
fn guid_b(g: (u32, u32)) -> Vec<u8> {
    [u(g.0), u(g.1)].concat()
}
fn array(items: Vec<Vec<u8>>) -> Vec<u8> {
    let mut o = u(items.len() as u32);
    for i in items {
        o.extend(i);
    }
    o
}
fn solid(r: f32, g: f32, b: f32) -> Vec<u8> {
    message(vec![
        (1, u(0)),
        (2, f(&[r, g, b, 1.0])),
        (3, f(&[1.0])),
        (4, vec![1]),
    ])
}

struct N {
    guid: (u32, u32),
    parent: Option<((u32, u32), &'static str)>,
    ty: u32,
    name: &'static str,
    extra: Vec<(u32, Vec<u8>)>,
}

fn node(n: N) -> Vec<u8> {
    let mut fields = vec![(1, guid_b(n.guid))];
    if let Some((p, pos)) = n.parent {
        fields.push((2, [guid_b(p), s(pos)].concat()));
    }
    fields.push((3, u(n.ty)));
    fields.push((4, s(n.name)));
    fields.extend(n.extra);
    message(fields)
}

fn translate(x: f32, y: f32) -> Vec<u8> {
    f(&[1.0, 0.0, x, 0.0, 1.0, y])
}

fn sample_message() -> Vec<u8> {
    let page = (0, 1);
    let frame = (1, 1);
    let symbol = (2, 1);
    let nodes = vec![
        node(N {
            guid: (0, 0),
            parent: None,
            ty: 0,
            name: "Document",
            extra: vec![],
        }),
        node(N {
            guid: page,
            parent: Some(((0, 0), "!")),
            ty: 1,
            name: "Page 1",
            extra: vec![],
        }),
        node(N {
            guid: frame,
            parent: Some((page, "!")),
            ty: 2,
            name: "Card",
            extra: vec![
                (7, f(&[200.0, 100.0])),
                (8, translate(10.0, 20.0)),
                (9, array(vec![solid(1.0, 1.0, 1.0)])),
                (14, f(&[8.0])),
            ],
        }),
        // Children are listed out of order; positions decide paint order.
        node(N {
            guid: (1, 3),
            parent: Some((frame, "#")),
            ty: 4,
            name: "Icon",
            extra: vec![
                (7, f(&[10.0, 10.0])),
                (9, array(vec![solid(1.0, 0.0, 0.0)])),
                (13, array(vec![message(vec![(1, u(0)), (2, u(0))])])),
            ],
        }),
        node(N {
            guid: (1, 2),
            parent: Some((frame, "!")),
            ty: 3,
            name: "Title",
            extra: vec![
                (7, f(&[60.0, 30.0])),
                (8, translate(16.0, 16.0)),
                (9, array(vec![solid(0.0, 0.0, 0.0)])),
                (10, message(vec![(1, s("Hello"))])),
                (11, f(&[24.0])),
                (12, [s("Inter"), s("Semi Bold")].concat()),
            ],
        }),
        node(N {
            guid: symbol,
            parent: Some((page, "#")),
            ty: 6,
            name: "Button",
            extra: vec![(7, f(&[80.0, 30.0])), (8, translate(300.0, 0.0))],
        }),
        node(N {
            guid: (2, 2),
            parent: Some((symbol, "!")),
            ty: 7,
            name: "Bg",
            extra: vec![
                (7, f(&[80.0, 30.0])),
                (9, array(vec![solid(0.0, 0.0, 1.0)])),
            ],
        }),
        node(N {
            guid: (3, 1),
            parent: Some((page, "$")),
            ty: 5,
            name: "Button instance",
            extra: vec![
                (7, f(&[80.0, 30.0])),
                (8, translate(0.0, 200.0)),
                (15, message(vec![(1, guid_b(symbol))])),
            ],
        }),
    ];
    let mut commands = vec![1u8];
    for v in [0.0f32, 0.0] {
        commands.extend(v.to_le_bytes());
    }
    for (x, y) in [(10.0f32, 0.0f32), (10.0, 10.0)] {
        commands.push(2);
        commands.extend(x.to_le_bytes());
        commands.extend(y.to_le_bytes());
    }
    commands.push(0);
    let blob = message(vec![(1, [u(commands.len() as u32), commands].concat())]);
    message(vec![(1, array(nodes)), (2, array(vec![blob]))])
}

fn deflate(data: &[u8]) -> Vec<u8> {
    let mut e = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
    e.write_all(data).unwrap();
    e.finish().unwrap()
}

fn fig(zstd_message: bool) -> Vec<u8> {
    let mut out = b"fig-kiwi".to_vec();
    out.extend(15u32.to_le_bytes());
    let msg = if zstd_message {
        zstd::encode_all(&sample_message()[..], 3).unwrap()
    } else {
        deflate(&sample_message())
    };
    for chunk in [deflate(&schema()), msg] {
        out.extend((chunk.len() as u32).to_le_bytes());
        out.extend(chunk);
    }
    out
}

fn zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
    ude_format_zip::write_stored(entries)
}

fn check(im: &Imported) {
    let doc = &im.doc;
    let pages = doc.children(doc.root());
    assert_eq!(pages.len(), 1);
    let top = doc.children(pages[0]);
    let name = |id| doc.strings.resolve(doc.node(id).unwrap().name).to_owned();
    assert_eq!(
        top.iter().map(|&i| name(i)).collect::<Vec<_>>(),
        ["Card", "Button", "Button instance"]
    );

    let card = doc.node(top[0]).unwrap();
    assert_eq!(card.transform.translation(), Point::new(10.0, 20.0));
    assert_eq!(card.size, ude_core::Size::new(200.0, 100.0));
    assert_eq!(card.corner_radius, 8.0);
    assert!(card.flags.contains(NodeFlags::CLIPS_CONTENT));
    let kids = doc.children(top[0]);
    assert_eq!(name(kids[0]), "Title", "sorted by position");
    let NodeKind::Text(t) = &doc.node(kids[0]).unwrap().kind else {
        panic!("title is text")
    };
    assert_eq!(&*t.content, "Hello");
    assert_eq!(doc.strings.resolve(t.styles[0].family), "Inter");
    assert_eq!((t.styles[0].weight, t.styles[0].size), (600, 24.0));
    let NodeKind::Vector(v) = &doc.node(kids[1]).unwrap().kind else {
        panic!("icon is a vector")
    };
    assert_eq!(v.paths[0].path.verbs().len(), 4);

    assert!(matches!(
        doc.node(top[1]).unwrap().kind,
        NodeKind::Component(_)
    ));
    let inst = doc.node(top[2]).unwrap();
    let NodeKind::Instance(data) = &inst.kind else {
        panic!("instance kind")
    };
    assert_eq!(data.component, ComponentRef::Local(top[1]));
    assert_eq!(doc.children(top[2]).len(), 1, "expanded from the component");
    assert!(im.warnings.is_empty(), "{:?}", im.warnings);
}

#[test]
fn imports_bare_canvas_deflate() {
    check(&import(&fig(false), "t.fig").unwrap());
}

#[test]
fn imports_zip_container_with_zstd_message() {
    let bytes = zip(&[("meta.json", b"{}"), ("canvas.fig", &fig(true))]);
    check(&import(&bytes, "t.fig").unwrap());
}

#[test]
fn rejects_bad_input() {
    assert!(import(b"hello", "x").is_err());
    let mut f = fig(false);
    f.truncate(40);
    assert!(import(&f, "x").is_err());
    assert!(import(&zip(&[("x.txt", b"hi")]), "x").is_err());
}

#[test]
fn font_styles() {
    assert_eq!(
        ude_core::typography::parse_font_style("Semi Bold Italic"),
        (600, true)
    );
    assert_eq!(
        ude_core::typography::parse_font_style("ExtraLight"),
        (200, false)
    );
    assert_eq!(
        ude_core::typography::parse_font_style("Regular"),
        (400, false)
    );
    assert_eq!(
        ude_core::typography::parse_font_style("Black"),
        (900, false)
    );
}
