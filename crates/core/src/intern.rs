//! String interning: layer names, font families, style names become 4-byte symbols.

use std::collections::HashMap;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct Symbol(u32);

impl Symbol {
    /// The empty string, always present.
    pub const EMPTY: Symbol = Symbol(0);
}

/// Each string is stored once; the map and the table share the allocation.
pub struct Interner {
    map: HashMap<Arc<str>, Symbol>,
    strings: Vec<Arc<str>>,
}

impl Default for Interner {
    fn default() -> Self {
        let empty: Arc<str> = Arc::from("");
        let mut map = HashMap::new();
        map.insert(empty.clone(), Symbol::EMPTY);
        Self {
            map,
            strings: vec![empty],
        }
    }
}

impl Interner {
    pub fn intern(&mut self, s: &str) -> Symbol {
        if let Some(&sym) = self.map.get(s) {
            return sym;
        }
        let sym = Symbol(u32::try_from(self.strings.len()).expect("interner overflow"));
        let arc: Arc<str> = Arc::from(s);
        self.strings.push(arc.clone());
        self.map.insert(arc, sym);
        sym
    }

    /// Returns `""` for symbols from another interner rather than panicking.
    pub fn resolve(&self, sym: Symbol) -> &str {
        self.strings.get(sym.0 as usize).map_or("", |s| s)
    }

    pub fn get(&self, s: &str) -> Option<Symbol> {
        self.map.get(s).copied()
    }

    pub fn len(&self) -> usize {
        self.strings.len()
    }

    pub fn is_empty(&self) -> bool {
        self.strings.len() <= 1
    }

    pub fn heap_bytes(&self) -> usize {
        let text: usize = self
            .strings
            .iter()
            .map(|s| s.len() + 2 * size_of::<usize>())
            .sum();
        text + self.strings.capacity() * size_of::<Arc<str>>()
            + self.map.capacity() * (size_of::<Arc<str>>() + size_of::<Symbol>() + 8)
    }
}
