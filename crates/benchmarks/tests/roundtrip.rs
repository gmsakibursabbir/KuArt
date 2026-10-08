//! M1 acceptance: deterministic, byte-identical `.design` round trips at scale.
#![allow(clippy::unwrap_used)]

use ude_bench::{SynthSpec, generate};
use ude_core::{Affine, History, Op, Prop, Transaction};
use ude_format_native::{Limits, load, save};

#[test]
fn synthetic_round_trip_is_byte_identical() {
    let doc = generate(SynthSpec {
        nodes: 20_000,
        seed: 1,
    });
    let a = save(&doc).unwrap();
    let loaded = load(&a, &Limits::default()).unwrap();
    assert_eq!(loaded.descendants(loaded.root()).count(), 20_002);
    assert_eq!(save(&loaded).unwrap(), a);
}

#[test]
fn same_content_same_bytes_regardless_of_history() {
    let fresh = generate(SynthSpec {
        nodes: 2_000,
        seed: 9,
    });

    // Same document, but edited and undone, with a deleted-then-purged node in between:
    // arena slots and interner contents differ, the saved bytes must not.
    let mut edited = generate(SynthSpec {
        nodes: 2_000,
        seed: 9,
    });
    let mut history = History::default();
    let target = edited.children(edited.children(edited.root())[0])[0];
    let mut tx = Transaction::new("Move");
    tx.apply(
        &mut edited,
        Op::Set {
            id: target,
            value: Prop::Transform(Affine::translate(1.0, 2.0)),
        },
    )
    .unwrap();
    history.commit(&mut edited, tx);
    edited.strings.intern("unused string");
    history.undo(&mut edited).unwrap();
    history.clear(&mut edited);

    assert_eq!(save(&fresh).unwrap(), save(&edited).unwrap());
}

#[test]
fn corrupted_files_fail_cleanly() {
    let bytes = save(&generate(SynthSpec {
        nodes: 500,
        seed: 3,
    }))
    .unwrap();
    let limits = Limits::default();
    // Flip one byte at many positions and truncate at many lengths: never panic.
    for pos in (0..bytes.len()).step_by(97) {
        let mut b = bytes.clone();
        b[pos] ^= 0x5a;
        let _ = load(&b, &limits);
    }
    for len in (0..bytes.len()).step_by(131) {
        assert!(load(&bytes[..len], &limits).is_err());
    }
}
