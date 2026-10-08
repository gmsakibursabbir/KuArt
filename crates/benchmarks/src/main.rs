//! `ude-bench [sizes...]`: model/format benchmarks on synthetic documents.
//! Writes `target/bench/m1.json`. Process RAM/VRAM/FPS scenarios arrive with the GUI (M13).

use std::time::Instant;

use ude_bench::{SynthSpec, generate};
use ude_core::{Affine, HeapSize, History, NodeKind, Op, Prop, Transaction};
use ude_format_native::{Limits, load, save};

struct Row {
    nodes: u32,
    generate_ms: f64,
    model_bytes: usize,
    bytes_per_node: f64,
    save_ms: f64,
    file_bytes: usize,
    load_ms: f64,
    resave_identical: bool,
    move500_commit_ms: f64,
    undo500_ms: f64,
}

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}

fn run(nodes: u32) -> Row {
    let t = Instant::now();
    let mut doc = generate(SynthSpec { nodes, seed: 42 });
    let generate_ms = ms(t);
    // Memory is measured on the loaded document: that is what a user has after opening a file.

    let t = Instant::now();
    let bytes = save(&doc).expect("save");
    let save_ms = ms(t);

    let t = Instant::now();
    let loaded = load(&bytes, &Limits::default()).expect("load");
    let load_ms = ms(t);
    let resave_identical = save(&loaded).expect("resave") == bytes;
    let model_bytes = loaded.memory_report().total();
    let bytes_per_node =
        loaded.heap_size() as f64 / loaded.descendants(loaded.root()).count() as f64;

    // Undo cost for moving 500 nodes in one transaction.
    let targets: Vec<_> = doc
        .descendants(doc.root())
        .filter(|&id| !doc.node(id).is_some_and(|n| n.kind.is_container()))
        .take(500)
        .collect();
    let mut history = History::default();
    let t = Instant::now();
    let mut tx = Transaction::new("Move");
    for &id in &targets {
        let moved =
            Affine::translate(10.0, 10.0) * doc.node(id).map_or(Affine::IDENTITY, |n| n.transform);
        tx.apply(
            &mut doc,
            Op::Set {
                id,
                value: Prop::Transform(moved),
            },
        )
        .expect("move");
    }
    history.commit(&mut doc, tx);
    let move500_commit_ms = ms(t);
    let t = Instant::now();
    history.undo(&mut doc).expect("undo");
    let undo500_ms = ms(t);

    assert!(
        doc.node(doc.root())
            .is_some_and(|n| matches!(n.kind, NodeKind::Document))
    );
    Row {
        nodes,
        generate_ms,
        model_bytes,
        bytes_per_node,
        save_ms,
        file_bytes: bytes.len(),
        load_ms,
        resave_identical,
        move500_commit_ms,
        undo500_ms,
    }
}

fn main() {
    let sizes: Vec<u32> = {
        let args: Vec<u32> = std::env::args()
            .skip(1)
            .filter_map(|a| a.replace('_', "").parse().ok())
            .collect();
        if args.is_empty() {
            vec![1_000, 5_000, 10_000, 50_000, 100_000]
        } else {
            args
        }
    };
    if cfg!(debug_assertions) {
        eprintln!("warning: debug build; use `cargo run --release -p ude-bench` for real numbers");
    }

    println!(
        "{:>8} {:>9} {:>10} {:>8} {:>9} {:>10} {:>9} {:>9} {:>10} {:>9}",
        "nodes",
        "gen ms",
        "model MB",
        "B/node",
        "save ms",
        "file KB",
        "load ms",
        "resave",
        "move500ms",
        "undo ms"
    );
    let mut json = String::from("[\n");
    for (i, &n) in sizes.iter().enumerate() {
        let r = run(n);
        println!(
            "{:>8} {:>9.1} {:>10.2} {:>8.0} {:>9.1} {:>10.1} {:>9.1} {:>9} {:>10.2} {:>9.2}",
            r.nodes,
            r.generate_ms,
            r.model_bytes as f64 / 1048576.0,
            r.bytes_per_node,
            r.save_ms,
            r.file_bytes as f64 / 1024.0,
            r.load_ms,
            if r.resave_identical {
                "identical"
            } else {
                "DIFFERS"
            },
            r.move500_commit_ms,
            r.undo500_ms
        );
        json += &format!(
            "  {{\"nodes\":{},\"generate_ms\":{:.3},\"model_bytes\":{},\"bytes_per_node\":{:.1},\"save_ms\":{:.3},\"file_bytes\":{},\"load_ms\":{:.3},\"resave_identical\":{},\"move500_commit_ms\":{:.3},\"undo500_ms\":{:.3}}}{}\n",
            r.nodes,
            r.generate_ms,
            r.model_bytes,
            r.bytes_per_node,
            r.save_ms,
            r.file_bytes,
            r.load_ms,
            r.resave_identical,
            r.move500_commit_ms,
            r.undo500_ms,
            if i + 1 < sizes.len() { "," } else { "" }
        );
        if !r.resave_identical {
            std::process::exit(1);
        }
    }
    json += "]\n";
    let dir = std::path::Path::new("target/bench");
    if std::fs::create_dir_all(dir).is_ok() && std::fs::write(dir.join("m1.json"), json).is_ok() {
        println!("wrote target/bench/m1.json");
    }
}
