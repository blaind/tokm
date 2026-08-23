//! Shared raw-text counting entry point.

use std::sync::Arc;

use serde::Serialize;

#[cfg(feature = "hf")]
use crate::tokenizer::HuggingFaceTokenCounter;
#[cfg(feature = "builtin-tokenizers")]
use crate::tokenizer::{BuiltinEncoding, BuiltinTokenCounter};
use crate::{
    CountError, TextPolicy, TextPolicyMetadata,
    tokenizer::{TokenCounter, TokenizerMetadata},
};

/// Configuration shared by raw text, file, scan, and frontend entry points.
#[derive(Clone)]
pub struct CountOptions {
    tokenizer: Arc<dyn TokenCounter>,
    text_policy: TextPolicy,
}

impl CountOptions {
    /// Construct options around a native token counter.
    #[must_use]
    pub fn new(tokenizer: Arc<dyn TokenCounter>) -> Self {
        Self {
            tokenizer,
            text_policy: TextPolicy::Literal,
        }
    }

    /// Construct options for a built-in exact encoding.
    #[cfg(feature = "builtin-tokenizers")]
    #[must_use]
    pub fn for_encoding(encoding: BuiltinEncoding) -> Self {
        Self::new(Arc::new(BuiltinTokenCounter::new(encoding)))
    }

    /// Construct options by resolving a supported model alias.
    #[cfg(feature = "builtin-tokenizers")]
    pub fn for_model(model: &str) -> Result<Self, crate::TokenizerError> {
        BuiltinEncoding::for_model(model).map(Self::for_encoding)
    }

    /// Construct options from a local Hugging Face `tokenizer.json` artifact.
    #[cfg(feature = "hf")]
    pub fn for_tokenizer_file(
        path: impl AsRef<std::path::Path>,
    ) -> Result<Self, crate::TokenizerError> {
        HuggingFaceTokenCounter::from_file(path).map(|counter| Self::new(Arc::new(counter)))
    }

    /// Select the transformation applied before tokenization.
    #[must_use]
    pub fn with_text_policy(mut self, text_policy: TextPolicy) -> Self {
        self.text_policy = text_policy;
        self
    }

    /// Return the configured tokenizer.
    #[must_use]
    pub fn tokenizer(&self) -> &dyn TokenCounter {
        self.tokenizer.as_ref()
    }

    /// Return the configured text policy.
    #[must_use]
    pub const fn text_policy(&self) -> TextPolicy {
        self.text_policy
    }
}

#[cfg(feature = "builtin-tokenizers")]
impl Default for CountOptions {
    fn default() -> Self {
        Self::for_encoding(BuiltinEncoding::default())
    }
}

/// Result of measuring one decoded string.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct CountResult {
    /// Number of tokens after applying the selected text policy.
    pub tokens: u64,

    /// Size of the original UTF-8 string before policy transformations.
    pub bytes: u64,

    /// Stable identity of the tokenizer used for the count.
    pub tokenizer: TokenizerMetadata,

    /// Versioned text transformation applied before tokenization.
    pub text_policy: TextPolicyMetadata,
}

/// Count tokens in one already-decoded string.
///
/// All higher-level entry points route through this function so tokenizer and
/// text-policy semantics remain identical.
pub fn count_text(text: &str, options: &CountOptions) -> Result<CountResult, CountError> {
    let measured_text = options.text_policy.apply(text);
    let tokens = options.tokenizer.count(&measured_text)?;
    let bytes = u64::try_from(text.len()).map_err(|_| crate::TokenizerError::CountOverflow)?;

    Ok(CountResult {
        tokens,
        bytes,
        tokenizer: options.tokenizer.metadata().clone(),
        text_policy: options.text_policy.metadata(),
    })
}

#[cfg(test)]
mod tests {
    use super::{CountOptions, count_text};
    use crate::{TextPolicy, tokenizer::BuiltinEncoding};

    #[test]
    fn default_count_uses_o200k_base_literal_semantics() {
        let result = count_text("hello world", &CountOptions::default()).unwrap();

        assert_eq!(result.tokens, 2);
        assert_eq!(result.tokenizer.name, "o200k_base");
        assert_eq!(result.text_policy.mode, "literal");
    }

    #[test]
    fn normalization_changes_only_the_measured_view() {
        let literal = CountOptions::for_encoding(BuiltinEncoding::Cl100kBase);
        let normalized = literal.clone().with_text_policy(TextPolicy::Normalized);
        let input = "one\r\n\r\ntwo";

        let literal_result = count_text(input, &literal).unwrap();
        let normalized_result = count_text(input, &normalized).unwrap();
        let expected_result = count_text("one\n\ntwo", &literal).unwrap();

        assert_eq!(literal_result.bytes, normalized_result.bytes);
        assert_eq!(normalized_result.tokens, expected_result.tokens);
    }

    #[test]
    fn model_alias_uses_resolved_encoding_metadata() {
        let model = CountOptions::for_model("gpt-5").unwrap();
        let encoding = CountOptions::for_encoding(BuiltinEncoding::O200kBase);
        let model_result = count_text("hello world", &model).unwrap();
        let encoding_result = count_text("hello world", &encoding).unwrap();

        assert_eq!(model_result, encoding_result);
        assert_eq!(model_result.tokenizer.name, "o200k_base");
    }
}
