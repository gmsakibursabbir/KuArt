//! Universal Design Model (UDM).
//!
//! The single in-memory representation shared by importers, the editor, the
//! renderer and code generators. See `docs/universal-model.md`.
//!
//! Design rules:
//! * compact hot data in [`Node`], rare data in [`NodeExt`] behind a `Box`;
//! * generational [`NodeId`]s instead of pointers;
//! * `Arc`-shared immutable payloads (paths, paints, gradients);
//! * every mutation goes through [`Document::apply`] returning an inverse [`Op`].

pub mod assets;
pub mod document;
pub mod geometry;
pub mod history;
pub mod id;
pub mod intern;
pub mod layout;
pub mod memory;
pub mod paths;
pub mod scene;
pub mod styles;
pub mod typography;

pub use assets::{AssetEntry, AssetId, AssetKind, AssetSource, AssetStore, ContentHash};
pub use document::{ChangeSet, DocError, Document, Op, Prop};
pub use geometry::{Affine, FillRule, PathData, PathVerb, Point, Rect, Size};
pub use history::{History, Transaction};
pub use id::NodeId;
pub use intern::{Interner, Symbol};
pub use memory::{HeapSize, MemoryReport};
pub use scene::{Node, NodeExt, NodeFlags, NodeKind, Preserved, SourceFormat, SupportLevel};
pub use styles::{BlendMode, Color, Effect, Paint, PaintKind, Paints, Stroke};
