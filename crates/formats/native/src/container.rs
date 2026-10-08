//! Chunked container: header, chunk table, then chunk payloads.
//!
//! ```text
//! "UDESIGN\0" | version u16 LE | flags u16 LE | chunk count u32 LE
//! table entry (61 B): tag [4] | offset u64 | stored len u64 | raw len u64 | codec u8 | blake3(raw) [32]
//! payloads
//! ```

use crate::{Limits, NativeError};

pub const MAGIC: &[u8; 8] = b"UDESIGN\0";
pub const VERSION: u16 = 0;
const HEADER_LEN: usize = 16;
const ENTRY_LEN: usize = 61;
/// Chunks smaller than this are stored uncompressed.
const MIN_COMPRESS: usize = 64;
const ZSTD_LEVEL: i32 = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChunkCodec {
    Stored = 0,
    Zstd = 1,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChunkInfo {
    pub tag: [u8; 4],
    pub offset: u64,
    pub stored_len: u64,
    pub raw_len: u64,
    pub codec: ChunkCodec,
    pub hash: [u8; 32],
}

impl ChunkInfo {
    pub fn tag_str(&self) -> String {
        String::from_utf8_lossy(&self.tag).into_owned()
    }
}

/// A chunk to be written. `compress: false` is for payloads that are already compressed.
pub struct OutChunk {
    pub tag: [u8; 4],
    pub data: Vec<u8>,
    pub compress: bool,
}

pub fn write(chunks: &[OutChunk]) -> Result<Vec<u8>, NativeError> {
    let mut stored: Vec<(ChunkCodec, std::borrow::Cow<'_, [u8]>)> =
        Vec::with_capacity(chunks.len());
    for c in chunks {
        if c.compress && c.data.len() >= MIN_COMPRESS {
            let z = zstd::bulk::compress(&c.data, ZSTD_LEVEL).map_err(NativeError::Io)?;
            if z.len() < c.data.len() {
                stored.push((ChunkCodec::Zstd, z.into()));
                continue;
            }
        }
        stored.push((ChunkCodec::Stored, c.data.as_slice().into()));
    }

    let table_end = HEADER_LEN + ENTRY_LEN * chunks.len();
    let total = table_end + stored.iter().map(|(_, s)| s.len()).sum::<usize>();
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&VERSION.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    let count =
        u32::try_from(chunks.len()).map_err(|_| NativeError::LimitExceeded("chunk count"))?;
    out.extend_from_slice(&count.to_le_bytes());

    let mut offset = table_end as u64;
    for (c, (codec, s)) in chunks.iter().zip(&stored) {
        out.extend_from_slice(&c.tag);
        out.extend_from_slice(&offset.to_le_bytes());
        out.extend_from_slice(&(s.len() as u64).to_le_bytes());
        out.extend_from_slice(&(c.data.len() as u64).to_le_bytes());
        out.push(*codec as u8);
        out.extend_from_slice(blake3::hash(&c.data).as_bytes());
        offset += s.len() as u64;
    }
    for (_, s) in &stored {
        out.extend_from_slice(s);
    }
    Ok(out)
}

/// Parses and validates the header and chunk table without touching payloads.
pub fn read_table(file: &[u8], limits: &Limits) -> Result<Vec<ChunkInfo>, NativeError> {
    let header = file.get(..HEADER_LEN).ok_or(NativeError::Truncated)?;
    if &header[..8] != MAGIC {
        return Err(NativeError::BadMagic);
    }
    let version = u16::from_le_bytes([header[8], header[9]]);
    if version > VERSION {
        return Err(NativeError::UnsupportedVersion(version));
    }
    let count = u32::from_le_bytes([header[12], header[13], header[14], header[15]]) as usize;
    if count > limits.max_chunks {
        return Err(NativeError::LimitExceeded("chunk count"));
    }
    let table = file
        .get(HEADER_LEN..HEADER_LEN + count * ENTRY_LEN)
        .ok_or(NativeError::Truncated)?;

    let le64 = |b: &[u8]| u64::from_le_bytes(b.try_into().unwrap_or([0; 8]));
    let mut out = Vec::with_capacity(count);
    for e in table.chunks_exact(ENTRY_LEN) {
        let info = ChunkInfo {
            tag: [e[0], e[1], e[2], e[3]],
            offset: le64(&e[4..12]),
            stored_len: le64(&e[12..20]),
            raw_len: le64(&e[20..28]),
            codec: match e[28] {
                0 => ChunkCodec::Stored,
                1 => ChunkCodec::Zstd,
                _ => return Err(NativeError::Corrupt("chunk codec")),
            },
            hash: e[29..61].try_into().unwrap_or([0; 32]),
        };
        let end = info
            .offset
            .checked_add(info.stored_len)
            .ok_or(NativeError::Truncated)?;
        if end > file.len() as u64 {
            return Err(NativeError::Truncated);
        }
        if info.raw_len > limits.max_chunk_bytes {
            return Err(NativeError::LimitExceeded("chunk size"));
        }
        if info.codec == ChunkCodec::Stored && info.raw_len != info.stored_len {
            return Err(NativeError::Corrupt("stored chunk length"));
        }
        out.push(info);
    }
    Ok(out)
}

/// Decompresses and verifies one chunk.
pub fn read_chunk(file: &[u8], info: &ChunkInfo) -> Result<Vec<u8>, NativeError> {
    let start = usize::try_from(info.offset).map_err(|_| NativeError::Truncated)?;
    let len = usize::try_from(info.stored_len).map_err(|_| NativeError::Truncated)?;
    let stored = file.get(start..start + len).ok_or(NativeError::Truncated)?;
    let raw_len = usize::try_from(info.raw_len).map_err(|_| NativeError::Truncated)?;
    let raw = match info.codec {
        ChunkCodec::Stored => stored.to_vec(),
        // Capacity = declared size: output beyond it is rejected (decompression-bomb guard).
        ChunkCodec::Zstd => {
            zstd::bulk::decompress(stored, raw_len).map_err(|_| NativeError::Corrupt("zstd"))?
        }
    };
    if raw.len() != raw_len {
        return Err(NativeError::Corrupt("chunk length"));
    }
    if blake3::hash(&raw).as_bytes() != &info.hash {
        return Err(NativeError::ChecksumMismatch(info.tag));
    }
    Ok(raw)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_corruption() {
        let chunks = vec![
            OutChunk {
                tag: *b"AAAA",
                data: vec![1, 2, 3],
                compress: true,
            },
            OutChunk {
                tag: *b"BBBB",
                data: vec![7; 10_000],
                compress: true,
            },
        ];
        let file = write(&chunks).unwrap();
        let limits = Limits::default();
        let table = read_table(&file, &limits).unwrap();
        assert_eq!(table[1].codec, ChunkCodec::Zstd);
        assert_eq!(read_chunk(&file, &table[0]).unwrap(), vec![1, 2, 3]);
        assert_eq!(read_chunk(&file, &table[1]).unwrap(), vec![7; 10_000]);

        let mut bad = file.clone();
        let last = table[0].offset as usize;
        bad[last] ^= 0xff;
        assert!(matches!(
            read_chunk(&bad, &table[0]),
            Err(NativeError::ChecksumMismatch(_))
        ));

        assert!(matches!(
            read_table(&file[..20], &limits),
            Err(NativeError::Truncated)
        ));
        assert!(matches!(
            read_table(b"NOTADESIGNFILE!!", &limits),
            Err(NativeError::BadMagic)
        ));
    }
}
