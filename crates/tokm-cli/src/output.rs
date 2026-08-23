//! Deterministic human and JSON reporting.

use std::{
    cmp::Ordering,
    collections::{BTreeMap, BTreeSet},
    io,
    io::Write,
    num::NonZeroU64,
    path::{Path, PathBuf},
};

use serde::Serialize;
use tokm_core::{
    DiffFileResult, DiffResult, DirectoryResult, InvalidUtf8Policy, Language, ScanFileResult,
    ScanResult, ScanTotals, SkipDetail, SkippedSummary, TokenDelta, aggregate_directories,
    tokenizer::TokenizerMetadata,
};

use crate::{
    app::{DiffExecution, Execution},
    args::{Args, InputPrice, OutputFormat, SortField},
};

const JSON_SCHEMA_VERSION: u32 = 1;

pub(crate) fn write_report(
    mut writer: impl Write,
    result: &Execution,
    args: &Args,
) -> io::Result<()> {
    match result {
        Execution::Scan(result) => write_scan_report(&mut writer, result, args),
        Execution::Diff(diff) => write_diff_report(&mut writer, diff, args),
    }
}

fn write_scan_report(writer: &mut impl Write, result: &ScanResult, args: &Args) -> io::Result<()> {
    match args.format {
        OutputFormat::Json => write_json(writer, result, args),
        OutputFormat::Table if args.quiet => Ok(()),
        OutputFormat::Table if args.dirs => {
            let directories = aggregate_directories(&result.files, args.paths.clone())
                .map_err(io::Error::other)?;
            write_directory_table(writer, &directories, &result.totals, args.sort)?;
            write_skip_summary(writer, &result.skipped)?;
            write_measurement_summaries(writer, result.totals.tokens, args)
        }
        OutputFormat::Table if args.tree => {
            let directories = aggregate_directories(&result.files, args.paths.clone())
                .map_err(io::Error::other)?;
            write_directory_tree(writer, &directories)?;
            write_skip_summary(writer, &result.skipped)?;
            write_measurement_summaries(writer, result.totals.tokens, args)
        }
        OutputFormat::Table if args.files => {
            write_file_table(writer, result, args.sort)?;
            write_measurement_summaries(writer, result.totals.tokens, args)
        }
        OutputFormat::Table if args.density => {
            write_density_table(writer, result)?;
            write_measurement_summaries(writer, result.totals.tokens, args)
        }
        OutputFormat::Table => {
            write_language_table(writer, result)?;
            write_measurement_summaries(writer, result.totals.tokens, args)
        }
    }
}

pub(crate) fn write_verbose_skips(mut writer: impl Write, result: &Execution) -> io::Result<()> {
    match result {
        Execution::Scan(result) => write_skip_details(&mut writer, None, &result.skipped.details),
        Execution::Diff(diff) => {
            write_skip_details(&mut writer, Some("base"), &diff.result.base_skipped.details)?;
            write_skip_details(&mut writer, Some("head"), &diff.result.head_skipped.details)
        }
    }
}

fn write_skip_details(
    writer: &mut impl Write,
    snapshot: Option<&str>,
    details: &[SkipDetail],
) -> io::Result<()> {
    for detail in details {
        write!(writer, "skipped")?;
        if let Some(snapshot) = snapshot {
            write!(writer, " [{snapshot}]")?;
        }
        write!(
            writer,
            " ({}): {}",
            detail.reason.as_str(),
            detail.path.display()
        )?;
        if let Some(specific) = &detail.detail {
            write!(writer, " ({specific})")?;
        }
        writeln!(writer)?;
    }

    Ok(())
}

pub(crate) fn write_budget_failure(
    mut writer: impl Write,
    total: u64,
    budget: u64,
) -> io::Result<()> {
    writeln!(writer, "Token budget exceeded")?;
    writeln!(writer, "  Total:       {} tokens", format_number(total))?;
    writeln!(writer, "  Budget:      {} tokens", format_number(budget))?;
    writeln!(
        writer,
        "  Exceeded by: {} tokens",
        format_number(total - budget)
    )
}

fn write_diff_report(writer: &mut impl Write, diff: &DiffExecution, args: &Args) -> io::Result<()> {
    match args.format {
        OutputFormat::Json => write_diff_json(writer, diff, args),
        OutputFormat::Table if args.quiet => Ok(()),
        OutputFormat::Table => write_diff_table(writer, diff, args),
    }
}

