//! Deterministic scan aggregation.

use std::{
    cmp::Reverse,
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use path_clean::PathClean;
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

/// Recursive totals for one logical directory.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DirectoryResult {
    /// Directory path, with `.` representing a relative scan root.
    pub path: PathBuf,

    /// Number of measured files beneath this directory.
    pub files: u64,

    /// Sum of token counts beneath this directory.
    pub tokens: u64,

    /// Sum of original file sizes beneath this directory.
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

/// Roll measured files into every ancestor directory up to their input root.
///
/// Roots use the same logical paths as scan inputs. A root equal to a measured
/// file is treated as an explicit file input and rolls up to its parent. When
/// roots overlap, the widest matching root wins so each file contributes once
/// to the complete recursive hierarchy.
pub fn aggregate_directories<I, P>(
    files: &[ScanFileResult],
    roots: I,
) -> Result<Vec<DirectoryResult>, ScanError>
where
    I: IntoIterator<Item = P>,
    P: Into<PathBuf>,
{
    let roots = roots
        .into_iter()
        .map(Into::into)
        .map(normalize_root)
        .collect::<Vec<_>>();
    let mut by_directory: BTreeMap<PathBuf, ScanTotals> = BTreeMap::new();

    for file in files {
        let path = file.path.clone().clean();
        let directory = path.parent().unwrap_or_else(|| Path::new("")).to_path_buf();
        let root = roots
            .iter()
            .map(|root| {
                if root == &path {
                    directory.clone()
                } else {
                    root.clone()
                }
            })
            .filter(|root| is_within(&directory, root))
            .min_by_key(|root| root.components().count())
            .unwrap_or_else(|| directory.clone());
        let mut current = directory;

        loop {
            let display_path = if current.as_os_str().is_empty() {
                PathBuf::from(".")
            } else {
                current.clone()
            };
            add_file(
                by_directory.entry(display_path).or_default(),
                file.tokens,
                file.bytes,
            )?;
            if current == root {
                break;
            }
            let Some(parent) = current.parent() else {
                break;
            };
            current = parent.to_path_buf();
        }
    }

    Ok(by_directory
        .into_iter()
        .map(|(path, totals)| DirectoryResult {
            path,
            files: totals.files,
            tokens: totals.tokens,
            bytes: totals.bytes,
        })
        .collect())
}

fn normalize_root(path: PathBuf) -> PathBuf {
    let path = path.clean();
    if path == Path::new(".") {
        PathBuf::new()
    } else {
        path
    }
}

fn is_within(path: &Path, root: &Path) -> bool {
    if root.as_os_str().is_empty() {
        !path.is_absolute()
    } else {
        path.starts_with(root)
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

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{ScanFileResult, aggregate_directories};
    use crate::Language;

    fn file(path: &str, tokens: u64, bytes: u64) -> ScanFileResult {
        ScanFileResult {
            path: PathBuf::from(path),
            language: Language::Rust,
            tokens,
            bytes,
            decoded_lossily: false,
        }
    }

    #[test]
    fn directory_totals_roll_up_recursively_to_the_scan_root() {
        let files = [
            file("src/api/server.rs", 10, 100),
            file("src/main.rs", 5, 50),
            file("README.md", 3, 30),
        ];

        let directories = aggregate_directories(&files, ["."]).unwrap();

        assert_eq!(
            directories
                .iter()
                .map(|directory| (
                    directory.path.to_string_lossy().into_owned(),
                    directory.files,
                    directory.tokens,
                    directory.bytes,
                ))
                .collect::<Vec<_>>(),
            [
                (".".to_owned(), 3, 18, 180),
                ("src".to_owned(), 2, 15, 150),
                ("src/api".to_owned(), 1, 10, 100),
            ]
        );
    }

    #[test]
    fn overlapping_roots_do_not_double_count_files() {
        let files = [file("src/lib.rs", 7, 20)];

        let directories = aggregate_directories(&files, [".", "src"]).unwrap();

        assert_eq!(directories.len(), 2);
        assert!(directories.iter().all(|directory| directory.files == 1));
        assert_eq!(
            directories
                .iter()
                .map(|directory| directory.tokens)
                .sum::<u64>(),
            14
        );
    }

    #[test]
    fn explicit_file_inputs_roll_up_to_their_parent() {
        let files = [file("docs/guide.md", 4, 12)];

        let directories = aggregate_directories(&files, ["docs/guide.md"]).unwrap();

        assert_eq!(directories.len(), 1);
        assert_eq!(directories[0].path, PathBuf::from("docs"));
        assert_eq!(directories[0].tokens, 4);
    }
}
