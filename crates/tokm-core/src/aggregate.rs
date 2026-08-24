//! Deterministic scan aggregation.

use std::{cmp::Reverse, collections::BTreeMap, path::PathBuf};

use serde::Serialize;

use crate::{
    FileResult, InvalidUtf8Policy, Language, ScanError, ScanOptions, SkipDetail, SkipReason,
    TextPolicyMetadata, tokenizer::TokenizerMetadata,
};

/// Totals across all measured files.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
pub struct ScanTotals {
    /// Number of measured logical files.
    pub files: u64,

    /// Sum of file token counts.
    pub tokens: u64,

    /// Sum of original file sizes.
    pub bytes: u64,
}

/// Aggregate totals for one classified language family.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct LanguageResult {
    /// Language or text family.
    pub name: Language,

    /// Number of measured files in this family.
    pub files: u64,

    /// Sum of token counts in this family.
    pub tokens: u64,

    /// Sum of original file sizes in this family.
    pub bytes: u64,
}

/// Compact per-file entry returned by a recursive scan.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ScanFileResult {
    /// Original discovered path.
    pub path: PathBuf,

    /// Path-based language classification.
    pub language: Language,

    /// Exact tokenizer count.
    pub tokens: u64,

    /// Original file size in bytes.
    pub bytes: u64,

    /// Whether invalid bytes were replaced before tokenization.
    pub decoded_lossily: bool,
}

impl From<FileResult> for ScanFileResult {
    fn from(result: FileResult) -> Self {
        Self {
            path: result.path,
            language: result.language,
            tokens: result.tokens,
            bytes: result.bytes,
            decoded_lossily: result.decoded_lossily,
        }
    }
}

/// Aggregate and detailed accounting for omitted paths.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct SkippedSummary {
    /// Total number of skipped paths.
    pub total: u64,
    /// Binary paths.
    pub binary: u64,
    /// Paths exceeding the file-size limit.
    pub oversized: u64,
    /// Paths rejected as invalid UTF-8.
    pub invalid_utf8: u64,
    /// Paths with a recognizable unsupported encoding.
    pub unsupported_encoding: u64,
    /// Paths that could not be read.
    pub unreadable: u64,
    /// Paths that were not regular files.
    pub unsupported_filesystem_type: u64,
    /// Symbolic links not followed by policy.
    pub symlink: u64,
    /// Deterministically ordered per-path details.
    pub details: Vec<SkipDetail>,
}

impl SkippedSummary {
    fn from_details(mut details: Vec<SkipDetail>) -> Result<Self, ScanError> {
        details.sort_by(|left, right| {
            left.path
                .cmp(&right.path)
                .then_with(|| left.reason.cmp(&right.reason))
        });
        let mut summary = Self {
            total: u64::try_from(details.len()).map_err(|_| ScanError::AggregateOverflow)?,
            details,
            ..Self::default()
        };

        for detail in &summary.details {
            let count = match detail.reason {
                SkipReason::Binary => &mut summary.binary,
                SkipReason::Oversized => &mut summary.oversized,
                SkipReason::InvalidUtf8 => &mut summary.invalid_utf8,
                SkipReason::UnsupportedEncoding => &mut summary.unsupported_encoding,
                SkipReason::Unreadable => &mut summary.unreadable,
                SkipReason::UnsupportedFilesystemType => &mut summary.unsupported_filesystem_type,
                SkipReason::Symlink => &mut summary.symlink,
            };
            *count = count.checked_add(1).ok_or(ScanError::AggregateOverflow)?;
        }

        Ok(summary)
    }
}

/// Complete deterministic result of one filesystem scan.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ScanResult {
    /// Stable identity of the selected tokenizer.
    pub tokenizer: TokenizerMetadata,

    /// Versioned text transformation applied to every decoded file.
    pub text_policy: TextPolicyMetadata,

    /// Invalid UTF-8 policy active for the scan.
    pub invalid_utf8: InvalidUtf8Policy,

    /// Totals across measured files.
    pub totals: ScanTotals,

    /// Per-language totals sorted by tokens descending, then name.
    pub languages: Vec<LanguageResult>,

    /// Measured files sorted by normalized logical path.
    pub files: Vec<ScanFileResult>,

    /// Aggregate and per-path skip accounting.
    pub skipped: SkippedSummary,
}

pub(crate) fn aggregate(
    measured: Vec<FileResult>,
    skipped: Vec<SkipDetail>,
    options: &ScanOptions,
) -> Result<ScanResult, ScanError> {
    let mut totals = ScanTotals::default();
    let mut by_language: BTreeMap<Language, ScanTotals> = BTreeMap::new();
    let mut files = Vec::with_capacity(measured.len());

    for file in measured {
        add_file(&mut totals, file.tokens, file.bytes)?;
        let language = by_language.entry(file.language).or_default();
        add_file(language, file.tokens, file.bytes)?;
        files.push(ScanFileResult::from(file));
    }

    let mut languages = by_language
        .into_iter()
        .map(|(name, totals)| LanguageResult {
            name,
            files: totals.files,
            tokens: totals.tokens,
            bytes: totals.bytes,
        })
        .collect::<Vec<_>>();
    languages.sort_by_key(|result| (Reverse(result.tokens), result.name.as_str()));
    files.sort_by(|left, right| left.path.cmp(&right.path));

    Ok(ScanResult {
        tokenizer: options.count_options().tokenizer().metadata().clone(),
        text_policy: options.count_options().text_policy().metadata(),
        invalid_utf8: options.invalid_utf8(),
        totals,
        languages,
        files,
        skipped: SkippedSummary::from_details(skipped)?,
    })
}

fn add_file(totals: &mut ScanTotals, tokens: u64, bytes: u64) -> Result<(), ScanError> {
    totals.files = totals
        .files
        .checked_add(1)
        .ok_or(ScanError::AggregateOverflow)?;
    totals.tokens = totals
        .tokens
        .checked_add(tokens)
        .ok_or(ScanError::AggregateOverflow)?;
    totals.bytes = totals
        .bytes
        .checked_add(bytes)
        .ok_or(ScanError::AggregateOverflow)?;

    Ok(())
}
