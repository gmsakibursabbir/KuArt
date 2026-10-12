//! Comments survive a save/load round trip; files without comments carry no CMNT chunk.

use ude_core::comments::{Comment, Reply};
use ude_core::{Document, Node, NodeKind, Symbol};
use ude_format_native::{Limits, inspect, load, save};

#[test]
fn comments_round_trip() {
    let mut doc = Document::new();
    let root = doc.root();
    doc.build_child(root, Node::new(NodeKind::Page, Symbol::EMPTY))
        .unwrap();
    let plain = save(&doc).unwrap();
    assert!(
        inspect(&plain)
            .unwrap()
            .iter()
            .all(|c| c.tag_str() != "CMNT")
    );

    doc.comments.push(Comment {
        page: 0,
        x: 12.5,
        y: -3.0,
        author: "Ana".into(),
        text: "Make the CTA larger \u{1f44d}".into(),
        created: 1_790_000_000,
        resolved: true,
        replies: vec![Reply {
            author: "Ben".into(),
            text: "Done".into(),
            created: 1_790_000_100,
        }],
        source: "PDF annotation".into(),
    });
    let bytes = save(&doc).unwrap();
    let back = load(&bytes, &Limits::default()).unwrap();
    assert_eq!(back.comments, doc.comments);
    assert!(
        back.meta.preserved_sections.is_empty(),
        "CMNT is understood, not preserved"
    );
    assert_eq!(save(&back).unwrap(), bytes, "deterministic");
}
