//! Byte-level encoding: LEB128 varints, little-endian floats, length-prefixed blobs.
//! The reader is bounds-checked everywhere; malformed input yields `Err`, never a panic.

use crate::NativeError;

#[derive(Default)]
pub struct Writer {
    pub buf: Vec<u8>,
}

impl Writer {
    pub fn u8(&mut self, v: u8) {
        self.buf.push(v);
    }

    pub fn bool(&mut self, v: bool) {
        self.buf.push(v as u8);
    }

    pub fn var(&mut self, mut v: u64) {
        loop {
            let byte = (v & 0x7f) as u8;
            v >>= 7;
            if v == 0 {
                self.buf.push(byte);
                return;
            }
            self.buf.push(byte | 0x80);
        }
    }

    pub fn len(&mut self, n: usize) {
        self.var(n as u64);
    }

    pub fn f32(&mut self, v: f32) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    pub fn f32s(&mut self, vs: &[f32]) {
        for &v in vs {
            self.f32(v);
        }
    }

    pub fn raw(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
    }

    pub fn bytes(&mut self, bytes: &[u8]) {
        self.len(bytes.len());
        self.raw(bytes);
    }

    pub fn str(&mut self, s: &str) {
        self.bytes(s.as_bytes());
    }

    /// Writes `f`'s output as a length-prefixed record, so older readers can skip
    /// fields appended by newer writers.
    pub fn record(&mut self, f: impl FnOnce(&mut Writer)) {
        let mut inner = Writer::default();
        f(&mut inner);
        self.bytes(&inner.buf);
    }
}

#[derive(Clone)]
pub struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    pub fn is_empty(&self) -> bool {
        self.pos >= self.data.len()
    }

    pub fn remaining(&self) -> usize {
        self.data.len().saturating_sub(self.pos)
    }

    pub fn take(&mut self, n: usize) -> Result<&'a [u8], NativeError> {
        let end = self.pos.checked_add(n).ok_or(NativeError::Truncated)?;
        let s = self.data.get(self.pos..end).ok_or(NativeError::Truncated)?;
        self.pos = end;
        Ok(s)
    }

    pub fn array<const N: usize>(&mut self) -> Result<[u8; N], NativeError> {
        let mut a = [0u8; N];
        a.copy_from_slice(self.take(N)?);
        Ok(a)
    }

    pub fn u8(&mut self) -> Result<u8, NativeError> {
        Ok(self.take(1)?[0])
    }

    pub fn bool(&mut self) -> Result<bool, NativeError> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(NativeError::Corrupt("bool")),
        }
    }

    pub fn var(&mut self) -> Result<u64, NativeError> {
        let mut v = 0u64;
        for shift in (0..64).step_by(7) {
            let b = self.u8()?;
            v |= u64::from(b & 0x7f) << shift;
            if b & 0x80 == 0 {
                return Ok(v);
            }
        }
        Err(NativeError::Corrupt("varint too long"))
    }

    pub fn u32(&mut self) -> Result<u32, NativeError> {
        u32::try_from(self.var()?).map_err(|_| NativeError::Corrupt("u32 out of range"))
    }

    pub fn u16(&mut self) -> Result<u16, NativeError> {
        u16::try_from(self.var()?).map_err(|_| NativeError::Corrupt("u16 out of range"))
    }

    /// A length that must also fit in the remaining input (each item ≥ `min_item` bytes),
    /// which stops forged counts from triggering huge allocations.
    pub fn count(&mut self, min_item: usize) -> Result<usize, NativeError> {
        let n = usize::try_from(self.var()?).map_err(|_| NativeError::Truncated)?;
        if n.saturating_mul(min_item.max(1)) > self.remaining() {
            return Err(NativeError::Truncated);
        }
        Ok(n)
    }

    pub fn f32(&mut self) -> Result<f32, NativeError> {
        Ok(f32::from_le_bytes(self.array()?))
    }

    pub fn f32s<const N: usize>(&mut self) -> Result<[f32; N], NativeError> {
        let mut out = [0.0; N];
        for v in &mut out {
            *v = self.f32()?;
        }
        Ok(out)
    }

    pub fn bytes(&mut self) -> Result<&'a [u8], NativeError> {
        let n = self.count(1)?;
        self.take(n)
    }

    pub fn str(&mut self) -> Result<&'a str, NativeError> {
        std::str::from_utf8(self.bytes()?).map_err(|_| NativeError::Corrupt("utf-8"))
    }

    /// Reads a length-prefixed record and returns a reader over exactly its bytes.
    pub fn record(&mut self) -> Result<Reader<'a>, NativeError> {
        Ok(Reader::new(self.bytes()?))
    }
}

/// Stable wire codes for unit enums. The variant order is the on-disk contract:
/// append new variants at the end, never reorder.
pub trait WireEnum: Sized + Copy + PartialEq + 'static {
    const VARIANTS: &'static [Self];
    const NAME: &'static str;

    fn code(self) -> u8 {
        // Exhaustiveness is checked by the macro, so the variant is always listed.
        self.code_opt().unwrap_or(u8::MAX)
    }

    fn code_opt(self) -> Option<u8> {
        Self::VARIANTS
            .iter()
            .position(|v| *v == self)
            .map(|i| i as u8)
    }

    fn from_code(c: u8) -> Result<Self, NativeError> {
        Self::VARIANTS
            .get(c as usize)
            .copied()
            .ok_or(NativeError::Corrupt(Self::NAME))
    }
}

#[macro_export]
#[doc(hidden)]
macro_rules! wire_enum {
    ($ty:ident { $($var:ident),* $(,)? }) => {
        impl $crate::wire::WireEnum for $ty {
            const VARIANTS: &'static [Self] = &[$($ty::$var),*];
            const NAME: &'static str = stringify!($ty);
        }
        const _: () = {
            // Compile error if a variant is added to the core enum but not listed here.
            #[allow(dead_code)]
            fn exhaustive(v: $ty) {
                match v {
                    $($ty::$var)|* => {}
                }
            }
        };
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn varint_round_trip() {
        let mut w = Writer::default();
        for v in [0u64, 1, 127, 128, 300, u32::MAX as u64, u64::MAX] {
            w.var(v);
        }
        let mut r = Reader::new(&w.buf);
        for v in [0u64, 1, 127, 128, 300, u32::MAX as u64, u64::MAX] {
            assert_eq!(r.var().unwrap(), v);
        }
        assert!(r.is_empty());
    }

    #[test]
    fn truncated_input_is_an_error() {
        let mut r = Reader::new(&[0x80]);
        assert!(r.var().is_err());
        let mut r = Reader::new(&[5, 1, 2]);
        assert!(r.bytes().is_err());
        let mut r = Reader::new(&[0xff, 0xff, 0xff, 0xff, 0x0f]);
        assert!(r.count(1).is_err(), "forged count larger than input");
    }
}
