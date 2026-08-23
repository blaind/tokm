//! Token-footprint comparison between complete scan snapshots.

use std::{cmp::Reverse, collections::BTreeMap, path::PathBuf};

use serde::Serialize;

use crate::{
    DiffError, InvalidUtf8Policy, Language, ScanFileResult, ScanResult, ScanTotals, SkippedSummary,
    TextPolicyMetadata, tokenizer::TokenizerMetadata,
};

/// Added, removed, and net token footprint for one comparison scope.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
pub struct TokenDelta {
    /// Token count introduced by files whose footprint grew.
    pub added: u64,

    /// Token count removed by files whose footprint shrank.
    pub removed: u64,

    /// Signed change from the base snapshot to the head snapshot.
    pub net: i128,
}

impl TokenDelta {
    fn between(base: u64, head: u64) -> Self {
        if head >= base {
            let added = head - base;
            Self {
                added,
                removed: 0,
                net: i128::from(added),
            }
        } else {
            let removed = base - head;
            Self {
                added: 0,
                removed,
                net: -i128::from(removed),
            }
        }
    }

    fn add(&mut self, other: Self) {
        self.added += other.added;
        self.removed += other.removed;
        self.net += other.net;
    }
}

/// Aggregate token delta for one language family.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct LanguageDelta {
    /// Language or text family.
    pub name: Language,

    /// Token footprint change for this language.
    #[serde(flatten)]
    pub tokens: TokenDelta,
}

/// Token-footprint change for one normalized logical path.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DiffFileResult {
    /// Logical file path shared across the snapshots.
    pub path: PathBuf,

    /// Path-based language classification.
    pub language: Language,

    /// Token count in the base, or `None` for a new file.
    pub base_tokens: Option<u64>,

    /// Token count in the head, or `None` for a removed file.
    pub head_tokens: Option<u64>,

    /// Token footprint change for this path.
    #[serde(flatten)]
    pub tokens: TokenDelta,
}

/// Deterministic comparison of two complete scan snapshots.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DiffResult {
    /// Stable identity of the selected tokenizer.
    pub tokenizer: TokenizerMetadata,

    /// Versioned text transformation shared by both snapshots.
    pub text_policy: TextPolicyMetadata,

    /// Invalid UTF-8 policy shared by both snapshots.
    pub invalid_utf8: InvalidUtf8Policy,

    /// Totals in the base snapshot.
    pub base: ScanTotals,

    /// Totals in the head snapshot.
    pub head: ScanTotals,

    /// Token footprint change across all paths.
    pub tokens: TokenDelta,

    /// Changed language totals sorted by impact descending, then name.
    pub languages: Vec<LanguageDelta>,

    /// Changed files sorted by normalized logical path.
    pub files: Vec<DiffFileResult>,

    /// Omission accounting from the base snapshot.
    pub base_skipped: SkippedSummary,

    /// Omission accounting from the head snapshot.
    pub head_skipped: SkippedSummary,
}

/// Compare token footprints from two complete scans rather than patch hunks.
pub fn compare_snapshots(base: &ScanResult, head: &ScanResult) -> Result<DiffResult, DiffError> {
    ensure_compatible(base, head)?;

    let base_files = files_by_path(&base.files);
    let head_files = files_by_path(&head.files);
    let mut paths = base_files
        .keys()
        .chain(head_files.keys())
        .collect::<Vec<_>>();
    paths.sort();
    paths.dedup();

    let mut tokens = TokenDelta::default();
    let mut by_language = BTreeMap::<Language, TokenDelta>::new();
    let mut files = Vec::new();

    for path in paths {
        let base_file = base_files.get(path).copied();
        let head_file = head_files.get(path).copied();
        let base_tokens = base_file.map(|file| file.tokens).unwrap_or(0);
        let head_tokens = head_file.map(|file| file.tokens).unwrap_or(0);
        if base_tokens == head_tokens {
            continue;
        }

        let delta = match (base_file, head_file) {
            (Some(base), Some(head)) if base.language != head.language => TokenDelta {
                added: head.tokens,
                removed: base.tokens,
                net: i128::from(head.tokens) - i128::from(base.tokens),
            },
            _ => TokenDelta::between(base_tokens, head_tokens),
        };
        tokens.add(delta);
        add_language_delta(&mut by_language, base_file, head_file, delta);
        let language = match (base_file, head_file) {
            (_, Some(head)) => head.language,
            (Some(base), None) => base.language,
            (None, None) => unreachable!("path union contains one snapshot"),
        };
        files.push(DiffFileResult {
            path: (*path).clone(),
            language,
            base_tokens: base_file.map(|file| file.tokens),
            head_tokens: head_file.map(|file| file.tokens),
            tokens: delta,
        });
    }

    let mut languages = by_language
        .into_iter()
        .map(|(name, tokens)| LanguageDelta { name, tokens })
        .collect::<Vec<_>>();
    languages.sort_by_key(|result| {
        (
            Reverse(u128::from(result.tokens.added) + u128::from(result.tokens.removed)),
            result.name.as_str(),
        )
    });

    Ok(DiffResult {
        tokenizer: base.tokenizer.clone(),
        text_policy: base.text_policy,
        invalid_utf8: base.invalid_utf8,
        base: base.totals,
        head: head.totals,
        tokens,
        languages,
        files,
        base_skipped: base.skipped.clone(),
        head_skipped: head.skipped.clone(),
    })
}

