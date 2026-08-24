//! Parallel recursive scanning over deterministic discovery results.

use std::path::PathBuf;

use rayon::prelude::*;

#[cfg(feature = "cache")]
use crate::cache::Cache;
use crate::{
    FileOutcome, ScanError, ScanOptions,
    aggregate::{ScanResult, aggregate},
    file::count_file_with_cache,
    walk::discover,
};

/// Measure files and directory trees under one shared semantic policy.
///
/// Overlapping inputs are deduplicated by normalized path identity before any
/// file is read or tokenized.
pub fn scan<I, P>(inputs: I, options: &ScanOptions) -> Result<ScanResult, ScanError>
where
    I: IntoIterator<Item = P>,
    P: Into<PathBuf>,
{
    let inputs = inputs.into_iter().map(Into::into).collect();
    let discovery = discover(inputs, options)?;
    #[cfg(feature = "cache")]
    let cache = Cache::resolve(options.cache_policy());
    #[cfg(not(feature = "cache"))]
    let cache = ();
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(options.workers().get())
        .build()
        .map_err(|error| ScanError::WorkerPool {
            message: error.to_string(),
        })?;
    let outcomes = pool.install(|| {
        discovery
            .paths
            .par_iter()
            .map(|path| count_file_with_cache(path.clone(), options, &cache))
            .collect::<Vec<_>>()
    });
    let mut measured = Vec::new();
    let mut skipped = discovery.skipped;

    for outcome in outcomes {
        match outcome? {
            FileOutcome::Measured(result) => measured.push(result),
            FileOutcome::Skipped(detail) => skipped.push(detail),
        }
    }

    aggregate(measured, skipped, options)
}

#[cfg(test)]
mod tests {
    use std::{fs, time::SystemTime};

    use filetime::{FileTime, set_file_mtime};
    use tempfile::tempdir;

    use super::scan;
    use crate::{CountOptions, Language, ScanOptions, count_text};

    fn options() -> ScanOptions {
        ScanOptions::default().with_cache_enabled(false)
    }

    #[test]
    fn scan_respects_ignore_hidden_and_filter_policy() {
        let directory = tempdir().unwrap();
        fs::create_dir(directory.path().join("src")).unwrap();
        fs::write(
            directory.path().join("src/lib.rs"),
            "pub fn answer() -> u8 { 42 }",
        )
        .unwrap();
        fs::write(directory.path().join("src/data.json"), "{}").unwrap();
        fs::write(directory.path().join("ignored.rs"), "ignored").unwrap();
        fs::write(directory.path().join(".hidden.rs"), "hidden").unwrap();
        fs::write(directory.path().join(".gitignore"), "ignored.rs\n").unwrap();

        let result = scan([directory.path()], &options().with_include("*.rs")).unwrap();

        assert_eq!(result.totals.files, 1);
        assert_eq!(result.files[0].language, Language::Rust);
        assert!(result.files[0].path.ends_with("src/lib.rs"));
    }

    #[test]
    fn overlapping_inputs_are_counted_once() {
        let directory = tempdir().unwrap();
        let source = directory.path().join("src");
        fs::create_dir(&source).unwrap();
        fs::write(source.join("lib.rs"), "pub fn item() {}").unwrap();

        let result = scan([directory.path(), &source], &options()).unwrap();

        assert_eq!(result.totals.files, 1);
        assert_eq!(result.languages[0].files, 1);
    }

    #[test]
    fn aggregation_and_paths_are_deterministic() {
        let directory = tempdir().unwrap();
        fs::write(directory.path().join("z.md"), "one two three").unwrap();
        fs::write(directory.path().join("a.rs"), "fn a() {}").unwrap();

        let first = scan([directory.path()], &options()).unwrap();
        let second = scan([directory.path()], &options()).unwrap();

        assert_eq!(first, second);
        assert!(first.files[0].path.ends_with("a.rs"));
    }

    #[test]
    fn no_ignore_and_hidden_are_independent_opt_ins() {
        let directory = tempdir().unwrap();
        fs::write(directory.path().join("visible.rs"), "visible").unwrap();
        fs::write(directory.path().join("ignored.rs"), "ignored").unwrap();
        fs::write(directory.path().join(".hidden.rs"), "hidden").unwrap();
        fs::write(directory.path().join(".gitignore"), "ignored.rs\n").unwrap();

        let no_ignore = scan([directory.path()], &options().with_no_ignore(true)).unwrap();
        let everything = scan(
            [directory.path()],
            &options().with_no_ignore(true).with_hidden(true),
        )
        .unwrap();

        assert_eq!(no_ignore.totals.files, 2);
        assert_eq!(everything.totals.files, 4);
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_are_visible_skips_unless_following_is_enabled() {
        use std::os::unix::fs::symlink;

        let directory = tempdir().unwrap();
        fs::write(directory.path().join("target.rs"), "fn target() {}").unwrap();
        symlink("target.rs", directory.path().join("link.rs")).unwrap();

        let default = scan([directory.path()], &options()).unwrap();
        let followed = scan([directory.path()], &options().with_follow_symlinks(true)).unwrap();

        assert_eq!(default.totals.files, 1);
        assert_eq!(default.skipped.symlink, 1);
        assert_eq!(followed.totals.files, 2);
        assert_eq!(followed.skipped.total, 0);
    }

    #[test]
    fn cold_and_warm_cache_scans_agree() {
        let directory = tempdir().unwrap();
        let cache = tempdir().unwrap();
        fs::write(directory.path().join("input.md"), "cached content").unwrap();
        let options = ScanOptions::default().with_cache_directory(cache.path());

        let cold = scan([directory.path()], &options).unwrap();
        let warm = scan([directory.path()], &options).unwrap();

        assert_eq!(cold, warm);
    }

    #[test]
    fn racily_clean_metadata_rehashes_same_size_content() {
        let directory = tempdir().unwrap();
        let cache = tempdir().unwrap();
        let path = directory.path().join("input.txt");
        let modified = FileTime::from_system_time(SystemTime::now());
        fs::write(&path, "abcd").unwrap();
        set_file_mtime(&path, modified).unwrap();
        let options = ScanOptions::default().with_cache_directory(cache.path());
        let first = scan([&path], &options).unwrap();

        fs::write(&path, " a a").unwrap();
        set_file_mtime(&path, modified).unwrap();
        let second = scan([&path], &options).unwrap();
        let expected = count_text(" a a", &CountOptions::default()).unwrap().tokens;

        assert_ne!(first.totals.tokens, expected);
        assert_eq!(second.totals.tokens, expected);
    }
}
