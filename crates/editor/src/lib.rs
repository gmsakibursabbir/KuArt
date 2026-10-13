//! Editor logic independent of the UI: selection rules, hit resolution, transform
//! gestures, snapping and commands. See `docs/architecture.md` §5.

pub mod commands;
pub mod components;
pub mod create;
pub mod layout;
pub mod path_edit;
pub mod selection;
pub mod snap;
pub mod transform;

pub use commands::{Align, Arrange, Clipboard};
pub use create::{Create, Tool, default_node};
pub use selection::{Selection, SelectionMode, drill, hit_test, marquee_select, resolve_target};
pub use snap::{SnapResult, Snapper};
pub use transform::{Feedback, Gesture, HandleIndex, Modifiers};
