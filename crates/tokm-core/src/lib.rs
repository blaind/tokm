//! Shared token measurement semantics for `tokm` interfaces.

mod aggregate;
#[cfg(feature = "cache")]
mod cache;
mod classify;
mod count;
mod decode;
mod diff;
mod error;
mod file;
#[cfg(feature = "git")]
pub mod git;
mod scan;
mod text_policy;
pub mod tokenizer;
mod walk;

pub use aggregate::{LanguageResult, ScanFileResult, ScanResult, ScanTotals, SkippedSummary};
pub use classify::{Language, classify};
pub use count::{CountOptions, CountResult, count_text};
pub use diff::{DiffFileResult, DiffResult, LanguageDelta, TokenDelta, compare_snapshots};
pub use error::{CountError, DiffError, GitError, ScanError, TokenizerError};
pub use file::{
    ByteCountResult, ByteOutcome, ByteSkip, DEFAULT_MAX_FILE_SIZE, DEFAULT_MAX_WORKERS,
    FileOutcome, FileResult, InvalidUtf8Policy, MaxFileSize, ScanOptions, SkipDetail, SkipReason,
    count_bytes, count_file,
};
pub use scan::scan;
pub use text_policy::{NORMALIZATION_VERSION, TextPolicy, TextPolicyMetadata};

/// Current counting semantics revision.
pub const COUNTING_SEMANTICS_VERSION: u32 = 1;
