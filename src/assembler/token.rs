//! The tokens of one Source line.
//!
//! The kinds are the list of `docs/grammar.md` 1.14, and each token carries the
//! [`Span`] it covers plus the two facts the line grammar turns on: whether it
//! starts in column 1 (`label_rule`, 1.4) and whether whitespace came before it
//! (`operand_field_extent`, 1.5). Nothing here decides what a token *means*: a
//! register is an [`Identifier`](TokenKind::Identifier) like any other name, and
//! a `.b` is a [`SizeSuffix`](TokenKind::SizeSuffix) whether or not `b` is a
//! size, because both readings belong to the parser.

use serde::Serialize;

use super::source::Span;

/// The base a [`Number`](TokenKind::Number) token was written in.
///
/// The prefixes are EASy68K's (`Directives/operators.htm`): `$` hexadecimal,
/// `%` binary, `@` octal, and no prefix at all for decimal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NumberBase {
    /// `1234`, written with no prefix.
    Decimal,
    /// `$ff`, written with a `$`.
    Hexadecimal,
    /// `%1010`, written with a `%`.
    Binary,
    /// `@17`, written with an `@`.
    Octal,
}

impl NumberBase {
    /// The radix the digits are read in.
    pub fn radix(&self) -> u32 {
        match self {
            NumberBase::Decimal => 10,
            NumberBase::Hexadecimal => 16,
            NumberBase::Binary => 2,
            NumberBase::Octal => 8,
        }
    }

    /// The name a diagnostic calls the base ("hexadecimal").
    pub fn name(&self) -> &'static str {
        match self {
            NumberBase::Decimal => "decimal",
            NumberBase::Hexadecimal => "hexadecimal",
            NumberBase::Binary => "binary",
            NumberBase::Octal => "octal",
        }
    }

    /// The digits of the base, as a diagnostic's hint spells them out.
    pub fn digits(&self) -> &'static str {
        match self {
            NumberBase::Decimal => "0-9",
            NumberBase::Hexadecimal => "0-9 and a-f",
            NumberBase::Binary => "0 and 1",
            NumberBase::Octal => "0-7",
        }
    }

    /// The prefix the base is written with, empty for decimal.
    pub fn prefix(&self) -> &'static str {
        match self {
            NumberBase::Decimal => "",
            NumberBase::Hexadecimal => "$",
            NumberBase::Binary => "%",
            NumberBase::Octal => "@",
        }
    }

    /// The base a prefix character introduces, if it introduces one.
    pub fn from_prefix(character: char) -> Option<NumberBase> {
        match character {
            '$' => Some(NumberBase::Hexadecimal),
            '%' => Some(NumberBase::Binary),
            '@' => Some(NumberBase::Octal),
            _ => None,
        }
    }

    /// Whether `character` is a digit of this base, case insensitively.
    pub fn is_digit(&self, character: char) -> bool {
        character.is_digit(self.radix())
    }
}

/// Which quote a string or character literal was written with.
///
/// Both are accepted; the single quote is EASy68K's and the double quote raises
/// the `double_quoted_string` suggestion once per File (`docs/grammar.md` 1.9),
/// which is the Assembler's to raise and not the tokenizer's, because "once per
/// File" is not something one line can know.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum QuoteKind {
    /// `'Hello'`, EASy68K's own.
    Single,
    /// `"Hello"`, accepted as the lenient direction of ADR 0001.
    Double,
}

impl QuoteKind {
    /// The quote character itself.
    pub fn character(&self) -> char {
        match self {
            QuoteKind::Single => '\'',
            QuoteKind::Double => '"',
        }
    }
}

