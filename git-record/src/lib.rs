//! UI component to interactively select changes to include in a commit. This
//! component is meant to be embedded in source control tooling.
//!
//! You can think of this as an interactive replacement for `git add -p`, or a
//! reimplementation of `hg crecord`. Given a set of changes made by the user,
//! this component presents them to the user and lets them select which of those
//! changes should be staged for commit.

#![warn(missing_docs)]
#![warn(clippy::all, clippy::as_conversions)]
#![allow(clippy::too_many_arguments, clippy::blocks_in_if_conditions)]

mod cursive_utils;
mod tristate;
mod types;
mod ui;

pub use types::{FileHunks, Hunk, HunkChangedLine, RecordError, RecordState};
pub use ui::Recorder;