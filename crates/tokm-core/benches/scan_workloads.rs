//! Synthetic end-to-end filesystem and cache workloads.

use std::{
    cell::Cell,
    fs,
    hint::black_box,
    path::PathBuf,
    time::{Duration, SystemTime},
};

use criterion::{BatchSize, BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use filetime::{FileTime, set_file_mtime};
use tempfile::{TempDir, tempdir};
use tokm_core::{ScanOptions, scan};

struct Workload {
    _directory: TempDir,
    root: PathBuf,
    bytes: u64,
}

fn benchmark_uncached_scans(criterion: &mut Criterion) {
    let workloads = [
        ("many_tiny", many_tiny_files()),
        ("polyglot", polyglot_project()),
        ("documentation", documentation_tree()),
        ("json_corpus", json_corpus()),
    ];
    let options = ScanOptions::default().with_cache_enabled(false);
    let mut group = criterion.benchmark_group("scan/no_cache");
    group
        .sample_size(10)
        .measurement_time(Duration::from_secs(10));

    for (name, workload) in &workloads {
        group.throughput(Throughput::Bytes(workload.bytes));
        group.bench_with_input(
            BenchmarkId::from_parameter(name),
            workload,
            |bencher, workload| {
                bencher.iter(|| scan([black_box(&workload.root)], black_box(&options)).unwrap());
            },
        );
    }
    group.finish();
}

fn benchmark_cache_paths(criterion: &mut Criterion) {
    let workload = many_tiny_files();
    let warm_cache = tempdir().unwrap();
    let warm_options = ScanOptions::default().with_cache_directory(warm_cache.path());
    scan([&workload.root], &warm_options).unwrap();
    let mut group = criterion.benchmark_group("scan/cache");
    group
        .sample_size(10)
        .measurement_time(Duration::from_secs(10))
        .throughput(Throughput::Bytes(workload.bytes));

    group.bench_function("cold", |bencher| {
        bencher.iter_batched(
            || tempdir().unwrap(),
            |cache| {
                let options = ScanOptions::default().with_cache_directory(cache.path());
                scan([black_box(&workload.root)], black_box(&options)).unwrap()
            },
            BatchSize::SmallInput,
        );
    });
    group.bench_function("warm", |bencher| {
        bencher.iter(|| scan([black_box(&workload.root)], black_box(&warm_options)).unwrap());
    });

    let changed = workload.root.join("module-0/file-0.rs");
    let revision = Cell::new(false);
    group.bench_function("single_file_modified", |bencher| {
        bencher.iter_batched(
            || {
                revision.set(!revision.get());
                let value = if revision.get() { '1' } else { '2' };
                fs::write(&changed, format!("pub const VALUE: u8 = {value};\n")).unwrap();
            },
            |()| scan([black_box(&workload.root)], black_box(&warm_options)).unwrap(),
            BatchSize::SmallInput,
        );
    });
    group.finish();
}

fn many_tiny_files() -> Workload {
    create_workload("many-tiny", 2_000, |index| {
        (
            format!("module-{}/file-{index}.rs", index % 50),
            format!("pub const VALUE_{index}: u32 = {index};\n"),
        )
    })
}

fn polyglot_project() -> Workload {
    const EXTENSIONS: [&str; 8] = ["rs", "py", "ts", "js", "md", "json", "yaml", "toml"];

    create_workload("polyglot", 500, |index| {
        let extension = EXTENSIONS[index % EXTENSIONS.len()];
        let body = format!(
            "identifier_{index} = {}\n",
            "structured content ".repeat(100)
        );
        (
            format!("language-{extension}/file-{index}.{extension}"),
            body,
        )
    })
}

fn documentation_tree() -> Workload {
    create_workload("documentation", 200, |index| {
        let body = format!(
            "# Document {index}\n\n{}",
            "Token measurement documentation with `inline_code`.\n\n".repeat(150)
        );
        (
            format!("guide/chapter-{}/page-{index}.md", index % 20),
            body,
        )
    })
}

fn json_corpus() -> Workload {
    create_workload("json", 8, |index| {
        let item = format!(r#"{{"id":{index},"enabled":true,"text":"measurement"}}"#);
        let body = format!("[{}]", vec![item; 20_000].join(","));
        (format!("dataset/part-{index}.json"), body)
    })
}

fn create_workload(
    name: &str,
    files: usize,
    mut content: impl FnMut(usize) -> (String, String),
) -> Workload {
    let directory = tempdir().unwrap();
    let root = directory.path().join(name);
    let mut bytes = 0_u64;

    for index in 0..files {
        let (relative, body) = content(index);
        let path = root.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, &body).unwrap();
        set_file_mtime(
            &path,
            FileTime::from_system_time(SystemTime::now() - Duration::from_secs(5)),
        )
        .unwrap();
        bytes += u64::try_from(body.len()).expect("fixture size fits u64");
    }

    Workload {
        _directory: directory,
        root,
        bytes,
    }
}

criterion_group!(benches, benchmark_uncached_scans, benchmark_cache_paths);
criterion_main!(benches);
