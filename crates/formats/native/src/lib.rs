//! Native `.design` format: chunked, versioned, deterministic, zstd-compressed,
//! content-hashed. See `docs/format-strategy.md` §8.
//!
//! Chunk layout (in file order):
//! `META` · `STRS` · `ASET` · `ROOT` · `PAGE`×pages · `BLOB`×assets · preserved unknown chunks.

mod container;
mod model;
pub mod wire;

use std::fmt;
use std::path::Path;
use std::sync::Arc;

use ude_core::assets::{AssetEntry, AssetKind, AssetSource, Codec, ContentHash};
use ude_core::{DocError, Document, Node, NodeId, NodeKind, Op, Symbol};

pub use container::{ChunkCodec, ChunkInfo};
use container::{OutChunk, read_chunk, read_table};
use model::{Dec, Enc};
use wire::{Reader, WireEnum, Writer};

const T_META: [u8; 4] = *b"META";
const T_STRS: [u8; 4] = *b"STRS";
const T_ASET: [u8; 4] = *b"ASET";
const T_ROOT: [u8; 4] = *b"ROOT";
const T_PAGE: [u8; 4] = *b"PAGE";
const T_BLOB: [u8; 4] = *b"BLOB";
/// Smallest possible encoded node record; bounds forged node counts.
const MIN_NODE_BYTES: u64 = 40;

/// Resource limits for loading untrusted files.
#[derive(Clone, Debug)]
pub struct Limits {
    pub max_nodes: u32,
    pub max_chunks: usize,
    pub max_chunk_bytes: u64,
    /// Nesting of node kinds inside instance overrides.
    pub max_depth: u32,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_nodes: 5_000_000,
            max_chunks: 1 << 20,
            max_chunk_bytes: 4 << 30,
            max_depth: 64,
        }
    }
}

#[derive(Debug)]
pub enum NativeError {
    Io(std::io::Error),
    BadMagic,
    UnsupportedVersion(u16),
    Truncated,
    Corrupt(&'static str),
    ChecksumMismatch([u8; 4]),
    LimitExceeded(&'static str),
    MissingChunk(&'static str),
    /// An asset still points into an unloaded source file and cannot be embedded.
    UnresolvedAsset,
    Doc(DocError),
}

impl fmt::Display for NativeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NativeError::Io(e) => write!(f, "I/O error: {e}"),
            NativeError::BadMagic => write!(f, "not a .design file"),
            NativeError::UnsupportedVersion(v) => {
                write!(f, "file format version {v} is newer than this app supports")
            }
            NativeError::Truncated => write!(f, "file is truncated"),
            NativeError::Corrupt(what) => write!(f, "file is corrupt ({what})"),
            NativeError::ChecksumMismatch(tag) => {
                write!(
                    f,
                    "checksum mismatch in {} chunk",
                    String::from_utf8_lossy(tag)
                )
            }
            NativeError::LimitExceeded(what) => {
                write!(f, "file exceeds the safety limit for {what}")
            }
            NativeError::MissingChunk(tag) => write!(f, "required {tag} chunk is missing"),
            NativeError::UnresolvedAsset => {
                write!(f, "an asset references an unloaded source file")
            }
            NativeError::Doc(e) => write!(f, "invalid document structure: {e}"),
        }
    }
}

impl std::error::Error for NativeError {}

impl From<DocError> for NativeError {
    fn from(e: DocError) -> Self {
        NativeError::Doc(e)
    }
}

impl From<std::io::Error> for NativeError {
    fn from(e: std::io::Error) -> Self {
        NativeError::Io(e)
    }
}

