//! The C runtime calls EASy68K's text tasks are written with, which is where
//! `trap #15`'s rules for numbers come from.
//!
//! EASy68K displays a number with `itoa`, `ultoa` and `sprintf`, and reads one
//! with `atoi` (`CODE9.CPP` and `simIOu.cpp` of Sim68K 5.16.1). The Interpreter
//! follows them here, so that a number reaches the host as the text to display
//! and a typed number reaches the Interpreter as the line that was typed: the
//! host neither formats nor parses.

/// Task 3, and the number of task 17: `itoa(D1.L, buf, 10)`, the long as a
/// signed decimal number.
pub fn decimal(value: u32) -> String {
    (value as i32).to_string()
}

/// Task 15: `ultoa(D1.L, buf, base)` and then `UpperCase()`, the long as an
/// unsigned number in a base of 2 to 36, with the digits `0` to `9` and then
/// `A` to `Z`.
///
/// `None` for a base outside 2 to 36, where EASy68K displays nothing.
pub fn in_base(value: u32, base: u32) -> Option<String> {
    const DIGITS: &[u8; 36] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ";
    if !(2..=36).contains(&base) {
        return None;
    }
    let mut digits = Vec::new();
    let mut rest = value;
    loop {
        digits.push(DIGITS[(rest % base) as usize] as char);
        rest /= base;
        if rest == 0 {
            break;
        }
    }
    Some(digits.iter().rev().collect())
}

/// Task 20: `sprintf(buf, "%*d", (char)D2.B, D1.L)`, the long as a signed
/// decimal number in a field of D2.B columns.
///
/// The width is a signed byte, as `(char)` makes it: a positive width pads with
/// spaces on the left, a negative one pads on the right — C reads a negative
/// `*` width as the `-` flag — and a number longer than its field is never cut.
pub fn in_field(value: u32, width: u8) -> String {
    let width = width as i8 as i32;
    let number = decimal(value);
    let columns = width.unsigned_abs() as usize;
    if width < 0 {
        format!("{number:<columns$}")
    } else {
        format!("{number:>columns$}")
    }
}

/// Tasks 4 and 18: `atoi` over the line that was typed, in Windows-1252 bytes.
///
/// It skips leading white space (space, tab, line feed, vertical tab, form feed
/// and carriage return, C's `isspace`), takes one optional `+` or `-`, and reads
/// decimal digits up to the first byte that is not one. Nothing is an error: a
/// line with no digits where they are expected is 0, so `"12abc"` is 12 and
/// `"abc"` and `""` are 0.
///
/// A number too long for 32 bits **wraps**: the digits are accumulated modulo
/// 2^32, as the accumulating loop of a C runtime's `atoi` does in a 32-bit
/// `int`, and then negated for a `-`. Borland's own `atoi` was not available to
/// check; this choice keeps every number from −2147483648 to 4294967295 exact
/// in D1.L, so `4294967295` is `$FFFFFFFF` and reads back as itself to a program
/// that treats the long as unsigned, which saturating at 2147483647 would not.
pub fn atoi(bytes: &[u8]) -> i32 {
    let mut rest = bytes
        .iter()
        .copied()
        .skip_while(|byte| matches!(byte, b' ' | b'\t' | b'\n' | 0x0B | 0x0C | b'\r'))
        .peekable();
    let negative = match rest.peek() {
        Some(b'-') => {
            rest.next();
            true
        }
        Some(b'+') => {
            rest.next();
            false
        }
        _ => false,
    };
    let mut value: u32 = 0;
    for byte in rest {
        if !byte.is_ascii_digit() {
            break;
        }
        value = value.wrapping_mul(10).wrapping_add(u32::from(byte - b'0'));
    }
    if negative {
        value.wrapping_neg() as i32
    } else {
        value as i32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decimal_is_signed() {
        assert_eq!(decimal(42), "42");
        assert_eq!(decimal(0), "0");
        assert_eq!(decimal(0xFFFF_FFFB), "-5");
        assert_eq!(decimal(0x8000_0000), "-2147483648");
        assert_eq!(decimal(0x7FFF_FFFF), "2147483647");
    }

    #[test]
    fn in_base_is_unsigned_with_uppercase_digits() {
        assert_eq!(in_base(255, 16).as_deref(), Some("FF"));
        assert_eq!(in_base(0xDEAD_BEEF, 16).as_deref(), Some("DEADBEEF"));
        assert_eq!(in_base(0xFFFF_FFFF, 16).as_deref(), Some("FFFFFFFF"));
        assert_eq!(in_base(5, 2).as_deref(), Some("101"));
        assert_eq!(in_base(35, 36).as_deref(), Some("Z"));
        assert_eq!(in_base(36, 36).as_deref(), Some("10"));
        assert_eq!(in_base(0, 16).as_deref(), Some("0"));
        assert_eq!(in_base(0xFFFF_FFFF, 10).as_deref(), Some("4294967295"));
    }

    #[test]
    fn in_base_has_nothing_for_a_base_outside_two_to_thirty_six() {
        assert_eq!(in_base(10, 0), None);
        assert_eq!(in_base(10, 1), None);
        assert_eq!(in_base(10, 37), None);
    }

    #[test]
    fn in_field_right_justifies_and_a_negative_width_left_justifies() {
        assert_eq!(in_field(0xFFFF_FFFB, 6), "    -5");
        assert_eq!(in_field(42, 0), "42");
        assert_eq!(in_field(12345, 3), "12345", "a long number is never cut");
        // -6 as a byte is $FA: the `-` flag, six columns
        assert_eq!(in_field(0xFFFF_FFFB, 0xFA), "-5    ");
        assert_eq!(in_field(7, 0xFF), "7", "-1 is one column, which 7 fills");
        assert_eq!(in_field(1, 0x80).len(), 128, "-128 is 128 columns");
        assert_eq!(in_field(1, 0x7F).len(), 127);
    }

    #[test]
    fn atoi_reads_the_digits_it_can_and_never_fails() {
        assert_eq!(atoi(b"42"), 42);
        assert_eq!(atoi(b"12abc"), 12);
        assert_eq!(atoi(b"abc"), 0);
        assert_eq!(atoi(b""), 0);
        assert_eq!(atoi(b"  \t-42"), -42);
        assert_eq!(atoi(b"+7"), 7);
        assert_eq!(atoi(b"- 5"), 0, "the sign has to touch the digits");
        assert_eq!(atoi(b"--5"), 0);
        assert_eq!(atoi(b"1.5"), 1);
        assert_eq!(atoi(b"1e3"), 1);
        assert_eq!(atoi(b"0x1F"), 0);
        assert_eq!(atoi(b"007"), 7);
        assert_eq!(atoi(b"\xA012"), 0, "a no-break space is not white space");
    }

    #[test]
    fn atoi_wraps_a_number_too_long_for_32_bits() {
        assert_eq!(atoi(b"2147483647"), i32::MAX);
        assert_eq!(atoi(b"-2147483648"), i32::MIN);
        assert_eq!(atoi(b"2147483648"), i32::MIN);
        assert_eq!(
            atoi(b"4294967295"),
            -1,
            "$FFFFFFFF, the unsigned reading of the long"
        );
        assert_eq!(atoi(b"4294967296"), 0);
        assert_eq!(atoi(b"4294967297"), 1);
        assert_eq!(atoi(b"-4294967295"), 1);
    }
}
