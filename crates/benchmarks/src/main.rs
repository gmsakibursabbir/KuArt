//! `ude-bench [sizes...]`: model, format (M1) and spatial/selection (M3) benchmarks on
//! synthetic documents. Writes `target/bench/m1.json` and `m3.json`.

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

    println!(
        "\n{:>8} {:>10} {:>10} {:>12} {:>12} {:>13}",
        "nodes", "index ms", "index MB", "hit avg µs", "hit max µs", "marquee ms"
    );
    let mut json = String::from("[\n");
    for (i, &n) in sizes.iter().enumerate() {
        let s = spatial(n);
        println!(
            "{:>8} {:>10.1} {:>10.2} {:>12.1} {:>12.1} {:>13.3}",
            n,
            s.build_ms,
            s.index_bytes as f64 / 1048576.0,
            s.hit_avg_us,
            s.hit_max_us,
            s.marquee_ms
        );
        json += &format!(
            "  {{\"nodes\":{n},\"index_build_ms\":{:.3},\"index_bytes\":{},\"hit_avg_us\":{:.2},\"hit_max_us\":{:.2},\"marquee_ms\":{:.4}}}{}\n",
            s.build_ms,
            s.index_bytes,
            s.hit_avg_us,
            s.hit_max_us,
            s.marquee_ms,
            if i + 1 < sizes.len() { "," } else { "" }
        );
    }
    json += "]\n";
    if std::fs::write(dir.join("m3.json"), json).is_ok() {
        println!("wrote target/bench/m3.json");
    }
}

struct SpatialRow {
    build_ms: f64,
    index_bytes: usize,
    hit_avg_us: f64,
    hit_max_us: f64,
    marquee_ms: f64,
}

/// M3 acceptance: click-to-select (hit test + selection rule) under 1 ms at 100k nodes.
fn spatial(nodes: u32) -> SpatialRow {
    use ude_editor::{SelectionMode, hit_test, marquee_select, resolve_target};
    let doc = generate(SynthSpec { nodes, seed: 42 });
    let page = doc.children(doc.root())[0];
    let t = Instant::now();
    let index = ude_spatial::SpatialIndex::build(&doc, page);
    let build_ms = ms(t);

    // Random clicks over the occupied area (frames are laid out 10 per row, 1600×1200 apart).
    let frames = nodes.div_ceil(100).max(1);
    let (w, h) = (
        16_000.0_f64.min(f64::from(frames) * 1600.0),
        f64::from(frames.div_ceil(10)) * 1200.0,
    );
    let mut rng = ude_bench::Rng::new(7);
    let (mut total, mut max) = (0.0, 0.0f64);
    const CLICKS: u32 = 2000;
    for _ in 0..CLICKS {
        let (x, y) = (
            f64::from(rng.range(0.0, w as f32)),
            f64::from(rng.range(0.0, h as f32)),
        );
        let t = Instant::now();
        let hit = hit_test(&doc, &index, x, y, 2.0);
        let target = hit.and_then(|id| resolve_target(&doc, page, id, SelectionMode::GroupFirst));
        let us = t.elapsed().as_secs_f64() * 1e6;
        std::hint::black_box(target);
        total += us;
        max = max.max(us);
    }

    let t = Instant::now();
    const MARQUEES: u32 = 200;
    for _ in 0..MARQUEES {
        let (x, y) = (rng.range(0.0, w as f32), rng.range(0.0, h as f32));
        let rect = ude_core::Rect::new(x, y, x + 300.0, y + 300.0);
        std::hint::black_box(marquee_select(
            &doc,
            &index,
            rect,
            SelectionMode::GroupFirst,
        ));
    }
    let marquee_ms = ms(t) / f64::from(MARQUEES);

    SpatialRow {
        build_ms,
        index_bytes: index.heap_bytes(),
        hit_avg_us: total / f64::from(CLICKS),
        hit_max_us: max,
        marquee_ms,
    }
}