fn write_diff_table(writer: &mut impl Write, diff: &DiffExecution, args: &Args) -> io::Result<()> {
    let result = &diff.result;
    writeln!(writer, " Token delta ({} -> {})", diff.base, diff.head)?;
    writeln!(writer, "{}", "-".repeat(42))?;
    writeln!(
        writer,
        " Added tokens          +{}",
        format_number(result.tokens.added)
    )?;
    writeln!(
        writer,
        " Removed tokens        -{}",
        format_number(result.tokens.removed)
    )?;
    writeln!(
        writer,
        " Net                    {}",
        format_signed(result.tokens.net)
    )?;

    if args.files {
        write_diff_file_table(writer, result, args.sort)?;
    } else {
        write_diff_language_table(writer, result)?;
    }
    write_diff_skip_summary(writer, "Base", &result.base_skipped)?;
    write_diff_skip_summary(writer, "Head", &result.head_skipped)?;
    write_measurement_summaries(writer, result.head.tokens, args)
}

fn write_measurement_summaries(
    writer: &mut impl Write,
    tokens: u64,
    args: &Args,
) -> io::Result<()> {
    write_context_summary(writer, tokens, args.context)?;
    write_cost_summary(writer, tokens, args.price_input, args.context.is_none())
}

fn write_context_summary(
    writer: &mut impl Write,
    tokens: u64,
    context: Option<NonZeroU64>,
) -> io::Result<()> {
    let Some(context) = context else {
        return Ok(());
    };
    let comparison = ContextComparison::new(tokens, context);

    writeln!(writer)?;
    writeln!(writer, "Tokens:          {}", format_number(tokens))?;
    writeln!(
        writer,
        "Context window:  {}",
        format_number(comparison.window_tokens)
    )?;
    writeln!(
        writer,
        "Utilization:     {}.{}%",
        format_unsigned(comparison.utilization_percent_tenths / 10),
        comparison.utilization_percent_tenths % 10,
    )?;
    if comparison.exceeded_by_tokens == 0 {
        writeln!(
            writer,
            "Remaining:       {}",
            format_number(comparison.remaining_tokens)
        )
    } else {
        writeln!(
            writer,
            "Exceeded by:     {}",
            format_number(comparison.exceeded_by_tokens)
        )
    }
}

fn write_cost_summary(
    writer: &mut impl Write,
    tokens: u64,
    price: Option<InputPrice>,
    include_tokens: bool,
) -> io::Result<()> {
    let Some(price) = price else {
        return Ok(());
    };
    let estimate = CostEstimate::new(tokens, price);

    if include_tokens {
        writeln!(writer)?;
        writeln!(writer, "Tokens:          {}", format_number(tokens))?;
    }
    writeln!(
        writer,
        "Price:           ${} / 1M tokens",
        format_input_price(price, true)
    )?;
    writeln!(
        writer,
        "Estimated cost:  ${}",
        format_estimated_cost(estimate.ten_thousandths, true)
    )
}

fn write_diff_language_table(writer: &mut impl Write, result: &DiffResult) -> io::Result<()> {
    if result.languages.is_empty() {
        return Ok(());
    }

    writeln!(writer)?;
    writeln!(
        writer,
        " Language                       Added       Removed           Net"
    )?;
    writeln!(writer, "{}", "-".repeat(66))?;
    for language in &result.languages {
        writeln!(
            writer,
            " {:<26}  {:>11}  {:>11}  {:>12}",
            language.name.as_str(),
            format!("+{}", format_number(language.tokens.added)),
            format!("-{}", format_number(language.tokens.removed)),
            format_signed(language.tokens.net),
        )?;
    }

    Ok(())
}

fn write_diff_file_table(
    writer: &mut impl Write,
    result: &DiffResult,
    sort: SortField,
) -> io::Result<()> {
    if result.files.is_empty() {
        return Ok(());
    }
    let mut files = result.files.iter().collect::<Vec<_>>();
    files.sort_by(|left, right| compare_diff_files(left, right, sort));

    writeln!(writer)?;
    writeln!(writer, "          Net  Language          File")?;
    writeln!(writer, "{}", "-".repeat(66))?;
    for file in files {
        writeln!(
            writer,
            " {:>12}  {:<16}  {}",
            format_signed(file.tokens.net),
            file.language.as_str(),
            file.path.display(),
        )?;
    }

    Ok(())
}

fn compare_diff_files(left: &DiffFileResult, right: &DiffFileResult, sort: SortField) -> Ordering {
    match sort {
        SortField::Tokens => diff_impact(right)
            .cmp(&diff_impact(left))
            .then_with(|| left.path.cmp(&right.path)),
        SortField::Path => left.path.cmp(&right.path),
        SortField::Language => left
            .language
            .as_str()
            .cmp(right.language.as_str())
            .then_with(|| left.path.cmp(&right.path)),
    }
}

