//! Characters as EASy68K stores them: one byte each, in Windows-1252.
//!
//! A character is one byte ([ADR
//! 0004](../docs/adr/0004-characters-are-latin-1-bytes.md), amended on
//! 2026-10-05). The Assembler stores a source character as its byte, the text
//! tasks of `trap #15` decode the bytes they display, and the line or the key a
//! program reads is encoded before it reaches memory — all three with this one
//! table, so a byte means the same character wherever it is seen.
//!
//! Windows-1252 is the code page EASy68K runs in. It is Latin-1 with 27
//! printable characters where Latin-1 has the control codes `$80` to `$9F`:
//! `€ ‚ ƒ „ … † ‡ ˆ ‰ Š ‹ Œ Ž ‘ ’ “ ” • – — ˜ ™ š › œ ž Ÿ`. The five codes it
//! leaves undefined, `$81`, `$8D`, `$8F`, `$90` and `$9D`, decode to the control
//! character of the same number, as the WHATWG Encoding Standard has them and
//! so as a browser's `TextDecoder("windows-1252")` does. That keeps decoding
//! total — no byte stops a program that displays it — keeps a host that decodes
//! bytes itself in agreement with this table, and makes every byte encode back
//! to the byte it was.

/// The characters of the bytes `$80` to `$9F`, in order.
const HIGH_CHARACTERS: [char; 32] = [
    '\u{20AC}', '\u{0081}', '\u{201A}', '\u{0192}', '\u{201E}', '\u{2026}', '\u{2020}', '\u{2021}',
    '\u{02C6}', '\u{2030}', '\u{0160}', '\u{2039}', '\u{0152}', '\u{008D}', '\u{017D}', '\u{008F}',
    '\u{0090}', '\u{2018}', '\u{2019}', '\u{201C}', '\u{201D}', '\u{2022}', '\u{2013}', '\u{2014}',
    '\u{02DC}', '\u{2122}', '\u{0161}', '\u{203A}', '\u{0153}', '\u{009D}', '\u{017E}', '\u{0178}',
];

/// The character a byte stands for. Every byte stands for one.
pub fn character(byte: u8) -> char {
    match byte {
        0x80..=0x9F => HIGH_CHARACTERS[(byte - 0x80) as usize],
        _ => byte as char,
    }
}

/// The byte a character is stored as, or `None` when Windows-1252 has no byte
/// for it: a character above `ÿ` that is not one of the 27, or one of the
/// control codes `$80` to `$9F` those 27 took the place of.
pub fn byte(character: char) -> Option<u8> {
    match character as u32 {
        0x00..=0x7F | 0xA0..=0xFF => Some(character as u8),
        _ => HIGH_CHARACTERS
            .iter()
            .position(|&high| high == character)
            .map(|index| 0x80 + index as u8),
    }
}

/// The byte a character a user typed is stored as: its own, or `?` when
/// Windows-1252 has none, which is the character Windows substitutes when a
/// program in that code page is given one it cannot represent.
///
/// Typed text is never refused for what it holds: a phone keyboard can type
/// anything, and a program reading a line is better served by a `?` than by an
/// answer it never gets.
pub fn typed_byte(character: char) -> u8 {
    byte(character).unwrap_or(b'?')
}

/// The text a run of bytes stands for, one character per byte.
pub fn decode(bytes: &[u8]) -> String {
    bytes.iter().map(|&byte| character(byte)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_byte_decodes_and_encodes_back_to_itself() {
        for value in 0..=255u8 {
            assert_eq!(
                byte(character(value)),
                Some(value),
                "${value:02X} decodes to {:?}, which has to encode back",
                character(value)
            );
        }
    }

    #[test]
    fn latin_1_is_where_it_was() {
        assert_eq!(character(b'A'), 'A');
        assert_eq!(character(0xE9), 'é');
        assert_eq!(character(0xA0), '\u{A0}');
        assert_eq!(character(0xFF), 'ÿ');
        assert_eq!(byte('é'), Some(0xE9));
        assert_eq!(byte('\0'), Some(0));
    }

    #[test]
    fn the_twenty_seven_characters_take_the_place_of_the_c1_controls() {
        let written = "€‚ƒ„…†‡ˆ‰Š‹ŒŽ‘’“”•–—˜™š›œžŸ";
        assert_eq!(written.chars().count(), 27);
        for character in written.chars() {
            let stored = byte(character).expect("one of the 27 has a byte");
            assert!(
                (0x80..=0x9F).contains(&stored),
                "{character} is stored in $80 to $9F"
            );
        }
        assert_eq!(byte('€'), Some(0x80));
        assert_eq!(byte('’'), Some(0x92));
        assert_eq!(byte('—'), Some(0x97));
        assert_eq!(byte('Ÿ'), Some(0x9F));
        assert_eq!(character(0x80), '€');
    }

    #[test]
    fn the_five_undefined_codes_are_their_own_control_characters() {
        // As the WHATWG index has them, which is what a browser decodes.
        for value in [0x81u8, 0x8D, 0x8F, 0x90, 0x9D] {
            assert_eq!(character(value), char::from(value));
            assert_eq!(byte(char::from(value)), Some(value));
        }
    }

    #[test]
    fn a_character_windows_1252_cannot_encode_has_no_byte() {
        assert_eq!(byte('→'), None, "an arrow");
        assert_eq!(byte('😀'), None, "a character outside the basic plane");
        assert_eq!(byte('Ā'), None, "a Latin letter Windows-1252 does not have");
        // the control codes the 27 replaced are gone
        assert_eq!(byte('\u{80}'), None);
        assert_eq!(byte('\u{92}'), None);
        assert_eq!(byte('\u{9F}'), None);
    }

    #[test]
    fn a_typed_character_without_a_byte_is_a_question_mark() {
        assert_eq!(typed_byte('€'), 0x80);
        assert_eq!(typed_byte('a'), b'a');
        assert_eq!(typed_byte('→'), b'?');
        assert_eq!(typed_byte('😀'), b'?');
    }

    #[test]
    fn decoding_is_one_character_per_byte() {
        assert_eq!(decode(b"caf\xE9 \x80 \x93ok\x94"), "café € “ok”");
        assert_eq!(decode(&[]), "");
    }
}
