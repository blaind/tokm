//! Versioned transformations applied before tokenization.

use std::borrow::Cow;

use serde::Serialize;

/// Current opt-in normalization revision.
pub const NORMALIZATION_VERSION: u32 = 1;

/// Policy applied to decoded text before tokenization.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum TextPolicy {
    /// Measure decoded text without changing it.
    #[default]
    Literal,

    /// Remove a leading UTF-8 BOM and normalize line endings to LF.
    Normalized,
}

impl TextPolicy {
    /// Return the stable identity used by cache keys.
    #[must_use]
    pub const fn fingerprint(self) -> &'static str {
        match self {
            Self::Literal => "literal-v1",
            Self::Normalized => "normalized-v1",
        }
    }

    /// Return structured public metadata for this policy.
    #[must_use]
    pub const fn metadata(self) -> TextPolicyMetadata {
        match self {
            Self::Literal => TextPolicyMetadata {
                mode: "literal",
                version: None,
            },
            Self::Normalized => TextPolicyMetadata {
                mode: "normalized",
                version: Some(NORMALIZATION_VERSION),
            },
        }
    }

    pub(crate) fn apply(self, text: &str) -> Cow<'_, str> {
        match self {
            Self::Literal => Cow::Borrowed(text),
            Self::Normalized => normalize(text),
        }
    }
}

/// Serialized description of the selected text policy.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct TextPolicyMetadata {
    /// Stable mode name.
    pub mode: &'static str,

    /// Policy revision when the mode performs a transformation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<u32>,
}

fn normalize(text: &str) -> Cow<'_, str> {
    let without_bom = text.strip_prefix('\u{feff}').unwrap_or(text);
    if !without_bom.contains('\r') {
        return Cow::Borrowed(without_bom);
    }

    let mut normalized = String::with_capacity(without_bom.len());
    let mut characters = without_bom.chars().peekable();
    while let Some(character) = characters.next() {
        if character == '\r' {
            if characters.peek() == Some(&'\n') {
                characters.next();
            }
            normalized.push('\n');
        } else {
            normalized.push(character);
        }
    }

    Cow::Owned(normalized)
}

#[cfg(test)]
mod tests {
    use super::TextPolicy;

    #[test]
    fn literal_policy_borrows_unchanged_text() {
        let input = "\u{feff}a\r\nb\rc";

        assert_eq!(TextPolicy::Literal.apply(input), input);
    }

    #[test]
    fn normalization_removes_only_a_leading_bom_and_normalizes_line_endings() {
        let input = "\u{feff}a\r\nb\rc\n\u{feff}";

        assert_eq!(TextPolicy::Normalized.apply(input), "a\nb\nc\n\u{feff}");
    }
}
