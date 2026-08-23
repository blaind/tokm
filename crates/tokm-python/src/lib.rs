//! Direct Python bindings for the shared token measurement engine.

use std::{collections::BTreeMap, num::NonZeroUsize, path::PathBuf};

use pyo3::{
    create_exception,
    exceptions::{PyException, PyValueError},
    prelude::*,
    types::PyAny,
};
use tokm_core::{
    CountError, CountOptions, FileOutcome, InvalidUtf8Policy, LanguageResult, MaxFileSize,
    ScanError, ScanFileResult, ScanOptions, ScanResult, ScanTotals, SkipDetail, SkippedSummary,
    TextPolicy, TextPolicyMetadata, TokenizerError as CoreTokenizerError,
    count_file as core_count_file, count_text as core_count_text, scan as core_scan,
    tokenizer::{
        AccuracyClass, BuiltinEncoding, RawTextSemantics, TokenizerKind, TokenizerMetadata,
    },
};

create_exception!(
    _tokm,
    TokmError,
    PyException,
    "Base class for errors raised by tokm."
);
create_exception!(
    _tokm,
    TokenizerError,
    TokmError,
    "Tokenizer selection or execution failed."
);
create_exception!(
    _tokm,
    ConfigurationError,
    TokmError,
    "Measurement options are invalid."
);
create_exception!(
    _tokm,
    InputError,
    TokmError,
    "An explicit input could not be scanned."
);

#[pyclass(
    name = "TokenizerMetadata",
    module = "tokm._tokm",
    frozen,
    get_all,
    skip_from_py_object
)]
#[derive(Clone, Debug)]
struct PyTokenizerMetadata {
    kind: String,
    name: String,
    fingerprint: String,
    accuracy: String,
    raw_text_semantics: String,
}

impl From<TokenizerMetadata> for PyTokenizerMetadata {
    fn from(metadata: TokenizerMetadata) -> Self {
        Self {
            kind: tokenizer_kind(metadata.kind).to_owned(),
            name: metadata.name,
            fingerprint: metadata.fingerprint,
            accuracy: accuracy_class(metadata.accuracy).to_owned(),
            raw_text_semantics: raw_text_semantics(metadata.raw_text_semantics).to_owned(),
        }
    }
}

#[pyclass(
    name = "TextPolicy",
    module = "tokm._tokm",
    frozen,
    get_all,
    skip_from_py_object
)]
#[derive(Clone, Debug)]
struct PyTextPolicy {
    mode: String,
    version: Option<u32>,
    invalid_utf8: Option<String>,
}

impl PyTextPolicy {
    fn from_count(metadata: TextPolicyMetadata) -> Self {
        Self {
            mode: metadata.mode.to_owned(),
            version: metadata.version,
            invalid_utf8: None,
        }
    }

    fn from_scan(metadata: TextPolicyMetadata, invalid_utf8: InvalidUtf8Policy) -> Self {
        Self {
            mode: metadata.mode.to_owned(),
            version: metadata.version,
            invalid_utf8: Some(invalid_utf8_name(invalid_utf8).to_owned()),
        }
    }
}

#[pyclass(
    name = "CountResult",
    module = "tokm._tokm",
    frozen,
    get_all,
    skip_from_py_object
)]
#[derive(Clone, Debug)]
struct PyCountResult {
    tokens: u64,
    bytes: u64,
    tokenizer: PyTokenizerMetadata,
    text_policy: PyTextPolicy,
}

#[pyclass(
    name = "SkipDetail",
    module = "tokm._tokm",
    frozen,
    get_all,
    skip_from_py_object
)]
#[derive(Clone, Debug)]
struct PySkipDetail {
    path: String,
    reason: String,
    detail: Option<String>,
}

impl From<SkipDetail> for PySkipDetail {
    fn from(detail: SkipDetail) -> Self {
        Self {
            path: detail.path.to_string_lossy().into_owned(),
            reason: detail.reason.as_str().to_owned(),
            detail: detail.detail,
        }
    }
}

#[pyclass(
    name = "FileResult",
    module = "tokm._tokm",
    frozen,
    get_all,
    skip_from_py_object
)]
#[derive(Clone, Debug)]
struct PyFileResult {
    path: String,
    language: Option<String>,
    tokens: Option<u64>,
    bytes: Option<u64>,
    decoded_lossily: bool,
    skipped: Option<PySkipDetail>,
    tokenizer: PyTokenizerMetadata,
    text_policy: PyTextPolicy,
}

#[pyclass(
    name = "ScanTotals",
    module = "tokm._tokm",
    frozen,
    get_all,
    skip_from_py_object
)]
#[derive(Clone, Copy, Debug)]
struct PyScanTotals {
    files: u64,
    tokens: u64,
    bytes: u64,
}

