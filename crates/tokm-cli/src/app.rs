//! CLI execution over the shared core engine.

use std::{fmt, io, io::Read, path::PathBuf};

use tokm_core::{
    ByteOutcome, ByteSkip, DiffResult, Language, LanguageResult, MaxFileSize, ScanFileResult,
    ScanOptions, ScanResult, ScanTotals, SkipDetail, SkipReason, SkippedSummary, count_bytes,
    git::{diff_trees, diff_worktree},
    scan,
};

use crate::args::{Args, Command, InputMode};

pub(crate) fn execute(args: &Args) -> Result<Execution, CliError> {
    let options = args
        .scan_options()
        .map_err(|error| CliError::Operational(error.to_string()))?;

    match &args.command {
        Some(Command::Diff {
            comparison,
            repository,
        }) => execute_diff(comparison, repository, &options),
        None => execute_scan(args, &options),
    }
}

#[derive(Debug)]
pub(crate) enum Execution {
    Scan(ScanResult),
    Diff(DiffExecution),
}

impl Execution {
    pub(crate) const fn measured_tokens(&self) -> u64 {
        match self {
            Self::Scan(result) => result.totals.tokens,
            Self::Diff(diff) => diff.result.head.tokens,
        }
    }
}

#[derive(Debug)]
pub(crate) struct DiffExecution {
    pub(crate) base: String,
    pub(crate) head: String,
    pub(crate) result: DiffResult,
}

fn execute_scan(args: &Args, options: &ScanOptions) -> Result<Execution, CliError> {
    let input = args.input_mode().map_err(CliError::Usage)?;

    match input {
        InputMode::Filesystem(paths) => scan(paths, options)
            .map(Execution::Scan)
            .map_err(|error| CliError::Operational(error.to_string())),
        InputMode::Stdin => {
            let stdin = io::stdin();
            let bytes = read_bounded(stdin.lock(), options.max_file_size())
                .map_err(|error| CliError::Operational(format!("cannot read stdin: {error}")))?;
            let outcome = count_bytes(&bytes, options)
                .map_err(|error| CliError::Operational(error.to_string()))?;

            Ok(Execution::Scan(stdin_result(outcome, options)))
        }
    }
}

fn execute_diff(
    comparison: &str,
    repository: &std::path::Path,
    options: &ScanOptions,
) -> Result<Execution, CliError> {
    if let Some((base, head)) = comparison.split_once("..") {
        if base.is_empty() || head.is_empty() || head.starts_with('.') || head.contains("..") {
            return Err(CliError::Usage(
                "Git tree comparison must use non-empty A..B revisions".to_owned(),
            ));
        }
        let result = diff_trees(repository, base, head, options)
            .map_err(|error| CliError::Operational(error.to_string()))?;
        Ok(Execution::Diff(DiffExecution {
            base: base.to_owned(),
            head: head.to_owned(),
            result,
        }))
    } else if comparison.is_empty() {
        Err(CliError::Usage(
            "Git base revision cannot be empty".to_owned(),
        ))
    } else {
        let result = diff_worktree(repository, comparison, options)
            .map_err(|error| CliError::Operational(error.to_string()))?;
        Ok(Execution::Diff(DiffExecution {
            base: comparison.to_owned(),
            head: "worktree".to_owned(),
            result,
        }))
    }
}

#[derive(Debug)]
pub(crate) enum CliError {
    Usage(String),
    Operational(String),
}

impl CliError {
    pub(crate) const fn exit_code(&self) -> u8 {
        match self {
            Self::Usage(_) => 2,
            Self::Operational(_) => 1,
        }
    }
}

impl fmt::Display for CliError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Usage(message) | Self::Operational(message) => formatter.write_str(message),
        }
    }
}

fn read_bounded(mut reader: impl Read, max_file_size: MaxFileSize) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    match max_file_size {
        MaxFileSize::Unlimited => reader.read_to_end(&mut bytes)?,
        MaxFileSize::Limited(limit) => reader
            .take(limit.saturating_add(1))
            .read_to_end(&mut bytes)?,
    };

    Ok(bytes)
}

fn stdin_result(outcome: ByteOutcome, options: &ScanOptions) -> ScanResult {
    match outcome {
        ByteOutcome::Measured(result) => ScanResult {
            tokenizer: result.tokenizer,
            text_policy: result.text_policy,
            invalid_utf8: result.invalid_utf8,
            totals: ScanTotals {
                files: 1,
                tokens: result.tokens,
                bytes: result.bytes,
            },
            languages: vec![LanguageResult {
                name: Language::Unknown,
                files: 1,
                tokens: result.tokens,
                bytes: result.bytes,
            }],
            files: vec![ScanFileResult {
                path: PathBuf::from("-"),
                language: Language::Unknown,
                tokens: result.tokens,
                bytes: result.bytes,
                decoded_lossily: result.decoded_lossily,
            }],
            skipped: SkippedSummary::default(),
        },
        ByteOutcome::Skipped(skip) => ScanResult {
            tokenizer: options.count_options().tokenizer().metadata().clone(),
            text_policy: options.count_options().text_policy().metadata(),
            invalid_utf8: options.invalid_utf8(),
            totals: ScanTotals::default(),
            languages: Vec::new(),
            files: Vec::new(),
            skipped: stdin_skip_summary(skip),
        },
    }
}

fn stdin_skip_summary(skip: ByteSkip) -> SkippedSummary {
    let mut summary = SkippedSummary {
        total: 1,
        details: vec![SkipDetail {
            path: PathBuf::from("-"),
            reason: skip.reason,
            detail: skip.detail,
        }],
        ..SkippedSummary::default()
    };
    match skip.reason {
        SkipReason::Binary => summary.binary = 1,
        SkipReason::Oversized => summary.oversized = 1,
        SkipReason::InvalidUtf8 => summary.invalid_utf8 = 1,
        SkipReason::UnsupportedEncoding => summary.unsupported_encoding = 1,
        SkipReason::Unreadable => summary.unreadable = 1,
        SkipReason::UnsupportedFilesystemType => summary.unsupported_filesystem_type = 1,
        SkipReason::Symlink => summary.symlink = 1,
    }

    summary
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use tokm_core::MaxFileSize;

    use super::read_bounded;

    #[test]
    fn bounded_stdin_reads_one_byte_past_the_limit_for_skip_detection() {
        let bytes = read_bounded(Cursor::new(b"abcdef"), MaxFileSize::Limited(3)).unwrap();

        assert_eq!(bytes, b"abcd");
    }
}