fn diff_impact(file: &DiffFileResult) -> u128 {
    u128::from(file.tokens.added) + u128::from(file.tokens.removed)
}

fn write_diff_skip_summary(
    writer: &mut impl Write,
    snapshot: &str,
    skipped: &SkippedSummary,
) -> io::Result<()> {
    if skipped.total == 0 {
        return Ok(());
    }
    writeln!(writer)?;
    writeln!(writer, "{snapshot} snapshot:")?;
    write_skip_summary(writer, skipped)
}

fn write_language_table(writer: &mut impl Write, result: &ScanResult) -> io::Result<()> {
    let percentages = allocated_percent_tenths(
        &result
            .languages
            .iter()
            .map(|language| language.tokens)
            .collect::<Vec<_>>(),
    );
    let language_width = result
        .languages
        .iter()
        .map(|language| language.name.as_str().len())
        .max()
        .unwrap_or(8)
        .max("Language".len());
    let files_width = result
        .languages
        .iter()
        .map(|language| format_number(language.files).len())
        .max()
        .unwrap_or(5)
        .max("Files".len());
    let tokens_width = result
        .languages
        .iter()
        .map(|language| format_number(language.tokens).len())
        .chain([format_number(result.totals.tokens).len()])
        .max()
        .unwrap_or(6)
        .max("Tokens".len());
    let table_width = language_width + files_width + tokens_width + 13;

    writeln!(
        writer,
        " {:<language_width$}  {:>files_width$}  {:>tokens_width$}  {:>7}",
        "Language", "Files", "Tokens", "Percent"
    )?;
    writeln!(writer, "{}", "-".repeat(table_width))?;
    for (language, percent) in result.languages.iter().zip(percentages) {
        let percent = format!("{}.{}%", percent / 10, percent % 10);
        writeln!(
            writer,
            " {:<language_width$}  {:>files_width$}  {:>tokens_width$}  {:>7}",
            language.name.as_str(),
            format_number(language.files),
            format_number(language.tokens),
            percent,
        )?;
    }
    writeln!(writer, "{}", "-".repeat(table_width))?;
    let total_percent = if result.totals.tokens == 0 {
        "0.0%"
    } else {
        "100.0%"
    };
    writeln!(
        writer,
        " {:<language_width$}  {:>files_width$}  {:>tokens_width$}  {:>7}",
        "Total",
        format_number(result.totals.files),
        format_number(result.totals.tokens),
        total_percent,
    )?;

    write_skip_summary(writer, &result.skipped)
}

fn write_density_table(writer: &mut impl Write, result: &ScanResult) -> io::Result<()> {
    let language_width = result
        .languages
        .iter()
        .map(|language| language.name.as_str().len())
        .max()
        .unwrap_or(8)
        .max("Language".len());
    let files_width = result
        .languages
        .iter()
        .map(|language| format_number(language.files).len())
        .chain([format_number(result.totals.files).len()])
        .max()
        .unwrap_or(5)
        .max("Files".len());
    let tokens_width = result
        .languages
        .iter()
        .map(|language| format_number(language.tokens).len())
        .chain([format_number(result.totals.tokens).len()])
        .max()
        .unwrap_or(6)
        .max("Tokens".len());
    let table_width = language_width + files_width + tokens_width + 30;

    writeln!(
        writer,
        " {:<language_width$}  {:>files_width$}  {:>tokens_width$}  {:>9}  {:>11}",
        "Language", "Files", "Tokens", "Tokens/KB", "Bytes/Token"
    )?;
    writeln!(writer, "{}", "-".repeat(table_width))?;
    for language in &result.languages {
        writeln!(
            writer,
            " {:<language_width$}  {:>files_width$}  {:>tokens_width$}  {:>9}  {:>11}",
            language.name.as_str(),
            format_number(language.files),
            format_number(language.tokens),
            format_decimal_tenths(tokens_per_kb_tenths(language.tokens, language.bytes)),
            format_decimal_tenths(bytes_per_token_tenths(language.bytes, language.tokens)),
        )?;
    }
    writeln!(writer, "{}", "-".repeat(table_width))?;
    writeln!(
        writer,
        " {:<language_width$}  {:>files_width$}  {:>tokens_width$}  {:>9}  {:>11}",
        "Total",
        format_number(result.totals.files),
        format_number(result.totals.tokens),
        format_decimal_tenths(tokens_per_kb_tenths(
            result.totals.tokens,
            result.totals.bytes,
        )),
        format_decimal_tenths(bytes_per_token_tenths(
            result.totals.bytes,
            result.totals.tokens,
        )),
    )?;

    write_skip_summary(writer, &result.skipped)
}

