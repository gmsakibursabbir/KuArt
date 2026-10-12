use serde_json::json;

use super::*;

fn rgb(r: u8, g: u8, b: u8) -> Value {
    json!({"mode": "RGB", "value": {"r": r, "g": g, "b": b}, "alpha": 1})
}

fn sample() -> Vec<u8> {
    let manifest = json!({
        "name": "Sample",
        "children": [{"path": "artwork", "children": [
            {"path": "pasteboard", "name": "pasteboard"},
            {"path": "artboard-a1", "name": "iPhone", "uxdesign#bounds": {"x": 100, "y": 50, "width": 375, "height": 812}}
        ]}]
    });
    let resources = json!({
        "resources": {
            "gradients": {"g1": {"type": "linear", "stops": [
                {"offset": 0, "color": rgb(255, 0, 0)}, {"offset": 1, "color": rgb(0, 0, 255)}
            ]}},
            "meta": {"ux": {"symbols": [{"id": "sym1", "type": "symbol", "group": {"children": [
                {"type": "shape", "name": "Sym rect", "shape": {"type": "rect", "x": 0, "y": 0, "width": 40, "height": 20},
                 "style": {"fill": {"type": "solid", "color": rgb(0, 128, 0)}}}
            ]}}]}}
        }
    });
    let artboard = json!({"children": [{"type": "artboard", "style": {"fill": {"type": "solid", "color": rgb(250, 250, 250)}},
    "artboard": {"children": [
        {"type": "shape", "name": "Card", "transform": {"a": 1, "b": 0, "c": 0, "d": 1, "tx": 20, "ty": 30},
         "shape": {"type": "rect", "x": 0, "y": 0, "width": 200, "height": 100, "r": [8, 8, 8, 8]},
         "style": {"fill": {"type": "gradient", "gradient": {"ref": "g1", "units": "objectBoundingBox", "x1": 0, "y1": 0, "x2": 1, "y2": 0}},
                   "stroke": {"type": "solid", "color": rgb(0, 0, 0), "width": 2, "align": "inside"},
                   "filters": [{"type": "dropShadow", "params": {"dropShadows": [{"dx": 0, "dy": 3, "r": 6, "color": rgb(0, 0, 0)}]}}]}},
        {"type": "group", "name": "Icons", "group": {"children": [
            {"type": "shape", "name": "Dot", "shape": {"type": "ellipse", "cx": 10, "cy": 10, "rx": 10, "ry": 5}},
            {"type": "shape", "name": "Tri", "shape": {"type": "path", "path": "M10 10 L30 10 L20 30 Z"}}
        ]}},
        {"type": "text", "name": "Title", "transform": {"a": 1, "b": 0, "c": 0, "d": 1, "tx": 20, "ty": 200},
         "text": {"rawText": "Hello XD", "frame": {"type": "positioned"}, "paragraphs": [{"lines": [[{"x": 0, "y": 0, "from": 0, "to": 8}]]}]},
         "style": {"font": {"family": "Inter", "style": "Semibold", "size": 20}, "fill": {"type": "solid", "color": rgb(17, 17, 17)},
                   "textAttributes": {"paragraphAlign": "center", "letterSpacing": 50}},
         "meta": {"ux": {"rangedStyles": [
             {"length": 6, "fontFamily": "Inter", "fontStyle": "Semibold", "fontSize": 20},
             {"length": 2, "fontFamily": "Inter", "fontStyle": "Bold Italic", "fontSize": 20}
         ]}}},
        {"type": "syncRef", "name": "Button", "syncSourceGuid": "sym1"},
        {"type": "mystery"}
    ]}}]});
    ude_format_zip::write_stored(&[
        ("mimetype", b"application/vnd.adobe.sparkler.project+dcx"),
        ("manifest", manifest.to_string().as_bytes()),
        (
            "resources/graphics/graphicContent.agc",
            resources.to_string().as_bytes(),
        ),
        (
            "artwork/artboard-a1/graphics/graphicContent.agc",
            artboard.to_string().as_bytes(),
        ),
    ])
}

#[test]
fn imports_artboards_shapes_text_and_symbols() {
    let im = import(&sample(), "sample.xd").unwrap();
    let doc = &im.doc;
    let page = doc.children(doc.root())[0];
    let boards = doc.children(page);
    assert_eq!(boards.len(), 1);
    let board = doc.node(boards[0]).unwrap();
    assert_eq!(doc.strings.resolve(board.name), "iPhone");
    assert_eq!(board.transform.translation(), Point::new(100.0, 50.0));
    assert_eq!((board.size.w, board.size.h), (375.0, 812.0));

    let kids = doc.children(boards[0]);
    let name = |id| doc.strings.resolve(doc.node(id).unwrap().name).to_owned();
    assert_eq!(
        kids.iter().map(|&k| name(k)).collect::<Vec<_>>(),
        ["Card", "Icons", "Title", "Button"]
    );

    let card = doc.node(kids[0]).unwrap();
    assert_eq!(card.corner_radius, 8.0);
    assert!(matches!(
        card.fills.as_slice()[0].kind,
        PaintKind::Gradient(_)
    ));
    let ext = card.ext.as_ref().unwrap();
    assert_eq!(ext.strokes.as_ref().unwrap().align, StrokeAlign::Inside);
    assert!(matches!(ext.effects[0], Effect::DropShadow(_)));

    let icons = doc.children(kids[1]);
    let dot = doc.node(icons[0]).unwrap();
    assert!(matches!(dot.kind, NodeKind::Ellipse(_)));
    assert_eq!((dot.size.w, dot.size.h), (20.0, 10.0));
    assert_eq!(dot.transform.translation(), Point::new(0.0, 5.0));
    let tri = doc.node(icons[1]).unwrap();
    assert_eq!(
        tri.transform.translation(),
        Point::new(10.0, 10.0),
        "box starts at path bounds"
    );
    assert_eq!((tri.size.w, tri.size.h), (20.0, 20.0));

    let NodeKind::Text(t) = &doc.node(kids[2]).unwrap().kind else {
        panic!("title is text")
    };
    assert_eq!(&*t.content, "Hello XD");
    assert_eq!(t.align_h, TextAlignH::Center);
    assert_eq!(t.runs.len(), 2);
    assert_eq!(t.styles[t.runs[1].style as usize].weight, 700);
    assert!(t.styles[t.runs[1].style as usize].italic);
    assert_eq!(t.styles[0].letter_spacing, LetterSpacing::Percent(5.0));

    assert_eq!(doc.children(kids[3]).len(), 1, "symbol expanded");
    assert!(
        im.warnings.iter().any(|w| w.contains("mystery")),
        "{:?}",
        im.warnings
    );
}

#[test]
fn rejects_bad_input() {
    assert!(import(b"not a zip", "x").is_err());
    assert!(import(&ude_format_zip::write_stored(&[("a", b"b")]), "x").is_err());
}

#[test]
fn affine_multiply_order() {
    let t = mul(Affine::translate(10.0, 0.0), Affine::scale(2.0, 2.0));
    assert_eq!(t.apply(Point::new(1.0, 1.0)), Point::new(12.0, 2.0));
}
