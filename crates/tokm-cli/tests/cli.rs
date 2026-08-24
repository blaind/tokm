//! Black-box Phase 1 CLI correctness gates.

use std::{
    fs,
    io::Write,
    path::PathBuf,
    process::{Command, Output, Stdio},
};

use serde_json::Value;
use tempfile::tempdir;
use tokm_core::{CountOptions, TextPolicy, count_text, tokenizer::BuiltinEncoding};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn tokm() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_tokm"));
    command.arg("--no-cache");
    command
}

fn run_stdin(arguments: &[&str], bytes: &[u8]) -> Output {
    let mut command = tokm();
    let mut child = command
        .args(arguments)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(bytes).unwrap();

    child.wait_with_output().unwrap()
}

fn parse_json(output: &Output) -> Value {
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn fixture_bytes(name: &str) -> Vec<u8> {
    let encoded = fs::read_to_string(fixture(name).join("input.hex")).unwrap();
    encoded
        .split_ascii_whitespace()
        .map(|byte| u8::from_str_radix(byte, 16).unwrap())
        .collect()
}

#[test]
fn json_scan_is_deterministic_complete_and_diagnostic_free() {
    let input = fixture("mixed_languages");
    let first = tokm()
        .args(["--format", "json", "--quiet"])
        .arg(&input)
        .output()
        .unwrap();
    let second = tokm()
        .args(["--format", "json", "--quiet"])
        .arg(&input)
        .output()
        .unwrap();
    let json = parse_json(&first);

    assert!(first.stderr.is_empty());
    assert_eq!(first.stdout, second.stdout);
    assert_eq!(json["schema_version"], 1);
    assert_eq!(json["totals"]["files"], 4);
    assert_eq!(json["files"].as_array().unwrap().len(), 4);
    assert!(json["files"].as_array().unwrap().iter().all(|file| {
        !file["path"]
            .as_str()
            .expect("path is a string")
            .contains('\\')
    }));
}

#[test]
fn cli_and_core_match_both_exact_encodings() {
    let path = fixture("special_tokens").join("input.txt");
    let text = fs::read_to_string(&path).unwrap();

    for encoding in [BuiltinEncoding::O200kBase, BuiltinEncoding::Cl100kBase] {
        let output = tokm()
            .args(["--format", "json", "--encoding", encoding.as_str()])
            .arg(&path)
            .output()
            .unwrap();
        let json = parse_json(&output);
        let expected = count_text(&text, &CountOptions::for_encoding(encoding))
            .unwrap()
            .tokens;

        assert_eq!(json["totals"]["tokens"], expected, "{encoding}");
    }
}

#[test]
fn local_tokenizer_file_uses_artifact_semantics_and_metadata() {
    let tokenizer = fixture("hf_tokenizer").join("tokenizer.json");
    let output = run_stdin(
        &[
            "--format",
            "json",
            "--tokenizer-file",
            tokenizer.to_str().unwrap(),
            "-",
        ],
        b"hello world",
    );
    let json = parse_json(&output);

    assert_eq!(json["totals"]["tokens"], 2);
    assert_eq!(json["tokenizer"]["kind"], "local_file");
    assert_eq!(json["tokenizer"]["name"], "tokenizer.json");
    assert_eq!(json["tokenizer"]["accuracy"], "exact");
    assert_eq!(json["tokenizer"]["raw_text_semantics"], "artifact");
    assert!(
        json["tokenizer"]["fingerprint"]
            .as_str()
            .unwrap()
            .starts_with("huggingface:blake3:")
    );
}

#[test]
fn tokenizer_selector_conflict_is_a_usage_error() {
    let output = tokm()
        .args([
            "--encoding",
            "o200k_base",
            "--tokenizer-file",
            "tokenizer.json",
        ])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
}

#[test]
fn stdin_uses_literal_and_normalized_core_semantics() {
    let bytes = fixture_bytes("bom");
    let text = std::str::from_utf8(&bytes).unwrap();
    let literal = run_stdin(&["--format", "json", "-"], &bytes);
    let normalized = run_stdin(&["--format", "json", "--normalize", "-"], &bytes);
    let literal_json = parse_json(&literal);
    let normalized_json = parse_json(&normalized);
    let literal_expected = count_text(text, &CountOptions::default()).unwrap().tokens;
    let normalized_expected = count_text(
        text,
        &CountOptions::default().with_text_policy(TextPolicy::Normalized),
    )
    .unwrap()
    .tokens;

    assert_eq!(literal_json["totals"]["tokens"], literal_expected);
    assert_eq!(normalized_json["totals"]["tokens"], normalized_expected);
    assert_eq!(normalized_json["text_policy"]["version"], 1);
}

#[test]
fn skips_are_aggregated_and_verbose_details_stay_on_stderr() {
    let directory = tempdir().unwrap();
    fs::write(directory.path().join("valid.txt"), "ok").unwrap();
    fs::write(directory.path().join("binary.dat"), fixture_bytes("binary")).unwrap();
    fs::write(
        directory.path().join("invalid.txt"),
        fixture_bytes("invalid_utf8"),
    )
    .unwrap();
    fs::write(
        directory.path().join("utf16.txt"),
        fixture_bytes("unsupported_encoding"),
    )
    .unwrap();
    fs::copy(
        fixture("oversized").join("input.txt"),
        directory.path().join("large.txt"),
    )
    .unwrap();

    let output = tokm()
        .args(["--format", "json", "--max-file-size", "5", "--verbose"])
        .arg(directory.path())
        .output()
        .unwrap();
    let json = parse_json(&output);
    let stderr = String::from_utf8(output.stderr).unwrap();
    let lossy = tokm()
        .args([
            "--format",
            "json",
            "--max-file-size",
            "5",
            "--invalid-utf8",
            "lossy",
        ])
        .arg(directory.path())
        .output()
        .unwrap();
    let lossy = parse_json(&lossy);

    assert_eq!(json["totals"]["files"], 1);
    assert_eq!(json["skipped"]["total"], 4);
    assert_eq!(json["skipped"]["binary"], 1);
    assert_eq!(json["skipped"]["oversized"], 1);
    assert_eq!(json["skipped"]["invalid_utf8"], 1);
    assert_eq!(json["skipped"]["unsupported_encoding"], 1);
    assert!(stderr.contains("skipped (binary)"));
    assert!(stderr.contains("UTF-16LE"));
    assert_eq!(lossy["totals"]["files"], 2);
    assert_eq!(lossy["skipped"]["invalid_utf8"], 0);
    assert!(lossy["files"].as_array().unwrap().iter().any(|file| {
        file["path"].as_str().unwrap().ends_with("invalid.txt")
            && file["decoded_lossily"].as_bool() == Some(true)
    }));
}

#[test]
fn ignore_hidden_and_overlapping_input_contracts_hold() {
    let ignored = fixture("gitignored");
    let default = tokm()
        .args(["--format", "json"])
        .arg(&ignored)
        .output()
        .unwrap();
    let all = tokm()
        .args(["--format", "json", "--hidden", "--no-ignore"])
        .arg(&ignored)
        .output()
        .unwrap();
    let overlap = fixture("overlapping_inputs");
    let overlapping = tokm()
        .args(["--format", "json"])
        .arg(&overlap)
        .arg(overlap.join("src"))
        .output()
        .unwrap();

    assert_eq!(parse_json(&default)["totals"]["files"], 1);
    assert_eq!(parse_json(&all)["totals"]["files"], 4);
    assert_eq!(parse_json(&overlapping)["totals"]["files"], 1);
}

#[test]
fn budget_quiet_and_usage_exit_codes_are_stable() {
    let budget = tokm()
        .args(["--quiet", "--max-tokens", "0"])
        .arg(fixture("basic"))
        .output()
        .unwrap();
    let mixed_stdin = tokm().arg("-").arg(fixture("basic")).output().unwrap();
    let missing = tokm().arg(fixture("does-not-exist")).output().unwrap();

    assert_eq!(budget.status.code(), Some(3));
    assert!(budget.stdout.is_empty());
    assert!(String::from_utf8_lossy(&budget.stderr).contains("Token budget exceeded"));
    assert_eq!(mixed_stdin.status.code(), Some(2));
    assert_eq!(missing.status.code(), Some(1));
}

#[test]
fn git_diff_supports_worktree_and_two_tree_forms() {
    let directory = tempdir().unwrap();
    git(directory.path(), &["init", "--quiet"]);
    git(directory.path(), &["config", "user.name", "tokm tests"]);
    git(
        directory.path(),
        &["config", "user.email", "tokm@example.invalid"],
    );
    fs::create_dir(directory.path().join("src")).unwrap();
    fs::write(directory.path().join(".gitignore"), "ignored.rs\n").unwrap();
    fs::write(directory.path().join("src/lib.rs"), "one").unwrap();
    git(directory.path(), &["add", "."]);
    git(directory.path(), &["commit", "--quiet", "-m", "base"]);
    let base = git_stdout(directory.path(), &["rev-parse", "HEAD"]);

    fs::write(directory.path().join("src/lib.rs"), "one two").unwrap();
    fs::write(directory.path().join("notes.md"), "untracked notes").unwrap();
    fs::write(directory.path().join("ignored.rs"), "ignored").unwrap();
    let worktree = tokm()
        .args(["diff", &base, "--format", "json", "--repository"])
        .arg(directory.path())
        .output()
        .unwrap();
    let repeated = tokm()
        .args(["diff", &base, "--format", "json", "--repository"])
        .arg(directory.path())
        .output()
        .unwrap();
    let worktree_json = parse_json(&worktree);
    let paths = worktree_json["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|file| file["path"].as_str().unwrap())
        .collect::<Vec<_>>();

    assert_eq!(worktree.stdout, repeated.stdout);
    assert!(worktree.stderr.is_empty());
    assert_eq!(worktree_json["report"], "diff");
    assert_eq!(worktree_json["snapshots"]["base"]["name"], base);
    assert_eq!(worktree_json["snapshots"]["head"]["name"], "worktree");
    assert_eq!(paths, ["notes.md", "src/lib.rs"]);
    assert!(!paths.contains(&"ignored.rs"));

    git(directory.path(), &["add", "."]);
    git(directory.path(), &["commit", "--quiet", "-m", "head"]);
    let head = git_stdout(directory.path(), &["rev-parse", "HEAD"]);
    let comparison = format!("{base}..{head}");
    let trees = tokm()
        .args(["diff", &comparison, "--format", "json", "--repository"])
        .arg(directory.path())
        .output()
        .unwrap();
    let trees_json = parse_json(&trees);
    let human = tokm()
        .args(["diff", &comparison, "--files", "--repository"])
        .arg(directory.path())
        .output()
        .unwrap();

    assert_eq!(trees_json["snapshots"]["base"]["name"], base);
    assert_eq!(trees_json["snapshots"]["head"]["name"], head);
    assert_eq!(trees_json["tokens"]["net"], worktree_json["tokens"]["net"]);
    assert!(
        String::from_utf8(human.stdout)
            .unwrap()
            .contains("Token delta")
    );

    let malformed = tokm()
        .args(["diff", "HEAD...HEAD", "--repository"])
        .arg(directory.path())
        .output()
        .unwrap();
    assert_eq!(malformed.status.code(), Some(2));
}

fn git(directory: &std::path::Path, arguments: &[&str]) {
    let output = Command::new("git")
        .args(["-C", directory.to_str().unwrap()])
        .args(arguments)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn git_stdout(directory: &std::path::Path, arguments: &[&str]) -> String {
    let output = Command::new("git")
        .args(["-C", directory.to_str().unwrap()])
        .args(arguments)
        .output()
        .unwrap();
    assert!(output.status.success());
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

#[cfg(unix)]
#[test]
fn concurrent_processes_share_one_cache_safely() {
    let input = tempdir().unwrap();
    let cache = tempdir().unwrap();
    for index in 0..64 {
        fs::write(
            input.path().join(format!("input-{index}.rs")),
            format!(
                "pub const VALUE_{index}: &str = \"{}\";",
                "value".repeat(100)
            ),
        )
        .unwrap();
    }

    let make_command = || {
        let mut command = Command::new(env!("CARGO_BIN_EXE_tokm"));
        command
            .args(["--format", "json", "--threads", "4"])
            .arg(input.path())
            .env("XDG_CACHE_HOME", cache.path())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        command
    };
    let first = make_command().spawn().unwrap();
    let second = make_command().spawn().unwrap();
    let first = first.wait_with_output().unwrap();
    let second = second.wait_with_output().unwrap();

    assert_eq!(parse_json(&first), parse_json(&second));
    assert!(first.stderr.is_empty());
    assert!(second.stderr.is_empty());
    assert!(cache.path().join("tokm/counts/1").is_dir());
}

#[test]
fn fixture_paths_exist_for_every_phase_one_gate() {
    for name in [
        "basic",
        "mixed_languages",
        "gitignored",
        "unicode",
        "invalid_utf8",
        "bom",
        "crlf",
        "binary",
        "oversized",
        "special_tokens",
        "overlapping_inputs",
        "racy_cache",
    ] {
        assert!(fixture(name).is_dir(), "missing fixture: {name}");
    }
}
