//! Exact raw-text counting from local Hugging Face tokenizer artifacts.

use std::{fmt, fs, path::Path};

use tokenizers::Tokenizer;

use super::{AccuracyClass, RawTextSemantics, TokenCounter, TokenizerKind, TokenizerMetadata};
use crate::TokenizerError;

/// Counter loaded from the exact bytes of one local `tokenizer.json` artifact.
pub struct HuggingFaceTokenCounter {
    tokenizer: Tokenizer,
    metadata: TokenizerMetadata,
}

impl HuggingFaceTokenCounter {
    /// Load a tokenizer artifact without downloading any additional data.
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self, TokenizerError> {
        let path = path.as_ref();
        let bytes = fs::read(path).map_err(|error| TokenizerError::ArtifactRead {
            path: path.to_path_buf(),
            message: error.to_string(),
        })?;
        let mut tokenizer =
            Tokenizer::from_bytes(&bytes).map_err(|error| TokenizerError::ArtifactInvalid {
                path: path.to_path_buf(),
                message: error.to_string(),
            })?;

        // A measurement must cover the complete input and must not introduce
        // fixed-width padding, regardless of serialized runtime settings.
        tokenizer
            .with_truncation(None)
            .map_err(|error| TokenizerError::ArtifactInvalid {
                path: path.to_path_buf(),
                message: error.to_string(),
            })?;
        tokenizer.with_padding(None);

        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("tokenizer.json")
            .to_owned();
        let fingerprint = format!(
            "huggingface:blake3:{}:raw-v1",
            blake3::hash(&bytes).to_hex()
        );

        Ok(Self {
            tokenizer,
            metadata: TokenizerMetadata {
                kind: TokenizerKind::LocalFile,
                name,
                fingerprint,
                accuracy: AccuracyClass::Exact,
                raw_text_semantics: RawTextSemantics::Artifact,
            },
        })
    }
}

impl fmt::Debug for HuggingFaceTokenCounter {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HuggingFaceTokenCounter")
            .field("metadata", &self.metadata)
            .finish_non_exhaustive()
    }
}

impl TokenCounter for HuggingFaceTokenCounter {
    fn metadata(&self) -> &TokenizerMetadata {
        &self.metadata
    }

    fn count(&self, text: &str) -> Result<u64, TokenizerError> {
        let encoding = self
            .tokenizer
            .encode(text, false)
            .map_err(|error| TokenizerError::Backend(error.to_string()))?;

        u64::try_from(encoding.len()).map_err(|_| TokenizerError::CountOverflow)
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, path::PathBuf};

    use serde::Deserialize;
    use tempfile::tempdir;

    use super::HuggingFaceTokenCounter;
    use crate::{
        CountOptions, count_text,
        tokenizer::{AccuracyClass, RawTextSemantics, TokenCounter, TokenizerKind},
    };

    fn fixture() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/hf_tokenizer/tokenizer.json")
    }

    #[derive(Deserialize)]
    struct ReferenceCounts {
        reference: String,
        add_special_tokens: bool,
        counts: Vec<ReferenceCount>,
    }

    #[derive(Deserialize)]
    struct ReferenceCount {
        text: String,
        tokens: u64,
    }

    #[test]
    fn follows_artifact_raw_text_semantics_without_framing() {
        let counter = HuggingFaceTokenCounter::from_file(fixture()).unwrap();
        let reference_path = fixture().with_file_name("reference-counts.json");
        let reference: ReferenceCounts =
            serde_json::from_slice(&fs::read(reference_path).unwrap()).unwrap();

        assert_eq!(reference.reference, "huggingface-tokenizers-python-0.22.2");
        assert!(!reference.add_special_tokens);
        for count in reference.counts {
            assert_eq!(
                counter.count(&count.text).unwrap(),
                count.tokens,
                "{:?}",
                count.text
            );
        }

        let metadata = counter.metadata();
        assert_eq!(metadata.kind, TokenizerKind::LocalFile);
        assert_eq!(metadata.name, "tokenizer.json");
        assert_eq!(metadata.accuracy, AccuracyClass::Exact);
        assert_eq!(metadata.raw_text_semantics, RawTextSemantics::Artifact);
    }

    #[test]
    fn factory_routes_through_shared_counting_semantics() {
        let options = CountOptions::for_tokenizer_file(fixture()).unwrap();
        let result = count_text("HÉLLO WORLD", &options).unwrap();

        assert_eq!(result.tokens, 2);
        assert_eq!(result.tokenizer.kind, TokenizerKind::LocalFile);
    }

    #[test]
    fn fingerprint_uses_exact_contents_instead_of_path() {
        let directory = tempdir().unwrap();
        let bytes = fs::read(fixture()).unwrap();
        let first_path = directory.path().join("first.json");
        let second_path = directory.path().join("second.json");
        let changed_path = directory.path().join("changed.json");
        fs::write(&first_path, &bytes).unwrap();
        fs::write(&second_path, &bytes).unwrap();

        let mut changed = bytes;
        changed.push(b'\n');
        fs::write(&changed_path, changed).unwrap();

        let first = HuggingFaceTokenCounter::from_file(first_path).unwrap();
        let second = HuggingFaceTokenCounter::from_file(second_path).unwrap();
        let changed = HuggingFaceTokenCounter::from_file(changed_path).unwrap();

        assert_eq!(first.metadata().fingerprint, second.metadata().fingerprint);
        assert_ne!(first.metadata().fingerprint, changed.metadata().fingerprint);
    }

    #[test]
    fn artifact_errors_retain_the_supplied_path() {
        let directory = tempdir().unwrap();
        let missing = directory.path().join("missing.json");
        let invalid = directory.path().join("invalid.json");
        fs::write(&invalid, b"not JSON").unwrap();

        let read_error = HuggingFaceTokenCounter::from_file(&missing).unwrap_err();
        let parse_error = HuggingFaceTokenCounter::from_file(&invalid).unwrap_err();

        assert!(read_error.to_string().contains(missing.to_str().unwrap()));
        assert!(parse_error.to_string().contains(invalid.to_str().unwrap()));
    }
}
