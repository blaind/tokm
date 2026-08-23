//! Repeatable microbenchmarks for counting, hashing, and aggregation.

use std::{hint::black_box, path::PathBuf};

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use tokm_core::{
    CountOptions, Language, ScanFileResult, aggregate_directories, count_text,
    tokenizer::BuiltinEncoding,
};

fn benchmark_tokenization(criterion: &mut Criterion) {
    let cases = [
        (
            "rust/1KiB",
            repeat_to_size("pub fn answer() -> u64 { 42 }\n", 1024),
        ),
        (
            "rust/100KiB",
            repeat_to_size("pub fn answer() -> u64 { 42 }\n", 100 * 1024),
        ),
        ("json/1MiB", json_to_size(1024 * 1024)),
        (
            "markdown/100KiB",
            repeat_to_size(
                "## Measurement\n\nToken-aware prose with `code`.\n\n",
                100 * 1024,
            ),
        ),
        (
            "unicode/100KiB",
            repeat_to_size("你好，世界 👋🏽 café naïve Ελληνικά\n", 100 * 1024),
        ),
    ];
    let mut group = criterion.benchmark_group("count_text");

    for encoding in [BuiltinEncoding::O200kBase, BuiltinEncoding::Cl100kBase] {
        let options = CountOptions::for_encoding(encoding);
        for (name, text) in &cases {
            group.throughput(Throughput::Bytes(
                u64::try_from(text.len()).expect("fixture size fits u64"),
            ));
            group.bench_with_input(
                BenchmarkId::new(encoding.as_str(), name),
                text,
                |bencher, text| {
                    bencher.iter(|| count_text(black_box(text), black_box(&options)).unwrap());
                },
            );
        }
    }
    group.finish();
}

fn benchmark_content_hashing(criterion: &mut Criterion) {
    let cases = [
        ("1KiB", vec![b'a'; 1024]),
        ("1MiB", vec![b'a'; 1024 * 1024]),
    ];
    let mut group = criterion.benchmark_group("content_hash");

    for (name, bytes) in &cases {
        group.throughput(Throughput::Bytes(
            u64::try_from(bytes.len()).expect("fixture size fits u64"),
        ));
        group.bench_with_input(
            BenchmarkId::from_parameter(name),
            bytes,
            |bencher, bytes| {
                bencher.iter(|| blake3::hash(black_box(bytes)));
            },
        );
    }
    group.finish();
}

fn benchmark_aggregation(criterion: &mut Criterion) {
    let files = (0..100_000)
        .map(|index| ScanFileResult {
            path: PathBuf::from(format!("src/module-{}/file-{index}.rs", index % 100)),
            language: Language::Rust,
            tokens: 100,
            bytes: 400,
            decoded_lossily: false,
        })
        .collect::<Vec<_>>();

    criterion
        .benchmark_group("aggregation")
        .throughput(Throughput::Elements(
            u64::try_from(files.len()).expect("fixture count fits u64"),
        ))
        .bench_function("100k_directory_results", |bencher| {
            bencher.iter(|| aggregate_directories(black_box(&files), ["."]).unwrap());
        });
}

fn repeat_to_size(fragment: &str, minimum_bytes: usize) -> String {
    let repetitions = minimum_bytes.div_ceil(fragment.len());
    fragment.repeat(repetitions)
}

fn json_to_size(minimum_bytes: usize) -> String {
    let item = r#"{"identifier":123456,"enabled":true}"#;
    let repetitions = minimum_bytes.div_ceil(item.len() + 1);

    format!("[{}]", vec![item; repetitions].join(","))
}

criterion_group!(
    benches,
    benchmark_tokenization,
    benchmark_content_hashing,
    benchmark_aggregation
);
criterion_main!(benches);
