# tokm

`tokm` is a fast, local token measurement engine for text, files, and directory trees.

It keeps traversal, ignore matching, decoding, tokenization, caching, classification, and
aggregation in one native Rust pipeline. Input is never uploaded, executed, or sent to a model
API.

```console
$ tokm .
 Language          Files  Tokens  Percent
-----------------------------------------
 Rust                  8  13,492    94.1%
 Markdown              1     841     5.9%
-----------------------------------------
 Total                 9  14,333   100.0%
```

The first release series provides the Rust core library and native CLI. Python bindings, local
Hugging Face `tokenizer.json` support, and Git tree diffs follow in later phases.

## Usage

Measure a file, one or more directory trees, or stdin:

```console
tokm README.md
tokm src tests
tokm . --files
cat prompt.txt | tokm -
```

The default tokenizer is exact local `o200k_base` ordinary encoding. It is a versioned public
default and does not recognize special-token-looking strings as control instructions.

```console
tokm . --encoding cl100k_base
tokm . --normalize
tokm . --invalid-utf8 lossy
```

Decoded text is literal by default. `--normalize` removes one leading UTF-8 BOM, converts CRLF to
LF, and converts standalone CR to LF. Invalid UTF-8 is skipped by default; lossy mode applies Rust
replacement semantics and records that policy in structured output.

Directory traversal respects `.gitignore`, `.ignore`, and Git excludes. Hidden paths and symlinks
are not followed by default.

```console
tokm . --include '*.rs' --exclude 'vendor/**'
tokm . --hidden --no-ignore
tokm . --follow
```

Files larger than 20 MiB are skipped visibly. The limit accepts binary K/M/G suffixes.

```console
tokm dataset --max-file-size 100M
tokm dataset --max-file-size unlimited
```

## JSON and CI

JSON output is deterministic and always includes complete per-file entries. Machine-readable paths
use `/` separators. stdout contains only JSON; warnings, verbose skip details, and budget failures
go to stderr.

```console
tokm . --format json | jq '.totals.tokens'
tokm . --format json --quiet
```

The schema begins with `schema_version: 1` and records tokenizer identity, exact/proxy accuracy,
raw-text semantics, text policy, totals, languages, files, and skip accounting.

Token budgets use a dedicated exit code:

```console
tokm . --max-tokens 200000
```

| Exit | Meaning |
| ---: | --- |
| 0 | Successful measurement |
| 1 | Operational failure |
| 2 | Invalid invocation |
| 3 | Token budget exceeded |

A successful scan may contain skipped files; omissions are explicit result data rather than an
implicit failure.

## Cache

Persistent caching is enabled by default and shared by native consumers. Disable it with
`--no-cache`.

Counts are keyed by:

- BLAKE3 content hash;
- tokenizer fingerprint;
- text-policy and invalid-UTF-8 fingerprints; and
- counting-semantics revision.

Path, size, and mtime only provide a hash fast path. Potentially racy metadata is rehashed. Count
entries are immutable in meaning, publication is atomic, and locks are scoped to individual cache
entries so concurrent processes can safely converge on the same result. Cache corruption is a
miss, never scan data.

## Rust API

`tokm-core` owns all measurement semantics used by the CLI:

```rust
use tokm_core::{CountOptions, ScanOptions, count_text, scan};

let count = count_text("hello world", &CountOptions::default())?;
assert_eq!(count.tokens, 2);

let result = scan(["src", "tests"], &ScanOptions::default())?;
println!("{}", result.totals.tokens);
# Ok::<(), Box<dyn std::error::Error>>(())
```

Heavy future integrations remain feature-gated. A minimal consumer can omit built-in tokenizers and
the persistent cache:

```toml
[dependencies]
tokm-core = { version = "0.1", default-features = false }
```

## Development

The workspace pins its Rust toolchain. Run the same host gates as CI:

```console
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
cargo test --locked --workspace --all-features
RUSTFLAGS='-D warnings' cargo check --locked -p tokm-core --no-default-features
cargo deny --locked --workspace --all-features check
```

Install the development CLI directly from the workspace:

```console
cargo install --path crates/tokm-cli --locked
```

## License

MIT
