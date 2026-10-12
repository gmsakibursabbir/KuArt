//! Minimal ZIP reader for design containers (.fig, .xd): stored and deflated entries, no ZIP64.

use std::io::Read;

pub struct Zip<'a> {
    bytes: &'a [u8],
    entries: Vec<Entry>,
}

struct Entry {
    name: String,
    method: u16,
    offset: usize,
    compressed: usize,
    size: usize,
}

fn u16_at(b: &[u8], i: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(i..i + 2)?.try_into().ok()?))
}

fn u32_at(b: &[u8], i: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(i..i + 4)?.try_into().ok()?))
}

impl<'a> Zip<'a> {
    pub fn is_zip(bytes: &[u8]) -> bool {
        bytes.starts_with(b"PK\x03\x04")
    }

    pub fn open(bytes: &'a [u8]) -> Result<Self, String> {
        let bad = || "corrupt ZIP container".to_string();
        // End of central directory: last 22 bytes plus an optional comment (≤ 64 KiB).
        let from = bytes.len().saturating_sub(22 + 0xFFFF);
        let eocd = (from..bytes.len().saturating_sub(21))
            .rev()
            .find(|&i| u32_at(bytes, i) == Some(0x0605_4b50))
            .ok_or_else(bad)?;
        let count = usize::from(u16_at(bytes, eocd + 10).ok_or_else(bad)?);
        let mut p = u32_at(bytes, eocd + 16).ok_or_else(bad)? as usize;
        let mut entries = Vec::with_capacity(count);
        for _ in 0..count {
            if u32_at(bytes, p) != Some(0x0201_4b50) {
                return Err(bad());
            }
            let method = u16_at(bytes, p + 10).ok_or_else(bad)?;
            let compressed = u32_at(bytes, p + 20).ok_or_else(bad)?;
            let size = u32_at(bytes, p + 24).ok_or_else(bad)?;
            if compressed == u32::MAX || size == u32::MAX {
                return Err("ZIP64 containers are not supported".into());
            }
            let name_len = usize::from(u16_at(bytes, p + 28).ok_or_else(bad)?);
            let extra_len = usize::from(u16_at(bytes, p + 30).ok_or_else(bad)?);
            let comment_len = usize::from(u16_at(bytes, p + 32).ok_or_else(bad)?);
            let local = u32_at(bytes, p + 42).ok_or_else(bad)? as usize;
            let name =
                String::from_utf8_lossy(bytes.get(p + 46..p + 46 + name_len).ok_or_else(bad)?)
                    .into_owned();
            if u32_at(bytes, local) != Some(0x0403_4b50) {
                return Err(bad());
            }
            let lname = usize::from(u16_at(bytes, local + 26).ok_or_else(bad)?);
            let lextra = usize::from(u16_at(bytes, local + 28).ok_or_else(bad)?);
            entries.push(Entry {
                name,
                method,
                offset: local + 30 + lname + lextra,
                compressed: compressed as usize,
                size: size as usize,
            });
            p += 46 + name_len + extra_len + comment_len;
        }
        Ok(Self { bytes, entries })
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.entries.iter().map(|e| e.name.as_str())
    }

    /// Entry contents, at most `limit` bytes.
    pub fn read(&self, name: &str, limit: usize) -> Result<Option<Vec<u8>>, String> {
        let Some(e) = self.entries.iter().find(|e| e.name == name) else {
            return Ok(None);
        };
        if e.size > limit {
            return Err(format!("{name} is too large"));
        }
        let data = self
            .bytes
            .get(e.offset..e.offset + e.compressed)
            .ok_or("truncated ZIP entry")?;
        match e.method {
            0 => Ok(Some(data.to_vec())),
            8 => {
                let mut out = Vec::with_capacity(e.size);
                flate2::read::DeflateDecoder::new(data)
                    .take(limit as u64)
                    .read_to_end(&mut out)
                    .map_err(|err| format!("{name}: {err}"))?;
                Ok(Some(out))
            }
            m => Err(format!("{name}: unsupported ZIP compression {m}")),
        }
    }
}

/// Writes an uncompressed ZIP (no CRCs). For building test fixtures.
pub fn write_stored(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut central = Vec::new();
    for (name, data) in entries {
        let offset = out.len() as u32;
        let header = |sig: u32, central: bool| {
            let mut h = sig.to_le_bytes().to_vec();
            if central {
                h.extend([20, 0]); // made by
            }
            h.extend([20, 0, 0, 0, 0, 0, 0, 0, 0, 0]); // version, flags, method, time, date
            h.extend(0u32.to_le_bytes()); // crc
            h.extend((data.len() as u32).to_le_bytes());
            h.extend((data.len() as u32).to_le_bytes());
            h.extend((name.len() as u16).to_le_bytes());
            h.extend(0u16.to_le_bytes()); // extra length
            h
        };
        out.extend(header(0x0403_4b50, false));
        out.extend(name.as_bytes());
        out.extend(*data);
        central.extend(header(0x0201_4b50, true));
        central.extend([0u8; 6]); // comment length, disk, internal attributes
        central.extend(0u32.to_le_bytes()); // external attributes
        central.extend(offset.to_le_bytes());
        central.extend(name.as_bytes());
    }
    let cd_offset = out.len() as u32;
    out.extend(&central);
    out.extend(0x0605_4b50u32.to_le_bytes());
    out.extend([0u8; 4]);
    out.extend((entries.len() as u16).to_le_bytes());
    out.extend((entries.len() as u16).to_le_bytes());
    out.extend((central.len() as u32).to_le_bytes());
    out.extend(cd_offset.to_le_bytes());
    out.extend([0u8; 2]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_stored() {
        let z = write_stored(&[("a.txt", b"hello"), ("dir/b", b"")]);
        let zip = Zip::open(&z).unwrap();
        assert_eq!(zip.names().collect::<Vec<_>>(), ["a.txt", "dir/b"]);
        assert_eq!(zip.read("a.txt", 100).unwrap().unwrap(), b"hello");
        assert!(zip.read("missing", 100).unwrap().is_none());
        assert!(zip.read("a.txt", 2).is_err(), "size limit");
        assert!(Zip::open(b"PK\x03\x04 broken").is_err());
    }
}
