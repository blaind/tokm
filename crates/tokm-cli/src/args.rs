//! Command-line contract and conversion to core options.

use std::{num::NonZeroU64, num::NonZeroUsize, path::PathBuf};

use clap::{Parser, Subcommand, ValueEnum};
use tokm_core::{
    CountOptions, InvalidUtf8Policy, MaxFileSize, ScanOptions, TextPolicy, TokenizerError,
    tokenizer::BuiltinEncoding,
};

/// Native token measurement for text and files.
#[derive(Clone, Debug, Parser)]
#[command(version, about)]
pub(crate) struct Args {
    /// Select a specialized measurement command.
    #[command(subcommand)]
    pub(crate) command: Option<Command>,

    /// Files, directories, or `-` for stdin.
    #[arg(value_name = "PATH", default_value = ".")]
    pub(crate) paths: Vec<PathBuf>,

    /// Select a built-in exact encoding (default: o200k_base).
    #[arg(long, global = true, value_parser = parse_encoding, conflicts_with_all = ["model", "tokenizer_file"], help_heading = "Tokenizer")]
    pub(crate) encoding: Option<BuiltinEncoding>,

    /// Resolve a supported model alias to a built-in encoding.
    #[arg(long, global = true, value_parser = parse_model, conflicts_with_all = ["encoding", "tokenizer_file"], help_heading = "Tokenizer")]
    pub(crate) model: Option<String>,

    /// Load a local Hugging Face tokenizer.json artifact.
    #[arg(
        long,
        global = true,
        value_name = "PATH",
        conflicts_with_all = ["encoding", "model"],
        help_heading = "Tokenizer"
    )]
    pub(crate) tokenizer_file: Option<PathBuf>,

    /// Remove a leading UTF-8 BOM and normalize line endings.
    #[arg(long, global = true, help_heading = "Text")]
    pub(crate) normalize: bool,

    /// Select invalid UTF-8 handling.
    #[arg(
        long,
        global = true,
        value_enum,
        default_value_t,
        help_heading = "Text"
    )]
    pub(crate) invalid_utf8: InvalidUtf8Arg,

    /// Show per-file results instead of language totals.
    #[arg(long, global = true, conflicts_with = "dirs", help_heading = "Display")]
    pub(crate) files: bool,

    /// Show recursive directory totals instead of language totals.
    #[arg(
        long,
        global = true,
        conflicts_with = "files",
        help_heading = "Display"
    )]
    pub(crate) dirs: bool,

    /// Select the output format.
    #[arg(
        long,
        global = true,
        value_enum,
        default_value_t,
        help_heading = "Display"
    )]
    pub(crate) format: OutputFormat,

    /// Sort per-file results.
    #[arg(
        long,
        global = true,
        value_enum,
        default_value_t,
        help_heading = "Display"
    )]
    pub(crate) sort: SortField,

    /// Compare the measured total with a context-window size.
    #[arg(long, global = true, value_name = "TOKENS", help_heading = "Display")]
    pub(crate) context: Option<NonZeroU64>,

    /// Include paths matching this glob; may be repeated.
    #[arg(long, global = true, value_name = "GLOB", help_heading = "Filtering")]
    pub(crate) include: Vec<String>,

    /// Exclude paths matching this glob; may be repeated.
    #[arg(long, global = true, value_name = "GLOB", help_heading = "Filtering")]
    pub(crate) exclude: Vec<String>,

    /// Include hidden paths.
    #[arg(long, global = true, help_heading = "Filtering")]
    pub(crate) hidden: bool,

    /// Disable ignore-file processing.
    #[arg(long, global = true, help_heading = "Filtering")]
    pub(crate) no_ignore: bool,

    /// Follow symbolic links.
    #[arg(long, global = true, help_heading = "Filtering")]
    pub(crate) follow: bool,

    /// Fail when the total exceeds this token budget.
    #[arg(long, global = true, value_name = "TOKENS", help_heading = "Limits")]
    pub(crate) max_tokens: Option<u64>,

    /// Set the individual-file byte limit, or `unlimited`.
    #[arg(long, global = true, default_value = "20M", value_parser = parse_file_size, help_heading = "Limits")]
    pub(crate) max_file_size: MaxFileSize,

    /// Disable the persistent local cache.
    #[arg(long, global = true, help_heading = "Performance")]
    pub(crate) no_cache: bool,

    /// Bound concurrent file workers.
    #[arg(
        short = 'j',
        long,
        global = true,
        value_name = "N",
        help_heading = "Performance"
    )]
    pub(crate) threads: Option<NonZeroUsize>,

    /// List skipped paths on stderr.
    #[arg(
        short,
        long,
        global = true,
        conflicts_with = "quiet",
        help_heading = "Diagnostics"
    )]
    pub(crate) verbose: bool,

    /// Suppress successful human-readable output.
    #[arg(
        short,
        long,
        global = true,
        conflicts_with = "verbose",
        help_heading = "Diagnostics"
    )]
    pub(crate) quiet: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Subcommand)]