fn write_file_table(
    writer: &mut impl Write,
    result: &ScanResult,
    sort: SortField,
) -> io::Result<()> {
    let mut files = result.files.iter().collect::<Vec<_>>();
    files.sort_by(|left, right| compare_files(left, right, sort));
    let tokens_width = files
        .iter()
        .map(|file| format_number(file.tokens).len())
        .max()
        .unwrap_or(6)
        .max("Tokens".len());
    let language_width = files
        .iter()
        .map(|file| file.language.as_str().len())
        .max()
        .unwrap_or(8)
        .max("Language".len());
    let path_width = files
        .iter()
        .map(|file| file.path.display().to_string().len())
        .max()
        .unwrap_or(4)
        .max("File".len());
    let table_width = tokens_width + language_width + path_width + 8;

    writeln!(
        writer,
        " {:>tokens_width$}  {:<language_width$}  File",
        "Tokens", "Language"
    )?;
    writeln!(writer, "{}", "-".repeat(table_width))?;
    for file in files {
        writeln!(
            writer,
            " {:>tokens_width$}  {:<language_width$}  {}",
            format_number(file.tokens),
            file.language.as_str(),
            file.path.display(),
        )?;
    }
    writeln!(writer, "{}", "-".repeat(table_width))?;
    writeln!(
        writer,
        " {:>tokens_width$}  {:<language_width$}  Total ({} files)",
        format_number(result.totals.tokens),
        "",
        format_number(result.totals.files),
    )?;

    write_skip_summary(writer, &result.skipped)
}

fn write_directory_table(
    writer: &mut impl Write,
    directories: &[DirectoryResult],
    totals: &ScanTotals,
    sort: SortField,
) -> io::Result<()> {
    let mut directories = directories.iter().collect::<Vec<_>>();
    directories.sort_by(|left, right| compare_directories(left, right, sort));
    let tokens_width = directories
        .iter()
        .map(|directory| format_number(directory.tokens).len())
        .chain([format_number(totals.tokens).len()])
        .max()
        .unwrap_or(6)
        .max("Tokens".len());
    let files_width = directories
        .iter()
        .map(|directory| format_number(directory.files).len())
        .chain([format_number(totals.files).len()])
        .max()
        .unwrap_or(5)
        .max("Files".len());
    let path_width = directories
        .iter()
        .map(|directory| directory.path.display().to_string().len())
        .max()
        .unwrap_or(9)
        .max("Directory".len());
    let table_width = tokens_width + files_width + path_width + 7;

    writeln!(
        writer,
        " {:>tokens_width$}  {:>files_width$}  Directory",
        "Tokens", "Files"
    )?;
    writeln!(writer, "{}", "-".repeat(table_width))?;
    for directory in directories {
        writeln!(
            writer,
            " {:>tokens_width$}  {:>files_width$}  {}",
            format_number(directory.tokens),
            format_number(directory.files),
            directory.path.display(),
        )?;
    }
    writeln!(writer, "{}", "-".repeat(table_width))?;
    writeln!(
        writer,
        " {:>tokens_width$}  {:>files_width$}  Total",
        format_number(totals.tokens),
        format_number(totals.files),
    )
}

fn write_directory_tree(
    writer: &mut impl Write,
    directories: &[DirectoryResult],
) -> io::Result<()> {
    if directories.is_empty() {
        return writeln!(writer, " 0 (no measured directories)");
    }

    let paths = directories
        .iter()
        .map(|directory| directory.path.clone())
        .collect::<BTreeSet<_>>();
    let mut children: BTreeMap<PathBuf, Vec<&DirectoryResult>> = BTreeMap::new();
    let mut roots = Vec::new();
    for directory in directories {
        match tree_parent(&directory.path).filter(|parent| paths.contains(parent)) {
            Some(parent) => children.entry(parent).or_default().push(directory),
            None => roots.push(directory),
        }
    }
    roots.sort_by(|left, right| left.path.cmp(&right.path));
    for siblings in children.values_mut() {
        siblings.sort_by(|left, right| left.path.cmp(&right.path));
    }

    let tokens_width = directories
        .iter()
        .map(|directory| format_number(directory.tokens).len())
        .max()
        .unwrap_or(1);
    for root in roots {
        writeln!(
            writer,
            "{:>tokens_width$} {}",
            format_number(root.tokens),
            root.path.display(),
        )?;
        write_tree_children(writer, &root.path, &children, "", tokens_width)?;
    }

    Ok(())
}

