//! Ignore-aware filesystem discovery and path-identity deduplication.

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use globset::{Glob, GlobSet, GlobSetBuilder};
use ignore::WalkBuilder;
use path_clean::PathClean;

use crate::{ScanError, ScanOptions, SkipDetail, SkipReason};

pub(crate) struct Discovery {
    pub(crate) paths: Vec<PathBuf>,
    pub(crate) skipped: Vec<SkipDetail>,
}

pub(crate) fn discover(
    inputs: Vec<PathBuf>,
    options: &ScanOptions,
) -> Result<Discovery, ScanError> {
    if inputs.is_empty() {
        return Err(ScanError::NoInputs);
    }

    let current_directory =
        std::env::current_dir().map_err(|error| ScanError::WorkingDirectory {
            message: error.to_string(),
        })?;
    let filters = Filters::new(options.includes(), options.excludes())?;
    let mut paths = BTreeMap::new();
    let mut skipped = Vec::new();

    for input in inputs {
        let input = input.clean();
        let link_metadata = fs::symlink_metadata(&input).map_err(|error| ScanError::Input {
            path: input.clone(),
            message: error.to_string(),
        })?;
        let metadata = if link_metadata.file_type().is_symlink() && options.follow_symlinks() {
            fs::metadata(&input).map_err(|error| ScanError::Input {
                path: input.clone(),
                message: error.to_string(),
            })?
        } else {
            link_metadata
        };

        if metadata.is_dir() {
            discover_directory(
                &input,
                &current_directory,
                options,
                &filters,
                &mut paths,
                &mut skipped,
            );
        } else {
            insert_path(&input, &current_directory, &filters, &mut paths);
        }
    }

    Ok(Discovery {
        paths: paths.into_values().collect(),
        skipped,
    })
}

fn discover_directory(
    root: &Path,
    current_directory: &Path,
    options: &ScanOptions,
    filters: &Filters,
    paths: &mut BTreeMap<PathBuf, PathBuf>,
    skipped: &mut Vec<SkipDetail>,
) {
    let use_ignore = !options.no_ignore();
    let mut builder = WalkBuilder::new(root);
    builder
        .hidden(!options.hidden())
        .parents(use_ignore)
        .ignore(use_ignore)
        .git_global(use_ignore)
        .git_ignore(use_ignore)
        .git_exclude(use_ignore)
        .require_git(false)
        .follow_links(options.follow_symlinks());

    for entry in builder.build() {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                skipped.push(SkipDetail {
                    path: walk_error_path(&error)
                        .unwrap_or(root)
                        .to_path_buf()
                        .clean(),
                    reason: SkipReason::Unreadable,
                    detail: None,
                });
                continue;
            }
        };
        let file_type = entry.file_type();
        let is_directory = file_type.is_some_and(|kind| kind.is_dir())
            || (options.follow_symlinks() && entry.path().is_dir());
        if is_directory {
            continue;
        }

        insert_path(entry.path(), current_directory, filters, paths);
    }
}

fn insert_path(
    path: &Path,
    current_directory: &Path,
    filters: &Filters,
    paths: &mut BTreeMap<PathBuf, PathBuf>,
) {
    let display_path = path.clean();
    if !filters.admits(&display_path, current_directory) {
        return;
    }

    let identity = if display_path.is_absolute() {
        display_path.clone()
    } else {
        current_directory.join(&display_path).clean()
    };
    paths.entry(identity).or_insert(display_path);
}

fn walk_error_path(error: &ignore::Error) -> Option<&Path> {
    match error {
        ignore::Error::Partial(errors) => errors.iter().find_map(walk_error_path),
        ignore::Error::WithLineNumber { err, .. } | ignore::Error::WithDepth { err, .. } => {
            walk_error_path(err)
        }
        ignore::Error::WithPath { path, .. } => Some(path),
        ignore::Error::Loop { child, .. } => Some(child),
        ignore::Error::Io(_)
        | ignore::Error::Glob { .. }
        | ignore::Error::UnrecognizedFileType(_)
        | ignore::Error::InvalidDefinition => None,
    }
}

struct Filters {
    includes: Option<GlobSet>,
    excludes: Option<GlobSet>,
}

impl Filters {
    fn new(includes: &[String], excludes: &[String]) -> Result<Self, ScanError> {
        Ok(Self {
            includes: compile_globs(includes)?,
            excludes: compile_globs(excludes)?,
        })
    }

    fn admits(&self, path: &Path, current_directory: &Path) -> bool {
        let match_path = path.strip_prefix(current_directory).unwrap_or(path);
        let match_path = forward_slash_path(match_path);

        if self
            .excludes
            .as_ref()
            .is_some_and(|set| set.is_match(&match_path))
        {
            return false;
        }

        self.includes
            .as_ref()
            .is_none_or(|set| set.is_match(&match_path))
    }
}

fn compile_globs(patterns: &[String]) -> Result<Option<GlobSet>, ScanError> {
    if patterns.is_empty() {
        return Ok(None);
    }

    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        let glob = Glob::new(pattern).map_err(|error| ScanError::InvalidGlob {
            pattern: pattern.clone(),
            message: error.to_string(),
        })?;
        builder.add(glob);
    }
    let set = builder.build().map_err(|error| ScanError::InvalidGlob {
        pattern: patterns.join(", "),
        message: error.to_string(),
    })?;

    Ok(Some(set))
}

fn forward_slash_path(path: &Path) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}