impl From<ScanTotals> for PyScanTotals {
    fn from(totals: ScanTotals) -> Self {
        Self {
            files: totals.files,
            tokens: totals.tokens,
            bytes: totals.bytes,
        }
    }
}

#[pyclass(
    name = "LanguageResult",
    module = "tokm._tokm",
    frozen,
    get_all,
    skip_from_py_object
)]
#[derive(Clone, Debug)]
struct PyLanguageResult {
    name: String,
    files: u64,
    tokens: u64,
    bytes: u64,
}

impl From<LanguageResult> for PyLanguageResult {
    fn from(result: LanguageResult) -> Self {
        Self {
            name: result.name.as_str().to_owned(),
            files: result.files,
            tokens: result.tokens,
            bytes: result.bytes,
        }
    }
}

#[pyclass(
    name = "ScanFileResult",
    module = "tokm._tokm",
    frozen,
    get_all,
    skip_from_py_object
)]
#[derive(Clone, Debug)]
struct PyScanFileResult {
    path: String,
    language: String,
    tokens: u64,
    bytes: u64,
    decoded_lossily: bool,
}

impl From<ScanFileResult> for PyScanFileResult {
    fn from(result: ScanFileResult) -> Self {
        Self {
            path: result.path.to_string_lossy().into_owned(),
            language: result.language.as_str().to_owned(),
            tokens: result.tokens,
            bytes: result.bytes,
            decoded_lossily: result.decoded_lossily,
        }
    }
}

#[pyclass(
    name = "SkippedSummary",
    module = "tokm._tokm",
    frozen,
    get_all,
    skip_from_py_object
)]
#[derive(Clone, Debug)]
struct PySkippedSummary {
    total: u64,
    binary: u64,
    oversized: u64,
    invalid_utf8: u64,
    unsupported_encoding: u64,
    unreadable: u64,
    unsupported_filesystem_type: u64,
    symlink: u64,
    details: Vec<PySkipDetail>,
}

impl From<SkippedSummary> for PySkippedSummary {
    fn from(summary: SkippedSummary) -> Self {
        Self {
            total: summary.total,
            binary: summary.binary,
            oversized: summary.oversized,
            invalid_utf8: summary.invalid_utf8,
            unsupported_encoding: summary.unsupported_encoding,
            unreadable: summary.unreadable,
            unsupported_filesystem_type: summary.unsupported_filesystem_type,
            symlink: summary.symlink,
            details: summary.details.into_iter().map(Into::into).collect(),
        }
    }
}

#[pyclass(
    name = "ScanResult",
    module = "tokm._tokm",
    frozen,
    get_all,
    skip_from_py_object
)]
#[derive(Clone, Debug)]
struct PyScanResult {
    tokenizer: PyTokenizerMetadata,
    text_policy: PyTextPolicy,
    totals: PyScanTotals,
    languages: BTreeMap<String, PyLanguageResult>,
    files: Vec<PyScanFileResult>,
    skipped: PySkippedSummary,
}

#[pymethods]
impl PyScanResult {
    #[getter]
    const fn total_tokens(&self) -> u64 {
        self.totals.tokens
    }
}

impl From<ScanResult> for PyScanResult {
    fn from(result: ScanResult) -> Self {
        let invalid_utf8 = result.invalid_utf8;
        Self {
            tokenizer: result.tokenizer.into(),
            text_policy: PyTextPolicy::from_scan(result.text_policy, invalid_utf8),
            totals: result.totals.into(),
            languages: result
                .languages
                .into_iter()
                .map(|language| {
                    let language = PyLanguageResult::from(language);
                    (language.name.clone(), language)
                })
                .collect(),
            files: result.files.into_iter().map(Into::into).collect(),
            skipped: result.skipped.into(),
        }
    }
}

#[pyfunction(
    name = "count",
    signature = (text, *, encoding = None, tokenizer_file = None, normalize = false)
)]
fn count_text(
    py: Python<'_>,
    text: String,
    encoding: Option<&str>,
    tokenizer_file: Option<PathBuf>,
    normalize: bool,
) -> PyResult<PyCountResult> {
    let options = count_options(encoding, tokenizer_file.as_deref(), normalize)?;
    let result = py
        .detach(move || core_count_text(&text, &options))
        .map_err(map_count_error)?;

    Ok(PyCountResult {
        tokens: result.tokens,
        bytes: result.bytes,
        tokenizer: result.tokenizer.into(),
        text_policy: PyTextPolicy::from_count(result.text_policy),
    })
}