pub(crate) enum Command {
    /// Compare complete-file token footprints across Git snapshots.
    Diff {
        /// BASE for a worktree comparison, or A..B for two committed trees.
        comparison: String,

        /// Discover the Git repository from this path.
        #[arg(long, default_value = ".", value_name = "PATH")]
        repository: PathBuf,
    },
}

impl Args {
    pub(crate) fn input_mode(&self) -> Result<InputMode, String> {
        let stdin_count = self
            .paths
            .iter()
            .filter(|path| path.as_os_str() == "-")
            .count();
        if stdin_count == 0 {
            return Ok(InputMode::Filesystem(self.paths.clone()));
        }
        if stdin_count == 1 && self.paths.len() == 1 {
            return Ok(InputMode::Stdin);
        }

        Err("stdin (`-`) cannot be combined with filesystem paths".to_owned())
    }

    pub(crate) fn scan_options(&self) -> Result<ScanOptions, TokenizerError> {
        let text_policy = if self.normalize {
            TextPolicy::Normalized
        } else {
            TextPolicy::Literal
        };
        let count = match (&self.tokenizer_file, &self.model, self.encoding) {
            (Some(path), None, None) => CountOptions::for_tokenizer_file(path)?,
            (None, Some(model), None) => CountOptions::for_model(model)?,
            (None, None, encoding) => CountOptions::for_encoding(encoding.unwrap_or_default()),
            _ => unreachable!("clap enforces mutually exclusive tokenizer selectors"),
        }
        .with_text_policy(text_policy);
        let mut options = ScanOptions::new(count)
            .with_invalid_utf8(self.invalid_utf8.into())
            .with_max_file_size(self.max_file_size)
            .with_hidden(self.hidden)
            .with_no_ignore(self.no_ignore)
            .with_follow_symlinks(self.follow)
            .with_cache_enabled(!self.no_cache);

        for pattern in &self.include {
            options = options.with_include(pattern);
        }
        for pattern in &self.exclude {
            options = options.with_exclude(pattern);
        }
        if let Some(threads) = self.threads {
            options = options.with_workers(threads);
        }

        Ok(options)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum InputMode {
    Filesystem(Vec<PathBuf>),
    Stdin,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, ValueEnum)]
pub(crate) enum InvalidUtf8Arg {
    #[default]
    Skip,
    Lossy,
}

impl From<InvalidUtf8Arg> for InvalidUtf8Policy {
    fn from(value: InvalidUtf8Arg) -> Self {
        match value {
            InvalidUtf8Arg::Skip => Self::Skip,
            InvalidUtf8Arg::Lossy => Self::Lossy,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, ValueEnum)]
pub(crate) enum OutputFormat {
    #[default]
    Table,
    Json,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, ValueEnum)]
pub(crate) enum SortField {
    #[default]
    Tokens,
    Path,
    Language,
}

fn parse_encoding(value: &str) -> Result<BuiltinEncoding, String> {
    value
        .parse::<BuiltinEncoding>()
        .map_err(|error| error.to_string())
}

fn parse_model(value: &str) -> Result<String, String> {
    BuiltinEncoding::for_model(value)
        .map(|_| value.to_owned())
        .map_err(|error| error.to_string())
}

fn parse_file_size(value: &str) -> Result<MaxFileSize, String> {
    if value.eq_ignore_ascii_case("unlimited") {
        return Ok(MaxFileSize::Unlimited);
    }

    let suffix_start = value
        .find(|character: char| !character.is_ascii_digit())
        .unwrap_or(value.len());
    let (number, suffix) = value.split_at(suffix_start);
    if number.is_empty() {
        return Err(
            "file size must be an integer followed by an optional K, M, or G suffix".into(),
        );
    }
    let number = number
        .parse::<u64>()
        .map_err(|_| "file size does not fit in u64".to_owned())?;
    let multiplier = match suffix.to_ascii_lowercase().as_str() {
        "" | "b" => 1,
        "k" | "kb" | "kib" => 1024,
        "m" | "mb" | "mib" => 1024 * 1024,
        "g" | "gb" | "gib" => 1024 * 1024 * 1024,
        _ => return Err(format!("unsupported file-size suffix: {suffix}")),
    };
    let bytes = number
        .checked_mul(multiplier)
        .ok_or_else(|| "file size does not fit in u64".to_owned())?;

    Ok(MaxFileSize::Limited(bytes))
}

#[cfg(test)]
mod tests {
    use clap::Parser;
    use tokm_core::{InvalidUtf8Policy, MaxFileSize, TextPolicy, tokenizer::BuiltinEncoding};

