//! Process memory as the OS reports it (benchmark-plan.md §3: "RAM comes from OS counters, not
//! our own accounting"). Windows: working set and private bytes. Linux: `smaps_rollup` Rss and
//! Private_Clean + Private_Dirty. Elsewhere: resident size for both.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProcessMemory {
    /// Pages in RAM (working set / RSS).
    pub resident: u64,
    /// Memory owned by this process alone (private bytes / Private_*).
    pub private: u64,
}

impl ProcessMemory {
    pub fn private_mb(&self) -> f64 {
        self.private as f64 / 1_048_576.0
    }
    pub fn resident_mb(&self) -> f64 {
        self.resident as f64 / 1_048_576.0
    }
}

#[cfg(target_os = "linux")]
pub fn process_memory() -> Option<ProcessMemory> {
    let text = std::fs::read_to_string("/proc/self/smaps_rollup").ok()?;
    let kb = |key: &str| -> u64 {
        text.lines()
            .find(|l| l.starts_with(key))
            .and_then(|l| l.split_whitespace().nth(1))
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(0)
            * 1024
    };
    Some(ProcessMemory {
        resident: kb("Rss:"),
        private: kb("Private_Clean:") + kb("Private_Dirty:"),
    })
}

#[cfg(not(target_os = "linux"))]
pub fn process_memory() -> Option<ProcessMemory> {
    let s = memory_stats::memory_stats()?;
    Some(ProcessMemory {
        resident: s.physical_mem as u64,
        // On Windows `virtual_mem` is PagefileUsage, which equals private bytes.
        private: if cfg!(windows) {
            s.virtual_mem as u64
        } else {
            s.physical_mem as u64
        },
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn reports_plausible_numbers() {
        let m = super::process_memory().expect("counters available");
        assert!(m.resident > 1_000_000, "{m:?}");
        assert!(m.private > 100_000, "{m:?}");
        // Allocating and touching 64 MB shows up in private memory.
        let before = m.private;
        let v = vec![1u8; 64 << 20];
        let after = super::process_memory().unwrap().private;
        std::hint::black_box(&v);
        assert!(after >= before + (48 << 20), "{before} → {after}");
    }
}
