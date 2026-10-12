//! Kiwi (github.com/evanw/kiwi) binary schema and schema-driven decoding. Figma stores the
//! schema in every file, so decoding follows whatever version wrote the file.

use std::collections::HashMap;

pub struct Schema {
    defs: Vec<Def>,
    by_name: HashMap<String, usize>,
}

struct Def {
    kind: u8,
    fields: Vec<Field>,
}

struct Field {
    name: String,
    ty: i32,
    array: bool,
    value: u32,
}

const ENUM: u8 = 0;
const STRUCT: u8 = 1;
const MESSAGE: u8 = 2;
const MAX_DEPTH: usize = 96;

/// A decoded value. Objects keep schema field names (borrowed from the schema).
#[derive(Debug, Clone)]
pub enum Value<'s> {
    Bool(bool),
    Int(i64),
    Uint(u64),
    Float(f32),
    Str(String),
    Bytes(Vec<u8>),
    Enum(&'s str),
    Array(Vec<Value<'s>>),
    Object(Vec<(&'s str, Value<'s>)>),
}

impl<'s> Value<'s> {
    pub fn get(&self, key: &str) -> Option<&Value<'s>> {
        match self {
            Value::Object(fields) => fields.iter().find(|(k, _)| *k == key).map(|(_, v)| v),
            _ => None,
        }
    }
    pub fn f32(&self, key: &str) -> Option<f32> {
        match self.get(key)? {
            Value::Float(f) => Some(*f),
            Value::Int(i) => Some(*i as f32),
            Value::Uint(u) => Some(*u as f32),
            _ => None,
        }
    }
    pub fn u64(&self, key: &str) -> Option<u64> {
        match self.get(key)? {
            Value::Uint(u) => Some(*u),
            Value::Int(i) => u64::try_from(*i).ok(),
            _ => None,
        }
    }
    pub fn bool(&self, key: &str) -> Option<bool> {
        match self.get(key)? {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }
    pub fn str(&self, key: &str) -> Option<&str> {
        match self.get(key)? {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }
    pub fn enum_(&self, key: &str) -> Option<&'s str> {
        match self.get(key)? {
            Value::Enum(e) => Some(e),
            _ => None,
        }
    }
    pub fn array(&self, key: &str) -> &[Value<'s>] {
        match self.get(key) {
            Some(Value::Array(a)) => a,
            _ => &[],
        }
    }
    pub fn bytes(&self, key: &str) -> Option<&[u8]> {
        match self.get(key)? {
            Value::Bytes(b) => Some(b),
            _ => None,
        }
    }
}

pub struct Reader<'b> {
    b: &'b [u8],
    pos: usize,
}

impl<'b> Reader<'b> {
    pub fn new(b: &'b [u8]) -> Self {
        Self { b, pos: 0 }
    }
    fn byte(&mut self) -> Result<u8, String> {
        let v = *self.b.get(self.pos).ok_or("unexpected end of Kiwi data")?;
        self.pos += 1;
        Ok(v)
    }
    fn remaining(&self) -> usize {
        self.b.len() - self.pos
    }
    fn take(&mut self, n: usize) -> Result<&'b [u8], String> {
        if n > self.remaining() {
            return Err("unexpected end of Kiwi data".into());
        }
        let s = &self.b[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }
    pub fn uint(&mut self) -> Result<u32, String> {
        let mut value = 0u32;
        for shift in (0..35).step_by(7) {
            let b = self.byte()?;
            value |= u32::from(b & 127) << shift;
            if b & 128 == 0 {
                break;
            }
        }
        Ok(value)
    }
    fn int(&mut self) -> Result<i32, String> {
        let v = self.uint()?;
        Ok(if v & 1 != 0 {
            !(v >> 1) as i32
        } else {
            (v >> 1) as i32
        })
    }
    fn uint64(&mut self) -> Result<u64, String> {
        let mut value = 0u64;
        for i in 0..9 {
            let b = self.byte()?;
            if i == 8 {
                value |= u64::from(b) << 56;
                break;
            }
            value |= u64::from(b & 127) << (7 * i);
            if b & 128 == 0 {
                break;
            }
        }
        Ok(value)
    }
    fn float(&mut self) -> Result<f32, String> {
        let first = self.byte()?;
        if first == 0 {
            return Ok(0.0);
        }
        let bits = u32::from(first)
            | (u32::from(self.byte()?) << 8)
            | (u32::from(self.byte()?) << 16)
            | (u32::from(self.byte()?) << 24);
        Ok(f32::from_bits(bits.rotate_left(23)))
    }
    fn string(&mut self) -> Result<String, String> {
        let rest = &self.b[self.pos..];
        let end = rest
            .iter()
            .position(|&c| c == 0)
            .ok_or("unterminated Kiwi string")?;
        self.pos += end + 1;
        Ok(String::from_utf8_lossy(&rest[..end]).into_owned())
    }
}

