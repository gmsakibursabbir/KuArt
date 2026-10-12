//! Comments pinned to a page: written in KuArt or read from imported files (PDF/Illustrator
//! annotations, SVG notes). Positions are in page coordinates.

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Reply {
    pub author: String,
    pub text: String,
    /// Unix seconds; 0 when unknown.
    pub created: u64,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Comment {
    /// Index of the page among the document's pages.
    pub page: u32,
    pub x: f32,
    pub y: f32,
    pub author: String,
    pub text: String,
    /// Unix seconds; 0 when unknown.
    pub created: u64,
    pub resolved: bool,
    pub replies: Vec<Reply>,
    /// Where an imported comment came from, e.g. "PDF annotation"; empty for KuArt comments.
    pub source: String,
}

impl Comment {
    pub fn heap_bytes(&self) -> usize {
        self.author.len()
            + self.text.len()
            + self.source.len()
            + self
                .replies
                .iter()
                .map(|r| size_of::<Reply>() + r.author.len() + r.text.len())
                .sum::<usize>()
    }
}

/// Current time as unix seconds.
pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}