/// Serializes the attached part of `doc` (history and detached nodes are not saved).
pub fn save(doc: &Document) -> Result<Vec<u8>, NativeError> {
    let root = doc.root();
    let order: Vec<NodeId> = doc.descendants(root).collect();
    let mut enc = Enc::new(&order);
    let node = |id| doc.node(id).ok_or(NativeError::Doc(DocError::NotFound(id)));

    let mut root_w = Writer::default();
    enc.node(&mut root_w, node(root)?);

    let pages = doc.children(root);
    let mut page_chunks = Vec::with_capacity(pages.len());
    for &page in pages {
        let mut w = Writer::default();
        for id in doc.descendants(page) {
            enc.node(&mut w, node(id)?);
        }
        page_chunks.push(OutChunk {
            tag: T_PAGE,
            data: w.buf,
            compress: true,
        });
    }

    let mut strs = Writer::default();
    strs.len(enc.sym_order.len());
    for &s in &enc.sym_order {
        strs.str(doc.strings.resolve(s));
    }

    let mut aset = Writer::default();
    aset.len(enc.asset_order.len());
    let mut blobs = Vec::with_capacity(enc.asset_order.len());
    for &id in &enc.asset_order {
        let entry = doc.assets.get(id).ok_or(NativeError::Corrupt("asset id"))?;
        let AssetSource::Memory { bytes, codec } = &entry.source else {
            return Err(NativeError::UnresolvedAsset);
        };
        aset.record(|w| {
            w.raw(&entry.hash.0);
            match entry.kind {
                AssetKind::Image { width, height } => {
                    w.u8(0);
                    w.var(u64::from(width));
                    w.var(u64::from(height));
                }
                AssetKind::Font => w.u8(1),
                AssetKind::Blob => w.u8(2),
            }
            w.u8(codec.code());
        });
        let already_compressed = matches!(
            codec,
            Codec::Png | Codec::Jpeg | Codec::WebP | Codec::Zlib | Codec::Zstd
        );
        blobs.push(OutChunk {
            tag: T_BLOB,
            data: bytes.to_vec(),
            compress: !already_compressed,
        });
    }

    let mut meta = Writer::default();
    meta.record(|w| {
        w.str(&doc.meta.title);
        w.bool(doc.meta.source.is_some());
        if let Some(s) = doc.meta.source {
            w.u8(s.code());
        }
        w.len(order.len());
        w.len(pages.len());
    });

    let mut chunks = vec![
        OutChunk {
            tag: T_META,
            data: meta.buf,
            compress: true,
        },
        OutChunk {
            tag: T_STRS,
            data: strs.buf,
            compress: true,
        },
        OutChunk {
            tag: T_ASET,
            data: aset.buf,
            compress: true,
        },
        OutChunk {
            tag: T_ROOT,
            data: root_w.buf,
            compress: true,
        },
    ];
    chunks.extend(page_chunks);
    chunks.extend(blobs);
    for (tag, data) in &doc.meta.preserved_sections {
        chunks.push(OutChunk {
            tag: *tag,
            data: data.to_vec(),
            compress: true,
        });
    }
    container::write(&chunks)
}

