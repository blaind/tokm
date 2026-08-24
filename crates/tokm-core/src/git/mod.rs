//! Feature-gated measurement of committed Git trees.

mod diff;
mod tree;

pub use diff::{diff_trees, diff_worktree};
pub use tree::scan_tree;