/// What a token is.
///
/// One kind per entry of `docs/grammar.md` 1.14. Registers are not a kind of
/// their own: `d0` is an [`Identifier`](TokenKind::Identifier), and
/// [`ast::Register::parse`](super::ast::Register::parse) is what recognises one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TokenKind {
    /// A Global label, Symbol or operation name: `move`, `count`, `_start`.
    Identifier,
    /// A Local label: a `.` where a name may begin, then the name (`.loop`).
    LocalIdentifier,
    /// A number, with the base its prefix asked for. A token whose digits do
    /// not all belong to the base is still a `Number`, and `invalid_number` has
    /// already been raised for it.
    Number(NumberBase),
    /// A quoted literal, read as a character literal or as a string by the
    /// place it stands in (`docs/grammar.md` 1.9). A literal that reaches the
    /// end of the line is still a `StringLiteral`, and `unterminated_string`
    /// has already been raised for it.
    StringLiteral(QuoteKind),
    /// A `.` in suffix position and the whole run of name characters after it
    /// (`docs/grammar.md` 1.11): `.b`, but also `.q` and `.length`, which the
    /// parser answers with `unknown_size_suffix` or `dot_in_name`.
    SizeSuffix,
    /// `#`, which opens an Immediate.
    Hash,
    /// `,`, which separates Operands and register list items.
    Comma,
    /// `(`.
    LeftParen,
    /// `)`.
    RightParen,
    /// `:`, which makes the identifier before it a Label.
    Colon,
    /// `+`: postincrement, or addition.
    Plus,
    /// `-`: predecrement, subtraction, unary minus, or a register range.
    Minus,
    /// `*`: the current address, or multiplication (`docs/grammar.md` 1.6).
    Star,
    /// `/`: division, or a register list separator.
    Slash,
    /// `\`: modulus.
    Backslash,
    /// `&`: logical AND.
    Ampersand,
    /// `!`: logical OR, EASy68K's other spelling of `|`.
    Bang,
    /// `|`: logical OR.
    Pipe,
    /// `^`: exclusive OR.
    Caret,
    /// `~`: one's complement.
    Tilde,
    /// `<<`: shift left.
    ShiftLeft,
    /// `>>`: shift right.
    ShiftRight,
    /// A run of spaces, tabs, form feeds and vertical tabs. The parser needs
    /// them: whitespace is what ends the Operand field (`docs/grammar.md` 1.5).
    Whitespace,
    /// A Comment, from its `;` or `*` marker to the end of the line. Its text
    /// is never tokenized (`docs/grammar.md` 1.6).
    Comment,
    /// The end of the line. A tokenizer that has run out returns this for ever.
    EndOfLine,
    /// A character that starts no token. A diagnostic has already been raised
    /// for it, and the tokenizer carries on with the next character.
    Error,
}

/// One token of one Source line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Token {
    /// What the token is.
    pub kind: TokenKind,
    /// The bytes it covers, inside its line.
    pub span: Span,
    /// Whether it starts at the first character of the line — "column 1" in the
    /// grammar's 1-based prose, column 0 in a [`Location`](super::source::Location).
    /// This is what makes `label_rule` (1.4) decidable.
    pub column_one: bool,
    /// Whether a run of whitespace comes immediately before it. This is what
    /// makes `operand_field_extent` (1.5) and the `.` of `size_suffix` (1.7)
    /// decidable.
    pub preceded_by_whitespace: bool,
}

impl Token {
    /// A token of `kind` covering `span`, with both flags cleared. The
    /// tokenizer sets the flags; this is for tests and for tokens a parser
    /// synthesises.
    pub fn new(kind: TokenKind, span: Span) -> Self {
        Self {
            kind,
            span,
            column_one: false,
            preceded_by_whitespace: false,
        }
    }

    /// Whether this is [`TokenKind::Whitespace`].
    pub fn is_whitespace(&self) -> bool {
        self.kind == TokenKind::Whitespace
    }

    /// Whether this is [`TokenKind::EndOfLine`].
    pub fn is_end_of_line(&self) -> bool {
        self.kind == TokenKind::EndOfLine
    }

    /// The text the token covers in its line.
    pub fn text<'a>(&self, line: &'a str) -> &'a str {
        self.span.text(line)
    }
}

/// Whether `character` is whitespace for the grammar: space, tab, form feed or
/// vertical tab (`docs/grammar.md` 1.1). A no-break space is deliberately not
/// one of them.
pub fn is_whitespace(character: char) -> bool {
    matches!(character, ' ' | '\t' | '\u{0C}' | '\u{0B}')
}

/// Whether `character` may start a name: a letter or `_` (`docs/grammar.md` 1.7).
pub fn is_name_start(character: char) -> bool {
    character.is_ascii_alphabetic() || character == '_'
}

/// Whether `character` may continue a name, a number's digit run or a size
/// suffix's run: a letter, a digit or `_`.
pub fn is_name_continuation(character: char) -> bool {
    character.is_ascii_alphanumeric() || character == '_'
}