pub fn load(file: &[u8], limits: &Limits) -> Result<Document, NativeError> {
    let table = read_table(file, limits)?;
    let find = |tag: [u8; 4], name: &'static str| {
        table
            .iter()
            .find(|c| c.tag == tag)
            .ok_or(NativeError::MissingChunk(name))
    };

    let mut doc = Document::new();

    // META
    let meta_raw = read_chunk(file, find(T_META, "META")?)?;
    let mut meta = Reader::new(&meta_raw).record()?;
    doc.meta.title = meta.str()?.to_owned();
    doc.meta.source = if meta.bool()? {
        Some(WireEnum::from_code(meta.u8()?)?)
    } else {
        None
    };
    let node_count = meta.u32()?;
    let page_count = meta.u32()?;

    let tree_bytes: u64 = table
        .iter()
        .filter(|c| c.tag == T_ROOT || c.tag == T_PAGE)
        .map(|c| c.raw_len)
        .sum();
    if node_count == 0 || node_count > limits.max_nodes {
        return Err(NativeError::LimitExceeded("node count"));
    }
    if u64::from(node_count) * MIN_NODE_BYTES > tree_bytes {
        return Err(NativeError::Corrupt("node count larger than tree data"));
    }

    // STRS
    let strs_raw = read_chunk(file, find(T_STRS, "STRS")?)?;
    let mut r = Reader::new(&strs_raw);
    let n = r.count(1)?;
    let mut syms = Vec::with_capacity(n);
    for _ in 0..n {
        syms.push(doc.strings.intern(r.str()?));
    }

    // ASET + BLOBs (asset i is the i-th BLOB chunk)
    let aset_raw = read_chunk(file, find(T_ASET, "ASET")?)?;
    let mut r = Reader::new(&aset_raw);
    let n = r.count(34)?;
    let mut blob_chunks = table.iter().filter(|c| c.tag == T_BLOB);
    let mut assets = Vec::with_capacity(n);
    for _ in 0..n {
        let mut e = r.record()?;
        let hash = ContentHash(e.array()?);
        let kind = match e.u8()? {
            0 => AssetKind::Image {
                width: e.u32()?,
                height: e.u32()?,
            },
            1 => AssetKind::Font,
            2 => AssetKind::Blob,
            _ => return Err(NativeError::Corrupt("asset kind")),
        };
        let codec = Codec::from_code(e.u8()?)?;
        let blob = blob_chunks
            .next()
            .ok_or(NativeError::MissingChunk("BLOB"))?;
        let bytes: Arc<[u8]> = read_chunk(file, blob)?.into();
        assets.push(doc.assets.insert(AssetEntry {
            hash,
            kind,
            source: AssetSource::Memory { bytes, codec },
        }));
    }

    // Reserve every node id up front so references (e.g. instance → component) resolve
    // regardless of order; records then fill them in.
    let root = doc.root();
    let mut ids = Vec::with_capacity(node_count as usize);
    ids.push(root);
    for _ in 1..node_count {
        ids.push(doc.create(Node::new(NodeKind::Group, Symbol::EMPTY)));
    }
    let mut dec = Dec::new(syms, assets, ids, limits.max_depth);

    // ROOT
    let root_raw = read_chunk(file, find(T_ROOT, "ROOT")?)?;
    let (root_node, root_children) = dec.node(&mut Reader::new(&root_raw))?;
    if root_children != page_count {
        return Err(NativeError::Corrupt("page count"));
    }
    doc.fill(root, root_node)?;

    // PAGEs: pre-order records with child counts.
    let mut next = 1usize;
    let mut pages_seen = 0u32;
    for chunk in table.iter().filter(|c| c.tag == T_PAGE) {
        pages_seen += 1;
        if pages_seen > page_count {
            return Err(NativeError::Corrupt("page count"));
        }
        let raw = read_chunk(file, chunk)?;
        let mut r = Reader::new(&raw);
        let mut stack: Vec<(NodeId, u32)> = Vec::new();
        let mut started = false;
        while !r.is_empty() {
            let parent = match stack.last_mut() {
                Some((p, remaining)) => {
                    *remaining -= 1;
                    *p
                }
                None if !started => root,
                None => return Err(NativeError::Corrupt("extra records after page")),
            };
            started = true;
            let id = *dec
                .ids
                .get(next)
                .ok_or(NativeError::Corrupt("more nodes than declared"))?;
            next += 1;
            let (node, children) = dec.node(&mut r)?;
            doc.fill(id, node)?;
            let index = u32::try_from(doc.children(parent).len())
                .map_err(|_| NativeError::Corrupt("child count"))?;
            doc.apply(Op::Attach { id, parent, index })?;
            if children > 0 {
                stack.push((id, children));
            }
            while stack.last().is_some_and(|&(_, rem)| rem == 0) {
                stack.pop();
            }
        }
        if !stack.is_empty() || !started {
            return Err(NativeError::Corrupt("incomplete page"));
        }
    }
    if pages_seen != page_count || next != node_count as usize {
        return Err(NativeError::Corrupt("node count mismatch"));
    }

    const KNOWN: [[u8; 4]; 6] = [T_META, T_STRS, T_ASET, T_ROOT, T_PAGE, T_BLOB];
    for c in table.iter().filter(|c| !KNOWN.contains(&c.tag)) {
        doc.meta
            .preserved_sections
            .push((c.tag, read_chunk(file, c)?.into()));
    }

    let _ = doc.take_changes();
    doc.shrink_to_fit();
    Ok(doc)
}

/// Header and chunk table only; does not decode payloads.
pub fn inspect(file: &[u8]) -> Result<Vec<ChunkInfo>, NativeError> {
    read_table(file, &Limits::default())
}

pub fn save_to_path(doc: &Document, path: &Path) -> Result<(), NativeError> {
    let bytes = save(doc)?;
    // Write to a sibling temp file, then rename, so a crash never leaves a half-written document.
    let tmp = path.with_extension("design.tmp");
    std::fs::write(&tmp, &bytes)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

pub fn load_from_path(path: &Path, limits: &Limits) -> Result<Document, NativeError> {
    let bytes = std::fs::read(path)?;
    load(&bytes, limits)
}
