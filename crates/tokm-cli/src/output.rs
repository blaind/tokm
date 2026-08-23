//! Deterministic human and JSON reporting.

use std::{cmp::Ordering, io, io::Write, path::Path};

use serde::Serialize;
use tokm_core::{
    DiffFileResult, DiffResult, InvalidUtf8Policy, Language, ScanFileResult, ScanResult,
    ScanTotals, SkipDetail, SkippedSummary, TokenDelta, tokenizer::TokenizerMetadata,
};

use crate::{
    app::{DiffExecution, Execution},
    args::{Args, OutputFormat, SortField},
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
        OutputFormat::Json => write_json(writer, result),
        OutputFormat::Table if args.quiet => Ok(()),
        OutputFormat::Table if args.files => write_file_table(writer, result, args.sort),
        OutputFormat::Table => write_language_table(writer, result),
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
        OutputFormat::Json => write_diff_json(writer, diff),
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
    write_diff_skip_summary(writer, "Head", &result.head_skipped)
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

fn write_json(writer: &mut impl Write, result: &ScanResult) -> io::Result<()> {
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
    let report = JsonReport {
        schema_version: JSON_SCHEMA_VERSION,
        tokenizer: &result.tokenizer,
        text_policy: JsonTextPolicy {
            mode: result.text_policy.mode,
            version: result.text_policy.version,
            invalid_utf8: result.invalid_utf8,
        },
        totals: &result.totals,
        languages: &result.languages,
        files,
        skipped: json_skipped(&result.skipped),
    };

    serde_json::to_writer_pretty(&mut *writer, &report).map_err(io::Error::other)?;
    writeln!(writer)
}

fn write_diff_json(writer: &mut impl Write, diff: &DiffExecution) -> io::Result<()> {
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
    languages: &'a [tokm_core::LanguageResult],
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
    languages: &'a [tokm_core::LanguageDelta],
    files: Vec<JsonDiffFile>,
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
    use super::{allocated_percent_tenths, format_number, format_signed};

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
}