fn write_tree_children(
    writer: &mut impl Write,
    parent: &Path,
    children: &BTreeMap<PathBuf, Vec<&DirectoryResult>>,
    prefix: &str,
    tokens_width: usize,
) -> io::Result<()> {
    let Some(siblings) = children.get(parent) else {
        return Ok(());
    };

    for (index, directory) in siblings.iter().enumerate() {
        let last = index + 1 == siblings.len();
        let connector = if last { "└── " } else { "├── " };
        writeln!(
            writer,
            "{prefix}{connector}{:>tokens_width$} {}",
            format_number(directory.tokens),
            tree_label(&directory.path),
        )?;
        let child_prefix = format!("{prefix}{}", if last { "    " } else { "│   " });
        write_tree_children(
            writer,
            &directory.path,
            children,
            &child_prefix,
            tokens_width,
        )?;
    }

    Ok(())
}

fn tree_parent(path: &Path) -> Option<PathBuf> {
    if path == Path::new(".") {
        return None;
    }
    let parent = path.parent()?;
    if parent.as_os_str().is_empty() {
        Some(PathBuf::from("."))
    } else {
        Some(parent.to_path_buf())
    }
}

fn tree_label(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

fn compare_directories(
    left: &DirectoryResult,
    right: &DirectoryResult,
    sort: SortField,
) -> Ordering {
    match sort {
        SortField::Tokens => right
            .tokens
            .cmp(&left.tokens)
            .then_with(|| left.path.cmp(&right.path)),
        SortField::Path => left.path.cmp(&right.path),
        SortField::Language => unreachable!("language sorting is rejected for directory output"),
    }
}

fn compare_files(left: &ScanFileResult, right: &ScanFileResult, sort: SortField) -> Ordering {
    match sort {
        SortField::Tokens => right
            .tokens
            .cmp(&left.tokens)
            .then_with(|| left.path.cmp(&right.path)),
        SortField::Path => left.path.cmp(&right.path),
        SortField::Language => left
            .language
            .as_str()
            .cmp(right.language.as_str())
            .then_with(|| left.path.cmp(&right.path)),
    }
}

fn write_skip_summary(writer: &mut impl Write, skipped: &SkippedSummary) -> io::Result<()> {
    if skipped.total == 0 {
        return Ok(());
    }

    let noun = if skipped.total == 1 { "file" } else { "files" };
    writeln!(writer)?;
    writeln!(writer, "Skipped: {} {noun}", format_number(skipped.total))?;
    for (label, count) in skip_counts(skipped) {
        if count > 0 {
            writeln!(writer, "  {label:<28} {}", format_number(count))?;
        }
    }

    Ok(())
}

fn skip_counts(skipped: &SkippedSummary) -> [(&'static str, u64); 7] {
    [
        ("binary:", skipped.binary),
        ("oversized:", skipped.oversized),
        ("invalid UTF-8:", skipped.invalid_utf8),
        ("unsupported encoding:", skipped.unsupported_encoding),
        ("unreadable:", skipped.unreadable),
        (
            "unsupported filesystem type:",
            skipped.unsupported_filesystem_type,
        ),
        ("symlink:", skipped.symlink),
    ]
}

fn write_json(writer: &mut impl Write, result: &ScanResult, args: &Args) -> io::Result<()> {
    let files = result
        .files
        .iter()
        .map(|file| JsonFile {
            path: machine_path(&file.path),
            language: file.language,
            tokens: file.tokens,
            bytes: file.bytes,
            decoded_lossily: file.decoded_lossily,
        })
        .collect();
    let languages = result
        .languages
        .iter()
        .map(|language| JsonLanguage {
            name: language.name,
            files: language.files,
            tokens: language.tokens,
            bytes: language.bytes,
            density: args
                .density
                .then(|| JsonDensity::new(language.tokens, language.bytes)),
        })
        .collect();
    let directories = if args.dirs || args.tree {
        Some(
            aggregate_directories(&result.files, args.paths.clone())
                .map_err(io::Error::other)?
                .into_iter()
                .map(|directory| JsonDirectory {
                    path: machine_path(&directory.path),
                    files: directory.files,
                    tokens: directory.tokens,
                    bytes: directory.bytes,
                })
                .collect(),
        )
    } else {
        None
    };
    let report = JsonReport {
        schema_version: JSON_SCHEMA_VERSION,
        tokenizer: &result.tokenizer,
        text_policy: JsonTextPolicy {
            mode: result.text_policy.mode,
            version: result.text_policy.version,
            invalid_utf8: result.invalid_utf8,
        },
        totals: &result.totals,
        context: args
            .context
            .map(|context| ContextComparison::new(result.totals.tokens, context)),
        cost: args
            .price_input
            .map(|price| JsonCostEstimate::new(result.totals.tokens, price)),
        directories,
        languages,
        files,
        skipped: json_skipped(&result.skipped),
    };

    serde_json::to_writer_pretty(&mut *writer, &report).map_err(io::Error::other)?;
    writeln!(writer)
}

fn write_diff_json(writer: &mut impl Write, diff: &DiffExecution, args: &Args) -> io::Result<()> {
    let result = &diff.result;
    let report = JsonDiffReport {
        schema_version: JSON_SCHEMA_VERSION,
        report: "diff",
        tokenizer: &result.tokenizer,
        text_policy: JsonTextPolicy {
            mode: result.text_policy.mode,
            version: result.text_policy.version,
            invalid_utf8: result.invalid_utf8,
        },
        snapshots: JsonDiffSnapshots {
            base: JsonDiffSnapshot {
                name: &diff.base,
                totals: &result.base,
                skipped: json_skipped(&result.base_skipped),
            },
            head: JsonDiffSnapshot {
                name: &diff.head,
                totals: &result.head,
                skipped: json_skipped(&result.head_skipped),
            },
        },
        tokens: &result.tokens,
        context: args
            .context
            .map(|context| ContextComparison::new(result.head.tokens, context)),
        cost: args
            .price_input
            .map(|price| JsonCostEstimate::new(result.head.tokens, price)),
        languages: &result.languages,
        files: result
            .files
            .iter()
            .map(|file| JsonDiffFile {
                path: machine_path(&file.path),
                language: file.language,
                base_tokens: file.base_tokens,
                head_tokens: file.head_tokens,
                tokens: file.tokens,
            })
            .collect(),
    };

    serde_json::to_writer_pretty(&mut *writer, &report).map_err(io::Error::other)?;
    writeln!(writer)
}

fn json_skipped(skipped: &SkippedSummary) -> JsonSkipped {
    JsonSkipped {
        total: skipped.total,
        binary: skipped.binary,
        oversized: skipped.oversized,
        invalid_utf8: skipped.invalid_utf8,
        unsupported_encoding: skipped.unsupported_encoding,
        unreadable: skipped.unreadable,
        unsupported_filesystem_type: skipped.unsupported_filesystem_type,
        symlink: skipped.symlink,
        details: skipped.details.iter().map(JsonSkipDetail::from).collect(),
    }
}

fn allocated_percent_tenths(tokens: &[u64]) -> Vec<u16> {
    let total = tokens.iter().map(|value| u128::from(*value)).sum::<u128>();
    if total == 0 {
        return vec![0; tokens.len()];
    }

    let mut allocated = Vec::with_capacity(tokens.len());
    let mut remainders = Vec::with_capacity(tokens.len());
    let mut floor_sum = 0_u16;
    for (index, tokens) in tokens.iter().enumerate() {
        let scaled = u128::from(*tokens) * 1000;
        let floor = u16::try_from(scaled / total).expect("percentage is at most 1000");
        allocated.push(floor);
        remainders.push((index, scaled % total));
        floor_sum += floor;
    }

    remainders.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
    for (index, _) in remainders.into_iter().take(usize::from(1000 - floor_sum)) {
        allocated[index] += 1;
    }

    allocated
}

fn format_number(value: u64) -> String {
    format_unsigned(u128::from(value))
}

fn tokens_per_kb_tenths(tokens: u64, bytes: u64) -> u128 {
    rounded_ratio_tenths(u128::from(tokens) * 1024, u128::from(bytes))
}

fn bytes_per_token_tenths(bytes: u64, tokens: u64) -> u128 {
    rounded_ratio_tenths(u128::from(bytes), u128::from(tokens))
}

fn rounded_ratio_tenths(numerator: u128, denominator: u128) -> u128 {
    if denominator == 0 {
        return 0;
    }

    (numerator * 10 + denominator / 2) / denominator
}

fn format_decimal_tenths(value: u128) -> String {
    format!("{}.{:01}", value / 10, value % 10)
}

fn format_signed(value: i128) -> String {
    let sign = if value < 0 { '-' } else { '+' };
    format!("{sign}{}", format_unsigned(value.unsigned_abs()))
}

fn format_unsigned(value: u128) -> String {
    let digits = value.to_string();
    let mut formatted = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, character) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            formatted.push(',');
        }
        formatted.push(character);
    }

    formatted
}

