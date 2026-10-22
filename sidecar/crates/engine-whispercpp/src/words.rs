//! Turning decoder tokens into timed pieces of text.

/// Text with whisper.cpp timestamps, in centiseconds of the decoded buffer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Piece {
    pub text: String,
    pub t0: i64,
    pub t1: i64,
}

/// Merges consecutive tokens until their bytes form complete UTF-8, so a
/// character split across tokens (common in CJK text and emoji) is kept whole
/// instead of becoming U+FFFD. Invalid bytes, and bytes still incomplete at
/// the end, are decoded lossily.
pub(crate) fn merge_tokens<'a>(
    tokens: impl IntoIterator<Item = (&'a [u8], i64, i64)>,
) -> Vec<Piece> {
    let mut pieces = Vec::new();
    // Bytes of an incomplete character, with the first token's t0 and the last token's t1.
    let mut pending: Option<(Vec<u8>, i64, i64)> = None;

    for (bytes, t0, t1) in tokens {
        let (mut buf, start) = pending
            .take()
            .map_or_else(|| (Vec::new(), t0), |(b, s, _)| (b, s));
        buf.extend_from_slice(bytes);
        match std::str::from_utf8(&buf) {
            Ok(text) => pieces.push(Piece {
                text: text.to_string(),
                t0: start,
                t1,
            }),
            // An incomplete character at the end: wait for the next token.
            Err(e) if e.error_len().is_none() => pending = Some((buf, start, t1)),
            Err(_) => pieces.push(Piece {
                text: String::from_utf8_lossy(&buf).into_owned(),
                t0: start,
                t1,
            }),
        }
    }

    if let Some((buf, t0, t1)) = pending {
        pieces.push(Piece {
            text: String::from_utf8_lossy(&buf).into_owned(),
            t0,
            t1,
        });
    }
    pieces
}

#[cfg(test)]
mod tests {
    use super::*;

    fn piece(text: &str, t0: i64, t1: i64) -> Piece {
        Piece {
            text: text.to_string(),
            t0,
            t1,
        }
    }

    #[test]
    fn ascii_tokens_pass_through() {
        let tokens: [(&[u8], i64, i64); 2] = [(b" Hello", 0, 50), (b".", 50, 60)];
        assert_eq!(
            merge_tokens(tokens),
            vec![piece(" Hello", 0, 50), piece(".", 50, 60)]
        );
    }

    #[test]
    fn character_split_across_tokens_is_merged() {
        // "日" is E6 97 A5; whisper's byte-level BPE can split it.
        let tokens: [(&[u8], i64, i64); 3] = [
            (b"\xE6\x97", 10, 20),
            (b"\xA5", 20, 30),
            (b"\xE6\x9C\xAC", 30, 40),
        ];
        assert_eq!(
            merge_tokens(tokens),
            vec![piece("日", 10, 30), piece("本", 30, 40)]
        );
    }

    #[test]
    fn invalid_bytes_do_not_swallow_later_tokens() {
        let tokens: [(&[u8], i64, i64); 2] = [(b"\xFFa", 0, 10), (b" b", 10, 20)];
        assert_eq!(
            merge_tokens(tokens),
            vec![piece("\u{FFFD}a", 0, 10), piece(" b", 10, 20)]
        );
    }

    #[test]
    fn incomplete_character_at_the_end_is_decoded_lossily() {
        let tokens: [(&[u8], i64, i64); 2] = [(b" a", 0, 10), (b"\xE6\x97", 10, 20)];
        assert_eq!(
            merge_tokens(tokens),
            vec![piece(" a", 0, 10), piece("\u{FFFD}", 10, 20)]
        );
    }

    #[test]
    fn no_tokens_gives_no_pieces() {
        assert!(merge_tokens(std::iter::empty()).is_empty());
    }
}
