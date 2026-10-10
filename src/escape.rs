//! Backslash escapes in the body, which libnotify's notify-send replaces with
//! GLib's `g_strcompress` before showing the notification.

/// Replaces the escapes `\b`, `\f`, `\n`, `\r`, `\t` and `\v`, one to three
/// octal digits (`\303\251` is `é` in UTF-8), and `\c` for any other `c`, so
/// `\\` is a backslash and `\"` a quote. A trailing backslash is dropped.
///
/// Like GLib, this works on bytes, and the result ends at a NUL (`\0`). Octal
/// escapes can therefore produce invalid UTF-8, which gives `None`.
pub fn compress(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while let Some(&byte) = bytes.get(i) {
        i += 1;
        if byte != b'\\' {
            out.push(byte);
            continue;
        }
        let Some(&escaped) = bytes.get(i) else {
            break;
        };
        i += 1;
        let byte = match escaped {
            b'0'..=b'7' => {
                // GLib computes this in a char, so values above 0o377 wrap.
                let mut value = escaped - b'0';
                for _ in 0..2 {
                    match bytes.get(i) {
                        Some(&digit @ b'0'..=b'7') => {
                            value = value.wrapping_mul(8).wrapping_add(digit - b'0');
                            i += 1;
                        }
                        _ => break,
                    }
                }
                value
            }
            b'b' => 0x08,
            b'f' => 0x0c,
            b'n' => b'\n',
            b'r' => b'\r',
            b't' => b'\t',
            b'v' => 0x0b,
            other => other,
        };
        if byte == 0 {
            // GLib returns a C string, which ends here.
            break;
        }
        out.push(byte);
    }
    String::from_utf8(out).ok()
}

#[cfg(test)]
mod tests {
    use super::compress;

    fn c(text: &str) -> String {
        compress(text).unwrap()
    }

    #[test]
    fn plain_text_is_unchanged() {
        assert_eq!(c("Build finished in 3 min"), "Build finished in 3 min");
        assert_eq!(c("çok güzel 🎉"), "çok güzel 🎉");
    }

    #[test]
    fn control_escapes() {
        assert_eq!(c(r"one\ntwo"), "one\ntwo");
        assert_eq!(c(r"a\tb\rc"), "a\tb\rc");
        assert_eq!(c(r"\b\f\v"), "\x08\x0c\x0b");
    }

    #[test]
    fn other_characters_lose_the_backslash() {
        assert_eq!(c(r"a\\b"), r"a\b");
        assert_eq!(c(r#"say \"hi\""#), r#"say "hi""#);
        assert_eq!(c(r"\q\é"), "qé");
        // The reason Windows paths need doubled backslashes in the body.
        assert_eq!(c(r"C:\new\temp"), "C:\new\temp");
        assert_eq!(c(r"C:\\new\\temp"), r"C:\new\temp");
    }

    #[test]
    fn octal_escapes() {
        assert_eq!(c(r"\101\102"), "AB");
        assert_eq!(c(r"\303\251t\303\251"), "été");
        // At most three digits; the rest is text.
        assert_eq!(c(r"\1011"), "A1");
        assert_eq!(c(r"\7x"), "\x07x");
        // \777 wraps to 0xFF, which is not valid UTF-8 on its own.
        assert_eq!(compress(r"\777"), None);
    }

    #[test]
    fn a_nul_ends_the_text() {
        assert_eq!(c(r"before\0after"), "before");
        assert_eq!(c(r"before\000after"), "before");
    }

    #[test]
    fn a_trailing_backslash_is_dropped() {
        assert_eq!(c("end\\"), "end");
        assert_eq!(c("\\"), "");
    }
}
