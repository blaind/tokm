//! Built-in tiktoken-compatible encodings.

use std::{fmt, str::FromStr};

use serde::Serialize;
use tiktoken_rs::{cl100k_base_singleton, o200k_base_singleton};

use super::{AccuracyClass, RawTextSemantics, TokenCounter, TokenizerKind, TokenizerMetadata};
use crate::TokenizerError;

/// Built-in exact encoding families supported by the initial release.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BuiltinEncoding {
    /// OpenAI's `o200k_base` encoding.
    #[default]
    O200kBase,

    /// OpenAI's `cl100k_base` encoding.
    Cl100kBase,
}

impl BuiltinEncoding {
    /// Return the accepted command-line name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::O200kBase => "o200k_base",
            Self::Cl100kBase => "cl100k_base",
        }
    }

    /// Return the stable tokenizer-data and semantics identity.
    #[must_use]
    pub const fn fingerprint(self) -> &'static str {
        match self {
            Self::O200kBase => "tiktoken:o200k_base:tiktoken-rs-0.12.0:ordinary-v1",
            Self::Cl100kBase => "tiktoken:cl100k_base:tiktoken-rs-0.12.0:ordinary-v1",
        }
    }
}

impl fmt::Display for BuiltinEncoding {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for BuiltinEncoding {
    type Err = TokenizerError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "o200k_base" => Ok(Self::O200kBase),
            "cl100k_base" => Ok(Self::Cl100kBase),
            _ => Err(TokenizerError::UnsupportedEncoding(value.to_owned())),
        }
    }
}

/// Exact counter backed by vendored tiktoken-compatible data.
#[derive(Clone, Debug)]
pub struct BuiltinTokenCounter {
    encoding: BuiltinEncoding,
    metadata: TokenizerMetadata,
}

impl BuiltinTokenCounter {
    /// Construct a counter for one supported encoding.
    #[must_use]
    pub fn new(encoding: BuiltinEncoding) -> Self {
        Self {
            encoding,
            metadata: TokenizerMetadata {
                kind: TokenizerKind::Builtin,
                name: encoding.as_str().to_owned(),
                fingerprint: encoding.fingerprint().to_owned(),
                accuracy: AccuracyClass::Exact,
                raw_text_semantics: RawTextSemantics::Ordinary,
            },
        }
    }

    /// Return the selected encoding family.
    #[must_use]
    pub const fn encoding(&self) -> BuiltinEncoding {
        self.encoding
    }
}

impl TokenCounter for BuiltinTokenCounter {
    fn metadata(&self) -> &TokenizerMetadata {
        &self.metadata
    }

    fn count(&self, text: &str) -> Result<u64, TokenizerError> {
        let count = match self.encoding {
            BuiltinEncoding::O200kBase => o200k_base_singleton().encode_ordinary(text).len(),
            BuiltinEncoding::Cl100kBase => cl100k_base_singleton().encode_ordinary(text).len(),
        };

        u64::try_from(count).map_err(|_| TokenizerError::CountOverflow)
    }
}

#[cfg(test)]
mod tests {
    use super::{BuiltinEncoding, BuiltinTokenCounter};
    use crate::tokenizer::TokenCounter;

    #[test]
    fn defaults_to_o200k_base() {
        assert_eq!(BuiltinEncoding::default(), BuiltinEncoding::O200kBase);
    }

    #[test]
    fn ordinary_encoding_treats_special_token_syntax_as_text() {
        let counter = BuiltinTokenCounter::new(BuiltinEncoding::O200kBase);

        assert!(counter.count("<|endoftext|>").unwrap() > 1);
    }

    #[test]
    fn counts_match_reference_tiktoken_ordinary_encoding() {
        let corpus = [
            "",
            "hello world",
            "<|endoftext|>",
            "<|fim_prefix|>",
            "tabs\tand  spaces",
            "line one\r\nline two\rline three\n",
            "你好，世界 👋🏽",
            "fn main() { println!(\"hello\"); }",
            r#"{"compact":[1,2,3],"ok":true}"#,
            "long_identifier_with_HTTP2_and_1234567890",
        ];
        let fixtures = [
            (BuiltinEncoding::O200kBase, [0, 2, 7, 6, 4, 9, 7, 9, 13, 11]),
            (
                BuiltinEncoding::Cl100kBase,
                [0, 2, 7, 7, 4, 9, 11, 9, 13, 11],
            ),
        ];

        for (encoding, expected) in fixtures {
            let counter = BuiltinTokenCounter::new(encoding);
            let actual = corpus.map(|text| counter.count(text).unwrap());

            assert_eq!(actual, expected, "{encoding}");
        }
    }

    #[test]
    fn supported_names_round_trip() {
        for encoding in [BuiltinEncoding::O200kBase, BuiltinEncoding::Cl100kBase] {
            assert_eq!(
                encoding.as_str().parse::<BuiltinEncoding>().unwrap(),
                encoding
            );
        }
    }
}
