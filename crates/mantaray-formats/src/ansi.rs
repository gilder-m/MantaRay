//! Windows-1252, the single-byte text of ORTEC's files.
//!
//! MAESTRO and GammaVision are Windows "ANSI" programs: the descriptions in a
//! `.Chn` or `.Spc` and the whole of a `.Spe` are one byte per character in
//! the Windows code page, which on the lab's (Western) PCs is Windows-1252.
//! That is Latin-1 with printable characters in `0x80..=0x9F`, so `µ` is
//! `0xB5` and `°` is `0xB0`. MantaRay 0.2.4 and earlier stored UTF-8 there,
//! which MAESTRO shows as `Âµ` and `Â°`.
//!
//! On read, text that is valid UTF-8 is taken as UTF-8 (every pure-ASCII
//! field, and what 0.2.4 wrote); anything else is Windows-1252. Single-byte
//! text with a character above `0x7F` is almost never valid UTF-8 by chance.

/// `0x80..=0x9F` in Windows-1252; `None` where the code page has no character.
const HIGH: [Option<char>; 32] = [
    Some('\u{20ac}'),
    None,
    Some('\u{201a}'),
    Some('\u{0192}'),
    Some('\u{201e}'),
    Some('\u{2026}'),
    Some('\u{2020}'),
    Some('\u{2021}'),
    Some('\u{02c6}'),
    Some('\u{2030}'),
    Some('\u{0160}'),
    Some('\u{2039}'),
    Some('\u{0152}'),
    None,
    Some('\u{017d}'),
    None,
    None,
    Some('\u{2018}'),
    Some('\u{2019}'),
    Some('\u{201c}'),
    Some('\u{201d}'),
    Some('\u{2022}'),
    Some('\u{2013}'),
    Some('\u{2014}'),
    Some('\u{02dc}'),
    Some('\u{2122}'),
    Some('\u{0161}'),
    Some('\u{203a}'),
    Some('\u{0153}'),
    None,
    Some('\u{017e}'),
    Some('\u{0178}'),
];

/// What a character becomes when Windows-1252 has no byte for it.
pub(crate) const REPLACEMENT: u8 = b'?';

/// The Windows-1252 byte for `c`, if it has one.
fn byte_of(c: char) -> Option<u8> {
    let code = u32::from(c);
    if code < 0x80 || (0xa0..=0xff).contains(&code) {
        return u8::try_from(code).ok();
    }
    HIGH.iter()
        .position(|high| *high == Some(c))
        .and_then(|index| u8::try_from(0x80 + index).ok())
}

/// The character for a Windows-1252 byte; `?` for the five it leaves unused.
fn char_of(byte: u8) -> char {
    match byte {
        0x80..=0x9f => HIGH
            .get(usize::from(byte - 0x80))
            .copied()
            .flatten()
            .unwrap_or('?'),
        _ => char::from(byte),
    }
}

/// Text encoded for one of ORTEC's fields.
#[derive(Debug, Default)]
pub(crate) struct Encoded {
    /// The bytes to store.
    pub bytes: Vec<u8>,
    /// Characters stored as `?` because Windows-1252 has none for them.
    pub replaced: Vec<char>,
    /// True if the text did not fit and its end was left off.
    pub cut: bool,
}

/// Encodes a one-line field of at most `width` bytes.
///
/// Control characters are dropped, as they always were: a line break has no
/// place in a fixed field. Each other character is one byte, so the text
/// always stops at a whole character.
pub(crate) fn field(text: &str, width: usize) -> Encoded {
    let mut out = Encoded::default();
    for c in text.chars().filter(|c| *c >= ' ') {
        if out.bytes.len() == width {
            out.cut = true;
            break;
        }
        out.bytes.push(byte_of(c).unwrap_or_else(|| {
            out.replaced.push(c);
            REPLACEMENT
        }));
    }
    out
}

/// Encodes a whole text file, line breaks and all.
pub(crate) fn text(text: &str) -> Encoded {
    let mut out = Encoded::default();
    out.bytes.reserve(text.len());
    for c in text.chars() {
        out.bytes.push(byte_of(c).unwrap_or_else(|| {
            out.replaced.push(c);
            REPLACEMENT
        }));
    }
    out
}

/// Decodes stored text: UTF-8 if it is valid UTF-8, Windows-1252 otherwise.
pub(crate) fn decode(bytes: &[u8]) -> String {
    match std::str::from_utf8(bytes) {
        Ok(text) => text.to_string(),
        Err(_) => bytes.iter().copied().map(char_of).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_byte_round_trips_except_the_five_unused() {
        for byte in 0u8..=255 {
            let c = char_of(byte);
            if [0x81, 0x8d, 0x8f, 0x90, 0x9d].contains(&byte) {
                assert_eq!(c, '?');
            } else {
                assert_eq!(byte_of(c), Some(byte), "{byte:#x}");
            }
        }
    }

    #[test]
    fn lab_symbols_are_single_bytes() {
        let encoded = field("1 \u{b5}Ci, 22 \u{b0}C, \u{20ac}5", 63);
        assert_eq!(encoded.bytes, b"1 \xb5Ci, 22 \xb0C, \x805");
        assert!(encoded.replaced.is_empty() && !encoded.cut);
        assert_eq!(decode(&encoded.bytes), "1 \u{b5}Ci, 22 \u{b0}C, \u{20ac}5");
    }

    #[test]
    fn a_field_stops_at_its_width_and_says_so() {
        let encoded = field("ab\u{b5}d", 3);
        assert_eq!(encoded.bytes, b"ab\xb5");
        assert!(encoded.cut);
        assert!(!field("abc", 3).cut);
    }

    #[test]
    fn a_missing_character_is_a_question_mark_and_named() {
        let encoded = text("\u{3b2}-\u{3b3}\n");
        assert_eq!(encoded.bytes, b"?-?\n");
        assert_eq!(encoded.replaced, vec!['\u{3b2}', '\u{3b3}']);
    }

    #[test]
    fn utf_8_from_older_versions_still_decodes() {
        assert_eq!(decode("\u{b5}Ci".as_bytes()), "\u{b5}Ci");
    }
}
