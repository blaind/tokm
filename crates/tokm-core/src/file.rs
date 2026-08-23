//! Individual file measurement and skip policy.

use std::{
    fs::{self, File},
    io::{self, Read},
    num::NonZeroUsize,
    path::{Path, PathBuf},
};

use serde::Serialize;

use crate::{
    CountError, CountOptions, TextPolicyMetadata, classify, count_text,
    decode::{DecodeOutcome, decode},
    tokenizer::TokenizerMetadata,
};

/// Default maximum size of one measured file: 20 MiB.
pub const DEFAULT_MAX_FILE_SIZE: u64 = 20 * 1024 * 1024;

/// Maximum number of file bodies measured concurrently by default.
pub const DEFAULT_MAX_WORKERS: usize = 8;

/// Policy used when bytes are not valid UTF-8.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InvalidUtf8Policy {
    /// Omit invalid UTF-8 and report a skip.
    #[default]
    Skip,

    /// Replace invalid byte sequences using Rust-compatible lossy decoding.
    Lossy,
}

impl InvalidUtf8Policy {
    /// Return the stable identity used by cache keys.
    #[must_use]
    pub const fn fingerprint(self) -> &'static str {
        match self {
            Self::Skip => "invalid-utf8-skip-v1",
            Self::Lossy => "invalid-utf8-lossy-v1",
        }
    }
}

/// Maximum byte size admitted for one file.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MaxFileSize {
    /// Admit files up to and including this byte count.
    Limited(u64),

    /// Disable the individual-file size limit.
    Unlimited,
}

impl Default for MaxFileSize {
    fn default() -> Self {
        Self::Limited(DEFAULT_MAX_FILE_SIZE)
    }
}

impl MaxFileSize {
    fn admits(self, bytes: u64) -> bool {
        match self {
            Self::Limited(limit) => bytes <= limit,
            Self::Unlimited => true,
        }
    }
}

/// Options shared by explicit file counting and recursive scans.
#[derive(Clone)]
pub struct ScanOptions {
    count: CountOptions,
    invalid_utf8: InvalidUtf8Policy,
    max_file_size: MaxFileSize,
    include: Vec<String>,
    exclude: Vec<String>,
    hidden: bool,
    no_ignore: bool,
    follow_symlinks: bool,
    workers: NonZeroUsize,
}

impl ScanOptions {
    /// Construct scan options around shared counting semantics.
    #[must_use]
    pub fn new(count: CountOptions) -> Self {
        Self {
            count,
            invalid_utf8: InvalidUtf8Policy::Skip,
            max_file_size: MaxFileSize::default(),
            include: Vec::new(),
            exclude: Vec::new(),
            hidden: false,
            no_ignore: false,
            follow_symlinks: false,
            workers: default_workers(),
        }
    }

    /// Select invalid UTF-8 handling.
    #[must_use]
    pub fn with_invalid_utf8(mut self, policy: InvalidUtf8Policy) -> Self {
        self.invalid_utf8 = policy;
        self
    }

    /// Select the maximum admitted individual-file size.
    #[must_use]
    pub fn with_max_file_size(mut self, limit: MaxFileSize) -> Self {
        self.max_file_size = limit;
        self
    }

    /// Add an inclusive path glob. At least one include must match when set.
    #[must_use]
    pub fn with_include(mut self, pattern: impl Into<String>) -> Self {
        self.include.push(pattern.into());
        self
    }

    /// Add an exclusive path glob.
    #[must_use]
    pub fn with_exclude(mut self, pattern: impl Into<String>) -> Self {
        self.exclude.push(pattern.into());
        self
    }

    /// Include hidden paths during discovery.
    #[must_use]
    pub fn with_hidden(mut self, hidden: bool) -> Self {
        self.hidden = hidden;
        self
    }

    /// Disable `.gitignore`, `.ignore`, and Git exclude processing.
    #[must_use]
    pub fn with_no_ignore(mut self, no_ignore: bool) -> Self {
        self.no_ignore = no_ignore;
        self
    }

    /// Follow symbolic links during discovery and file measurement.
    #[must_use]
    pub fn with_follow_symlinks(mut self, follow: bool) -> Self {
        self.follow_symlinks = follow;
        self
    }

    /// Bound the number of files measured concurrently.
    #[must_use]
    pub fn with_workers(mut self, workers: NonZeroUsize) -> Self {
        self.workers = workers;
        self
    }