#[pyfunction(
    name = "count_file",
    signature = (
        path,
        *,
        encoding = None,
        tokenizer_file = None,
        normalize = false,
        invalid_utf8 = "skip",
        max_file_size = Some(20_971_520),
        no_cache = false
    )
)]
#[allow(clippy::too_many_arguments)]
fn count_file(
    py: Python<'_>,
    path: PathBuf,
    encoding: Option<&str>,
    tokenizer_file: Option<PathBuf>,
    normalize: bool,
    invalid_utf8: &str,
    max_file_size: Option<u64>,
    no_cache: bool,
) -> PyResult<PyFileResult> {
    let options = scan_options(
        encoding,
        tokenizer_file.as_deref(),
        normalize,
        invalid_utf8,
        max_file_size,
        no_cache,
        None,
    )?;
    let metadata = options.count_options().tokenizer().metadata().clone();
    let text_policy = options.count_options().text_policy().metadata();
    let invalid_utf8 = options.invalid_utf8();
    let outcome = py
        .detach(move || core_count_file(path, &options))
        .map_err(map_count_error)?;

    Ok(match outcome {
        FileOutcome::Measured(result) => PyFileResult {
            path: result.path.to_string_lossy().into_owned(),
            language: Some(result.language.as_str().to_owned()),
            tokens: Some(result.tokens),
            bytes: Some(result.bytes),
            decoded_lossily: result.decoded_lossily,
            skipped: None,
            tokenizer: result.tokenizer.into(),
            text_policy: PyTextPolicy::from_scan(result.text_policy, result.invalid_utf8),
        },
        FileOutcome::Skipped(detail) => PyFileResult {
            path: detail.path.to_string_lossy().into_owned(),
            language: None,
            tokens: None,
            bytes: None,
            decoded_lossily: false,
            skipped: Some(detail.into()),
            tokenizer: metadata.into(),
            text_policy: PyTextPolicy::from_scan(text_policy, invalid_utf8),
        },
    })
}

#[pyfunction(
    name = "scan",
    signature = (
        paths,
        *,
        encoding = None,
        tokenizer_file = None,
        normalize = false,
        invalid_utf8 = "skip",
        include = None,
        exclude = None,
        hidden = false,
        no_ignore = false,
        follow = false,
        max_file_size = Some(20_971_520),
        no_cache = false,
        threads = None
    )
)]
#[allow(clippy::too_many_arguments)]
fn scan(
    py: Python<'_>,
    paths: &Bound<'_, PyAny>,
    encoding: Option<&str>,
    tokenizer_file: Option<PathBuf>,
    normalize: bool,
    invalid_utf8: &str,
    include: Option<Vec<String>>,
    exclude: Option<Vec<String>>,
    hidden: bool,
    no_ignore: bool,
    follow: bool,
    max_file_size: Option<u64>,
    no_cache: bool,
    threads: Option<usize>,
) -> PyResult<PyScanResult> {
    let paths = extract_paths(paths)?;
    let mut options = scan_options(
        encoding,
        tokenizer_file.as_deref(),
        normalize,
        invalid_utf8,
        max_file_size,
        no_cache,
        threads,
    )?
    .with_hidden(hidden)
    .with_no_ignore(no_ignore)
    .with_follow_symlinks(follow);
    for pattern in include.unwrap_or_default() {
        options = options.with_include(pattern);
    }
    for pattern in exclude.unwrap_or_default() {
        options = options.with_exclude(pattern);
    }

    py.detach(move || core_scan(paths, &options))
        .map(Into::into)
        .map_err(map_scan_error)
}

fn count_options(
    encoding: Option<&str>,
    tokenizer_file: Option<&std::path::Path>,
    normalize: bool,
) -> PyResult<CountOptions> {
    if encoding.is_some() && tokenizer_file.is_some() {
        return Err(ConfigurationError::new_err(
            "encoding and tokenizer_file are mutually exclusive",
        ));
    }
    let text_policy = if normalize {
        TextPolicy::Normalized
    } else {
        TextPolicy::Literal
    };
    let options = match tokenizer_file {
        Some(path) => CountOptions::for_tokenizer_file(path).map_err(map_tokenizer_error)?,
        None => {
            let encoding = encoding
                .unwrap_or("o200k_base")
                .parse::<BuiltinEncoding>()
                .map_err(map_tokenizer_error)?;
            CountOptions::for_encoding(encoding)
        }
    };

    Ok(options.with_text_policy(text_policy))
}

