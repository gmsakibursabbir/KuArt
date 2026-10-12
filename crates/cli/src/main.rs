//! `design-editor-cli`: headless operations. Never starts the GUI or touches the GPU.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Instant;

use ude_core::{Document, NodeKind};
use ude_format_native::{Limits, inspect, load, save_to_path};

mod mcp;

/// Like `println!`, but a closed pipe (`... | head`) ends output quietly instead of panicking.
macro_rules! out {
    ($($arg:tt)*) => {{
        use std::io::Write;
        let _ = writeln!(std::io::stdout().lock(), $($arg)*);
    }};
}

const USAGE: &str = "\
design-editor-cli: headless tools for KuArt (.krt) documents

USAGE:
    design-editor-cli inspect <file>
    design-editor-cli convert <input> <output>
    design-editor-cli generate <output.krt> [--nodes N] [--seed S]
    design-editor-cli code <input> <html|css|tailwind|react|vue|tokens> [output]
    design-editor-cli mcp              (MCP server on stdio, for AI agents)
    design-editor-cli --version

Input: .krt (and legacy .design), .fig/.jam/.deck/.buzz/.site/.make, .sketch, .xd, .ai, .pdf, .psd/.psb, .svg and images (.png .jpg .webp).
Output: .krt, .png, .jpg, .webp or .svg (first page), .pdf (one page per artboard).";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("inspect") if args.len() == 2 => cmd_inspect(Path::new(&args[1])),
        Some("convert") if args.len() == 3 => cmd_convert(Path::new(&args[1]), Path::new(&args[2])),
        Some("generate") if args.len() >= 2 => cmd_generate(&args[1..]),
        Some("mcp") if args.len() == 1 => mcp::run(),
        Some("code") if (3..=4).contains(&args.len()) => {
            cmd_code(Path::new(&args[1]), &args[2], args.get(3).map(Path::new))
        }
        Some("--version" | "-V") => {
            out!("design-editor-cli {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Some("--help" | "-h" | "help") => {
            out!("{USAGE}");
            Ok(())
        }
        _ => {
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

type CliResult = Result<(), String>;

fn extension(path: &Path) -> String {
    path.extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
}

fn open(path: &Path) -> Result<Document, String> {
    match extension(path).as_str() {
        "krt" | "design" => {
            let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
            load(&bytes, &Limits::default()).map_err(|e| format!("{}: {e}", path.display()))
        }
        "png" | "jpg" | "jpeg" | "webp" => {
            let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
            let name = path
                .file_name()
                .map_or_else(|| "image".into(), |n| n.to_string_lossy().into_owned());
            ude_format_raster::import(bytes, &name).map_err(|e| format!("{}: {e}", path.display()))
        }
        "svg" => {
            let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
            let name = path
                .file_name()
                .map_or_else(|| "drawing".into(), |n| n.to_string_lossy().into_owned());
            let im = ude_format_svg_import::import(&bytes, &name)
                .map_err(|e| format!("{}: {e}", path.display()))?;
            for w in &im.warnings {
                eprintln!("warning: {w}");
            }
            Ok(im.doc)
        }
        "psd" | "psb" => {
            let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
            let name = path
                .file_name()
                .map_or_else(|| "document".into(), |n| n.to_string_lossy().into_owned());
            let im = ude_format_psd::import(&bytes, &name)
                .map_err(|e| format!("{}: {e}", path.display()))?;
            for w in &im.warnings {
                eprintln!("warning: {w}");
            }
            Ok(im.doc)
        }
        "fig" | "jam" | "deck" | "buzz" | "site" | "make" => {
            let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
            let name = path
                .file_name()
                .map_or_else(|| "document".into(), |n| n.to_string_lossy().into_owned());
            let im = ude_format_fig::import(&bytes, &name)
                .map_err(|e| format!("{}: {e}", path.display()))?;
            for w in &im.warnings {
                eprintln!("warning: {w}");
            }
            Ok(im.doc)
        }
        "sketch" => {
            let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
            let name = path
                .file_name()
                .map_or_else(|| "document".into(), |n| n.to_string_lossy().into_owned());
            let im = ude_format_sketch::import(&bytes, &name)
                .map_err(|e| format!("{}: {e}", path.display()))?;
            for w in &im.warnings {
                eprintln!("warning: {w}");
            }
            Ok(im.doc)
        }
        "ai" | "pdf" => {
            let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
            let name = path
                .file_name()
                .map_or_else(|| "document".into(), |n| n.to_string_lossy().into_owned());
            let ai = extension(path) == "ai";
            let im = ude_format_pdf::import(&bytes, &name, ai)
                .map_err(|e| format!("{}: {e}", path.display()))?;
            for w in &im.warnings {
                eprintln!("warning: {w}");
            }
            Ok(im.doc)
        }
        "xd" => {
            let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
            let name = path
                .file_name()
                .map_or_else(|| "document".into(), |n| n.to_string_lossy().into_owned());
            let im = ude_format_xd::import(&bytes, &name)
                .map_err(|e| format!("{}: {e}", path.display()))?;
            for w in &im.warnings {
                eprintln!("warning: {w}");
            }
            Ok(im.doc)
        }
        other => Err(format!("unsupported input format '.{other}'")),
    }
}

fn cmd_inspect(path: &Path) -> CliResult {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if ude_format_native::is_native_extension(&extension(path)) {
        let table = inspect(&bytes).map_err(|e| e.to_string())?;
        out!(
            "{}  ({} bytes, {} chunks)",
            path.display(),
            bytes.len(),
            table.len()
        );
        // One row per chunk kind (a document can have thousands of BLOB chunks).
        let mut rows: Vec<(String, usize, u64, u64)> = Vec::new();
        for c in &table {
            let tag = c.tag_str();
            match rows.iter_mut().find(|r| r.0 == tag) {
                Some(r) => {
                    r.1 += 1;
                    r.2 += c.stored_len;
                    r.3 += c.raw_len;
                }
                None => rows.push((tag, 1, c.stored_len, c.raw_len)),
            }
        }
        out!(
            "  {:<6} {:>6} {:>12} {:>12}",
            "chunk",
            "count",
            "stored",
            "raw"
        );
        for (tag, count, stored, raw) in rows {
            out!("  {tag:<6} {count:>6} {stored:>12} {raw:>12}");
        }
    }
    let t = Instant::now();
    let doc = open(path)?;
    let elapsed = t.elapsed();
    summarize(&doc);
    out!("\nloaded in {:.1} ms", elapsed.as_secs_f64() * 1000.0);
    Ok(())
}

fn summarize(doc: &Document) {
    let mut kinds: BTreeMap<&'static str, usize> = BTreeMap::new();
    for id in doc.descendants(doc.root()) {
        if let Some(n) = doc.node(id) {
            *kinds.entry(kind_name(&n.kind)).or_default() += 1;
        }
    }
    out!("\ndocument \"{}\"", doc.meta.title);
    out!("  pages   {}", doc.children(doc.root()).len());
    out!("  nodes   {}", kinds.values().sum::<usize>());
    for (k, n) in &kinds {
        out!("    {k:<14}{n:>8}");
    }
    out!("  assets  {}", doc.assets.len());
    out!("  strings {}", doc.strings.len());
    if !doc.meta.preserved_sections.is_empty() {
        out!(
            "  preserved sections from newer versions: {}",
            doc.meta.preserved_sections.len()
        );
    }
    out!("\n{}", doc.memory_report());
}

fn kind_name(k: &NodeKind) -> &'static str {
    match k {
        NodeKind::Document => "document",
        NodeKind::Page => "page",
        NodeKind::Frame => "frame",
        NodeKind::Group => "group",
        NodeKind::Rectangle => "rectangle",
        NodeKind::Ellipse(_) => "ellipse",
        NodeKind::Polygon { .. } => "polygon",
        NodeKind::Star { .. } => "star",
        NodeKind::Line => "line",
        NodeKind::Vector(_) => "vector",
        NodeKind::Text(_) => "text",
        NodeKind::Raster { .. } => "raster",
        NodeKind::Component(_) => "component",
        NodeKind::ComponentSet => "component set",
        NodeKind::Instance(_) => "instance",
        NodeKind::BooleanOp(_) => "boolean op",
        NodeKind::Slice => "slice",
        NodeKind::Preserved { .. } => "preserved",
    }
}

fn cmd_convert(input: &Path, output: &Path) -> CliResult {
    let doc = open(input)?;
    match extension(output).as_str() {
        "krt" | "design" => save_to_path(&doc, output).map_err(|e| e.to_string())?,
        ext @ ("png" | "jpg" | "jpeg" | "webp" | "svg") => {
            // Exports the first page.
            let page = *doc
                .children(doc.root())
                .first()
                .ok_or("document has no pages")?;
            let bytes = match ude_renderer::RasterFormat::from_extension(ext) {
                Some(format) => ude_renderer::export_raster(
                    &doc,
                    page,
                    1.0,
                    format,
                    &mut ude_renderer::RenderCaches::default(),
                )?,
                None => {
                    let mut text = ude_renderer::TextEngine::default();
                    ude_format_svg::export_with(&doc, page, &mut |t, size| {
                        text.lines(t, &doc.strings, size)
                    })?
                    .into_bytes()
                }
            };
            std::fs::write(output, bytes).map_err(|e| format!("{}: {e}", output.display()))?;
        }
        "pdf" => {
            let targets = pdf_targets(&doc);
            if targets.is_empty() {
                return Err("document has no pages".into());
            }
            let (bytes, notes) = ude_renderer::export_pdf(
                &doc,
                &targets,
                &mut ude_renderer::RenderCaches::default(),
            )?;
            for n in notes {
                eprintln!("note: {n}");
            }
            std::fs::write(output, bytes).map_err(|e| format!("{}: {e}", output.display()))?;
        }
        other => return Err(format!("unsupported output format '.{other}'")),
    }
    out!("wrote {}", output.display());
    Ok(())
}

fn cmd_generate(args: &[String]) -> CliResult {
    let mut out: Option<PathBuf> = None;
    let (mut nodes, mut seed) = (1000u32, 42u64);
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--nodes" => {
                nodes = it
                    .next()
                    .and_then(|v| v.replace('_', "").parse().ok())
                    .ok_or("--nodes needs a number")?
            }
            "--seed" => {
                seed = it
                    .next()
                    .and_then(|v| v.parse().ok())
                    .ok_or("--seed needs a number")?
            }
            path if out.is_none() => out = Some(PathBuf::from(path)),
            other => return Err(format!("unexpected argument '{other}'")),
        }
    }
    let out = out.ok_or("missing output path")?;
    if !ude_format_native::is_native_extension(&extension(&out)) {
        return Err("output must be a .krt file".into());
    }
    let doc = ude_bench::generate(ude_bench::SynthSpec { nodes, seed });
    save_to_path(&doc, &out).map_err(|e| e.to_string())?;
    out!("wrote {} ({nodes} nodes, seed {seed})", out.display());
    Ok(())
}

