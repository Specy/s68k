//! The tokenizer of one Source line.
//!
//! It walks a line and answers with [`Token`]s. It never fails: a character
//! that starts no token becomes a [`TokenKind::Error`] token with a Diagnostic
//! beside it, and the walk carries on, so one bad character costs one message
//! and not a line.
//!
//! What it decides, and what it leaves to the parser, follows
//! `docs/grammar.md`:
//!
//! * it decides the disposition of a `.` (1.7), because that is positional: a
//!   `.` directly after a name, a number, a `)` or a quoted literal opens a
//!   [`SizeSuffix`](TokenKind::SizeSuffix) and anywhere else it opens a
//!   [`LocalIdentifier`](TokenKind::LocalIdentifier);
//! * it decides that a `;` starts a Comment, everywhere, and that a `*` starts
//!   one when it is the first non-blank character of the line (1.6). A `*`
//!   anywhere else is a [`Star`](TokenKind::Star) and the parser says whether
//!   it is the current address, multiplication, or the marker of a comment
//!   field;
//! * it raises the lexical Diagnostics — `character_above_latin1`,
//!   `non_breaking_space`, `unexpected_character`, `unterminated_string` and
//!   `invalid_number` — and no others. `unknown_size_suffix` and `dot_in_name`
//!   depend on the field the token stands in, `double_quoted_string` and
//!   `bare_comment` on what the rest of the File has already shown, and all
//!   four are raised further up.
//!
//! The Comment field is never tokenized (1.6), so the parser drives a
//! [`Tokenizer`] and stops it where the Operand field ends;
//! [`Tokenizer::mark`] and [`Tokenizer::rewind`] make the lookahead that rule
//! needs free of side effects. [`tokenize_line`] tokenizes a whole line in one
//! call and is what the tests use.

use super::diagnostics::{Diagnostic, DiagnosticKind};
use super::source::{Location, Span};
use super::token::{
    is_name_continuation, is_name_start, is_whitespace, NumberBase, QuoteKind, Token, TokenKind,
};

/// A place in a [`Tokenizer`]'s walk, to come back to.
///
/// Rewinding to a mark undoes the Diagnostics raised since it was taken as
/// well as the position, which is what makes it safe to look ahead into text
/// that may turn out to be a Comment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TokenizerMark {
    offset: usize,
    diagnostics: usize,
    previous: Option<Token>,
    seen_token: bool,
}

/// A tokenizer walking one Source line.
///
/// It holds the File and the line number only to build the [`Location`] of its
/// Diagnostics; it never looks at another line.
#[derive(Debug, Clone)]
pub struct Tokenizer<'a> {
    text: &'a str,
    file: String,
    line: usize,
    offset: usize,
    previous: Option<Token>,
    seen_token: bool,
    diagnostics: Vec<Diagnostic>,
}

impl<'a> Tokenizer<'a> {
    /// A tokenizer over `text`, the `line`th line of `file`.
    pub fn new(text: &'a str, file: &str, line: usize) -> Self {
        Self {
            text,
            file: file.to_string(),
            line,
            offset: 0,
            previous: None,
            seen_token: false,
            diagnostics: Vec::new(),
        }
    }

