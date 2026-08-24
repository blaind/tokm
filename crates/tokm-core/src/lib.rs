//! Shared token measurement semantics for `tokm` interfaces.

mod classify;
mod count;
mod decode;
mod error;
mod file;
mod text_policy;
pub mod tokenizer;

pub use classify::{Language, classify};
pub use count::{CountOptions, CountResult, count_text};
pub use error::{CountError, TokenizerError};
pub use file::{
    DEFAULT_MAX_FILE_SIZE, FileOutcome, FileResult, InvalidUtf8Policy, MaxFileSize, ScanOptions,
    SkipDetail, SkipReason, count_file,
};
pub use text_policy::{NORMALIZATION_VERSION, TextPolicy, TextPolicyMetadata};

/// Current counting semantics revision.
pub const COUNTING_SEMANTICS_VERSION: u32 = 1;
