//! Errors returned by counting and tokenizer resolution.

use std::path::PathBuf;

use thiserror::Error;

/// A tokenizer could not be resolved or execute a count.
#[derive(Debug, Error)]
pub enum TokenizerError {
    /// The requested model alias is unavailable.
    #[error("unsupported model: {0}")]
    UnsupportedModel(String),

    /// The requested built-in encoding is unavailable.
    #[error("unsupported encoding: {0}")]
    UnsupportedEncoding(String),

    /// A local tokenizer artifact could not be read.
    #[error("cannot read tokenizer artifact {path}: {message}")]
    ArtifactRead {
        /// Path supplied by the caller.
        path: PathBuf,

        /// Underlying operating-system error.
        message: String,
    },

    /// A local tokenizer artifact could not be parsed.
    #[error("invalid tokenizer artifact {path}: {message}")]
    ArtifactInvalid {
        /// Path supplied by the caller.
        path: PathBuf,

        /// Artifact parser error.
        message: String,
    },

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

/// Two complete scan snapshots could not be compared safely.
#[derive(Debug, Error)]
pub enum DiffError {
    /// Snapshot semantics differ, so their token totals are not comparable.
    #[error("cannot compare snapshots with different {0}")]
    IncompatibleSemantics(&'static str),
}

/// A Git repository or committed tree could not be measured.
#[derive(Debug, Error)]
pub enum GitError {
    /// No repository could be discovered from the supplied path.
    #[error("cannot open Git repository from {path}: {message}")]
    Repository {
        /// Path supplied by the caller.
        path: PathBuf,

        /// Repository discovery error.
        message: String,
    },

    /// A worktree comparison was requested for a bare repository.
    #[error("Git repository at {path} has no worktree")]
    WorktreeUnavailable {
        /// Discovered Git directory.
        path: PathBuf,
    },

    /// A revision expression could not be resolved to an object.
    #[error("cannot resolve Git revision {revision:?}: {message}")]
    Revision {
        /// User-supplied revision expression.
        revision: String,

        /// Revision parser error.
        message: String,
    },

    /// A revision object could not be peeled to a tree.
    #[error("cannot read Git tree for {revision:?}: {message}")]
    Tree {
        /// User-supplied revision expression.
        revision: String,

        /// Object or tree traversal error.
        message: String,
    },

    /// An object referenced by a committed tree could not be read.
    #[error("cannot read Git object for {path}: {message}")]
    Object {
        /// Logical path of the tree entry.
        path: PathBuf,

        /// Object database error.
        message: String,
    },

    /// Tokenization of a committed blob failed.
    #[error(transparent)]
    Count(#[from] CountError),

    /// Aggregation of the committed tree failed.
    #[error(transparent)]
    Scan(#[from] ScanError),

    /// The two measured snapshots were not comparable.
    #[error(transparent)]
    Diff(#[from] DiffError),
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
