//! Memory accounting used by the diagnostics panel and benchmarks.

use std::fmt;

/// Approximate heap bytes owned by a value (shared `Arc` data is split between owners).
pub trait HeapSize {
    fn heap_size(&self) -> usize;
}

/// One subsystem line in the memory panel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemoryLine {
    pub label: &'static str,
    pub bytes: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MemoryReport {
    pub lines: Vec<MemoryLine>,
}

impl MemoryReport {
    pub fn push(&mut self, label: &'static str, bytes: usize) {
        self.lines.push(MemoryLine { label, bytes });
    }

    pub fn merge(&mut self, other: MemoryReport) {
        self.lines.extend(other.lines);
    }

    pub fn total(&self) -> usize {
        self.lines.iter().map(|l| l.bytes).sum()
    }
}

fn human(bytes: usize) -> String {
    const MB: f64 = 1024.0 * 1024.0;
    let b = bytes as f64;
    if b >= MB {
        format!("{:.1} MB", b / MB)
    } else {
        format!("{:.1} KB", b / 1024.0)
    }
}

impl fmt::Display for MemoryReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Memory")?;
        writeln!(f, "──────────────────────────")?;
        for line in &self.lines {
            writeln!(f, "{:<14}{:>12}", line.label, human(line.bytes))?;
        }
        writeln!(f, "──────────────────────────")?;
        write!(f, "{:<14}{:>12}", "Total", human(self.total()))
    }
}
