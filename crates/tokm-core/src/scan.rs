//! Parallel recursive scanning over deterministic discovery results.

use std::path::PathBuf;

use rayon::prelude::*;

use crate::{
    FileOutcome, ScanError, ScanOptions,
    aggregate::{ScanResult, aggregate},
    count_file,
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
            .map(|path| count_file(path, options))
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
    use std::fs;

    use tempfile::tempdir;

    use super::scan;
    use crate::{Language, ScanOptions};

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

        let result = scan(
            [directory.path()],
            &ScanOptions::default().with_include("*.rs"),
        )
        .unwrap();

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

        let result = scan([directory.path(), &source], &ScanOptions::default()).unwrap();

        assert_eq!(result.totals.files, 1);
        assert_eq!(result.languages[0].files, 1);
    }

    #[test]
    fn aggregation_and_paths_are_deterministic() {
        let directory = tempdir().unwrap();
        fs::write(directory.path().join("z.md"), "one two three").unwrap();
        fs::write(directory.path().join("a.rs"), "fn a() {}").unwrap();

        let first = scan([directory.path()], &ScanOptions::default()).unwrap();
        let second = scan([directory.path()], &ScanOptions::default()).unwrap();

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

        let no_ignore = scan(
            [directory.path()],
            &ScanOptions::default().with_no_ignore(true),
        )
        .unwrap();
        let everything = scan(
            [directory.path()],
            &ScanOptions::default()
                .with_no_ignore(true)
                .with_hidden(true),
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

        let default = scan([directory.path()], &ScanOptions::default()).unwrap();
        let followed = scan(
            [directory.path()],
            &ScanOptions::default().with_follow_symlinks(true),
        )
        .unwrap();

        assert_eq!(default.totals.files, 1);
        assert_eq!(default.skipped.symlink, 1);
        assert_eq!(followed.totals.files, 2);
        assert_eq!(followed.skipped.total, 0);
    }
}
