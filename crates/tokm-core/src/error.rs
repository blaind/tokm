//! Errors returned by counting and tokenizer resolution.

use thiserror::Error;

/// A tokenizer could not be resolved or execute a count.
#[derive(Debug, Error)]
pub enum TokenizerError {
    /// The requested built-in encoding is unavailable.
    #[error("unsupported encoding: {0}")]
    UnsupportedEncoding(String),

    /// A backend returned a count that does not fit the public result type.
    #[error("token count does not fit in u64")]
    CountOverflow,

    /// A tokenizer backend failed while processing text.
    #[error("tokenizer backend failed: {0}")]
    Backend(String),
}

/// Counting failed before a result could be produced.
#[derive(Debug, Error)]
pub enum CountError {
    /// Tokenization failed.
    #[error(transparent)]
    Tokenizer(#[from] TokenizerError),
}