fn scan_options(
    encoding: Option<&str>,
    tokenizer_file: Option<&std::path::Path>,
    normalize: bool,
    invalid_utf8: &str,
    max_file_size: Option<u64>,
    no_cache: bool,
    threads: Option<usize>,
) -> PyResult<ScanOptions> {
    let invalid_utf8 = match invalid_utf8 {
        "skip" => InvalidUtf8Policy::Skip,
        "lossy" => InvalidUtf8Policy::Lossy,
        _ => {
            return Err(ConfigurationError::new_err(
                "invalid_utf8 must be 'skip' or 'lossy'",
            ));
        }
    };
    let max_file_size = max_file_size.map_or(MaxFileSize::Unlimited, MaxFileSize::Limited);
    let mut options = ScanOptions::new(count_options(encoding, tokenizer_file, normalize)?)
        .with_invalid_utf8(invalid_utf8)
        .with_max_file_size(max_file_size)
        .with_cache_enabled(!no_cache);
    if let Some(threads) = threads {
        let threads = NonZeroUsize::new(threads)
            .ok_or_else(|| ConfigurationError::new_err("threads must be greater than zero"))?;
        options = options.with_workers(threads);
    }

    Ok(options)
}

fn extract_paths(paths: &Bound<'_, PyAny>) -> PyResult<Vec<PathBuf>> {
    if let Ok(path) = paths.extract::<PathBuf>() {
        return Ok(vec![path]);
    }

    paths.extract::<Vec<PathBuf>>().map_err(|_| {
        PyValueError::new_err("paths must be a path-like object or a sequence of path-like objects")
    })
}

fn map_tokenizer_error(error: CoreTokenizerError) -> PyErr {
    TokenizerError::new_err(error.to_string())
}

fn map_count_error(error: CountError) -> PyErr {
    TokenizerError::new_err(error.to_string())
}

fn map_scan_error(error: ScanError) -> PyErr {
    match error {
        ScanError::InvalidGlob { .. } | ScanError::WorkerPool { .. } => {
            ConfigurationError::new_err(error.to_string())
        }
        ScanError::Count(error) => map_count_error(error),
        ScanError::NoInputs
        | ScanError::WorkingDirectory { .. }
        | ScanError::Input { .. }
        | ScanError::AggregateOverflow => InputError::new_err(error.to_string()),
    }
}

const fn tokenizer_kind(kind: TokenizerKind) -> &'static str {
    match kind {
        TokenizerKind::Builtin => "builtin",
        TokenizerKind::LocalFile => "local_file",
        TokenizerKind::Proxy => "proxy",
    }
}

const fn accuracy_class(accuracy: AccuracyClass) -> &'static str {
    match accuracy {
        AccuracyClass::Exact => "exact",
        AccuracyClass::Proxy => "proxy",
    }
}

const fn raw_text_semantics(semantics: RawTextSemantics) -> &'static str {
    match semantics {
        RawTextSemantics::Ordinary => "ordinary",
        RawTextSemantics::Artifact => "artifact",
    }
}

const fn invalid_utf8_name(policy: InvalidUtf8Policy) -> &'static str {
    match policy {
        InvalidUtf8Policy::Skip => "skip",
        InvalidUtf8Policy::Lossy => "lossy",
    }
}

#[pymodule]
fn _tokm(module: &Bound<'_, PyModule>) -> PyResult<()> {
    let module_name = "tokm._tokm";
    let tokm_error = module.py().get_type::<TokmError>();
    let tokenizer_error = module.py().get_type::<TokenizerError>();
    let configuration_error = module.py().get_type::<ConfigurationError>();
    let input_error = module.py().get_type::<InputError>();

    tokm_error.setattr("__module__", module_name)?;
    tokenizer_error.setattr("__module__", module_name)?;
    configuration_error.setattr("__module__", module_name)?;
    input_error.setattr("__module__", module_name)?;
    module.add("__version__", env!("CARGO_PKG_VERSION"))?;
    module.add("TokmError", tokm_error)?;
    module.add("TokenizerError", tokenizer_error)?;
    module.add("ConfigurationError", configuration_error)?;
    module.add("InputError", input_error)?;
    module.add_class::<PyTokenizerMetadata>()?;
    module.add_class::<PyTextPolicy>()?;
    module.add_class::<PyCountResult>()?;
    module.add_class::<PySkipDetail>()?;
    module.add_class::<PyFileResult>()?;
    module.add_class::<PyScanTotals>()?;
    module.add_class::<PyLanguageResult>()?;
    module.add_class::<PyScanFileResult>()?;
    module.add_class::<PySkippedSummary>()?;
    module.add_class::<PyScanResult>()?;
    module.add_function(wrap_pyfunction!(count_text, module)?)?;
    module.add_function(wrap_pyfunction!(count_file, module)?)?;
    module.add_function(wrap_pyfunction!(scan, module)?)?;
    Ok(())
}