fn format_input_price(price: InputPrice, grouped: bool) -> String {
    let micros = price.micros_per_million();
    let whole = micros / 1_000_000;
    let fraction = micros % 1_000_000;
    let whole = if grouped {
        format_number(whole)
    } else {
        whole.to_string()
    };
    if fraction == 0 {
        return whole;
    }

    format!("{whole}.{:06}", fraction)
        .trim_end_matches('0')
        .to_owned()
}

fn format_estimated_cost(ten_thousandths: u128, grouped: bool) -> String {
    let whole = ten_thousandths / 10_000;
    let whole = if grouped {
        format_unsigned(whole)
    } else {
        whole.to_string()
    };

    format!("{whole}.{:04}", ten_thousandths % 10_000)
}

#[cfg(windows)]
fn machine_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

#[cfg(not(windows))]
fn machine_path(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

#[derive(Serialize)]
struct JsonReport<'a> {
    schema_version: u32,
    tokenizer: &'a TokenizerMetadata,
    text_policy: JsonTextPolicy,
    totals: &'a ScanTotals,
    #[serde(skip_serializing_if = "Option::is_none")]
    context: Option<ContextComparison>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cost: Option<JsonCostEstimate>,
    #[serde(skip_serializing_if = "Option::is_none")]
    directories: Option<Vec<JsonDirectory>>,
    languages: Vec<JsonLanguage>,
    files: Vec<JsonFile>,
    skipped: JsonSkipped,
}

