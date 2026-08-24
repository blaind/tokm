//! Git-aware comparison built from complete tree and worktree scans.

use std::path::{Path, PathBuf};

use crate::{
    DiffResult, GitError, ScanOptions, ScanResult, compare_snapshots, git::scan_tree, scan,
    walk::forward_slash_path,
};

/// Compare two committed Git trees using complete-file token footprints.
pub fn diff_trees(
    repository: impl AsRef<Path>,
    base: &str,
    head: &str,
    options: &ScanOptions,
) -> Result<DiffResult, GitError> {
    let base = scan_tree(repository.as_ref(), base, options)?;
    let head = scan_tree(repository.as_ref(), head, options)?;

    compare_snapshots(&base, &head).map_err(Into::into)
}

/// Compare a committed Git tree with the current repository worktree.
///
/// Untracked, non-ignored files are included because the head snapshot is a
/// normal filesystem scan of the repository root.
pub fn diff_worktree(
    repository: impl AsRef<Path>,
    base: &str,
    options: &ScanOptions,
) -> Result<DiffResult, GitError> {
    let repository_path = repository.as_ref();
    let discovered = gix::discover(repository_path).map_err(|error| GitError::Repository {
        path: repository_path.to_path_buf(),
        message: error.to_string(),
    })?;
    let worktree = discovered
        .workdir()
        .ok_or_else(|| GitError::WorktreeUnavailable {
            path: discovered.git_dir().to_path_buf(),
        })?
        .to_path_buf();
    let base = scan_tree(&worktree, base, options)?;
    let metadata_path = forward_slash_path(&worktree.join(".git"));
    let worktree_options = options
        .clone()
        .with_exclude(".git")
        .with_exclude(".git/**")
        .with_exclude(&metadata_path)
        .with_exclude(format!("{metadata_path}/**"));
    let mut head = scan([&worktree], &worktree_options)?;
    relativize_snapshot(&mut head, &worktree);

    compare_snapshots(&base, &head).map_err(Into::into)
}

fn relativize_snapshot(snapshot: &mut ScanResult, root: &Path) {
    for file in &mut snapshot.files {
        file.path = relative_path(&file.path, root);
    }
    for detail in &mut snapshot.skipped.details {
        detail.path = relative_path(&detail.path, root);
    }
}

fn relative_path(path: &Path, root: &Path) -> PathBuf {
    path.strip_prefix(root).unwrap_or(path).to_path_buf()
}

#[cfg(test)]
mod tests {
    use std::{fs, path::Path};

    use tempfile::tempdir;

    use super::{diff_trees, diff_worktree};
    use crate::ScanOptions;

    #[test]
    fn tree_diff_compares_two_complete_committed_snapshots() {
        let directory = tempdir().unwrap();
        let repository = gix::init(directory.path()).unwrap();
        let base = commit(
            &repository,
            &[("src/lib.rs", b"one"), ("removed.md", b"removed words")],
        );
        let head = commit(
            &repository,
            &[
                ("src/lib.rs", b"one two"),
                ("added.json", br#"{"ok":true}"#),
            ],
        );
        drop(repository);

        let result = diff_trees(
            directory.path(),
            &base,
            &head,
            &ScanOptions::default().with_cache_enabled(false),
        )
        .unwrap();

        assert!(result.tokens.added > 0);
        assert!(result.tokens.removed > 0);
        assert_eq!(
            result
                .files
                .iter()
                .map(|file| file.path.to_str().unwrap())
                .collect::<Vec<_>>(),
            ["added.json", "removed.md", "src/lib.rs"]
        );
    }

    #[test]
    fn worktree_diff_includes_untracked_non_ignored_files() {
        let directory = tempdir().unwrap();
        let repository = gix::init(directory.path()).unwrap();
        let base = commit(
            &repository,
            &[
                (".gitignore", b"ignored.rs\n"),
                ("src/lib.rs", b"one"),
                ("removed.md", b"removed words"),
            ],
        );
        drop(repository);
        fs::create_dir(directory.path().join("src")).unwrap();
        fs::write(directory.path().join(".gitignore"), "ignored.rs\n").unwrap();
        fs::write(directory.path().join("src/lib.rs"), "one two").unwrap();
        fs::write(directory.path().join("notes.md"), "untracked notes").unwrap();
        fs::write(directory.path().join("ignored.rs"), "ignored").unwrap();

        let result = diff_worktree(
            directory.path(),
            &base,
            &ScanOptions::default().with_cache_enabled(false),
        )
        .unwrap();
        let without_ignore_rules = diff_worktree(
            directory.path(),
            &base,
            &ScanOptions::default()
                .with_cache_enabled(false)
                .with_hidden(true)
                .with_no_ignore(true),
        )
        .unwrap();
        let paths = result
            .files
            .iter()
            .map(|file| file.path.to_str().unwrap())
            .collect::<Vec<_>>();

        assert_eq!(paths, ["notes.md", "removed.md", "src/lib.rs"]);
        assert!(result.files.iter().all(|file| file.path.is_relative()));
        assert!(!paths.contains(&"ignored.rs"));
        assert!(
            without_ignore_rules
                .files
                .iter()
                .any(|file| file.path == Path::new("ignored.rs"))
        );
        assert!(
            without_ignore_rules
                .files
                .iter()
                .all(|file| !file.path.starts_with(".git"))
        );
    }

    #[test]
    fn bare_repository_has_no_worktree_head() {
        let directory = tempdir().unwrap();
        gix::init_bare(directory.path()).unwrap();

        let error = diff_worktree(directory.path(), "HEAD", &ScanOptions::default()).unwrap_err();

        assert!(error.to_string().contains("has no worktree"));
    }

    fn commit(repository: &gix::Repository, files: &[(&str, &[u8])]) -> String {
        let mut editor = repository.empty_tree().edit().unwrap();
        for (path, bytes) in files {
            let blob = repository.write_blob(bytes).unwrap();
            editor
                .upsert(*path, gix::object::tree::EntryKind::Blob, blob.detach())
                .unwrap();
        }
        let tree = editor.write().unwrap();
        let signature = gix::actor::SignatureRef {
            name: b"tokm".as_slice().into(),
            email: b"tokm@example.invalid".as_slice().into(),
            time: "0 +0000",
        };
        repository
            .new_commit_as(
                signature,
                signature,
                "fixture",
                tree.detach(),
                std::iter::empty::<gix::ObjectId>(),
            )
            .unwrap()
            .id()
            .to_string()
    }
}
