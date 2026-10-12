//! PDF fonts for text import: family/style from `BaseFont`, and character codes → Unicode
//! through the `ToUnicode` CMap (bfchar/bfrange), else Latin-1 (simple fonts) or UTF-16BE
//! (composite fonts without a map).

use std::collections::HashMap;

use lopdf::Dictionary;

pub struct Font {
    pub family: String,
    pub face: String,
    pub type3: bool,
    /// Composite (Type0) fonts use 2-byte codes.
    two_byte: bool,
    map: HashMap<u32, String>,
}

impl Font {
    pub fn load(pdf: &lopdf::Document, dict: &Dictionary) -> Self {
        let name = |k: &[u8]| {
            dict.get(k)
                .ok()
                .and_then(|o| o.as_name().ok())
                .map(|n| String::from_utf8_lossy(n).into_owned())
        };
        let subtype = name(b"Subtype").unwrap_or_default();
        let base = name(b"BaseFont").unwrap_or_default();
        let (family, face) = split_base_font(&base);
        let map = dict
            .get(b"ToUnicode")
            .ok()
            .and_then(|o| pdf.dereference(o).ok())
            .and_then(|(_, o)| o.as_stream().ok())
            .and_then(|s| s.decompressed_content().ok())
            .map(|data| parse_cmap(&data))
            .unwrap_or_default();
        Self {
            family,
            face,
            type3: subtype == "Type3",
            two_byte: subtype == "Type0",
            map,
        }
    }

    pub fn decode(&self, bytes: &[u8]) -> String {
        let codes: Vec<u32> = if self.two_byte {
            bytes
                .chunks(2)
                .map(|c| (u32::from(c[0]) << 8) | u32::from(*c.get(1).unwrap_or(&0)))
                .collect()
        } else {
            bytes.iter().map(|&b| u32::from(b)).collect()
        };
        let mut out = String::new();
        for code in codes {
            match self.map.get(&code) {
                Some(s) => out.push_str(s),
                None if self.two_byte => {
                    if let Some(c) = char::from_u32(code) {
                        out.push(c);
                    }
                }
                // WinAnsi/Standard encodings agree with Latin-1 for printable ASCII.
                None => out.push(char::from(code as u8)),
            }
        }
        out.replace('\0', "")
    }
}

/// "ABCDEF+MyriadPro-BoldIt" → ("Myriad Pro", "BoldIt"); "Arial,Bold" → ("Arial", "Bold").
pub fn split_base_font(base: &str) -> (String, String) {
    let base = match base.split_once('+') {
        Some((tag, rest)) if tag.len() == 6 && tag.chars().all(|c| c.is_ascii_uppercase()) => rest,
        _ => base,
    };
    let (family, face) = base.split_once(['-', ',']).unwrap_or((base, "Regular"));
    let face = face
        .replace("It", "Italic")
        .replace("ItalicItalic", "Italic");
    let mut spaced = String::new();
    let mut prev = ' ';
    for c in family.chars() {
        if c.is_ascii_uppercase() && prev.is_ascii_lowercase() {
            spaced.push(' ');
        }
        spaced.push(c);
        prev = c;
    }
    let spaced = spaced
        .trim_end_matches(" MT")
        .trim_end_matches("MT")
        .to_owned();
    (spaced, face)
}

/// `beginbfchar <src> <dst> endbfchar` and `beginbfrange <lo> <hi> <dst|[dsts]> endbfrange`.
pub fn parse_cmap(data: &[u8]) -> HashMap<u32, String> {
    let text = String::from_utf8_lossy(data);
    let mut map = HashMap::new();
    let tokens: Vec<&str> = text
        .split(|c: char| c.is_whitespace())
        .flat_map(|t| split_brackets(t))
        .filter(|t| !t.is_empty())
        .collect();
    let mut i = 0;
    while i < tokens.len() {
        match tokens[i] {
            "beginbfchar" => {
                i += 1;
                while i + 1 < tokens.len() && tokens[i] != "endbfchar" {
                    if let (Some(src), Some(dst)) = (hex(tokens[i]), utf16(tokens[i + 1])) {
                        map.insert(src, dst);
                    }
                    i += 2;
                }
            }
            "beginbfrange" => {
                i += 1;
                while i + 2 < tokens.len() && tokens[i] != "endbfrange" {
                    let (lo, hi) = (hex(tokens[i]), hex(tokens[i + 1]));
                    if tokens[i + 2] == "[" {
                        let mut j = i + 3;
                        let mut code = lo.unwrap_or(0);
                        while j < tokens.len() && tokens[j] != "]" {
                            if let Some(d) = utf16(tokens[j]) {
                                map.insert(code, d);
                            }
                            code += 1;
                            j += 1;
                        }
                        i = j + 1;
                    } else {
                        if let (Some(lo), Some(hi), Some(start)) =
                            (lo, hi, hex_units(tokens[i + 2]))
                        {
                            for (k, code) in (lo..=hi.min(lo + 0xFFFF)).enumerate() {
                                let mut units = start.clone();
                                if let Some(last) = units.last_mut() {
                                    *last = last.wrapping_add(k as u16);
                                }
                                map.insert(code, String::from_utf16_lossy(&units));
                            }
                        }
                        i += 3;
                    }
                }
            }
            _ => {}
        }
        i += 1;
    }
    map
}

/// Splits "<0041>[<0042>" style runs so brackets are their own tokens.
fn split_brackets(t: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    for (i, c) in t.char_indices() {
        if c == '[' || c == ']' {
            out.push(&t[start..i]);
            out.push(&t[i..i + 1]);
            start = i + 1;
        } else if c == '<' && i > start {
            out.push(&t[start..i]);
            start = i;
        }
    }
    out.push(&t[start..]);
    out
}

fn hex(t: &str) -> Option<u32> {
    u32::from_str_radix(t.strip_prefix('<')?.strip_suffix('>')?, 16).ok()
}

fn hex_units(t: &str) -> Option<Vec<u16>> {
    let h = t.strip_prefix('<')?.strip_suffix('>')?;
    if h.is_empty() || h.len() % 4 != 0 {
        // Some writers use 2-digit destinations.
        return u16::from_str_radix(h, 16).ok().map(|v| vec![v]);
    }
    (0..h.len())
        .step_by(4)
        .map(|i| u16::from_str_radix(&h[i..i + 4], 16).ok())
        .collect()
}

fn utf16(t: &str) -> Option<String> {
    hex_units(t).map(|u| String::from_utf16_lossy(&u))
}
