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

The first release series provides the Rust core library, native CLI, typed Python bindings, local
Hugging Face `tokenizer.json` support, and Git tree diffs.

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
tokm . --model gpt-5
tokm . --encoding cl100k_base
tokm . --normalize
tokm . --invalid-utf8 lossy
```

Model names are aliases for a resolved built-in encoding:

| Model family | Encoding |
| --- | --- |
| `gpt-5*`, `gpt-4.1`, `gpt-4.1-*`, `gpt-4o`, `gpt-4o-*` | `o200k_base` |
| `gpt-4`, `gpt-4-*`, `gpt-3.5-turbo`, `gpt-3.5-turbo-*` | `cl100k_base` |

Results and cache keys identify the resolved encoding, not the model alias. This keeps identical
tokenizer definitions semantically identical even when selected through different model names.

Load a Hugging Face tokenizer entirely from a local artifact:

```console
tokm . --tokenizer-file ./tokenizer.json
```

`--model`, `--encoding`, and `--tokenizer-file` are mutually exclusive. Artifact identity is a
BLAKE3 hash of the actual file contents, not its path or timestamp. Raw-text measurement follows
the artifact's normalization, pre-tokenization, model, and added-token matching. It disables
automatic framing, truncation, and padding so the complete supplied text is measured without
BOS/EOS insertion.

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

Show recursive directory rollups instead of language totals:

```console
tokm . --dirs
tokm . --dirs --sort path
tokm . --dirs --format json
```

Each directory includes every measured file beneath it. Overlapping input roots still count each
logical file once. `--dirs` is a filesystem view and cannot be combined with stdin, Git diff,
`--files`, or `--sort language`.

Files larger than 20 MiB are skipped visibly. The limit accepts binary K/M/G suffixes.

```console
tokm dataset --max-file-size 100M
tokm dataset --max-file-size unlimited
```

## Git token diff

Compare a committed tree with the current worktree:

```console
tokm diff origin/main
```

The worktree form includes modified tracked files and untracked, non-ignored files. Compare only
committed states with an explicit two-tree range:

```console
tokm diff origin/main..HEAD
tokm diff v1.0.0..v1.1.0 --files
```

Both forms scan complete file contents under the same tokenizer and text policy, then compare the
two snapshots. They do not tokenize line-oriented patch hunks. Git objects are read through `gix`;
the Git executable is not invoked by normal operation. Content-addressed caching lets unchanged
blobs reuse prior counts.

Use `--repository PATH` when discovery should begin somewhere other than the current directory.
All regular measurement selectors and filters remain available after `diff`.

```console
tokm diff main --tokenizer-file ./tokenizer.json
tokm diff main..HEAD --format json | jq '.tokens.net'
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

Compare the measured total with a context window without changing tokenizer identity:

```console
tokm . --context 128000
```

Human output reports utilization and either remaining tokens or the amount exceeded. JSON adds a
separate `context` object containing the window, utilization percentage, remaining tokens, and
exceeded tokens. For `tokm diff`, the comparison applies to the head snapshot.

Estimate input cost using an explicit USD price per million tokens:

```console
tokm . --price-input 1.25
```

Pricing is supplied by the user; `tokm` does not maintain or fetch provider prices. Calculations
use fixed-point decimal arithmetic and apply to the measured total, or to the head snapshot for a
Git diff. JSON returns the currency, price, and estimate as exact decimal strings in a separate
`cost` object.

| Exit | Meaning |
| ---: | --- |
| 0 | Successful measurement |
| 1 | Operational failure |
| 2 | Invalid invocation |
| 3 | Token budget exceeded |

A successful scan may contain skipped files; omissions are explicit result data rather than an
implicit failure.

## Python API

Python 3.11 and newer can call the same Rust engine directly. The extension uses the stable
`abi3-py311` ABI and does not invoke the CLI as a subprocess.

```python
import tokm

count = tokm.count("hello world")
assert count.tokens == 2

file = tokm.count_file("README.md")
result = tokm.scan(["src", "tests"], encoding="o200k_base")
model = tokm.scan(".", model="gpt-5")
local = tokm.count("hello world", tokenizer_file="./tokenizer.json")

print(file.tokens)
print(result.total_tokens)
print(result.languages["Rust"].tokens)
```

`count_file()` and `scan()` expose the same normalization, invalid UTF-8, size-limit, ignore,
filtering, thread, and cache policies as the Rust core. Per-file omissions are returned as typed
skip data; invalid options and explicit input failures raise typed exceptions. Native measurement
releases the GIL.

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

Heavy integrations remain feature-gated. A minimal consumer can omit built-in tokenizers, the
persistent cache, Hugging Face support, and Git:

```toml
[dependencies]
tokm-core = { version = "0.1", default-features = false }
```

Enable committed-tree scanning and snapshot comparison explicitly:

```toml
[dependencies]
tokm-core = { version = "0.1", features = ["git"] }
```

## Benchmarks

Criterion harnesses cover tokenizer throughput, content hashing, 100k-result aggregation,
synthetic filesystem workloads, cold and warm caches, a single-file modification, and a Git diff
with a small change set.

```console
cargo bench --locked -p tokm-core --bench micro
cargo bench --locked -p tokm-core --bench scan_workloads
cargo bench --locked -p tokm-core --bench git_diff_workload --features git
```

Use release builds over representative real projects for end-to-end wall-time and peak-memory
measurements. On platforms with GNU `time`, for example:

```console
cargo build --locked --release -p tokm
/usr/bin/time -v target/release/tokm --no-cache PATH
/usr/bin/time -v target/release/tokm PATH
```

Cold and warm comparisons should use identical tokenizer, filtering, ignore, and file-size
policies. Competitor comparisons must likewise use equivalent behavior. Benchmark results, rather
than the native architecture alone, are required before making relative performance claims.

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

Build and test the Python package with maturin:

```console
python -m venv .venv
. .venv/bin/activate
python -m pip install 'maturin>=1.7,<2' pytest mypy ruff
maturin develop --locked --release
python -m pytest
ruff check python
mypy
```

## License

MIT