    /// Return the shared text-counting options.
    #[must_use]
    pub const fn count_options(&self) -> &CountOptions {
        &self.count
    }

    /// Return invalid UTF-8 handling.
    #[must_use]
    pub const fn invalid_utf8(&self) -> InvalidUtf8Policy {
        self.invalid_utf8
    }

    /// Return the individual-file size limit.
    #[must_use]
    pub const fn max_file_size(&self) -> MaxFileSize {
        self.max_file_size
    }

    pub(crate) fn includes(&self) -> &[String] {
        &self.include
    }

    pub(crate) fn excludes(&self) -> &[String] {
        &self.exclude
    }

    pub(crate) const fn hidden(&self) -> bool {
        self.hidden
    }

    pub(crate) const fn no_ignore(&self) -> bool {
        self.no_ignore
    }

    pub(crate) const fn follow_symlinks(&self) -> bool {
        self.follow_symlinks
    }

    pub(crate) const fn workers(&self) -> NonZeroUsize {
        self.workers
    }
}

#[cfg(feature = "builtin-tokenizers")]
impl Default for ScanOptions {
    fn default() -> Self {
        Self::new(CountOptions::default())
    }
}

/// Reason a discovered input was not measured.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SkipReason {
    /// Initial bytes matched the binary-file heuristic.
    Binary,
    /// File exceeded the selected byte limit.
    Oversized,
    /// Bytes were not valid UTF-8 under skip policy.
    InvalidUtf8,
    /// A recognizable unsupported text encoding was found.
    UnsupportedEncoding,
    /// Metadata or contents could not be read.
    Unreadable,
    /// The path was not a regular file.
    UnsupportedFilesystemType,
    /// A symbolic link was not followed.
    Symlink,
}

impl SkipReason {
    /// Return a concise human-readable label.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Binary => "binary",
            Self::Oversized => "oversized",
            Self::InvalidUtf8 => "invalid UTF-8",
            Self::UnsupportedEncoding => "unsupported encoding",
            Self::Unreadable => "unreadable",
            Self::UnsupportedFilesystemType => "unsupported filesystem type",
            Self::Symlink => "symlink",
        }
    }
}

/// Structured record for one omitted path.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SkipDetail {
    /// Original path supplied to or discovered by the engine.
    pub path: PathBuf,

    /// Stable skip category.
    pub reason: SkipReason,

    /// Optional specific encoding or other deterministic detail.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// Measurement for one regular text file.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct FileResult {
    /// Original path supplied to or discovered by the engine.
    pub path: PathBuf,

    /// Path-based language classification.
    pub language: crate::Language,

    /// Exact tokenizer count after text-policy handling.
    pub tokens: u64,

    /// Original file size in bytes.
    pub bytes: u64,

    /// Whether invalid bytes were replaced before tokenization.
    pub decoded_lossily: bool,

    /// Stable identity of the tokenizer used for the count.
    pub tokenizer: TokenizerMetadata,

    /// Versioned text transformation applied before tokenization.
    pub text_policy: TextPolicyMetadata,

    /// Invalid UTF-8 policy active for this measurement.
    pub invalid_utf8: InvalidUtf8Policy,
}

/// Result of attempting to measure one filesystem path.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum FileOutcome {
    /// The path was measured successfully.
    Measured(FileResult),

    /// The path was intentionally omitted with a visible reason.
    Skipped(SkipDetail),
}