/// The value of a number's digits in `base`, or `None` when a digit does not
/// belong to the base or the value does not fit in 64 bits.
///
/// Values are computed in `i64` and range-checked against the Operand's size
/// later (`docs/grammar.md` 1.8), so this is the one place a written number
/// becomes a value.
pub fn parse_digits(base: NumberBase, digits: &str) -> Option<i64> {
    if digits.is_empty() {
        return None;
    }
    let mut value: i64 = 0;
    for character in digits.chars() {
        let digit = character.to_digit(base.radix())?;
        value = value.checked_mul(base.radix() as i64)?;
        value = value.checked_add(digit as i64)?;
    }
    Some(value)
}

/// The Latin-1 bytes a quoted literal stands for, `''` and `""` read as one
/// quote ([ADR 0004](../../../docs/adr/0004-characters-are-latin-1-bytes.md),
/// `docs/grammar.md` 1.9).
///
/// `text` is the token's own text, opening quote included; a closing quote is
/// optional, so an unterminated literal still gives the bytes it holds and the
/// parser can carry on after the diagnostic. A character above 255 has no byte
/// and is dropped — `character_above_latin1` has already been raised for it.
pub fn string_literal_bytes(text: &str) -> Vec<u8> {
    let mut characters = text.chars();
    let quote = match characters.next() {
        Some(quote) => quote,
        None => return Vec::new(),
    };
    let mut bytes = Vec::new();
    while let Some(character) = characters.next() {
        if character == quote {
            // A doubled quote is one quote; a single one closes the literal.
            match characters.clone().next() {
                Some(next) if next == quote => {
                    characters.next();
                }
                _ => break,
            }
        }
        if let Ok(byte) = u8::try_from(character as u32) {
            bytes.push(byte);
        }
    }
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn number_bases_know_their_digits() {
        assert_eq!(NumberBase::from_prefix('$'), Some(NumberBase::Hexadecimal));
        assert_eq!(NumberBase::from_prefix('%'), Some(NumberBase::Binary));
        assert_eq!(NumberBase::from_prefix('@'), Some(NumberBase::Octal));
        assert_eq!(NumberBase::from_prefix('0'), None);
        assert!(NumberBase::Hexadecimal.is_digit('F'));
        assert!(!NumberBase::Decimal.is_digit('f'));
        assert!(NumberBase::Binary.is_digit('1'));
        assert!(!NumberBase::Binary.is_digit('2'));
        assert!(NumberBase::Octal.is_digit('7'));
        assert!(!NumberBase::Octal.is_digit('8'));
    }

    #[test]
    fn parse_digits_reads_every_base() {
        assert_eq!(parse_digits(NumberBase::Hexadecimal, "ff"), Some(255));
        assert_eq!(parse_digits(NumberBase::Hexadecimal, "FF"), Some(255));
        assert_eq!(parse_digits(NumberBase::Binary, "1010"), Some(10));
        assert_eq!(parse_digits(NumberBase::Octal, "17"), Some(15));
        assert_eq!(parse_digits(NumberBase::Decimal, "010"), Some(10));
    }

    #[test]
    fn parse_digits_refuses_what_does_not_fit_or_does_not_belong() {
        assert_eq!(parse_digits(NumberBase::Hexadecimal, ""), None);
        assert_eq!(parse_digits(NumberBase::Hexadecimal, "1g"), None);
        assert_eq!(
            parse_digits(NumberBase::Decimal, "9223372036854775808"),
            None
        );
        assert_eq!(
            parse_digits(NumberBase::Hexadecimal, "7fffffffffffffff"),
            Some(i64::MAX)
        );
    }

    #[test]
    fn string_literal_bytes_unescape_a_doubled_quote() {
        assert_eq!(string_literal_bytes("'A'"), vec![b'A']);
        assert_eq!(string_literal_bytes("'ab'"), vec![b'a', b'b']);
        assert_eq!(string_literal_bytes("''''"), vec![b'\'']);
        assert_eq!(string_literal_bytes("'it''s'"), b"it's".to_vec());
        assert_eq!(string_literal_bytes("\"a\"\"b\""), b"a\"b".to_vec());
    }

    #[test]
    fn string_literal_bytes_are_latin_1() {
        assert_eq!(
            string_literal_bytes("'città'"),
            vec![b'c', b'i', b't', b't', 0xE0]
        );
    }

    #[test]
    fn string_literal_bytes_survive_an_unterminated_literal() {
        assert_eq!(string_literal_bytes("'abc"), b"abc".to_vec());
        assert_eq!(string_literal_bytes("''"), Vec::<u8>::new());
        assert_eq!(string_literal_bytes(""), Vec::<u8>::new());
    }
}
