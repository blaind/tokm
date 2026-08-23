"""Fast local token measurement backed by the native tokm engine."""

from collections.abc import Mapping, Sequence
from os import PathLike
from typing import final

__version__: str

class TokmError(Exception):
    """Base class for errors raised by tokm."""

class TokenizerError(TokmError):
    """Tokenizer selection or execution failed."""

class ConfigurationError(TokmError):
    """Measurement options are invalid."""

class InputError(TokmError):
    """An explicit input could not be scanned."""

@final
class TokenizerMetadata:
    @property
    def kind(self) -> str: ...
    @property
    def name(self) -> str: ...
    @property
    def fingerprint(self) -> str: ...
    @property
    def accuracy(self) -> str: ...
    @property
    def raw_text_semantics(self) -> str: ...

@final
class TextPolicy:
    @property
    def mode(self) -> str: ...
    @property
    def version(self) -> int | None: ...
    @property
    def invalid_utf8(self) -> str | None: ...

@final
class CountResult:
    @property
    def tokens(self) -> int: ...
    @property
    def bytes(self) -> int: ...
    @property
    def tokenizer(self) -> TokenizerMetadata: ...
    @property
    def text_policy(self) -> TextPolicy: ...

@final
class SkipDetail:
    @property
    def path(self) -> str: ...
    @property
    def reason(self) -> str: ...
    @property
    def detail(self) -> str | None: ...

@final
class FileResult:
    @property
    def path(self) -> str: ...
    @property
    def language(self) -> str | None: ...
    @property
    def tokens(self) -> int | None: ...
    @property
    def bytes(self) -> int | None: ...
    @property
    def decoded_lossily(self) -> bool: ...
    @property
    def skipped(self) -> SkipDetail | None: ...
    @property
    def tokenizer(self) -> TokenizerMetadata: ...
    @property
    def text_policy(self) -> TextPolicy: ...

@final
class ScanTotals:
    @property
    def files(self) -> int: ...
    @property
    def tokens(self) -> int: ...
    @property
    def bytes(self) -> int: ...

@final
class LanguageResult:
    @property
    def name(self) -> str: ...
    @property
    def files(self) -> int: ...
    @property
    def tokens(self) -> int: ...
    @property
    def bytes(self) -> int: ...

@final
class ScanFileResult:
    @property
    def path(self) -> str: ...
    @property
    def language(self) -> str: ...
    @property
    def tokens(self) -> int: ...
    @property
    def bytes(self) -> int: ...
    @property
    def decoded_lossily(self) -> bool: ...

@final
class SkippedSummary:
    @property
    def total(self) -> int: ...
    @property
    def binary(self) -> int: ...
    @property
    def oversized(self) -> int: ...
    @property
    def invalid_utf8(self) -> int: ...
    @property
    def unsupported_encoding(self) -> int: ...
    @property
    def unreadable(self) -> int: ...
    @property
    def unsupported_filesystem_type(self) -> int: ...
    @property
    def symlink(self) -> int: ...
    @property
    def details(self) -> list[SkipDetail]: ...

@final
class ScanResult:
    @property
    def total_tokens(self) -> int: ...
    @property
    def tokenizer(self) -> TokenizerMetadata: ...
    @property
    def text_policy(self) -> TextPolicy: ...
    @property
    def totals(self) -> ScanTotals: ...
    @property
    def languages(self) -> Mapping[str, LanguageResult]: ...
    @property
    def files(self) -> list[ScanFileResult]: ...
    @property
    def skipped(self) -> SkippedSummary: ...

def count(
    text: str,
    *,
    encoding: str | None = None,
    model: str | None = None,
    tokenizer_file: str | PathLike[str] | None = None,
    normalize: bool = False,
) -> CountResult: ...
def count_file(
    path: str | PathLike[str],
    *,
    encoding: str | None = None,
    model: str | None = None,
    tokenizer_file: str | PathLike[str] | None = None,
    normalize: bool = False,
    invalid_utf8: str = "skip",
    max_file_size: int | None = 20_971_520,
    no_cache: bool = False,
) -> FileResult: ...
def scan(
    paths: str | PathLike[str] | Sequence[str | PathLike[str]],
    *,
    encoding: str | None = None,
    model: str | None = None,
    tokenizer_file: str | PathLike[str] | None = None,
    normalize: bool = False,
    invalid_utf8: str = "skip",
    include: Sequence[str] | None = None,
    exclude: Sequence[str] | None = None,
    hidden: bool = False,
    no_ignore: bool = False,
    follow: bool = False,
    max_file_size: int | None = 20_971_520,
    no_cache: bool = False,
    threads: int | None = None,
) -> ScanResult: ...