/// Measure one filesystem path without aborting on per-file skips.
pub fn count_file(
    path: impl Into<PathBuf>,
    options: &ScanOptions,
) -> Result<FileOutcome, CountError> {
    let path = path.into();
    let link_metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(_) => return Ok(skipped(path, SkipReason::Unreadable, None)),
    };

    let metadata = if link_metadata.file_type().is_symlink() {
        if !options.follow_symlinks {
            return Ok(skipped(path, SkipReason::Symlink, None));
        }

        match fs::metadata(&path) {
            Ok(metadata) => metadata,
            Err(_) => return Ok(skipped(path, SkipReason::Unreadable, None)),
        }
    } else {
        link_metadata
    };
    if !metadata.is_file() {
        return Ok(skipped(path, SkipReason::UnsupportedFilesystemType, None));
    }
    if !options.max_file_size.admits(metadata.len()) {
        return Ok(skipped(path, SkipReason::Oversized, None));
    }

    let bytes = match read_bounded(&path, options.max_file_size) {
        Ok(Some(bytes)) => bytes,
        Ok(None) => return Ok(skipped(path, SkipReason::Oversized, None)),
        Err(_) => return Ok(skipped(path, SkipReason::Unreadable, None)),
    };
    let decoded = decode(&bytes, options.invalid_utf8);
    let (text, decoded_lossily) = match decoded {
        DecodeOutcome::Text {
            text,
            decoded_lossily,
        } => (text, decoded_lossily),
        DecodeOutcome::Skipped { reason, detail } => {
            return Ok(skipped(path, reason, detail.map(str::to_owned)));
        }
    };
    let count = count_text(&text, &options.count)?;

    Ok(FileOutcome::Measured(FileResult {
        language: classify(&path),
        path,
        tokens: count.tokens,
        bytes: u64::try_from(bytes.len()).map_err(|_| crate::TokenizerError::CountOverflow)?,
        decoded_lossily,
        tokenizer: count.tokenizer,
        text_policy: count.text_policy,
        invalid_utf8: options.invalid_utf8,
    }))
}

fn default_workers() -> NonZeroUsize {
    let available = std::thread::available_parallelism().map_or(1, NonZeroUsize::get);
    NonZeroUsize::new(available.min(DEFAULT_MAX_WORKERS)).expect("worker count is nonzero")
}

fn read_bounded(path: &Path, max_file_size: MaxFileSize) -> io::Result<Option<Vec<u8>>> {
    match max_file_size {
        MaxFileSize::Unlimited => fs::read(path).map(Some),
        MaxFileSize::Limited(limit) => {
            let file = File::open(path)?;
            let mut bytes = Vec::new();
            file.take(limit.saturating_add(1)).read_to_end(&mut bytes)?;
            let bytes_read = u64::try_from(bytes.len()).unwrap_or(u64::MAX);

            Ok((bytes_read <= limit).then_some(bytes))
        }
    }
}

fn skipped(path: PathBuf, reason: SkipReason, detail: Option<String>) -> FileOutcome {
    FileOutcome::Skipped(SkipDetail {
        path,
        reason,
        detail,
    })
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::{FileOutcome, InvalidUtf8Policy, MaxFileSize, ScanOptions, SkipReason, count_file};
    use crate::Language;

    #[test]
    fn measures_a_classified_utf8_file() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("main.rs");
        fs::write(&path, "fn main() {}\n").unwrap();

        let outcome = count_file(&path, &ScanOptions::default()).unwrap();

        let FileOutcome::Measured(result) = outcome else {
            panic!("text file was skipped");
        };
        assert_eq!(result.language, Language::Rust);
        assert_eq!(result.bytes, 13);
        assert!(!result.decoded_lossily);
    }

    #[test]
    fn reports_binary_and_oversized_files() {
        let directory = tempdir().unwrap();
        let binary_path = directory.path().join("binary");
        let large_path = directory.path().join("large.txt");
        fs::write(&binary_path, [b'a', 0, b'b']).unwrap();
        fs::write(&large_path, "four").unwrap();

        let binary = count_file(&binary_path, &ScanOptions::default()).unwrap();
        let oversized = count_file(
            &large_path,
            &ScanOptions::default().with_max_file_size(MaxFileSize::Limited(3)),
        )
        .unwrap();

        assert!(matches!(
            binary,
            FileOutcome::Skipped(detail) if detail.reason == SkipReason::Binary
        ));
        assert!(matches!(
            oversized,
            FileOutcome::Skipped(detail) if detail.reason == SkipReason::Oversized
        ));
    }

    #[test]
    fn invalid_utf8_policy_is_explicit() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("invalid.txt");
        fs::write(&path, [b'a', 0xff, b'b']).unwrap();

        let skipped = count_file(&path, &ScanOptions::default()).unwrap();
        let lossy = count_file(
            &path,
            &ScanOptions::default().with_invalid_utf8(InvalidUtf8Policy::Lossy),
        )
        .unwrap();

        assert!(matches!(
            skipped,
            FileOutcome::Skipped(detail) if detail.reason == SkipReason::InvalidUtf8
        ));
        assert!(matches!(
            lossy,
            FileOutcome::Measured(result) if result.decoded_lossily
        ));
    }
}
