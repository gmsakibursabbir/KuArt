use serde_json::json;

use super::*;

fn rgba(r: f64, g: f64, b: f64) -> Value {
    json!({"_class": "color", "red": r, "green": g, "blue": b, "alpha": 1})
}

fn frame(x: i32, y: i32, w: i32, h: i32) -> Value {
    json!({"_class": "rect", "x": x, "y": y, "width": w, "height": h})
}

fn fill(c: Value) -> Value {
    json!({"_class": "fill", "isEnabled": true, "fillType": 0, "color": c})
}

fn sample() -> Vec<u8> {
    let master = json!({
        "_class": "symbolMaster", "do_objectID": "M", "symbolID": "sym-1", "name": "Button",
        "frame": frame(0, 0, 100, 40),
        "layers": [{
            "_class": "text", "do_objectID": "label", "name": "Label", "frame": frame(10, 10, 80, 20),
            "textBehaviour": 0,
            "attributedString": {"string": "Buy", "attributes": [{"location": 0, "length": 3, "attributes": {
                "MSAttributedStringFontAttribute": {"attributes": {"name": "Inter-SemiBold", "size": 16}},
                "MSAttributedStringColorAttribute": rgba(1.0, 1.0, 1.0)
            }}]}
        }]
    });
    let page = json!({
        "_class": "page", "name": "Home", "frame": frame(0, 0, 0, 0),
        "layers": [
            {"_class": "artboard", "name": "Mobile", "frame": frame(100, 50, 375, 812),
             "hasBackgroundColor": true, "backgroundColor": rgba(1.0, 1.0, 1.0),
             "layers": [
                {"_class": "rectangle", "name": "Card", "frame": frame(16, 16, 200, 120), "fixedRadius": 12,
                 "style": {"fills": [fill(rgba(0.9, 0.9, 0.9))],
                           "borders": [{"isEnabled": true, "fillType": 0, "color": rgba(0.0, 0.0, 0.0), "thickness": 2, "position": 1}],
                           "shadows": [{"isEnabled": true, "color": rgba(0.0, 0.0, 0.0), "offsetX": 0, "offsetY": 4, "blurRadius": 8, "spread": 0}],
                           "contextSettings": {"opacity": 0.5, "blendMode": 2}}},
                {"_class": "shapePath", "name": "Triangle", "frame": frame(0, 200, 20, 20), "isClosed": true,
                 "points": [
                    {"point": "{0.5, 0}", "hasCurveFrom": false, "hasCurveTo": false},
                    {"point": "{1, 1}", "hasCurveFrom": false, "hasCurveTo": false},
                    {"point": "{0, 1}", "hasCurveFrom": false, "hasCurveTo": false}
                 ],
                 "style": {"fills": [fill(rgba(1.0, 0.0, 0.0))]}},
                {"_class": "oval", "name": "Dot", "frame": frame(50, 200, 10, 10), "rotation": 90},
                {"_class": "text", "name": "Title", "frame": frame(16, 300, 200, 30), "textBehaviour": 1,
                 "attributedString": {"string": "Hi 👋 there", "attributes": [
                    {"location": 0, "length": 3, "attributes": {"MSAttributedStringFontAttribute": {"attributes": {"name": "OpenSans-Bold", "size": 24}}}},
                    {"location": 3, "length": 8, "attributes": {"MSAttributedStringFontAttribute": {"attributes": {"name": "OpenSans-Italic", "size": 24}}, "paragraphStyle": {"alignment": 2}}}
                 ]}},
                {"_class": "symbolInstance", "name": "Buy button", "symbolID": "sym-1", "frame": frame(16, 400, 100, 40),
                 "overrideValues": [{"overrideName": "label_stringValue", "value": "Order"}]},
                {"_class": "mystery", "name": "?", "frame": frame(0, 0, 1, 1)}
             ]},
            master
        ]
    });
    let document = json!({"_class": "document", "pages": [{"_class": "MSJSONFileReference", "_ref": "pages/P1"}]});
    ude_format_zip::write_stored(&[
        ("document.json", document.to_string().as_bytes()),
        ("meta.json", b"{}"),
        ("pages/P1.json", page.to_string().as_bytes()),
    ])
}

#[test]
fn imports_artboards_shapes_text_and_symbols() {
    let im = import(&sample(), "app.sketch").unwrap();
    let doc = &im.doc;
    let page = doc.children(doc.root())[0];
    assert_eq!(doc.strings.resolve(doc.node(page).unwrap().name), "Home");
    let top = doc.children(page);
    assert_eq!(top.len(), 2, "artboard + symbol master");
    let board = doc.node(top[0]).unwrap();
    assert_eq!(board.transform.translation(), Point::new(100.0, 50.0));
    assert!(board.flags.contains(NodeFlags::CLIPS_CONTENT));

    let kids = doc.children(top[0]);
    assert_eq!(kids.len(), 5, "unknown layer skipped");
    let card = doc.node(kids[0]).unwrap();
    assert_eq!(card.corner_radius, 12.0);
    assert_eq!(card.opacity, 0.5);
    assert_eq!(card.blend, BlendMode::Multiply);
    let ext = card.ext.as_ref().unwrap();
    assert_eq!(ext.strokes.as_ref().unwrap().align, StrokeAlign::Inside);
    assert!(matches!(ext.effects[0], Effect::DropShadow(_)));

    let NodeKind::Vector(v) = &doc.node(kids[1]).unwrap().kind else {
        panic!("triangle")
    };
    let b = v.paths[0].path.bounds();
    assert_eq!((b.x0, b.y0, b.x1, b.y1), (0.0, 0.0, 20.0, 20.0));

    // Rotated 90° about its centre: the origin moves to the box's bottom-left.
    let dot = doc.node(kids[2]).unwrap();
    let o = dot.transform.apply(Point::new(0.0, 0.0));
    assert!(
        (o.x - 50.0).abs() < 1e-3 && (o.y - 210.0).abs() < 1e-3,
        "{o:?}"
    );

    let NodeKind::Text(t) = &doc.node(kids[3]).unwrap().kind else {
        panic!("text")
    };
    let run = |i: usize| &t.content[t.runs[i].range.start as usize..t.runs[i].range.end as usize];
    assert_eq!(run(0), "Hi ");
    assert_eq!(run(1), "\u{1f44b} there");
    assert_eq!(doc.strings.resolve(t.styles[0].family), "Open Sans");
    assert_eq!(t.styles[0].weight, 700);
    assert!(t.styles[1].italic);
    assert_eq!(t.align_h, TextAlignH::Center);

    let inst = doc.node(kids[4]).unwrap();
    let NodeKind::Instance(data) = &inst.kind else {
        panic!("instance")
    };
    assert_eq!(data.component, ComponentRef::Local(top[1]));
    let label = doc.children(kids[4])[0];
    let NodeKind::Text(t) = &doc.node(label).unwrap().kind else {
        panic!("label")
    };
    assert_eq!(&*t.content, "Order", "text override applied");
    assert!(
        im.warnings.iter().any(|w| w.contains("mystery")),
        "{:?}",
        im.warnings
    );
}

#[test]
fn rejects_bad_input() {
    assert!(import(b"nope", "x").is_err());
    assert!(import(&ude_format_zip::write_stored(&[("a", b"b")]), "x").is_err());
}
