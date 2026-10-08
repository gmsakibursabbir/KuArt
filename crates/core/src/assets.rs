//! Content-addressed, lazily decoded assets. Pixels never live in the model.

use std::collections::HashMap;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AssetId(u32);

/// blake3 digest, computed by importers (the core stays dependency-free).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ContentHash(pub [u8; 32]);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SourceFileId(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Codec {
    Png,
    Jpeg,
    WebP,
    /// One PSD layer's channels (raw/RLE/ZIP), decoded per tile.
    PsdChannels,
    Raw,
    Zlib,
    Zstd,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssetKind {
    Image {
        width: u32,
        height: u32,
    },
    Font,
    /// Opaque bytes, e.g. preserved source data or embedded smart-object files.
    Blob,
}

#[derive(Clone, Debug)]
pub enum AssetSource {
    Memory(Arc<[u8]>),
    /// Byte range inside a memory-mapped source file.
    FileRange {
        file: SourceFileId,
        offset: u64,
        len: u64,
        codec: Codec,
    },
}

#[derive(Clone, Debug)]
pub struct AssetEntry {
    pub hash: ContentHash,
    pub kind: AssetKind,
    pub source: AssetSource,
}

#[derive(Default)]
pub struct AssetStore {
    entries: Vec<AssetEntry>,
    by_hash: HashMap<ContentHash, AssetId>,
}

impl AssetStore {
    /// Inserts or returns the existing id for identical content.
    pub fn insert(&mut self, entry: AssetEntry) -> AssetId {
        if let Some(&id) = self.by_hash.get(&entry.hash) {
            return id;
        }
        let id = AssetId(u32::try_from(self.entries.len()).expect("too many assets"));
        self.by_hash.insert(entry.hash, id);
        self.entries.push(entry);
        id
    }

    pub fn get(&self, id: AssetId) -> Option<&AssetEntry> {
        self.entries.get(id.0 as usize)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Metadata plus in-memory payloads; file-backed bytes are not resident.
    pub fn heap_bytes(&self) -> usize {
        let inline: usize = self
            .entries
            .iter()
            .map(|e| match &e.source {
                AssetSource::Memory(b) => b.len() / Arc::strong_count(b),
                AssetSource::FileRange { .. } => 0,
            })
            .sum();
        inline
            + self.entries.capacity() * size_of::<AssetEntry>()
            + self.by_hash.capacity() * (size_of::<ContentHash>() + size_of::<AssetId>() + 8)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dedups_by_hash() {
        let mut store = AssetStore::default();
        let e = AssetEntry {
            hash: ContentHash([7; 32]),
            kind: AssetKind::Blob,
            source: AssetSource::Memory(Arc::from(&b"abc"[..])),
        };
        let a = store.insert(e.clone());
        let b = store.insert(e);
        assert_eq!(a, b);
        assert_eq!(store.len(), 1);
    }
}
