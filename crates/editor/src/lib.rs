//! Editor logic independent of the UI: selection rules, hit resolution, and (from M4)
//! transform tools and commands. See `docs/architecture.md` §5.

pub mod selection;

pub use selection::{Selection, SelectionMode, hit_test, marquee_select, resolve_target};
