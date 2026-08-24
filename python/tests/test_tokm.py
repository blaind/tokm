from __future__ import annotations

from pathlib import Path

import pytest
import tokm


def test_count_exposes_exact_semantics() -> None:
    result = tokm.count("hello world")

    assert result.tokens == 2
    assert result.bytes == 11
    assert result.tokenizer.name == "o200k_base"
    assert result.tokenizer.accuracy == "exact"
    assert result.tokenizer.raw_text_semantics == "ordinary"
    assert result.text_policy.mode == "literal"


def test_model_alias_uses_resolved_encoding_identity() -> None:
    default = tokm.count("hello world")
    aliased = tokm.count("hello world", model="gpt-5")

    assert aliased.tokens == default.tokens
    assert aliased.tokenizer.name == "o200k_base"
    assert aliased.tokenizer.fingerprint == default.tokenizer.fingerprint


def test_count_file_reports_per_file_skips(tmp_path: Path) -> None:
    valid = tmp_path / "valid.rs"
    invalid = tmp_path / "invalid.txt"
    valid.write_text("fn main() {}", encoding="utf-8")
    invalid.write_bytes(b"a\xffb")

    measured = tokm.count_file(valid, no_cache=True)
    skipped = tokm.count_file(invalid, no_cache=True)
    lossy = tokm.count_file(invalid, invalid_utf8="lossy", no_cache=True)

    assert measured.tokens is not None
    assert measured.language == "Rust"
    assert measured.skipped is None
    assert skipped.tokens is None
    assert skipped.skipped is not None
    assert skipped.skipped.reason == "invalid UTF-8"
    assert lossy.tokens is not None
    assert lossy.decoded_lossily


def test_scan_returns_mapping_and_native_aggregation(tmp_path: Path) -> None:
    (tmp_path / "src").mkdir()
    (tmp_path / "src" / "lib.rs").write_text("pub fn answer() -> u8 { 42 }", encoding="utf-8")
    (tmp_path / "README.md").write_text("# Project", encoding="utf-8")
    (tmp_path / "ignored.rs").write_text("ignored", encoding="utf-8")
    (tmp_path / ".gitignore").write_text("ignored.rs\n", encoding="utf-8")

    result = tokm.scan(tmp_path, no_cache=True)

    assert result.totals.files == 2
    assert result.total_tokens == result.totals.tokens
    assert result.totals.tokens == sum(file.tokens for file in result.files)
    assert result.languages["Rust"].files == 1
    assert result.languages["Markdown"].files == 1
    assert result.skipped.total == 0


def test_invalid_configuration_has_typed_errors(tmp_path: Path) -> None:
    with pytest.raises(tokm.TokenizerError, match="unsupported encoding"):
        tokm.count("text", encoding="missing")

    with pytest.raises(tokm.TokenizerError, match="unsupported model"):
        tokm.count("text", model="missing")

    with pytest.raises(tokm.ConfigurationError, match="invalid_utf8"):
        tokm.scan(tmp_path, invalid_utf8="replace")

    with pytest.raises(tokm.ConfigurationError, match="threads"):
        tokm.scan(tmp_path, threads=0)

    with pytest.raises(tokm.ConfigurationError, match="mutually exclusive"):
        tokm.count(
            "text",
            encoding="o200k_base",
            tokenizer_file=tmp_path / "tokenizer.json",
        )

    with pytest.raises(tokm.ConfigurationError, match="mutually exclusive"):
        tokm.count("text", encoding="o200k_base", model="gpt-5")

    with pytest.raises(tokm.ConfigurationError, match="mutually exclusive"):
        tokm.count("text", model="gpt-5", tokenizer_file=tmp_path / "tokenizer.json")


def test_missing_scan_input_is_an_input_error(tmp_path: Path) -> None:
    with pytest.raises(tokm.InputError, match="cannot inspect input"):
        tokm.scan(tmp_path / "missing")


def test_local_tokenizer_file_uses_shared_artifact_semantics() -> None:
    tokenizer_file = (
        Path(__file__).parents[2]
        / "crates"
        / "tokm-core"
        / "fixtures"
        / "hf_tokenizer"
        / "tokenizer.json"
    )

    result = tokm.count("HÉLLO <|im_start|> WORLD", tokenizer_file=tokenizer_file)

    assert result.tokens == 3
    assert result.tokenizer.kind == "local_file"
    assert result.tokenizer.accuracy == "exact"
    assert result.tokenizer.raw_text_semantics == "artifact"
    assert result.tokenizer.fingerprint.startswith("huggingface:blake3:")
