//! Complete-file scanning for one committed Git tree.

use std::path::Path;

use crate::{
    ByteOutcome, FileResult, GitError, ScanOptions, ScanResult, SkipDetail, SkipReason,
    aggregate::aggregate, classify, count_bytes, walk::Filters,
};

/// Measure every admitted blob reachable from one Git revision.
pub fn scan_tree(
    repository: impl AsRef<Path>,
    revision: &str,
    options: &ScanOptions,
) -> Result<ScanResult, GitError> {
    let repository_path = repository.as_ref();
    let repository = gix::discover(repository_path).map_err(|error| GitError::Repository {
        path: repository_path.to_path_buf(),
        message: error.to_string(),
    })?;
    let object = repository
        .rev_parse_single(revision)
        .map_err(|error| GitError::Revision {
            revision: revision.to_owned(),
            message: error.to_string(),
        })?
        .object()
        .map_err(|error| GitError::Tree {
            revision: revision.to_owned(),
            message: error.to_string(),
        })?;
    let tree = object.peel_to_tree().map_err(|error| GitError::Tree {
        revision: revision.to_owned(),
        message: error.to_string(),
    })?;
    let filters = Filters::new(options.includes(), options.excludes())?;
    let mut measured = Vec::new();
    let mut skipped = Vec::new();

    visit_tree(
        &tree,
        Path::new(""),
        revision,
        options,
        &filters,
        &mut measured,
        &mut skipped,
    )?;

    aggregate(measured, skipped, options).map_err(Into::into)
}

#[allow(clippy::too_many_arguments)]
fn visit_tree(
    tree: &gix::Tree<'_>,
    prefix: &Path,
    revision: &str,
    options: &ScanOptions,
    filters: &Filters,
    measured: &mut Vec<FileResult>,
    skipped: &mut Vec<SkipDetail>,
) -> Result<(), GitError> {
    for entry in tree.iter() {
        let entry = entry.map_err(|error| GitError::Tree {
            revision: revision.to_owned(),
            message: error.to_string(),
        })?;
        let component = gix::path::from_bstr(entry.filename()).into_owned();
        let path = prefix.join(component);
        if !options.hidden() && is_hidden(&path) {
            continue;
        }

        let mode = entry.mode();
        if mode.is_tree() {
            let subtree = entry
                .object()
                .map_err(|error| GitError::Object {
                    path: path.clone(),
                    message: error.to_string(),
                })?
                .try_into_tree()
                .map_err(|error| GitError::Object {
                    path: path.clone(),
                    message: error.to_string(),
                })?;
            visit_tree(
                &subtree, &path, revision, options, filters, measured, skipped,
            )?;
        } else if mode.is_blob() {
            if !filters.admits_relative(&path) {
                continue;
            }
            let size = entry
                .id()
                .header()
                .map_err(|error| GitError::Object {
                    path: path.clone(),
                    message: error.to_string(),
                })?
                .size();
            if !options.max_file_size().admits(size) {
                skipped.push(SkipDetail {
                    path,
                    reason: SkipReason::Oversized,
                    detail: None,
                });
                continue;
            }
            let blob = entry
                .object()
                .map_err(|error| GitError::Object {
                    path: path.clone(),
                    message: error.to_string(),
                })?
                .try_into_blob()
                .map_err(|error| GitError::Object {
                    path: path.clone(),
                    message: error.to_string(),
                })?;
            match count_bytes(&blob.data, options)? {
                ByteOutcome::Measured(result) => measured.push(FileResult {
                    language: classify(&path),
                    path,
                    tokens: result.tokens,
                    bytes: result.bytes,
                    decoded_lossily: result.decoded_lossily,
                    tokenizer: result.tokenizer,
                    text_policy: result.text_policy,
                    invalid_utf8: result.invalid_utf8,
                }),
                ByteOutcome::Skipped(detail) => skipped.push(SkipDetail {
                    path,
                    reason: detail.reason,
                    detail: detail.detail,
                }),
            }
        } else if filters.admits_relative(&path) {
            let (reason, detail) = if mode.is_link() {
                (SkipReason::Symlink, None)
            } else {
                (
                    SkipReason::UnsupportedFilesystemType,
                    Some("Git submodule".to_owned()),
                )
            };
            skipped.push(SkipDetail {
                path,
                reason,
                detail,
            });
        }
    }

    Ok(())
}

fn is_hidden(path: &Path) -> bool {
    path.components()
        .any(|component| component.as_os_str().to_string_lossy().starts_with('.'))
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::scan_tree;
    use crate::{Language, ScanOptions};

    #[test]
    fn scans_committed_blobs_without_a_git_subprocess() {
        let directory = tempdir().unwrap();
        let repository = gix::init(directory.path()).unwrap();
        let source = repository
            .write_blob(b"pub fn answer() -> u8 { 42 }")
            .unwrap();
        let hidden = repository.write_blob(b"hidden").unwrap();
        let binary = repository.write_blob([b'a', 0, b'b']).unwrap();
        let link = repository.write_blob(b"src/lib.rs").unwrap();
        let mut editor = repository.empty_tree().edit().unwrap();
        editor
            .upsert(
                "src/lib.rs",
                gix::object::tree::EntryKind::Blob,
                source.detach(),
            )
            .unwrap()
            .upsert(
                ".hidden.rs",
                gix::object::tree::EntryKind::Blob,
                hidden.detach(),
            )
            .unwrap()
            .upsert(
                "binary.dat",
                gix::object::tree::EntryKind::Blob,
                binary.detach(),
            )
            .unwrap()
            .upsert("link.rs", gix::object::tree::EntryKind::Link, link.detach())
            .unwrap();
        let tree = editor.write().unwrap();
        let signature = gix::actor::SignatureRef {
            name: b"tokm".as_slice().into(),
            email: b"tokm@example.invalid".as_slice().into(),
            time: "0 +0000",
        };
        let commit = repository
            .new_commit_as(
                signature,
                signature,
                "fixture",
                tree.detach(),
                std::iter::empty::<gix::ObjectId>(),
            )
            .unwrap();
        let revision = commit.id().to_string();
        drop(commit);
        drop(repository);
        let options = ScanOptions::default().with_cache_enabled(false);

        let result = scan_tree(directory.path(), &revision, &options).unwrap();
        let with_hidden = scan_tree(
            directory.path(),
            &revision,
            &options.clone().with_hidden(true),
        )
        .unwrap();

        assert_eq!(result.totals.files, 1);
        assert_eq!(result.files[0].path.to_str().unwrap(), "src/lib.rs");
        assert_eq!(result.files[0].language, Language::Rust);
        assert_eq!(result.skipped.binary, 1);
        assert_eq!(result.skipped.symlink, 1);
        assert_eq!(with_hidden.totals.files, 2);
    }

    #[test]
    fn invalid_repository_and_revision_errors_are_typed() {
        let directory = tempdir().unwrap();
        let missing_repository = scan_tree(directory.path(), "HEAD", &ScanOptions::default());
        gix::init(directory.path()).unwrap();
        let missing_revision = scan_tree(directory.path(), "missing", &ScanOptions::default());

        assert!(
            missing_repository
                .unwrap_err()
                .to_string()
                .contains("repository")
        );
        assert!(
            missing_revision
                .unwrap_err()
                .to_string()
                .contains("revision")
        );
    }
}