    /// The line being walked.
    pub fn text(&self) -> &'a str {
        self.text
    }

    /// The File the line is in.
    pub fn file(&self) -> &str {
        &self.file
    }

    /// The 0-based index of the line.
    pub fn line(&self) -> usize {
        self.line
    }

    /// How far the walk has got, as a byte offset into the line.
    pub fn offset(&self) -> usize {
        self.offset
    }

    /// Everything not yet tokenized, which is what a Comment field, a
    /// `message_text` or a `file_specification` is read from.
    pub fn rest(&self) -> Span {
        Span::new(self.offset, self.text.len())
    }

    /// The text a token covers.
    pub fn text_of(&self, token: &Token) -> &'a str {
        token.text(self.text)
    }

    /// The Location of `span` on this line.
    pub fn location(&self, span: Span) -> Location {
        Location::from_span(&self.file, self.line, self.text, span)
    }

    /// The Diagnostics raised so far.
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    /// Take the Diagnostics raised so far, leaving none behind.
    pub fn take_diagnostics(&mut self) -> Vec<Diagnostic> {
        std::mem::take(&mut self.diagnostics)
    }

    /// Remember where the walk is, to [`rewind`](Tokenizer::rewind) to later.
    pub fn mark(&self) -> TokenizerMark {
        TokenizerMark {
            offset: self.offset,
            diagnostics: self.diagnostics.len(),
            previous: self.previous,
            seen_token: self.seen_token,
        }
    }

    /// Go back to a mark, dropping the Diagnostics raised since it was taken.
    pub fn rewind(&mut self, mark: TokenizerMark) {
        self.offset = mark.offset;
        self.diagnostics.truncate(mark.diagnostics);
        self.previous = mark.previous;
        self.seen_token = mark.seen_token;
    }

    /// The next token. Once the line runs out this is
    /// [`TokenKind::EndOfLine`], for ever.
    pub fn next_token(&mut self) -> Token {
        let start = self.offset;
        let column_one = start == 0;
        let preceded_by_whitespace = self.previous.is_some_and(|token| token.is_whitespace());
        let character = match self.peek() {
            Some(character) => character,
            None => {
                return Token {
                    kind: TokenKind::EndOfLine,
                    span: Span::empty(start),
                    column_one,
                    preceded_by_whitespace,
                }
            }
        };

        let kind = if is_whitespace(character) {
            self.take_while(is_whitespace);
            TokenKind::Whitespace
        } else if character == ';' || (character == '*' && !self.seen_token) {
            // `;` starts a Comment everywhere; `*` only as the first non-blank
            // character of the line, which is `comment_line` (1.6).
            self.offset = self.text.len();
            TokenKind::Comment
        } else if is_name_start(character) {
            self.take_while(is_name_continuation);
            TokenKind::Identifier
        } else if character.is_ascii_digit() {
            let digits_start = self.offset;
            self.take_while(is_name_continuation);
            self.check_digits(NumberBase::Decimal, start, digits_start);
            TokenKind::Number(NumberBase::Decimal)
        } else if let Some(base) = NumberBase::from_prefix(character) {
            self.advance();
            let digits_start = self.offset;
            self.take_while(is_name_continuation);
            self.check_digits(base, start, digits_start);
            TokenKind::Number(base)
        } else if character == '.' {
            self.dot(start)
        } else if character == '\'' {
            self.string_literal(start, QuoteKind::Single)
        } else if character == '"' {
            self.string_literal(start, QuoteKind::Double)
        } else if character == '<' || character == '>' {
            self.advance();
            if self.peek() == Some(character) {
                self.advance();
                if character == '<' {
                    TokenKind::ShiftLeft
                } else {
                    TokenKind::ShiftRight
                }
            } else {
                self.raise(
                    DiagnosticKind::UnexpectedCharacter { character },
                    Span::new(start, self.offset),
                );
                TokenKind::Error
            }
        } else if let Some(kind) = single_character_token(character) {
            self.advance();
            kind
        } else {
            self.advance();
            let span = Span::new(start, self.offset);
            let kind = if character == '\u{A0}' {
                DiagnosticKind::NonBreakingSpace
            } else if character as u32 > 255 {
                DiagnosticKind::CharacterAboveLatin1 { character }
            } else {
                DiagnosticKind::UnexpectedCharacter { character }
            };
            self.raise(kind, span);
            TokenKind::Error
        };

        let token = Token {
            kind,
            span: Span::new(start, self.offset),
            column_one,
            preceded_by_whitespace,
        };
        self.previous = Some(token);
        if kind != TokenKind::Whitespace {
            self.seen_token = true;
        }
        token
    }

    /// The character at the walk's position.
    fn peek(&self) -> Option<char> {
        self.text[self.offset..].chars().next()
    }

    /// Step over one character.
    fn advance(&mut self) -> Option<char> {
        let character = self.peek()?;
        self.offset += character.len_utf8();
        Some(character)
    }

    /// Step over every character `accept` takes.
    fn take_while(&mut self, accept: impl Fn(char) -> bool) {
        while let Some(character) = self.peek() {
            if !accept(character) {
                break;
            }
            self.offset += character.len_utf8();
        }
    }

    /// Raise a Diagnostic about `span` of this line.
    fn raise(&mut self, kind: DiagnosticKind, span: Span) {
        let location = self.location(span);
        self.diagnostics.push(Diagnostic::new(kind, location));
    }

    /// Report the digits of a number that do not belong to its base, or a base
    /// prefix with no digits at all (`docs/grammar.md` 1.8). The token is a
    /// [`Number`](TokenKind::Number) either way: "longest run, then diagnose".
    fn check_digits(&mut self, base: NumberBase, start: usize, digits_start: usize) {
        let digits = &self.text[digits_start..self.offset];
        if digits.is_empty() {
            self.raise(
                DiagnosticKind::InvalidNumber { base, digit: None },
                Span::new(start, self.offset),
            );
            return;
        }
        if let Some((index, character)) = digits
            .char_indices()
            .find(|(_, character)| !base.is_digit(*character))
        {
            let at = digits_start + index;
            self.raise(
                DiagnosticKind::InvalidNumber {
                    base,
                    digit: Some(character),
                },
                Span::new(at, at + character.len_utf8()),
            );
        }
    }

    /// A `.`: a size suffix when it sits directly after a name, a number, a `)`
    /// or a quoted literal, and a Local label anywhere else (`docs/grammar.md`
    /// 1.7). Both take the whole run of name characters after the dot, so a bad
    /// one is a single token and never a cascade.
    fn dot(&mut self, start: usize) -> TokenKind {
        let suffix_position = match self.previous {
            Some(previous) => {
                previous.span.end == self.offset
                    && matches!(
                        previous.kind,
                        TokenKind::Identifier
                            | TokenKind::LocalIdentifier
                            | TokenKind::Number(_)
                            | TokenKind::RightParen
                            | TokenKind::StringLiteral(_)
                    )
            }
            None => false,
        };
        self.advance();
        let run_start = self.offset;
        self.take_while(is_name_continuation);
        if suffix_position {
            TokenKind::SizeSuffix
        } else if self.offset > run_start {
            TokenKind::LocalIdentifier
        } else {
            self.raise(
                DiagnosticKind::UnexpectedCharacter { character: '.' },
                Span::new(start, self.offset),
            );
            TokenKind::Error
        }
    }

    /// A quoted literal, `''` or `""` reading as one quote.
    ///
    /// Inside a literal every Latin-1 character is ordinary — a no-break space
    /// and a control character included, because both are bytes a `dc.b` can
    /// store — and only a character above 255, which has no byte at all, is
    /// refused (see the implementation notes, phase 1 step 3).
    fn string_literal(&mut self, start: usize, quote: QuoteKind) -> TokenKind {
        let quote_character = quote.character();
        self.advance();
        let mut closed = false;
        while let Some(character) = self.peek() {
            if character == quote_character {
                self.advance();
                if self.peek() == Some(quote_character) {
                    self.advance();
                    continue;
                }
                closed = true;
                break;
            }
            let at = self.offset;
            self.advance();
            if character as u32 > 255 {
                self.raise(
                    DiagnosticKind::CharacterAboveLatin1 { character },
                    Span::new(at, self.offset),
                );
            }
        }
        if !closed {
            self.raise(
                DiagnosticKind::UnterminatedString { quote },
                Span::new(start, self.offset),
            );
        }
        TokenKind::StringLiteral(quote)
    }
}

