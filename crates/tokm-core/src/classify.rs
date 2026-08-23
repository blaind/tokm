//! Path-based language classification used for aggregation.

use std::{fmt, path::Path};

use serde::Serialize;

/// Language or text-file family assigned to a measured file.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub enum Language {
    /// Rust source.
    Rust,
    /// Python source or type stubs.
    Python,
    /// TypeScript source.
    TypeScript,
    /// JavaScript source.
    JavaScript,
    /// Markdown prose.
    Markdown,
    /// JSON or JSON Lines data.
    #[serde(rename = "JSON")]
    Json,
    /// YAML data.
    #[serde(rename = "YAML")]
    Yaml,
    /// TOML data.
    #[serde(rename = "TOML")]
    Toml,
    /// Shell source.
    Shell,
    /// C source or headers.
    C,
    /// C++ source or headers.
    #[serde(rename = "C++")]
    Cpp,
    /// Java source.
    Java,
    /// Go source.
    Go,
    /// Measured text without a recognized family.
    #[serde(rename = "Text / Unknown")]
    Unknown,
}

impl Language {
    /// Return the stable display and machine-output name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Rust => "Rust",
            Self::Python => "Python",
            Self::TypeScript => "TypeScript",
            Self::JavaScript => "JavaScript",
            Self::Markdown => "Markdown",
            Self::Json => "JSON",
            Self::Yaml => "YAML",
            Self::Toml => "TOML",
            Self::Shell => "Shell",
            Self::C => "C",
            Self::Cpp => "C++",
            Self::Java => "Java",
            Self::Go => "Go",
            Self::Unknown => "Text / Unknown",
        }
    }
}

impl fmt::Display for Language {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Classify a file from its basename and extension.
#[must_use]
pub fn classify(path: &Path) -> Language {
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase);

    match extension.as_deref() {
        Some("rs") => Language::Rust,
        Some("py" | "pyi") => Language::Python,
        Some("ts" | "tsx" | "mts" | "cts") => Language::TypeScript,
        Some("js" | "jsx" | "mjs" | "cjs") => Language::JavaScript,
        Some("md" | "markdown" | "mdx") => Language::Markdown,
        Some("json" | "jsonl") => Language::Json,
        Some("yaml" | "yml") => Language::Yaml,
        Some("toml") => Language::Toml,
        Some("sh" | "bash" | "zsh" | "fish") => Language::Shell,
        Some("c" | "h") => Language::C,
        Some("cc" | "cpp" | "cxx" | "hh" | "hpp" | "hxx") => Language::Cpp,
        Some("java") => Language::Java,
        Some("go") => Language::Go,
        _ => Language::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{Language, classify};

    #[test]
    fn classifies_common_source_and_data_extensions() {
        let cases = [
            ("lib.rs", Language::Rust),
            ("types.PYI", Language::Python),
            ("view.tsx", Language::TypeScript),
            ("package.json", Language::Json),
            ("notes", Language::Unknown),
        ];

        for (path, expected) in cases {
            assert_eq!(classify(Path::new(path)), expected, "{path}");
        }
    }
}
