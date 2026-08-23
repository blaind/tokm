//! Complete-tree Git diff workloads with cold and warm count caches.

use std::{hint::black_box, time::Duration};

use criterion::{BatchSize, Criterion, Throughput, criterion_group, criterion_main};
use tempfile::{TempDir, tempdir};
use tokm_core::{ScanOptions, git::diff_trees};

const FILE_COUNT: usize = 2_000;
const CHANGED_FILES: usize = 12;

struct GitWorkload {
    _directory: TempDir,
    repository_path: std::path::PathBuf,
    base: String,
    head: String,
}

fn benchmark_git_diff(criterion: &mut Criterion) {
    let workload = git_workload();
    let warm_cache = tempdir().unwrap();
    let warm_options = ScanOptions::default().with_cache_directory(warm_cache.path());
    diff_trees(
        &workload.repository_path,
        &workload.base,
        &workload.head,
        &warm_options,
    )
    .unwrap();
    let mut group = criterion.benchmark_group("git_diff/2000_files_12_changed");
    group
        .sample_size(10)
        .measurement_time(Duration::from_secs(10))
        .throughput(Throughput::Elements(FILE_COUNT as u64));

    group.bench_function("cold_cache", |bencher| {
        bencher.iter_batched(
            || tempdir().unwrap(),
            |cache| {
                let options = ScanOptions::default().with_cache_directory(cache.path());
                diff_trees(
                    black_box(&workload.repository_path),
                    black_box(&workload.base),
                    black_box(&workload.head),
                    black_box(&options),
                )
                .unwrap()
            },
            BatchSize::SmallInput,
        );
    });
    group.bench_function("warm_cache", |bencher| {
        bencher.iter(|| {
            diff_trees(
                black_box(&workload.repository_path),
                black_box(&workload.base),
                black_box(&workload.head),
                black_box(&warm_options),
            )
            .unwrap()
        });
    });
    group.finish();
}

fn git_workload() -> GitWorkload {
    let directory = tempdir().unwrap();
    let repository = gix::init(directory.path()).unwrap();
    let base = commit_snapshot(&repository, false);
    let head = commit_snapshot(&repository, true);
    drop(repository);

    GitWorkload {
        repository_path: directory.path().to_path_buf(),
        _directory: directory,
        base,
        head,
    }
}

fn commit_snapshot(repository: &gix::Repository, changed: bool) -> String {
    let mut editor = repository.empty_tree().edit().unwrap();

    for index in 0..FILE_COUNT {
        let revision = usize::from(changed && index < CHANGED_FILES);
        let body = format!(
            "pub const VALUE_{index}: &str = \"revision-{revision}-{}\";\n",
            "content".repeat(20)
        );
        let blob = repository.write_blob(body).unwrap();
        editor
            .upsert(
                format!("src/module-{}/file-{index}.rs", index % 100),
                gix::object::tree::EntryKind::Blob,
                blob.detach(),
            )
            .unwrap();
    }

    let tree = editor.write().unwrap();
    let signature = gix::actor::SignatureRef {
        name: b"tokm benchmark".as_slice().into(),
        email: b"tokm@example.invalid".as_slice().into(),
        time: "0 +0000",
    };

    repository
        .new_commit_as(
            signature,
            signature,
            "benchmark snapshot",
            tree.detach(),
            std::iter::empty::<gix::ObjectId>(),
        )
        .unwrap()
        .id()
        .to_string()
}

criterion_group!(benches, benchmark_git_diff);
criterion_main!(benches);