    use super::{Args, Command, InputMode, OutputFormat};

    #[test]
    fn defaults_are_stable_public_semantics() {
        let args = Args::try_parse_from(["tokm"]).unwrap();
        let options = args.scan_options().unwrap();

        assert_eq!(args.paths, [std::path::PathBuf::from(".")]);
        assert_eq!(args.encoding, None);
        assert_eq!(args.model, None);
        assert_eq!(
            options.count_options().tokenizer().metadata().name,
            BuiltinEncoding::O200kBase.as_str()
        );
        assert_eq!(options.count_options().text_policy(), TextPolicy::Literal);
        assert_eq!(options.invalid_utf8(), InvalidUtf8Policy::Skip);
        assert_eq!(
            options.max_file_size(),
            MaxFileSize::Limited(20 * 1024 * 1024)
        );
    }

    #[test]
    fn stdin_must_be_the_only_input() {
        let stdin = Args::try_parse_from(["tokm", "-"]).unwrap();
        let mixed = Args::try_parse_from(["tokm", "-", "src"]).unwrap();

        assert_eq!(stdin.input_mode().unwrap(), InputMode::Stdin);
        assert!(mixed.input_mode().is_err());
    }

    #[test]
    fn size_parser_accepts_binary_suffixes_and_unlimited() {
        let limited = Args::try_parse_from(["tokm", "--max-file-size", "3MiB"]).unwrap();
        let unlimited = Args::try_parse_from(["tokm", "--max-file-size", "unlimited"]).unwrap();

        assert_eq!(limited.max_file_size, MaxFileSize::Limited(3 * 1024 * 1024));
        assert_eq!(unlimited.max_file_size, MaxFileSize::Unlimited);
    }

    #[test]
    fn context_window_must_be_positive() {
        assert!(Args::try_parse_from(["tokm", "--context", "128000"]).is_ok());
        assert!(Args::try_parse_from(["tokm", "--context", "0"]).is_err());
    }

    #[test]
    fn file_and_directory_views_are_mutually_exclusive() {
        assert!(Args::try_parse_from(["tokm", "--files", "--dirs"]).is_err());
    }

    #[test]
    fn tokenizer_selectors_are_mutually_exclusive() {
        let pairs = [
            ["--encoding", "cl100k_base", "--model", "gpt-5"],
            [
                "--encoding",
                "cl100k_base",
                "--tokenizer-file",
                "tokenizer.json",
            ],
            ["--model", "gpt-5", "--tokenizer-file", "tokenizer.json"],
        ];

        for arguments in pairs {
            assert!(
                Args::try_parse_from(std::iter::once("tokm").chain(arguments.into_iter())).is_err()
            );
        }
    }

    #[test]
    fn model_alias_resolves_through_core() {
        let args = Args::try_parse_from(["tokm", "--model", "gpt-5"]).unwrap();
        let options = args.scan_options().unwrap();

        assert_eq!(args.model.as_deref(), Some("gpt-5"));
        assert_eq!(
            options.count_options().tokenizer().metadata().name,
            BuiltinEncoding::O200kBase.as_str()
        );
        assert!(Args::try_parse_from(["tokm", "--model", "unknown"]).is_err());
    }

    #[test]
    fn diff_subcommand_accepts_global_measurement_options() {
        let args = Args::try_parse_from([
            "tokm",
            "diff",
            "HEAD",
            "--format",
            "json",
            "--encoding",
            "cl100k_base",
        ])
        .unwrap();

        assert!(matches!(
            args.command,
            Some(Command::Diff { comparison, .. }) if comparison == "HEAD"
        ));
        assert_eq!(args.format, OutputFormat::Json);
        assert_eq!(args.encoding, Some(BuiltinEncoding::Cl100kBase));
    }
}