/// The token a one-character operator or punctuation mark makes.
fn single_character_token(character: char) -> Option<TokenKind> {
    Some(match character {
        '#' => TokenKind::Hash,
        ',' => TokenKind::Comma,
        '(' => TokenKind::LeftParen,
        ')' => TokenKind::RightParen,
        ':' => TokenKind::Colon,
        '+' => TokenKind::Plus,
        '-' => TokenKind::Minus,
        '*' => TokenKind::Star,
        '/' => TokenKind::Slash,
        '\\' => TokenKind::Backslash,
        '&' => TokenKind::Ampersand,
        '!' => TokenKind::Bang,
        '|' => TokenKind::Pipe,
        '^' => TokenKind::Caret,
        '~' => TokenKind::Tilde,
        _ => return None,
    })
}

/// One line, tokenized to the end.
#[derive(Debug, Clone)]
pub struct TokenizedLine<'a> {
    text: &'a str,
    tokens: Vec<Token>,
    diagnostics: Vec<Diagnostic>,
}

impl<'a> TokenizedLine<'a> {
    /// The line that was tokenized.
    pub fn text(&self) -> &'a str {
        self.text
    }

    /// Every token of the line, in order. The end of the line is not among
    /// them: it is what a [`Tokenizer`] returns once the tokens run out.
    pub fn tokens(&self) -> &[Token] {
        &self.tokens
    }

    /// The Diagnostics the walk raised.
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    /// The kinds of every token, in order.
    pub fn kinds(&self) -> Vec<TokenKind> {
        self.tokens.iter().map(|token| token.kind).collect()
    }

    /// The text a token covers.
    pub fn text_of(&self, token: &Token) -> &'a str {
        token.text(self.text)
    }
}

