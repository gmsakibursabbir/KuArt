//! Layer channel data: the compact blob stored as a [`Codec::PsdChannels`] asset, and its
//! decoder (raw, RLE/PackBits, ZIP, ZIP with prediction; 8 and 16 bits per channel).
//!
//! [`Codec::PsdChannels`]: ude_core::assets::Codec::PsdChannels

use std::io::Read;

use crate::{ChannelRef, Fmt, IRect, LayerMaskInfo, Reader};

const MAGIC: &[u8; 4] = b"KPC1";

/// Packs one layer's channels (still compressed as in the file) with what decoding needs.
/// Layout (little-endian): magic, width, height, left, top, depth, flags, mask?, channels.
pub(crate) fn encode_blob(
    rect: IRect,
    channels: &[ChannelRef],
    mask: Option<&LayerMaskInfo>,
    fmt: Fmt,
) -> Vec<u8> {
    let mut out = Vec::with_capacity(64 + channels.iter().map(|c| c.data.len() + 8).sum::<usize>());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&rect.width().to_le_bytes());
    out.extend_from_slice(&rect.height().to_le_bytes());
    out.extend_from_slice(&rect.left.to_le_bytes());
    out.extend_from_slice(&rect.top.to_le_bytes());
    out.extend_from_slice(&fmt.depth.to_le_bytes());
    out.push(u8::from(fmt.psb) | (u8::from(fmt.gray) << 1));
    match mask {
        Some(m) => {
            out.push(1);
            for v in [m.rect.top, m.rect.left, m.rect.bottom, m.rect.right] {
                out.extend_from_slice(&v.to_le_bytes());
            }
            out.push(m.default_color);
            out.push(u8::from(m.disabled));
        }
        None => out.push(0),
    }
    // Only colour, transparency and the user mask are needed to draw the layer.
    let used: Vec<&ChannelRef> = channels.iter().filter(|c| matches!(c.id, -2..=2)).collect();
    out.extend_from_slice(&(used.len() as u16).to_le_bytes());
    for c in used {
        out.extend_from_slice(&c.id.to_le_bytes());
        out.extend_from_slice(&c.compression.to_le_bytes());
        out.extend_from_slice(&(c.data.len() as u32).to_le_bytes());
        out.extend_from_slice(c.data);
    }
    out
}

/// Little-endian cursor for the blob.
struct Le<'a>(&'a [u8], usize);

impl<'a> Le<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        let end = self
            .1
            .checked_add(n)
            .filter(|&e| e <= self.0.len())
            .ok_or("corrupt PSD layer data")?;
        let s = &self.0[self.1..end];
        self.1 = end;
        Ok(s)
    }
    fn arr<const N: usize>(&mut self) -> Result<[u8; N], String> {
        self.take(N)?
            .try_into()
            .map_err(|_| "corrupt PSD layer data".into())
    }
    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16, String> {
        Ok(u16::from_le_bytes(self.arr()?))
    }
    fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_le_bytes(self.arr()?))
    }
    fn i32(&mut self) -> Result<i32, String> {
        Ok(i32::from_le_bytes(self.arr()?))
    }
}