#[derive(Serialize)]
struct JsonDiffReport<'a> {
    schema_version: u32,
    report: &'static str,
    tokenizer: &'a TokenizerMetadata,
    text_policy: JsonTextPolicy,
    snapshots: JsonDiffSnapshots<'a>,
    tokens: &'a TokenDelta,
    #[serde(skip_serializing_if = "Option::is_none")]
    context: Option<ContextComparison>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cost: Option<JsonCostEstimate>,
    languages: &'a [tokm_core::LanguageDelta],
    files: Vec<JsonDiffFile>,
}

#[derive(Serialize)]
struct ContextComparison {
    window_tokens: u64,
    utilization_percent: f64,
    remaining_tokens: u64,
    exceeded_by_tokens: u64,
    #[serde(skip)]
    utilization_percent_tenths: u128,
}

impl ContextComparison {
    fn new(tokens: u64, window: NonZeroU64) -> Self {
        let window_tokens = window.get();
        let utilization_percent_tenths =
            (u128::from(tokens) * 1000 + u128::from(window_tokens) / 2) / u128::from(window_tokens);

        Self {
            window_tokens,
            utilization_percent: utilization_percent_tenths as f64 / 10.0,
            remaining_tokens: window_tokens.saturating_sub(tokens),
            exceeded_by_tokens: tokens.saturating_sub(window_tokens),
            utilization_percent_tenths,
        }
    }
}

struct CostEstimate {
    ten_thousandths: u128,
}

impl CostEstimate {
    fn new(tokens: u64, price: InputPrice) -> Self {
        const DIVISOR: u128 = 100_000_000;
        let numerator = u128::from(tokens) * u128::from(price.micros_per_million());
        let whole = numerator / DIVISOR;
        let remainder = numerator % DIVISOR;

        Self {
            ten_thousandths: whole + u128::from(remainder >= DIVISOR / 2),
        }
    }
}

#[derive(Serialize)]
struct JsonCostEstimate {
    currency: &'static str,
    price_per_million_tokens: String,
    estimated_input_cost: String,
}

impl JsonCostEstimate {
    fn new(tokens: u64, price: InputPrice) -> Self {
        let estimate = CostEstimate::new(tokens, price);

        Self {
            currency: "USD",
            price_per_million_tokens: format_input_price(price, false),
            estimated_input_cost: format_estimated_cost(estimate.ten_thousandths, false),
        }
    }
}

#[derive(Serialize)]
struct JsonDiffSnapshots<'a> {
    base: JsonDiffSnapshot<'a>,
    head: JsonDiffSnapshot<'a>,
}

#[derive(Serialize)]
struct JsonDiffSnapshot<'a> {
    name: &'a str,
    totals: &'a ScanTotals,
    skipped: JsonSkipped,
}