impl Schema {
    pub fn decode(bytes: &[u8]) -> Result<Self, String> {
        let mut r = Reader::new(bytes);
        let count = r.uint()? as usize;
        if count > r.remaining() {
            return Err("corrupt Kiwi schema".into());
        }
        let mut defs = Vec::with_capacity(count);
        let mut by_name = HashMap::new();
        for i in 0..count {
            let name = r.string()?;
            let kind = r.byte()?;
            let n = r.uint()? as usize;
            if n > r.remaining() || kind > MESSAGE {
                return Err("corrupt Kiwi schema".into());
            }
            let mut fields = Vec::with_capacity(n);
            for _ in 0..n {
                fields.push(Field {
                    name: r.string()?,
                    ty: r.int()?,
                    array: r.byte()? != 0,
                    value: r.uint()?,
                });
            }
            by_name.insert(name, i);
            defs.push(Def { kind, fields });
        }
        if defs
            .iter()
            .flat_map(|d| &d.fields)
            .any(|f| f.ty >= 0 && f.ty as usize >= defs.len())
        {
            return Err("corrupt Kiwi schema (type index)".into());
        }
        Ok(Self { defs, by_name })
    }

    /// Decodes a value of the named root type (Figma: "Message").
    pub fn decode_root<'s>(&'s self, root: &str, bytes: &[u8]) -> Result<Value<'s>, String> {
        let &def = self
            .by_name
            .get(root)
            .ok_or_else(|| format!("schema has no {root}"))?;
        self.def(def, &mut Reader::new(bytes), 0)
    }

    fn def<'s>(&'s self, index: usize, r: &mut Reader, depth: usize) -> Result<Value<'s>, String> {
        if depth > MAX_DEPTH {
            return Err("Kiwi data nested too deeply".into());
        }
        let d = &self.defs[index];
        match d.kind {
            ENUM => {
                let v = r.uint()?;
                Ok(Value::Enum(
                    d.fields
                        .iter()
                        .find(|f| f.value == v)
                        .map_or("", |f| f.name.as_str()),
                ))
            }
            STRUCT => {
                let mut out = Vec::with_capacity(d.fields.len());
                for f in &d.fields {
                    out.push((f.name.as_str(), self.field(f, r, depth)?));
                }
                Ok(Value::Object(out))
            }
            _ => {
                let mut out = Vec::new();
                loop {
                    let v = r.uint()?;
                    if v == 0 {
                        break;
                    }
                    let f = d
                        .fields
                        .iter()
                        .find(|f| f.value == v)
                        .ok_or("unknown Kiwi field")?;
                    out.push((f.name.as_str(), self.field(f, r, depth)?));
                }
                Ok(Value::Object(out))
            }
        }
    }

    fn field<'s>(&'s self, f: &Field, r: &mut Reader, depth: usize) -> Result<Value<'s>, String> {
        if !f.array {
            return self.value(f.ty, r, depth + 1);
        }
        let n = r.uint()? as usize;
        if f.ty == -2 {
            return Ok(Value::Bytes(r.take(n)?.to_vec()));
        }
        // Every element takes at least one byte, except empty structs.
        if n > r.remaining() + 1 {
            return Err("corrupt Kiwi array length".into());
        }
        let mut out = Vec::with_capacity(n);
        for _ in 0..n {
            out.push(self.value(f.ty, r, depth + 1)?);
        }
        Ok(Value::Array(out))
    }

    fn value<'s>(&'s self, ty: i32, r: &mut Reader, depth: usize) -> Result<Value<'s>, String> {
        Ok(match ty {
            -1 => Value::Bool(r.byte()? != 0),
            -2 => Value::Uint(u64::from(r.byte()?)),
            -3 => Value::Int(i64::from(r.int()?)),
            -4 => Value::Uint(u64::from(r.uint()?)),
            -5 => Value::Float(r.float()?),
            -6 => Value::Str(r.string()?),
            -7 => {
                let v = r.uint64()?;
                Value::Int(if v & 1 != 0 {
                    !(v >> 1) as i64
                } else {
                    (v >> 1) as i64
                })
            }
            -8 => Value::Uint(r.uint64()?),
            t if t >= 0 => self.def(t as usize, r, depth)?,
            t => return Err(format!("unknown Kiwi type {t}")),
        })
    }
}

/// Encoder used by tests to build fixtures in the real wire format.
#[cfg(test)]
pub mod write {
    pub fn uint(out: &mut Vec<u8>, mut v: u32) {
        loop {
            let b = (v & 127) as u8;
            v >>= 7;
            if v == 0 {
                out.push(b);
                break;
            }
            out.push(b | 128);
        }
    }
    pub fn int(out: &mut Vec<u8>, v: i32) {
        uint(out, ((v << 1) ^ (v >> 31)) as u32);
    }
    pub fn float(out: &mut Vec<u8>, v: f32) {
        let bits = v.to_bits().rotate_right(23);
        if bits & 255 == 0 {
            out.push(0);
            return;
        }
        out.extend_from_slice(&bits.to_le_bytes());
    }
    pub fn string(out: &mut Vec<u8>, s: &str) {
        out.extend_from_slice(s.as_bytes());
        out.push(0);
    }
}