/// Decodes a [`encode_blob`] blob to straight-alpha RGBA8 (width, height, pixels), with the
/// raster layer mask applied.
pub fn decode_rgba(blob: &[u8]) -> Result<(u32, u32, Vec<u8>), String> {
    let mut r = Le(blob, 0);
    if r.take(4)? != MAGIC {
        return Err("not PSD layer data".into());
    }
    let (w, h) = (r.u32()?, r.u32()?);
    let (left, top) = (r.i32()?, r.i32()?);
    let depth = r.u16()?;
    let flags = r.u8()?;
    let (psb, gray) = (flags & 1 != 0, flags & 2 != 0);
    let mask = if r.u8()? == 1 {
        let rect = IRect {
            top: r.i32()?,
            left: r.i32()?,
            bottom: r.i32()?,
            right: r.i32()?,
        };
        let default_color = r.u8()?;
        let disabled = r.u8()? != 0;
        Some(LayerMaskInfo {
            rect,
            default_color,
            disabled,
        })
    } else {
        None
    };
    let n = r.u16()?;
    if u64::from(w) * u64::from(h) > crate::MAX_PIXELS {
        return Err("layer is too large".into());
    }
    let len = w as usize * h as usize;
    let mut out = vec![0u8; len * 4];
    out.iter_mut().skip(3).step_by(4).for_each(|a| *a = 255);
    let mut mask_plane = None;
    for _ in 0..n {
        let id = r.u16()? as i16;
        let compression = r.u16()?;
        let size = r.u32()? as usize;
        let data = r.take(size)?;
        if id == -2 {
            if let Some(m) = &mask {
                mask_plane = Some(decode_plane(
                    compression,
                    data,
                    m.rect.width(),
                    m.rect.height(),
                    depth,
                    psb,
                )?);
            }
            continue;
        }
        let plane = decode_plane(compression, data, w, h, depth, psb)?;
        let offsets: &[usize] = match (id, gray) {
            (0, true) => &[0, 1, 2],
            (0, false) => &[0],
            (1, false) => &[1],
            (2, false) => &[2],
            (-1, _) => &[3],
            _ => &[],
        };
        for &o in offsets {
            for (px, &v) in out.chunks_exact_mut(4).zip(&plane) {
                px[o] = v;
            }
        }
    }
    if let (Some(m), Some(plane)) = (&mask, &mask_plane)
        && !m.disabled
    {
        let (mw, mh) = (i64::from(m.rect.width()), i64::from(m.rect.height()));
        for (i, px) in out.chunks_exact_mut(4).enumerate() {
            let x = (i % w as usize) as i64 + i64::from(left) - i64::from(m.rect.left);
            let y = (i / w as usize) as i64 + i64::from(top) - i64::from(m.rect.top);
            let v = if x >= 0 && y >= 0 && x < mw && y < mh {
                plane[(y * mw + x) as usize]
            } else {
                m.default_color
            };
            px[3] = ((u16::from(px[3]) * u16::from(v) + 127) / 255) as u8;
        }
    }
    Ok((w, h, out))
}

/// One channel → 8-bit plane of `w * h` bytes.
pub(crate) fn decode_plane(
    compression: u16,
    data: &[u8],
    w: u32,
    h: u32,
    depth: u16,
    psb: bool,
) -> Result<Vec<u8>, String> {
    let bpc = usize::from(depth / 8).max(1);
    let row = w as usize * bpc;
    let total = row * h as usize;
    let bytes = match compression {
        0 => data.get(..total).ok_or("truncated raw channel")?.to_vec(),
        1 => {
            let count_size = if psb { 4 } else { 2 };
            let counts_len = h as usize * count_size;
            let counts = data.get(..counts_len).ok_or("truncated RLE channel")?;
            let mut src = &data[counts_len..];
            let mut out = Vec::with_capacity(total);
            for c in counts.chunks_exact(count_size) {
                let n = if psb {
                    u32::from_be_bytes([c[0], c[1], c[2], c[3]]) as usize
                } else {
                    usize::from(u16::from_be_bytes([c[0], c[1]]))
                };
                let (line, rest) = src.split_at(n.min(src.len()));
                unpack_bits(line, row, &mut out);
                src = rest;
            }
            out
        }
        2 | 3 => {
            let mut out = Vec::with_capacity(total);
            flate2::read::ZlibDecoder::new(data)
                .take(total as u64)
                .read_to_end(&mut out)
                .map_err(|e| format!("ZIP channel: {e}"))?;
            out.resize(total, 0);
            if compression == 3 {
                unpredict(&mut out, row, bpc)?;
            }
            out
        }
        c => return Err(format!("unknown channel compression {c}")),
    };
    Ok(if bpc == 2 {
        // 16-bit big-endian: keep the high byte.
        bytes.chunks_exact(2).map(|p| p[0]).collect()
    } else {
        bytes
    })
}

