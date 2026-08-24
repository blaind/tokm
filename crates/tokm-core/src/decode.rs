//! Deterministic binary detection and UTF-8 decoding.

use std::borrow::Cow;

use crate::{InvalidUtf8Policy, SkipReason};

const BINARY_INSPECTION_BYTES: usize = 8 * 1024;

pub(crate) enum DecodeOutcome<'a> {
    Text {
        text: Cow<'a, str>,
        decoded_lossily: bool,
    },
    Skipped {
        reason: SkipReason,
        detail: Option<&'static str>,
    },
}

pub(crate) fn decode(bytes: &[u8], invalid_utf8: InvalidUtf8Policy) -> DecodeOutcome<'_> {
    if let Some(encoding) = unsupported_bom(bytes) {
        return DecodeOutcome::Skipped {
            reason: SkipReason::UnsupportedEncoding,
            detail: Some(encoding),
        };
    }

    if bytes
        .iter()
        .take(BINARY_INSPECTION_BYTES)
        .any(|byte| *byte == 0)
    {
        return DecodeOutcome::Skipped {
            reason: SkipReason::Binary,
            detail: None,
        };
    }

    match std::str::from_utf8(bytes) {
        Ok(text) => DecodeOutcome::Text {
            text: Cow::Borrowed(text),
            decoded_lossily: false,
        },
        Err(_) if invalid_utf8 == InvalidUtf8Policy::Skip => DecodeOutcome::Skipped {
            reason: SkipReason::InvalidUtf8,
            detail: None,
        },
        Err(_) => DecodeOutcome::Text {
            text: String::from_utf8_lossy(bytes),
            decoded_lossily: true,
        },
    }
}

fn unsupported_bom(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0xff, 0xfe, 0x00, 0x00]) {
        Some("UTF-32LE")
    } else if bytes.starts_with(&[0x00, 0x00, 0xfe, 0xff]) {
        Some("UTF-32BE")
    } else if bytes.starts_with(&[0xff, 0xfe]) {
        Some("UTF-16LE")
    } else if bytes.starts_with(&[0xfe, 0xff]) {
        Some("UTF-16BE")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::{DecodeOutcome, decode};
    use crate::{InvalidUtf8Policy, SkipReason};

    #[test]
    fn utf16_bom_is_more_specific_than_binary() {
        let outcome = decode(&[0xff, 0xfe, b'a', 0], InvalidUtf8Policy::Skip);

        assert!(matches!(
            outcome,
            DecodeOutcome::Skipped {
                reason: SkipReason::UnsupportedEncoding,
                detail: Some("UTF-16LE")
            }
        ));
    }

    #[test]
    fn lossy_decoding_uses_replacement_characters() {
        let outcome = decode(&[b'a', 0xff, b'b'], InvalidUtf8Policy::Lossy);

        match outcome {
            DecodeOutcome::Text {
                text,
                decoded_lossily,
            } => {
                assert_eq!(text, "a\u{fffd}b");
                assert!(decoded_lossily);
            }
            DecodeOutcome::Skipped { .. } => panic!("lossy input was skipped"),
        }
    }
}