/// Design → code for the first page (`docs/dev-mode.md`).
fn cmd_code(input: &Path, target: &str, output: Option<&Path>) -> CliResult {
    let target = ude_codegen::Target::parse(target).ok_or_else(|| {
        format!("unknown target '{target}' (html, css, tailwind, react, vue, tokens)")
    })?;
    let doc = open(input)?;
    let page = *doc
        .children(doc.root())
        .first()
        .ok_or("document has no pages")?;
    let out = ude_codegen::generate(&doc, page, target)?;
    for note in &out.notes {
        eprintln!("note: {note}");
    }
    match output {
        Some(path) => {
            std::fs::write(path, out.code).map_err(|e| format!("{}: {e}", path.display()))?;
            eprintln!("wrote {}", path.display());
        }
        None => out!("{}", out.code),
    }
    Ok(())
}

/// One PDF page per artboard (top-level frame); a page without artboards becomes one PDF page.
pub(crate) fn pdf_targets(doc: &Document) -> Vec<ude_core::NodeId> {
    let mut out = Vec::new();
    for &page in doc.children(doc.root()) {
        let frames: Vec<_> = doc
            .children(page)
            .iter()
            .copied()
            .filter(|&id| doc.is_artboard(id))
            .collect();
        if frames.is_empty() {
            out.push(page);
        } else {
            out.extend(frames);
        }
    }
    out
}