/// PackBits: decodes one row to exactly `len` bytes (padding or truncating corrupt rows).
fn unpack_bits(src: &[u8], len: usize, out: &mut Vec<u8>) {
    let start = out.len();
    let mut i = 0;
    while i < src.len() && out.len() - start < len {
        let n = src[i] as i8;
        i += 1;
        if n >= 0 {
            let end = (i + n as usize + 1).min(src.len());
            out.extend_from_slice(&src[i..end]);
            i = end;
        } else if n != -128 {
            if let Some(&b) = src.get(i) {
                out.extend(std::iter::repeat_n(b, (1 - isize::from(n)) as usize));
            }
            i += 1;
        }
    }
    out.resize(start + len, 0);
}

/// Undoes ZIP-with-prediction (per-row deltas).
fn unpredict(buf: &mut [u8], row: usize, bpc: usize) -> Result<(), String> {
    if row == 0 {
        return Ok(());
    }
    for line in buf.chunks_exact_mut(row) {
        match bpc {
            1 => {
                for i in 1..line.len() {
                    line[i] = line[i].wrapping_add(line[i - 1]);
                }
            }
            2 => {
                let mut prev = 0u16;
                for p in line.chunks_exact_mut(2) {
                    prev = prev.wrapping_add(u16::from_be_bytes([p[0], p[1]]));
                    p.copy_from_slice(&prev.to_be_bytes());
                }
            }
            _ => return Err("32-bit predicted channels are not supported".into()),
        }
    }
    Ok(())
}

/// The merged (flattened) image at the end of the file, for files without layers.
pub(crate) fn decode_merged(
    data: &[u8],
    w: u32,
    h: u32,
    channels: u16,
    fmt: Fmt,
) -> Result<Vec<u8>, String> {
    let mut r = Reader::new(data);
    let compression = r.u16()?;
    let colour = if fmt.gray { 1 } else { 3 };
    // Extra channels in a flat file are saved selections, not transparency.
    let n = colour;
    let bpc = usize::from(fmt.depth / 8);
    let row = w as usize * bpc;
    let plane_bytes = row * h as usize;
    let mut planes = Vec::with_capacity(n);
    match compression {
        0 => {
            for _ in 0..n {
                planes.push(decode_plane(
                    0,
                    r.take(plane_bytes)?,
                    w,
                    h,
                    fmt.depth,
                    fmt.psb,
                )?);
            }
        }
        1 => {
            // All row counts come first, then each channel's rows.
            let count_size = if fmt.psb { 4 } else { 2 };
            let all = usize::from(channels) * h as usize;
            let counts = r.take(all * count_size)?;
            for c in 0..n {
                let mut rows = Vec::with_capacity(plane_bytes);
                for y in 0..h as usize {
                    let k = (c * h as usize + y) * count_size;
                    let len = if fmt.psb {
                        u32::from_be_bytes([counts[k], counts[k + 1], counts[k + 2], counts[k + 3]])
                            as usize
                    } else {
                        usize::from(u16::from_be_bytes([counts[k], counts[k + 1]]))
                    };
                    unpack_bits(r.take(len)?, row, &mut rows);
                }
                planes.push(decode_plane(0, &rows, w, h, fmt.depth, fmt.psb)?);
            }
        }
        c => return Err(format!("unsupported merged image compression {c}")),
    }
    let len = w as usize * h as usize;
    let mut out = vec![255u8; len * 4];
    for (i, px) in out.chunks_exact_mut(4).enumerate() {
        if fmt.gray {
            px[..3].fill(planes[0][i]);
        } else {
            for c in 0..3 {
                px[c] = planes[c][i];
            }
        }
        if let Some(a) = planes.get(colour) {
            px[3] = a[i];
        }
    }
    Ok(out)
}
