//! Shared token measurement semantics for `tokm` interfaces.

mod count;
mod error;
mod text_policy;
pub mod tokenizer;

pub use count::{CountOptions, CountResult, count_text};
pub use error::{CountError, TokenizerError};
pub use text_policy::{NORMALIZATION_VERSION, TextPolicy, TextPolicyMetadata};

/// Current counting semantics revision.
pub const COUNTING_SEMANTICS_VERSION: u32 = 1;