/// Tokenize a whole line, the `line`th of `file`.
///
/// This is the convenience the tests use. The parser does **not** use it: a
/// Comment field is never tokenized (`docs/grammar.md` 1.6), and only the
/// parser knows where the Operand field ends, so it drives a [`Tokenizer`]
/// itself and stops it there.
pub fn tokenize_line<'a>(text: &'a str, file: &str, line: usize) -> TokenizedLine<'a> {
    let mut tokenizer = Tokenizer::new(text, file, line);
    let mut tokens = Vec::new();
    loop {
        let token = tokenizer.next_token();
        if token.is_end_of_line() {
            break;
        }
        tokens.push(token);
    }
    TokenizedLine {
        text,
        tokens,
        diagnostics: tokenizer.take_diagnostics(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assembler::token::string_literal_bytes;

    /// Tokenize a line of the test File, and give back the kinds and the codes.
    fn tokenize(text: &str) -> TokenizedLine<'_> {
        tokenize_line(text, "main.m68k", 0)
    }

    fn kinds(text: &str) -> Vec<TokenKind> {
        tokenize(text).kinds()
    }

    fn codes(text: &str) -> Vec<&'static str> {
        tokenize(text)
            .diagnostics()
            .iter()
            .map(|diagnostic| diagnostic.code())
            .collect()
    }

    fn texts(text: &str) -> Vec<String> {
        let line = tokenize(text);
        line.tokens()
            .iter()
            .map(|token| line.text_of(token).to_string())
            .collect()
    }

    #[test]
    fn source_line_is_tokenized_field_by_field() {
        assert_eq!(
            kinds("loop  move.l  #$10,d0"),
            vec![
                TokenKind::Identifier,
                TokenKind::Whitespace,
                TokenKind::Identifier,
                TokenKind::SizeSuffix,
                TokenKind::Whitespace,
                TokenKind::Hash,
                TokenKind::Number(NumberBase::Hexadecimal),
                TokenKind::Comma,
                TokenKind::Identifier,
            ]
        );
        assert!(codes("loop  move.l  #$10,d0").is_empty());
    }

    #[test]
    fn label_rule_knows_which_token_starts_in_column_one() {
        let line = tokenize("loop move.l d0,d1");
        assert!(line.tokens()[0].column_one);
        assert!(!line.tokens()[2].column_one);

        let indented = tokenize("    loop move.l d0,d1");
        assert!(
            indented.tokens()[0].column_one,
            "the whitespace is what starts in column 1"
        );
        assert!(
            !indented.tokens()[1].column_one,
            "`loop` is indented, so it is no label"
        );
    }

    #[test]
    fn operand_field_extent_knows_what_whitespace_precedes() {
        let line = tokenize("move.l d0 ,d1");
        let flags: Vec<bool> = line
            .tokens()
            .iter()
            .map(|token| token.preceded_by_whitespace)
            .collect();
        //           move  .l    _      d0     _      ,      d1
        assert_eq!(flags, vec![false, false, false, true, false, true, false]);
    }

    #[test]
    fn whitespace_is_one_token_for_a_whole_run() {
        assert_eq!(
            kinds("\t  nop"),
            vec![TokenKind::Whitespace, TokenKind::Identifier]
        );
        assert_eq!(texts("\t  nop"), vec!["\t  ", "nop"]);
        // A form feed and a vertical tab are whitespace; a no-break space is not.
        assert_eq!(
            kinds("\u{0C}\u{0B}nop"),
            vec![TokenKind::Whitespace, TokenKind::Identifier]
        );
    }

    #[test]
    fn identifier_and_global_identifier() {
        assert_eq!(kinds("_start"), vec![TokenKind::Identifier]);
        assert_eq!(texts("count_1 equ 10")[0], "count_1");
        // The tokenizer makes no register a kind of its own.
        assert_eq!(kinds("d0"), vec![TokenKind::Identifier]);
    }

    #[test]
    fn local_identifier_opens_wherever_a_name_may_begin() {
        assert_eq!(kinds(".retry"), vec![TokenKind::LocalIdentifier]);
        assert_eq!(
            kinds("bra .loop"),
            vec![
                TokenKind::Identifier,
                TokenKind::Whitespace,
                TokenKind::LocalIdentifier
            ]
        );
        // `.l` in column 1 is the Local label `.l`, not a bare size suffix.
        assert_eq!(kinds(".l"), vec![TokenKind::LocalIdentifier]);
        assert_eq!(texts(".l")[0], ".l");
    }

    #[test]
    fn size_suffix_opens_after_a_name_a_number_a_paren_or_a_literal() {
        assert_eq!(
            kinds("dc.b"),
            vec![TokenKind::Identifier, TokenKind::SizeSuffix]
        );
        assert_eq!(
            kinds("$1000.l"),
            vec![
                TokenKind::Number(NumberBase::Hexadecimal),
                TokenKind::SizeSuffix
            ]
        );
        assert_eq!(
            kinds("(a0).w"),
            vec![
                TokenKind::LeftParen,
                TokenKind::Identifier,
                TokenKind::RightParen,
                TokenKind::SizeSuffix
            ]
        );
        assert_eq!(
            kinds("'a'.b"),
            vec![
                TokenKind::StringLiteral(QuoteKind::Single),
                TokenKind::SizeSuffix
            ]
        );
        // Whitespace between breaks the adjacency, so the `.` opens a name.
        assert_eq!(
            kinds("dc .b"),
            vec![
                TokenKind::Identifier,
                TokenKind::Whitespace,
                TokenKind::LocalIdentifier
            ]
        );
    }

    #[test]
    fn size_suffix_takes_the_whole_run_after_the_dot() {
        // "longest run, then diagnose": the parser answers with
        // `unknown_size_suffix` or `dot_in_name`, never the tokenizer.
        assert_eq!(texts("move.ll d0,d1")[1], ".ll");
        assert_eq!(texts("move.l array.length,d0")[4], ".length");
        assert!(codes("move.l array.length,d0").is_empty());
        assert_eq!(texts("move.")[1], ".");
        assert_eq!(
            kinds("move."),
            vec![TokenKind::Identifier, TokenKind::SizeSuffix]
        );
    }

    #[test]
    fn a_lone_dot_where_a_name_may_begin_is_reported() {
        assert_eq!(kinds("move.l .,d0")[3], TokenKind::Error);
        assert_eq!(codes("move.l .,d0"), vec!["unexpected_character"]);
    }

    #[test]
    fn number_reads_the_four_bases() {
        assert_eq!(
            kinds("$ff"),
            vec![TokenKind::Number(NumberBase::Hexadecimal)]
        );
        assert_eq!(kinds("%1010"), vec![TokenKind::Number(NumberBase::Binary)]);
        assert_eq!(kinds("@17"), vec![TokenKind::Number(NumberBase::Octal)]);
        assert_eq!(kinds("010"), vec![TokenKind::Number(NumberBase::Decimal)]);
        assert!(codes("$ff %1010 @17 010").is_empty());
        assert_eq!(texts("$ff"), vec!["$ff"]);
    }

    #[test]
    fn invalid_number_names_the_digit_that_does_not_belong() {
        assert_eq!(codes("$1G"), vec!["invalid_number"]);
        assert_eq!(
            kinds("$1G"),
            vec![TokenKind::Number(NumberBase::Hexadecimal)]
        );
        let line = tokenize("$1G");
        assert_eq!(
            line.diagnostics()[0].message(),
            "`G` is not a hexadecimal digit."
        );
        assert_eq!(line.diagnostics()[0].location.column, 2);
        assert_eq!(codes("%2"), vec!["invalid_number"]);
        assert_eq!(codes("@8"), vec!["invalid_number"]);
        assert_eq!(codes("12ab"), vec!["invalid_number"]);
    }

    #[test]
    fn invalid_number_reports_a_prefix_with_no_digits() {
        let line = tokenize("move.l #$,d0");
        assert_eq!(
            line.diagnostics()
                .iter()
                .map(|diagnostic| diagnostic.message())
                .collect::<Vec<_>>(),
            vec!["`$` has no digits after it."]
        );
    }

    #[test]
    fn string_literal_reads_both_quotes_and_a_doubled_quote() {
        assert_eq!(
            kinds("'ab'"),
            vec![TokenKind::StringLiteral(QuoteKind::Single)]
        );
        assert_eq!(
            kinds("\"Hello\""),
            vec![TokenKind::StringLiteral(QuoteKind::Double)]
        );
        assert!(codes("dc.b 'it''s',0").is_empty());
        let line = tokenize("dc.b 'it''s',0");
        let literal = line
            .tokens()
            .iter()
            .find(|token| matches!(token.kind, TokenKind::StringLiteral(_)))
            .expect("a literal");
        assert_eq!(line.text_of(literal), "'it''s'");
        assert_eq!(
            string_literal_bytes(line.text_of(literal)),
            b"it's".to_vec()
        );
    }

    #[test]
    fn character_literal_holds_up_to_four_characters_for_the_evaluator() {
        // The tokenizer has no opinion on the length: 'ab' is one token and the
        // evaluator warns beyond four characters.
        let line = tokenize("move.l #'ab',d0");
        assert!(line.diagnostics().is_empty());
        assert_eq!(
            string_literal_bytes(line.text_of(&line.tokens()[4])),
            b"ab".to_vec()
        );
    }

    #[test]
    fn unterminated_string_is_reported_at_the_end_of_the_line() {
        assert_eq!(codes("dc.b 'Hello"), vec!["unterminated_string"]);
        assert_eq!(
            kinds("dc.b 'Hello")[3],
            TokenKind::StringLiteral(QuoteKind::Single)
        );
        // A doubled quote does not close the literal.
        assert_eq!(codes("dc.b 'it''s"), vec!["unterminated_string"]);
    }

    #[test]
    fn comment_rule_takes_a_whole_comment_line() {
        assert_eq!(kinds("* a comment line"), vec![TokenKind::Comment]);
        assert_eq!(kinds("; a comment line"), vec![TokenKind::Comment]);
        assert_eq!(
            kinds("    * an indented comment line"),
            vec![TokenKind::Whitespace, TokenKind::Comment]
        );
        assert_eq!(texts("* move.l d0,d1"), vec!["* move.l d0,d1"]);
    }

    #[test]
    fn comment_rule_takes_a_semicolon_anywhere() {
        assert_eq!(
            kinds("move.l d0,d1 ; copy"),
            vec![
                TokenKind::Identifier,
                TokenKind::SizeSuffix,
                TokenKind::Whitespace,
                TokenKind::Identifier,
                TokenKind::Comma,
                TokenKind::Identifier,
                TokenKind::Whitespace,
                TokenKind::Comment,
            ]
        );
        // A `;` ends the line whatever the parenthesis depth (1.5 rule 2).
        assert_eq!(
            *kinds("move.l (a0,d1  ; save it").last().unwrap(),
            TokenKind::Comment
        );
    }

    #[test]
    fn comment_rule_leaves_every_other_star_alone() {
        // A lone `*` in the Operand field is a Star; the parser reads it as the
        // current address (1.6).
        assert_eq!(
            kinds("lea *,a0"),
            vec![
                TokenKind::Identifier,
                TokenKind::Whitespace,
                TokenKind::Star,
                TokenKind::Comma,
                TokenKind::Identifier,
            ]
        );
        assert_eq!(
            kinds("org (*+1)&-2"),
            vec![
                TokenKind::Identifier,
                TokenKind::Whitespace,
                TokenKind::LeftParen,
                TokenKind::Star,
                TokenKind::Plus,
                TokenKind::Number(NumberBase::Decimal),
                TokenKind::RightParen,
                TokenKind::Ampersand,
                TokenKind::Minus,
                TokenKind::Number(NumberBase::Decimal),
            ]
        );
        assert_eq!(kinds("NOON equ 12*60*60*100").len(), 11);
    }

    #[test]
    fn operator_holds_the_easy68k_set() {
        assert_eq!(
            kinds("1+2-3*4/5\\6&7!8|9^0<<1>>2~3"),
            vec![
                TokenKind::Number(NumberBase::Decimal),
                TokenKind::Plus,
                TokenKind::Number(NumberBase::Decimal),
                TokenKind::Minus,
                TokenKind::Number(NumberBase::Decimal),
                TokenKind::Star,
                TokenKind::Number(NumberBase::Decimal),
                TokenKind::Slash,
                TokenKind::Number(NumberBase::Decimal),
                TokenKind::Backslash,
                TokenKind::Number(NumberBase::Decimal),
                TokenKind::Ampersand,
                TokenKind::Number(NumberBase::Decimal),
                TokenKind::Bang,
                TokenKind::Number(NumberBase::Decimal),
                TokenKind::Pipe,
                TokenKind::Number(NumberBase::Decimal),
                TokenKind::Caret,
                TokenKind::Number(NumberBase::Decimal),
                TokenKind::ShiftLeft,
                TokenKind::Number(NumberBase::Decimal),
                TokenKind::ShiftRight,
                TokenKind::Number(NumberBase::Decimal),
                TokenKind::Tilde,
                TokenKind::Number(NumberBase::Decimal),
            ]
        );
    }

    #[test]
    fn a_lone_angle_bracket_is_not_a_token() {
        // `<` only ever comes in pairs; the structured-control lines that write
        // one are raw text and never reach the tokenizer.
        assert_eq!(
            kinds("1<2"),
            vec![
                TokenKind::Number(NumberBase::Decimal),
                TokenKind::Error,
                TokenKind::Number(NumberBase::Decimal)
            ]
        );
        assert_eq!(codes("1<2"), vec!["unexpected_character"]);
    }

    #[test]
    fn punctuation_is_one_token_each() {
        assert_eq!(
            kinds("#(),:/"),
            vec![
                TokenKind::Hash,
                TokenKind::LeftParen,
                TokenKind::RightParen,
                TokenKind::Comma,
                TokenKind::Colon,
                TokenKind::Slash,
            ]
        );
    }

    #[test]
    fn case_rule_leaves_case_to_the_parser() {
        assert_eq!(kinds("MOVE.B D0,D1"), kinds("move.b d0,d1"));
        assert_eq!(kinds("$FF"), kinds("$ff"));
        assert!(codes("$FF").is_empty());
    }

    #[test]
    fn character_set_refuses_a_character_above_latin_1() {
        let line = tokenize("dc.b \u{2018}A\u{2019}");
        assert_eq!(
            line.diagnostics()
                .iter()
                .map(|diagnostic| diagnostic.code())
                .collect::<Vec<_>>(),
            vec!["character_above_latin1", "character_above_latin1"]
        );
        assert_eq!(line.diagnostics()[0].hint(), Some("Write `'`".to_string()));
    }

    #[test]
    fn character_set_refuses_a_character_above_latin_1_inside_a_literal() {
        // A byte has to be written for it and there is none (1.1).
        let line = tokenize("dc.b 'caf\u{e9} \u{2014}'");
        assert_eq!(
            line.diagnostics()
                .iter()
                .map(|diagnostic| diagnostic.code())
                .collect::<Vec<_>>(),
            vec!["character_above_latin1"]
        );
        assert_eq!(line.diagnostics()[0].hint(), Some("Write `-`".to_string()));
    }

    #[test]
    fn character_set_names_the_no_break_space() {
        let line = tokenize("move.l\u{A0}d0,d1");
        assert_eq!(
            line.diagnostics()
                .iter()
                .map(|diagnostic| diagnostic.code())
                .collect::<Vec<_>>(),
            vec!["non_breaking_space"]
        );
        // Inside a literal it is an ordinary Latin-1 byte.
        assert!(tokenize("dc.b '\u{A0}'").diagnostics().is_empty());
    }

    #[test]
    fn character_set_refuses_a_control_character() {
        assert_eq!(codes("move.l\u{1}d0"), vec!["unexpected_character"]);
        assert_eq!(
            tokenize("move.l\u{1}d0").diagnostics()[0].message(),
            "The character $01 cannot start anything here."
        );
        // A lone carriage return is not a line terminator and not whitespace.
        assert_eq!(codes("nop\r"), vec!["unexpected_character"]);
    }

    #[test]
    fn the_tokenizer_never_fails() {
        // Every character of a line of junk becomes a token, and the walk ends.
        let line = tokenize("?? \u{2014} = `` \u{A0}");
        assert!(!line.tokens().is_empty());
        assert!(line.diagnostics().len() >= 6);
        assert!(line
            .tokens()
            .iter()
            .all(|token| token.span.end <= line.text().len()));
    }

    #[test]
    fn the_end_of_the_line_repeats_for_ever() {
        let mut tokenizer = Tokenizer::new("nop", "main.m68k", 0);
        assert_eq!(tokenizer.next_token().kind, TokenKind::Identifier);
        for _ in 0..3 {
            let token = tokenizer.next_token();
            assert!(token.is_end_of_line());
            assert_eq!(token.span, Span::empty(3));
        }
    }

    #[test]
    fn a_mark_undoes_the_walk_and_its_diagnostics() {
        let mut tokenizer = Tokenizer::new("move.l d0 \u{2014}", "main.m68k", 0);
        for _ in 0..4 {
            tokenizer.next_token();
        }
        let mark = tokenizer.mark();
        assert!(tokenizer.diagnostics().is_empty());
        assert_eq!(tokenizer.next_token().kind, TokenKind::Whitespace);
        assert_eq!(tokenizer.next_token().kind, TokenKind::Error);
        assert_eq!(tokenizer.diagnostics().len(), 1);
        tokenizer.rewind(mark);
        assert!(tokenizer.diagnostics().is_empty());
        assert_eq!(tokenizer.offset(), 9);
        assert_eq!(tokenizer.rest().text(tokenizer.text()), " \u{2014}");
    }

    #[test]
    fn the_rest_of_the_line_is_never_tokenized_twice() {
        let mut tokenizer = Tokenizer::new("include io.x68", "main.m68k", 0);
        assert_eq!(tokenizer.next_token().kind, TokenKind::Identifier);
        assert_eq!(tokenizer.next_token().kind, TokenKind::Whitespace);
        assert_eq!(tokenizer.rest().text(tokenizer.text()), "io.x68");
    }

    #[test]
    fn the_tokens_of_a_line_cover_it_exactly() {
        // The one invariant of the walk: tokens are contiguous, none is empty,
        // and together they are the line. Checked over every line of the corpus
        // — the 30 programs the asm-editor ships and the 3 EASy68K originals —
        // which is also the proof that no line of a real program makes the
        // tokenizer loop or panic. It says nothing about the Diagnostics: a
        // Comment field is tokenized here and never is by the parser (1.6), so
        // the prose of a bare Comment raises errors that a real assembly would
        // not see.
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/corpus");
        let mut lines = 0usize;
        for directory in ["editor", "easy68k"] {
            let entries = std::fs::read_dir(root.join(directory))
                .unwrap_or_else(|error| panic!("cannot read {directory}: {error}"));
            for entry in entries {
                let path = entry.expect("a corpus entry").path();
                if !path.is_file() {
                    continue;
                }
                let text = std::fs::read_to_string(&path)
                    .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
                let name = path.file_name().expect("a file name").to_string_lossy();
                for (index, span) in crate::assembler::source::split_lines(&text)
                    .into_iter()
                    .enumerate()
                {
                    let line = span.text(&text);
                    let tokenized = tokenize_line(line, &name, index);
                    let mut offset = 0;
                    for token in tokenized.tokens() {
                        assert_eq!(
                            token.span.start, offset,
                            "{name}:{index} leaves a gap before {token:?}"
                        );
                        assert!(
                            !token.span.is_empty(),
                            "{name}:{index} has an empty token {token:?}"
                        );
                        offset = token.span.end;
                    }
                    assert_eq!(
                        offset,
                        line.len(),
                        "{name}:{index} is not covered to its end"
                    );
                    lines += 1;
                }
            }
        }
        assert_eq!(
            lines, 9672,
            "the corpus is 9672 lines (docs/grammar.md section 6)"
        );
    }

    #[test]
    fn a_location_counts_characters() {
        let line = tokenize("dc.b 'citt\u{e0}',\u{2014}");
        let diagnostic = &line.diagnostics()[0];
        assert_eq!(diagnostic.location.line, 0);
        assert_eq!(diagnostic.location.column, 13);
        assert_eq!(diagnostic.location.end_column, 14);
        assert_eq!(diagnostic.location.file, "main.m68k");
    }
}
