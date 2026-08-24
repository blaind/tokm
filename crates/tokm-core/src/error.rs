//! Errors returned by counting and tokenizer resolution.

use std::path::PathBuf;

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

/// A recursive scan could not produce a trustworthy aggregate result.
#[derive(Debug, Error)]
pub enum ScanError {
    /// No filesystem inputs were supplied.
    #[error("at least one filesystem input is required")]
    NoInputs,

    /// The process working directory could not be resolved.
    #[error("cannot resolve the current working directory: {message}")]
    WorkingDirectory {
        /// Underlying operating-system error.
        message: String,
    },

    /// An explicit input path could not be inspected.
    #[error("cannot inspect input {path}: {message}")]
    Input {
        /// Explicit input path.
        path: PathBuf,

        /// Underlying operating-system error.
        message: String,
    },

    /// An include or exclude expression was not a valid glob.
    #[error("invalid glob {pattern:?}: {message}")]
    InvalidGlob {
        /// User-supplied glob expression.
        pattern: String,

        /// Parser error.
        message: String,
    },

    /// The bounded worker pool could not be created.
    #[error("cannot create tokenization worker pool: {message}")]
    WorkerPool {
        /// Worker-pool construction error.
        message: String,
    },

    /// A tokenizer failed while scanning one file.
    #[error(transparent)]
    Count(#[from] CountError),

    /// File or token totals exceeded the public result representation.
    #[error("scan totals exceed u64")]
    AggregateOverflow,
}