fn ensure_compatible(base: &ScanResult, head: &ScanResult) -> Result<(), DiffError> {
    if base.tokenizer.fingerprint != head.tokenizer.fingerprint {
        return Err(DiffError::IncompatibleSemantics("tokenizers"));
    }
    if base.text_policy != head.text_policy {
        return Err(DiffError::IncompatibleSemantics("text policies"));
    }
    if base.invalid_utf8 != head.invalid_utf8 {
        return Err(DiffError::IncompatibleSemantics("invalid UTF-8 policies"));
    }

    Ok(())
}

fn files_by_path(files: &[ScanFileResult]) -> BTreeMap<&PathBuf, &ScanFileResult> {
    files.iter().map(|file| (&file.path, file)).collect()
}

fn add_language_delta(
    languages: &mut BTreeMap<Language, TokenDelta>,
    base: Option<&ScanFileResult>,
    head: Option<&ScanFileResult>,
    delta: TokenDelta,
) {
    match (base, head) {
        (Some(base), Some(head)) if base.language != head.language => {
            languages
                .entry(base.language)
                .or_default()
                .add(TokenDelta::between(base.tokens, 0));
            languages
                .entry(head.language)
                .or_default()
                .add(TokenDelta::between(0, head.tokens));
        }
        (_, Some(head)) => languages.entry(head.language).or_default().add(delta),
        (Some(base), None) => languages.entry(base.language).or_default().add(delta),
        (None, None) => unreachable!("path union contains one snapshot"),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{TokenDelta, compare_snapshots};
    use crate::{
        CountOptions, InvalidUtf8Policy, Language, ScanFileResult, ScanResult, ScanTotals,
        SkippedSummary,
    };

    fn snapshot(files: &[(&str, Language, u64)]) -> ScanResult {
        let options = CountOptions::default();
        ScanResult {
            tokenizer: options.tokenizer().metadata().clone(),
            text_policy: options.text_policy().metadata(),
            invalid_utf8: InvalidUtf8Policy::Skip,
            totals: ScanTotals {
                files: u64::try_from(files.len()).unwrap(),
                tokens: files.iter().map(|(_, _, tokens)| tokens).sum(),
                bytes: 0,
            },
            languages: Vec::new(),
            files: files
                .iter()
                .map(|(path, language, tokens)| ScanFileResult {
                    path: PathBuf::from(path),
                    language: *language,
                    tokens: *tokens,
                    bytes: 0,
                    decoded_lossily: false,
                })
                .collect(),
            skipped: SkippedSummary::default(),
        }
    }

    #[test]
    fn compares_complete_file_footprints_deterministically() {
        let base = snapshot(&[
            ("unchanged.rs", Language::Rust, 10),
            ("changed.rs", Language::Rust, 10),
            ("removed.md", Language::Markdown, 7),
        ]);
        let head = snapshot(&[
            ("unchanged.rs", Language::Rust, 10),
            ("changed.rs", Language::Rust, 14),
            ("added.json", Language::Json, 3),
        ]);

        let result = compare_snapshots(&base, &head).unwrap();

        assert_eq!(
            result.tokens,
            TokenDelta {
                added: 7,
                removed: 7,
                net: 0,
            }
        );
        assert_eq!(
            result
                .files
                .iter()
                .map(|file| file.path.to_str().unwrap())
                .collect::<Vec<_>>(),
            ["added.json", "changed.rs", "removed.md"]
        );
        assert_eq!(
            result
                .languages
                .iter()
                .map(|language| language.name)
                .collect::<Vec<_>>(),
            [Language::Markdown, Language::Rust, Language::Json]
        );
        assert_eq!(result.base, base.totals);
        assert_eq!(result.head, head.totals);
    }

    #[test]
    fn language_reclassification_is_full_removal_and_addition() {
        let base = snapshot(&[("same.path", Language::Rust, 10)]);
        let head = snapshot(&[("same.path", Language::Markdown, 6)]);

        let result = compare_snapshots(&base, &head).unwrap();

        assert_eq!(result.tokens.added, 6);
        assert_eq!(result.tokens.removed, 10);
        assert_eq!(result.tokens.net, -4);
        assert_eq!(
            result.languages.iter().map(|item| item.tokens).fold(
                TokenDelta::default(),
                |mut total, delta| {
                    total.add(delta);
                    total
                }
            ),
            result.tokens
        );
    }

    #[test]
    fn rejects_different_counting_semantics() {
        let base = snapshot(&[]);
        let mut head = snapshot(&[]);
        head.tokenizer.fingerprint.push_str(":changed");

        let error = compare_snapshots(&base, &head).unwrap_err();

        assert_eq!(
            error.to_string(),
            "cannot compare snapshots with different tokenizers"
        );
    }
}
