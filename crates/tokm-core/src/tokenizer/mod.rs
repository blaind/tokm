//! Token-counting backends and their stable identities.

#[cfg(feature = "builtin-tokenizers")]
mod builtin;

use serde::Serialize;

use crate::TokenizerError;

#[cfg(feature = "builtin-tokenizers")]
pub use builtin::{BuiltinEncoding, BuiltinTokenCounter};

/// Whether a tokenizer reproduces a reference definition or estimates it.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AccuracyClass {
    /// Reference-faithful local tokenization.
    Exact,

    /// An explicitly approximate substitute.
    Proxy,
}

/// Source of a tokenizer definition.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TokenizerKind {
    /// Tokenizer data shipped with `tokm`.
    Builtin,

    /// Tokenizer loaded from a local artifact.
    LocalFile,

    /// Approximate tokenizer selected for another target.
    Proxy,
}

/// Raw-text behavior used by a tokenizer.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RawTextSemantics {
    /// Tiktoken ordinary encoding with no recognized special tokens.
    Ordinary,

    /// Artifact-defined normalization, pre-tokenization, and added-token matching.
    Artifact,
}

/// Identity and accuracy information that accompanies every count.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct TokenizerMetadata {
    /// Source of the tokenizer definition.
    pub kind: TokenizerKind,

    /// Human-readable tokenizer name.
    pub name: String,

    /// Stable tokenizer-data and semantics identity.
    pub fingerprint: String,

    /// Whether the count is exact or a proxy.
    pub accuracy: AccuracyClass,

    /// Behavior used for raw strings.
    pub raw_text_semantics: RawTextSemantics,
}

/// Count-oriented tokenizer interface shared by every frontend.
pub trait TokenCounter: Send + Sync {
    /// Return stable metadata for this tokenizer instance.
    fn metadata(&self) -> &TokenizerMetadata;

    /// Count tokens in one already-decoded string.
    fn count(&self, text: &str) -> Result<u64, TokenizerError>;
}
