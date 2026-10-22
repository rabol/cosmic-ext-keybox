// SPDX-License-Identifier: GPL-3.0-only

//! Narrowly scoped secret buffers.
//!
//! Secret values live only in [`SecretText`], which wipes its memory on drop,
//! cannot be cloned and never prints its contents through `Debug`. Copies made
//! by the GUI toolkit, the clipboard or other processes are outside our control.

use std::fmt;

use zeroize::Zeroizing;

/// A text secret. Wiped on drop, never cloned, redacted in `Debug`.
pub struct SecretText(Zeroizing<String>);

impl SecretText {
    /// Gives access to the secret text. Keep the borrow as short as possible.
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The text with control characters written as escapes (`\n`, `\0`,
    /// `\u{1b}`, ...), for the escaped inspection view. The result is itself
    /// a secret.
    #[must_use]
    pub fn escaped(&self) -> SecretText {
        let mut out = Zeroizing::new(String::with_capacity(self.0.len()));
        for c in self.0.chars() {
            match c {
                '\\' => out.push_str("\\\\"),
                '\n' => out.push_str("\\n"),
                '\r' => out.push_str("\\r"),
                '\t' => out.push_str("\\t"),
                '\0' => out.push_str("\\0"),
                c if c.is_control() => {
                    use fmt::Write;
                    let _ = write!(out, "\\u{{{:x}}}", c as u32);
                }
                c => out.push(c),
            }
        }
        SecretText(out)
    }
}

impl fmt::Debug for SecretText {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretText([REDACTED])")
    }
}

/// How a retrieved secret can be shown.
#[derive(Debug)]
pub enum SecretValue {
    /// Valid UTF-8 without control characters. May be empty.
    Text(SecretText),
    /// Valid UTF-8 containing NUL or other control characters (including
    /// newlines and tabs). Show it only through [`SecretText::escaped`];
    /// copying must use the original text and needs a clear user action.
    TextWithControl(SecretText),
    /// Not valid UTF-8. The bytes are wiped immediately; only the length is kept.
    Binary { len: usize },
}

impl SecretValue {
    /// Classifies decrypted bytes. Classification is by content, not by the
    /// declared content type: providers label text inconsistently.
    #[must_use]
    pub fn from_bytes(bytes: Zeroizing<Vec<u8>>) -> Self {
        let len = bytes.len();
        // `String::from_utf8` takes the buffer without copying. On failure the
        // error hands the buffer back so it can be wiped.
        let owned: Vec<u8> = {
            let mut bytes = bytes;
            std::mem::take(&mut *bytes)
        };
        match String::from_utf8(owned) {
            Ok(text) => {
                let text = SecretText(Zeroizing::new(text));
                if text.expose().chars().any(char::is_control) {
                    Self::TextWithControl(text)
                } else {
                    Self::Text(text)
                }
            }
            Err(err) => {
                drop(Zeroizing::new(err.into_bytes()));
                Self::Binary { len }
            }
        }
    }

    /// The text to copy, if this value can be copied. Binary content cannot.
    #[must_use]
    pub fn copyable_text(&self) -> Option<&SecretText> {
        match self {
            Self::Text(text) | Self::TextWithControl(text) => Some(text),
            Self::Binary { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn classify(bytes: &[u8]) -> SecretValue {
        SecretValue::from_bytes(Zeroizing::new(bytes.to_vec()))
    }

    #[test]
    fn plain_text() {
        let SecretValue::Text(text) = classify(b"hunter2") else {
            panic!("expected text");
        };
        assert_eq!(text.expose(), "hunter2");
    }

    #[test]
    fn empty_is_text_not_error() {
        let SecretValue::Text(text) = classify(b"") else {
            panic!("expected text");
        };
        assert!(text.is_empty());
    }

    #[test]
    fn control_characters_are_flagged_and_preserved() {
        let SecretValue::TextWithControl(text) = classify(b"a\nb\0c\x1b") else {
            panic!("expected text with control characters");
        };
        assert_eq!(text.expose(), "a\nb\0c\x1b");
        assert_eq!(text.escaped().expose(), "a\\nb\\0c\\u{1b}");
    }

    #[test]
    fn invalid_utf8_is_binary() {
        let value = classify(&[0xff, 0xfe, 0x00, 0x41]);
        assert!(matches!(value, SecretValue::Binary { len: 4 }));
        assert!(value.copyable_text().is_none());
    }

    #[test]
    fn debug_is_redacted() {
        let value = classify(b"KBX-UNIT-SECRET");
        let printed = format!("{value:?}");
        assert!(!printed.contains("KBX-UNIT-SECRET"), "{printed}");
    }
}