#[derive(Serialize)]
struct JsonTextPolicy {
    mode: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    version: Option<u32>,
    invalid_utf8: InvalidUtf8Policy,
}

#[derive(Serialize)]
struct JsonFile {
    path: String,
    language: Language,
    tokens: u64,
    bytes: u64,
    decoded_lossily: bool,
}

#[derive(Serialize)]
struct JsonLanguage {
    name: Language,
    files: u64,
    tokens: u64,
    bytes: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    density: Option<JsonDensity>,
}

#[derive(Serialize)]
struct JsonDensity {
    tokens_per_kb: f64,
    bytes_per_token: f64,
}

impl JsonDensity {
    fn new(tokens: u64, bytes: u64) -> Self {
        Self {
            tokens_per_kb: tokens_per_kb_tenths(tokens, bytes) as f64 / 10.0,
            bytes_per_token: bytes_per_token_tenths(bytes, tokens) as f64 / 10.0,
        }
    }
}

#[derive(Serialize)]
struct JsonDirectory {
    path: String,
    files: u64,
    tokens: u64,
    bytes: u64,
}

#[derive(Serialize)]
struct JsonDiffFile {
    path: String,
    language: Language,
    base_tokens: Option<u64>,
    head_tokens: Option<u64>,
    tokens: TokenDelta,
}

#[derive(Serialize)]
struct JsonSkipped {
    total: u64,
    binary: u64,
    oversized: u64,
    invalid_utf8: u64,
    unsupported_encoding: u64,
    unreadable: u64,
    unsupported_filesystem_type: u64,
    symlink: u64,
    details: Vec<JsonSkipDetail>,
}

#[derive(Serialize)]
struct JsonSkipDetail {
    path: String,
    reason: tokm_core::SkipReason,
    #[serde(skip_serializing_if = "Option::is_none")]
    detail: Option<String>,
}

impl From<&SkipDetail> for JsonSkipDetail {
    fn from(detail: &SkipDetail) -> Self {
        Self {
            path: machine_path(&detail.path),
            reason: detail.reason,
            detail: detail.detail.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroU64;

    use super::{
        ContextComparison, CostEstimate, allocated_percent_tenths, bytes_per_token_tenths,
        format_estimated_cost, format_input_price, format_number, format_signed,
        tokens_per_kb_tenths,
    };
    use crate::args::InputPrice;

    #[test]
    fn largest_remainder_percentages_sum_to_exactly_one_hundred() {
        let percentages = allocated_percent_tenths(&[1, 1, 1]);

        assert_eq!(percentages, [334, 333, 333]);
        assert_eq!(percentages.iter().sum::<u16>(), 1000);
    }

    #[test]
    fn number_formatting_uses_stable_ascii_grouping() {
        assert_eq!(format_number(0), "0");
        assert_eq!(format_number(135_641), "135,641");
        assert_eq!(format_signed(12_402), "+12,402");
        assert_eq!(format_signed(-6_029), "-6,029");
    }

    #[test]
    fn density_metrics_use_deterministic_tenth_rounding() {
        assert_eq!(tokens_per_kb_tenths(2, 11), 1_862);
        assert_eq!(bytes_per_token_tenths(11, 2), 55);
        assert_eq!(tokens_per_kb_tenths(0, 0), 0);
        assert_eq!(bytes_per_token_tenths(0, 0), 0);
    }

    #[test]
    fn context_comparison_handles_remaining_and_exceeded_tokens() {
        let within = ContextComparison::new(93_241, NonZeroU64::new(128_000).unwrap());
        let exceeded = ContextComparison::new(150, NonZeroU64::new(100).unwrap());

        assert_eq!(within.utilization_percent_tenths, 728);
        assert_eq!(within.remaining_tokens, 34_759);
        assert_eq!(within.exceeded_by_tokens, 0);
        assert_eq!(exceeded.utilization_percent_tenths, 1500);
        assert_eq!(exceeded.remaining_tokens, 0);
        assert_eq!(exceeded.exceeded_by_tokens, 50);
    }

    #[test]
    fn cost_estimation_uses_rounded_fixed_point_math() {
        let price = InputPrice::from_micros(1_250_000);
        let estimate = CostEstimate::new(135_641, price);

        assert_eq!(estimate.ten_thousandths, 1_696);
        assert_eq!(format_input_price(price, false), "1.25");
        assert_eq!(
            format_estimated_cost(estimate.ten_thousandths, false),
            "0.1696"
        );
    }
}
