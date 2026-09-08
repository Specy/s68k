//! One Source line to a [`Line`], and one File to a [`ParsedFile`].
//!
//! The specification is `docs/grammar.md`, rule by rule ([ADR
//! 0002](../../../docs/adr/0002-hand-written-parser-with-a-grammar-document.md)):
//! sections 1.4 to 1.6 for the four fields, 2.2 to 2.5 for the shapes, 2.7 for
//! the Expressions and 3 for every ambiguity the shapes leave. The tests below
//! carry the rules' names.
//!
//! # How a line is read
//!
//! 1. **The fields are found first.** The parser drives a
//!    [`super::tokenizer::Tokenizer`] over the line, decides the
//!    Label by `label_rule` (1.4), reads the Operation and its `size_suffix`,
//!    and then collects the tokens of the Operand field, stopping exactly where
//!    `operand_field_extent` (1.5) says. Everything after that is the Comment
//!    field, and it is never tokenized (1.6).
//! 2. **The Operands are parsed from those tokens.** The second half of this
//!    module walks the collected tokens with a small cursor and builds the
//!    Addressing modes of 2.5 and the Expressions of 2.7 (a Pratt loop over
//!    [`BinaryOperator::precedence`]). No Diagnostic in that half can reach back
//!    into the Comment field, because the Comment field is not in the tokens.
//!
//! Recovery is the language's own: report, skip to the end of the Operand, and
//! carry on. A line that fails still gives back what was parsed, so the analyzer
//! and the editor see the Label and the Operation of a line whose third Operand
//! is broken.
//!
//! # What this module does not decide
//!
//! Whether a name is a Mnemonic, whether an Addressing mode may stand where it
//! stands, whether a value fits: all three are the analyzer's ([ADR
//! 0003](../../../docs/adr/0003-operands-are-parsed-independently-of-the-instruction.md)).
//! The parser asks [`names`] two questions and no more (see that module).

use super::ast::{
    is_reserved_name, BinaryOperator, Comment, CommentKind, Expr, IndexRegister, Label, Line,
    Operand, Operation, Register, RegisterKind, RegisterListItem, SizeSuffix, SpecialRegister,
    TextOperandField, UnaryOperator,
};
use super::diagnostics::{Diagnostic, DiagnosticKind, OperandShape};
use super::names::{self, TextOperandKind};
use super::source::{split_lines, Location, Span};
use super::token::{self, is_whitespace, NumberBase, QuoteKind, Token, TokenKind};
use super::tokenizer::Tokenizer;

/// Read one Source line into its four fields.
///
/// `line_index` is 0-based and only ever reaches the Diagnostics' Locations;
/// the parser never looks at another line, because no construct but a Macro
/// definition spans one (`docs/grammar.md` 1.3), and that one is
/// [`parse_file`]'s.
///
/// The Diagnostics come back in the order the line reads, left to right.
pub fn parse_line(text: &str, file: &str, line_index: usize) -> (Line, Vec<Diagnostic>) {
    let mut parser = Parser::new(text, file, line_index);
    let line = parser.parse();
    (line, parser.finish())
}

/// Every line of one File, and everything the parser found in them.
///
/// `lines` has one entry per Source line, so `lines[n]` is the `n`th line of
/// the File and an index is never off by one. The body of a Macro definition is
/// skipped whole and never tokenized (`macro_definition`, `docs/grammar.md`
/// 2.6), so its lines come back blank.
#[derive(Debug, Clone, Default)]
pub struct ParsedFile {
    /// One [`Line`] per Source line, in order.
    pub lines: Vec<Line>,
    /// Everything the parser found, in source order.
    pub diagnostics: Vec<Diagnostic>,
    /// The Macro definitions the File holds, in the order they were written.
    pub macros: Vec<MacroDefinition>,
}

/// A Macro this File defines, which is a name the Assembler knows about and
/// does not assemble.
///
/// The body is skipped whole and nothing in it is read (`macro_definition`,
/// `docs/grammar.md` 2.6), but the *name* is worth keeping: an invocation of it
/// further down is an Operation the instruction table has never heard of, and
/// "`DELAY` is not an instruction or a directive, start it in column 1 if it is
/// a label" is advice that would make the program worse. With this the analyzer
/// answers the feature instead ([`Context::macros`](super::analyzer::Context)).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MacroDefinition {
    /// The Macro's name, which is the Label of its `macro` line.
    pub name: String,
    /// Where that name was written.
    pub location: Location,
}

/// Read every line of one File.
///
/// This is [`parse_line`] over each line of `text`, plus the three things only
/// a whole File can decide (`docs/grammar.md` 1.3, 1.6, 1.9, 2.6):
///
/// * the Macro definition skip, the name of every Macro it finds, and the
///   `unterminated_macro_definition` error when no `endm` closes it;
/// * the `bare_comment` suggestion, raised on the **first** bare Comment field
///   of the File and never again;
/// * the `double_quoted_string` suggestion, likewise on the first
///   double-quoted literal.
///
/// The Assembler's `assemble` will call this once per File; it is here, and not
/// in `mod.rs`, because all three are the parser's Diagnostics
/// (`docs/grammar.md` section 4).
pub fn parse_file(file: &str, text: &str) -> ParsedFile {
    let mut parsed = ParsedFile::default();
    let mut macro_start: Option<(usize, Span)> = None;
    let mut seen_bare_comment = false;
    let mut seen_double_quote = false;

    for (index, span) in split_lines(text).into_iter().enumerate() {
        let line_text = span.text(text);
        if let Some((_, _)) = macro_start {
            // The body of a Macro definition is not tokenized, not even its
            // Label field, so `endm` is recognised from the raw text.
            if closes_a_macro_definition(line_text) {
                macro_start = None;
            }
            parsed.lines.push(Line::default());
            continue;
        }

        let (line, diagnostics) = parse_line(line_text, file, index);
        parsed.diagnostics.extend(diagnostics);

        if line.bare_comment && !seen_bare_comment {
            seen_bare_comment = true;
            if let Some(comment) = &line.comment {
                parsed.diagnostics.push(Diagnostic::new(
                    DiagnosticKind::BareComment,
                    location(file, index, line_text, comment.span),
                ));
            }
        }
        if !seen_double_quote {
            if let Some(quoted) = first_double_quoted_literal(&line) {
                seen_double_quote = true;
                parsed.diagnostics.push(Diagnostic::new(
                    DiagnosticKind::DoubleQuotedString,
                    location(file, index, line_text, quoted),
                ));
            }
        }
        if let Some(operation) = &line.operation {
            if names::is_macro_start(&operation.name) {
                macro_start = Some((index, operation.name_span));
                // `DELAY    MACRO` names the Macro in its Label field
                // (`quickStart.htm`, "Label Field"), which is the one thing
                // worth keeping out of a definition nothing else reads.
                if let Some(label) = &line.label {
                    parsed.macros.push(MacroDefinition {
                        name: label.name.clone(),
                        location: location(file, index, line_text, label.span),
                    });
                }
            }
        }
        parsed.lines.push(line);
    }

    if let Some((index, span)) = macro_start {
        let line_text = split_lines(text)
            .get(index)
            .map(|line| line.text(text))
            .unwrap_or("");
        parsed.diagnostics.push(Diagnostic::new(
            DiagnosticKind::UnterminatedMacroDefinition,
            location(file, index, line_text, span),
        ));
    }
    parsed
}

/// Whether a raw line inside a Macro definition is its `endm`.
///
/// The body is never tokenized, "not even its Label" (`docs/grammar.md` 2.6),
/// so this reads words rather than tokens; the question it answers is 2.6's
/// own, "is this line's **Operation** `endm`?", and the words are read by
/// `label_rule` (1.4) to find it. A word ending in `:` is a Label, and so is a
/// first word in column 1 that is not the name of an Operation, which leaves
/// the second word; anywhere else the first word is the Operation. A Comment
/// line has no Operation at all. So `endm`, `done: endm` and `done endm` close
/// the definition and `  bra endm`, `* endm` and `endm:` do not.
fn closes_a_macro_definition(line: &str) -> bool {
    let body = line.trim_start_matches(is_whitespace);
    if body.starts_with('*') || body.starts_with(';') {
        return false;
    }
    let column_one = body.len() == line.len();
    let mut words = line
        .split(|character: char| is_whitespace(character) || character == ';')
        .filter(|word| !word.is_empty());
    let operation = match words.next() {
        Some(word) if word.ends_with(':') => words.next(),
        Some(word) if column_one && !names::is_operation_name(word) => words.next(),
        word => word,
    };
    operation.is_some_and(names::is_macro_end)
}

/// The Location of `span` on one line of one File.
fn location(file: &str, line_index: usize, line_text: &str, span: Span) -> Location {
    Location::from_span(file, line_index, line_text, span)
}

/// The span of the first double-quoted literal of a line, if it has one.
///
/// The `double_quoted_string` suggestion is once per File and so cannot be
/// raised by [`parse_line`]; the tree carries the [`QuoteKind`] of every
/// literal, which is what lets [`parse_file`] find the first one
/// without the parser having to flag it (`docs/grammar.md` 1.9). A
/// `file_specification` is never a literal here, so it can never raise it.
pub fn first_double_quoted_literal(line: &Line) -> Option<Span> {
    let operation = line.operation.as_ref()?;
    operation.operands.iter().find_map(double_quote_in_operand)
}

/// The span of the first double-quoted literal inside one Operand.
fn double_quote_in_operand(operand: &Operand) -> Option<Span> {
    match operand {
        Operand::Immediate { value, .. } => double_quote_in_expression(value),
        Operand::Absolute { value, .. } => double_quote_in_expression(value),
        Operand::Displacement { displacement, .. } => double_quote_in_expression(displacement),
        Operand::PcDisplacement { displacement, .. } => double_quote_in_expression(displacement),
        Operand::Index { displacement, .. } | Operand::PcIndex { displacement, .. } => {
            displacement.as_ref().and_then(double_quote_in_expression)
        }
        Operand::DataRegisterDirect { .. }
        | Operand::AddressRegisterDirect { .. }
        | Operand::SpecialRegister { .. }
        | Operand::Indirect { .. }
        | Operand::Postincrement { .. }
        | Operand::Predecrement { .. }
        | Operand::RegisterList { .. } => None,
    }
}

/// The span of the first double-quoted literal inside one Expression.
fn double_quote_in_expression(expression: &Expr) -> Option<Span> {
    match expression {
        Expr::CharacterLiteral { quote, span, .. } => match quote {
            QuoteKind::Double => Some(*span),
            QuoteKind::Single => None,
        },
        Expr::Unary { operand, .. } => double_quote_in_expression(operand),
        Expr::Binary { left, right, .. } => {
            double_quote_in_expression(left).or_else(|| double_quote_in_expression(right))
        }
        Expr::Number { .. } | Expr::Symbol { .. } | Expr::CurrentAddress { .. } => None,
    }
}

/// Why an Expression term could not be read.
///
/// The difference is who says so: `Reported` means a Diagnostic naming the real
/// mistake has already been raised and the caller must stay quiet, `Absent`
/// means there was simply no term and the caller — which knows what the term
/// would have followed — says "an expression was expected after `#`".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TermError {
    /// A Diagnostic has been raised for it.
    Reported,
    /// Nothing here can start an Expression term.
    Absent,
}

/// A `size_suffix` token that has been read and judged.
#[derive(Debug, Clone, Copy)]
struct ParsedSizeSuffix {
    /// The size the run names, `None` when it names none and a Diagnostic has
    /// been raised.
    size: Option<SizeSuffix>,
    /// Where the whole suffix was written, dot included.
    span: Span,
}

/// How deep the Expression parser descends into nested parentheses before it
/// gives up and raises `nesting_too_deep`.
///
/// It is a backstop and not a rule of the language: nothing anyone writes nests
/// ten deep, while `parse_prefix` recurses once per `(` and the crate ships as
/// WebAssembly, whose stack is a megabyte and whose overflow is a trap and not
/// a Diagnostic. No diagnostic is worth a crash (the implementation notes,
/// phase 1 step 3), and neither is a pasted line of three thousand `(`.
const MAX_NESTING_DEPTH: usize = 64;

/// The parser of one Source line.
struct Parser<'a> {
    /// The line, without its terminator.
    text: &'a str,
    /// The File the line is in, for the Diagnostics' Locations.
    file: String,
    /// The 0-based index of the line, likewise.
    line_index: usize,
    /// The walk over the line, used only while the fields are being found.
    tokenizer: Tokenizer<'a>,
    /// What has been found, in the order it was found.
    diagnostics: Vec<Diagnostic>,
    /// The tokens of the Operand field, and nothing else.
    tokens: Vec<Token>,
    /// How far the Operand parser has walked them.
    index: usize,
    /// Where the Operand being parsed began, for the span of a
    /// `malformed_operand`.
    operand_start: usize,
    /// The first whitespace run the Operand field was continued across because
    /// a `,` followed it; the `space_before_comma` suggestion is raised once a
    /// line (`docs/grammar.md` 3.6).
    space_before_comma: Option<Span>,
    /// How many nested `(` the Expression parser is inside, against
    /// [`MAX_NESTING_DEPTH`].
    depth: usize,
}

impl<'a> Parser<'a> {
    /// A parser over one line.
    fn new(text: &'a str, file: &str, line_index: usize) -> Self {
        Self {
            text,
            file: file.to_string(),
            line_index,
            tokenizer: Tokenizer::new(text, file, line_index),
            diagnostics: Vec::new(),
            tokens: Vec::new(),
            index: 0,
            operand_start: 0,
            space_before_comma: None,
            depth: 0,
        }
    }

    /// Everything the line raised, in source order.
    ///
    /// The tokenizer's Diagnostics and the parser's are two lists — the first
    /// has to stay in the tokenizer until every lookahead has been rewound, or
    /// a rewind could not drop what a lookahead raised — so they are merged
    /// here and sorted by column, which is the order a reader expects.
    fn finish(mut self) -> Vec<Diagnostic> {
        if let Some(span) = self.space_before_comma {
            let location = self.location(span);
            self.diagnostics
                .push(Diagnostic::new(DiagnosticKind::SpaceBeforeComma, location));
        }
        let mut diagnostics = self.tokenizer.take_diagnostics();
        diagnostics.append(&mut self.diagnostics);
        diagnostics.sort_by_key(|diagnostic| diagnostic.location.column);
        diagnostics
    }

    // -- the fields -------------------------------------------------------

    /// Read the line, `source_line` of `docs/grammar.md` 2.2.
    fn parse(&mut self) -> Line {
        let mut token = self.tokenizer.next_token();
        if token.is_whitespace() {
            token = self.tokenizer.next_token();
        }
        if token.is_end_of_line() {
            return Line::default();
        }
        if token.kind == TokenKind::Comment {
            // `comment_line`: the first non-blank character is `*` or `;`.
            return Line {
                comment: self.comment_at(token.span.start, CommentKind::Line),
                ..Line::default()
            };
        }

        let mut label = None;
        loop {
            match token.kind {
                TokenKind::Colon => {
                    self.raise(DiagnosticKind::EmptyLabel, token.span);
                    token = self.next_field_token();
                }
                TokenKind::Identifier | TokenKind::LocalIdentifier => {
                    let mark = self.tokenizer.mark();
                    let next = self.tokenizer.next_token();
                    self.tokenizer.rewind(mark);
                    let colon = next.kind == TokenKind::Colon && next.span.start == token.span.end;
                    if colon {
                        self.declare_label(&mut label, token, true);
                        self.tokenizer.next_token();
                        token = self.next_field_token();
                        continue;
                    }
                    // `column_one_label`: in column 1, carrying no size suffix,
                    // and not the name of a Mnemonic or a Directive (1.4).
                    if label.is_none()
                        && token.column_one
                        && next.kind != TokenKind::SizeSuffix
                        && !names::is_operation_name(self.text_of(token))
                    {
                        self.declare_label(&mut label, token, false);
                        token = self.next_field_token();
                        continue;
                    }
                    break;
                }
                _ => break,
            }
        }

        match token.kind {
            TokenKind::EndOfLine => {
                return Line {
                    label,
                    ..Line::default()
                }
            }
            // A `*` where the Operation would stand is the second row of the
            // table of 1.6: the Operation field is optional (2.2), so this `*`
            // is the first non-blank character of the Comment field and marks
            // an `explicit_comment`. The tokenizer makes a Comment token only
            // of a `*` that opens the *line*, which is why `start: * entry
            // point` arrives here as a `Star`.
            TokenKind::Comment | TokenKind::Star => {
                let comment = self.comment_at(token.span.start, CommentKind::Explicit);
                return Line {
                    label,
                    comment,
                    ..Line::default()
                };
            }
            TokenKind::Identifier | TokenKind::LocalIdentifier => {}
            _ => {
                let found = self.text_of(token).to_string();
                self.raise(DiagnosticKind::OperationExpected { found }, token.span);
                return Line {
                    label,
                    ..Line::default()
                };
            }
        }

        let (operation, comment) = self.parse_operation(token);
        let bare_comment = matches!(
            comment,
            Some(Comment {
                kind: CommentKind::Bare,
                ..
            })
        );
        Line {
            label,
            operation: Some(operation),
            comment,
            bare_comment,
        }
    }

    /// The next token of a field: the one after this, with a run of whitespace
    /// stepped over.
    fn next_field_token(&mut self) -> Token {
        let token = self.tokenizer.next_token();
        if token.is_whitespace() {
            self.tokenizer.next_token()
        } else {
            token
        }
    }

    /// Record a Label, or say why it cannot be one.
    fn declare_label(&mut self, slot: &mut Option<Label>, token: Token, colon: bool) {
        let name = self.text_of(token).to_string();
        if is_reserved_name(&name) {
            self.raise(
                DiagnosticKind::ReservedNameAsSymbol { name: name.clone() },
                token.span,
            );
        }
        if slot.is_some() {
            self.raise(DiagnosticKind::TwoLabelsOnOneLine { name }, token.span);
            return;
        }
        *slot = Some(Label {
            name,
            colon,
            span: token.span,
        });
    }

    /// `operation_field` (2.4): the name, its `size_suffix`, and whichever of
    /// the two Operand fields (2.5) the name asks for.
    fn parse_operation(&mut self, name_token: Token) -> (Operation, Option<Comment>) {
        let name = self.text_of(name_token).to_string();
        let mut size = None;
        let mut size_span = None;
        let mark = self.tokenizer.mark();
        let suffix = self.tokenizer.next_token();
        if suffix.kind == TokenKind::SizeSuffix {
            size_span = Some(suffix.span);
            let run = self.suffix_run(suffix.span);
            match SizeSuffix::from_run(run) {
                Some(read) => size = Some(read),
                // In the operation field every run that is not one of the four
                // letters is an unknown size: an operation name holds no dot of
                // its own (1.7), so the run can be nothing else (1.11).
                None => self.raise(
                    DiagnosticKind::UnknownSizeSuffix {
                        suffix: run.to_string(),
                    },
                    suffix.span,
                ),
            }
        } else {
            self.tokenizer.rewind(mark);
        }

        let (text, operands, operand_field_span, comment) = match names::text_operation(&name) {
            Some(kind) => {
                let (text, comment) = self.parse_text_operand_field(kind);
                let span = text.as_ref().map(|field| field.span);
                (text, Vec::new(), span, comment)
            }
            None => {
                let (operands, span, comment) = self.parse_operand_field();
                (None, operands, span, comment)
            }
        };

        let end = operand_field_span
            .map(|span| span.end)
            .or(size_span.map(|span| span.end))
            .unwrap_or(name_token.span.end);
        let operation = Operation {
            name,
            name_span: name_token.span,
            size,
            size_span,
            operands,
            text,
            operand_field_span,
            span: Span::new(name_token.span.start, end),
        };
        (operation, comment)
    }

    /// `text_operand_field` (2.5): the Operand field of a `text_operation`,
    /// which is raw text and is never tokenized.
    ///
    /// The tokenizer is abandoned here — nothing after this point on the line
    /// is read as tokens — so a `.` in `..\lib\io.x68` is a character and not a
    /// `size_suffix`, and `FAIL ERROR, Argument missing` is one message.
    fn parse_text_operand_field(
        &mut self,
        kind: TextOperandKind,
    ) -> (Option<TextOperandField>, Option<Comment>) {
        let after_operation = self.tokenizer.offset();
        let rest = &self.text[after_operation..];
        let start = after_operation + (rest.len() - rest.trim_start_matches(is_whitespace).len());
        let (end, comment_start) = match kind {
            TextOperandKind::FileSpecification => self.file_specification_extent(start),
            TextOperandKind::MessageText | TextOperandKind::RawOperandField => {
                match self.text[start..].find(';') {
                    Some(offset) => (start + offset, Some(start + offset)),
                    None => (self.text.len(), None),
                }
            }
        };
        let span = Span::new(start, trimmed_end(self.text, start, end));
        let field = if span.is_empty() {
            None
        } else {
            Some(TextOperandField {
                text: span.text(self.text).to_string(),
                span,
            })
        };
        let comment = comment_start.and_then(|start| {
            // `include` and `incbin` end their field by all four rules of 1.5,
            // so what follows the filename can be EASy68K's bare Comment field
            // and `bare_comment` has to be able to fire on it. The other two
            // end at a `;` and nowhere else (1.5), so their Comment always
            // carries a marker.
            let kind = match kind {
                TextOperandKind::FileSpecification => comment_kind(self.text, start),
                TextOperandKind::MessageText | TextOperandKind::RawOperandField => {
                    CommentKind::Explicit
                }
            };
            self.comment_at(start, kind)
        });
        (field, comment)
    }

    /// Where a `file_specification` ends, by `operand_field_extent` (1.5) read
    /// over characters rather than over tokens.
    ///
    /// All four rules hold — rule 1 is what keeps the spaces of
    /// `include 'input output macros.x68'` — but the text inside is one raw
    /// filename (2.6). Answers where the field ends and where the Comment
    /// field begins, if it does.
    fn file_specification_extent(&self, start: usize) -> (usize, Option<usize>) {
        let mut characters = self.text[start..]
            .char_indices()
            .map(|(at, character)| (start + at, character));
        let mut pending: Option<(usize, char)> = characters.next();
        let mut quote: Option<char> = None;
        let mut depth: usize = 0;
        let mut previous: Option<char> = None;
        while let Some((at, character)) = pending {
            pending = characters.next();
            if let Some(open) = quote {
                if character == open {
                    if pending.map(|(_, next)| next) == Some(open) {
                        pending = characters.next();
                    } else {
                        quote = None;
                    }
                }
                previous = Some(character);
                continue;
            }
            match character {
                '\'' | '"' => quote = Some(character),
                '(' => depth += 1,
                ')' => depth = depth.saturating_sub(1),
                ';' => return (at, Some(at)),
                _ if is_whitespace(character) && depth == 0 => {
                    while let Some((_, next)) = pending {
                        if is_whitespace(next) {
                            pending = characters.next();
                        } else {
                            break;
                        }
                    }
                    match pending {
                        Some((_, ',')) => {}
                        _ if previous == Some(',') => {}
                        Some((next_at, _)) => return (at, Some(next_at)),
                        None => return (at, None),
                    }
                }
                _ => {}
            }
            previous = Some(character);
        }
        (self.text.len(), None)
    }

    /// `operand_list` (2.5): the tokens of the Operand field, then the Operands
    /// they make.
    ///
    /// The scan is `operand_field_extent` (1.5) itself: rule 1 is the
    /// tokenizer's (a quoted literal is one token), rule 2 is the `;` it turns
    /// into a Comment token, rule 3 is the end of the line, and rule 4 is the
    /// whitespace test below, whose lookahead is rewound so that nothing is
    /// ever raised about a Comment field.
    fn parse_operand_field(&mut self) -> (Vec<Operand>, Option<Span>, Option<Comment>) {
        let mut current = self.tokenizer.next_token();
        if current.is_whitespace() {
            current = self.tokenizer.next_token();
        }
        let mut depth: usize = 0;
        let mut comment_start = None;
        let mut ended_at_whitespace = None;
        loop {
            match current.kind {
                TokenKind::EndOfLine => break,
                TokenKind::Comment => {
                    comment_start = Some(current.span.start);
                    break;
                }
                TokenKind::Whitespace => {
                    if depth > 0 {
                        current = self.tokenizer.next_token();
                        continue;
                    }
                    let after_comma = self
                        .tokens
                        .last()
                        .is_some_and(|token| token.kind == TokenKind::Comma);
                    let mark = self.tokenizer.mark();
                    let next = self.tokenizer.next_token();
                    self.tokenizer.rewind(mark);
                    let before_comma = next.kind == TokenKind::Comma;
                    if after_comma || before_comma {
                        if before_comma && !after_comma && self.space_before_comma.is_none() {
                            self.space_before_comma = Some(current.span);
                        }
                        current = self.tokenizer.next_token();
                        continue;
                    }
                    if next.is_end_of_line() {
                        break;
                    }
                    comment_start = Some(next.span.start);
                    ended_at_whitespace = Some(current.span);
                    break;
                }
                _ => {
                    match current.kind {
                        TokenKind::LeftParen => depth += 1,
                        TokenKind::RightParen => depth = depth.saturating_sub(1),
                        _ => {}
                    }
                    self.tokens.push(current);
                    current = self.tokenizer.next_token();
                }
            }
        }

        let field_span = match (self.tokens.first(), self.tokens.last()) {
            (Some(first), Some(last)) => Some(Span::new(first.span.start, last.span.end)),
            _ => None,
        };
        let operands = self.parse_operand_list();
        let comment =
            comment_start.and_then(|start| self.comment_at(start, comment_kind(self.text, start)));
        if let (Some(whitespace), Some(field), Some(comment)) =
            (ended_at_whitespace, field_span, comment.as_ref())
        {
            self.check_expression_split_by_space(whitespace, field, comment);
        }
        (operands, field_span, comment)
    }

    /// `expression_split_by_space` (3.5): the Comment field reads as the
    /// continuation of the Operand's Expression.
    ///
    /// The trigger is narrow on purpose, because a `*` opening a Comment field
    /// is EASy68K's own style: a `binary_operator`, then a number or a
    /// character literal, then the end of the line, a `,` or another operator.
    /// The Comment field is tokenized only here, behind a
    /// [`mark`](Tokenizer::mark) that is rewound, so nothing it holds is ever
    /// reported.
    fn check_expression_split_by_space(
        &mut self,
        whitespace: Span,
        field: Span,
        comment: &Comment,
    ) {
        let mark = self.tokenizer.mark();
        let mut ahead = Vec::new();
        while ahead.len() < 3 {
            let token = self.tokenizer.next_token();
            if token.is_whitespace() {
                continue;
            }
            let end = token.is_end_of_line();
            ahead.push(token);
            if end {
                break;
            }
        }
        self.tokenizer.rewind(mark);

        let operator = match ahead.first() {
            Some(token) if BinaryOperator::from_token_kind(token.kind).is_some() => *token,
            _ => return,
        };
        match ahead.get(1).map(|token| token.kind) {
            Some(TokenKind::Number(_)) | Some(TokenKind::StringLiteral(_)) => {}
            _ => return,
        }
        let closes = match ahead.get(2).map(|token| token.kind) {
            Some(TokenKind::EndOfLine) | Some(TokenKind::Comma) => true,
            Some(kind) => BinaryOperator::from_token_kind(kind).is_some(),
            None => false,
        };
        if !closes {
            return;
        }
        let joined: String = field
            .text(self.text)
            .chars()
            .chain(comment.text.chars())
            .filter(|character| !is_whitespace(*character))
            .collect();
        self.raise(
            DiagnosticKind::ExpressionSplitBySpace {
                operator: self.text_of(operator).to_string(),
                joined,
                bare_comment: comment.kind == CommentKind::Bare,
            },
            whitespace,
        );
    }

    /// The Comment field from `start` to the end of the line, or `None` when
    /// there is nothing but whitespace there.
    fn comment_at(&self, start: usize, kind: CommentKind) -> Option<Comment> {
        let span = Span::new(start, trimmed_end(self.text, start, self.text.len()));
        if span.is_empty() {
            return None;
        }
        Some(Comment {
            kind,
            text: span.text(self.text).to_string(),
            span,
        })
    }

    // -- the Operands -----------------------------------------------------

    /// `operand_list = operand { "," operand }` (2.5).
    fn parse_operand_list(&mut self) -> Vec<Operand> {
        let mut operands = Vec::new();
        while !self.at_end() {
            // `dc.b ,` is one mistake: the `,` arm of `parse_operand` has
            // already said `operand_expected` about this Operand, so the check
            // after the `,` below stays quiet about the same comma.
            let empty = self.peek().map(|token| token.kind) == Some(TokenKind::Comma);
            match self.parse_operand() {
                Some(operand) => operands.push(operand),
                None => self.skip_to_next_operand(),
            }
            match self.peek() {
                None => break,
                Some(token) if token.kind == TokenKind::Comma => {
                    self.bump();
                    if self.at_end() {
                        if !empty {
                            self.raise(
                                DiagnosticKind::OperandExpected,
                                Span::empty(token.span.end),
                            );
                        }
                        break;
                    }
                }
                Some(token) => {
                    // The commonest failure of all: a token left over after an
                    // Operand that was complete without it (section 4). An
                    // `Error` token is the one exception, as it is in
                    // `parse_prefix`: the tokenizer has already said what is
                    // wrong with the character, and one bad character costs one
                    // message.
                    if token.kind != TokenKind::Error {
                        self.raise(
                            DiagnosticKind::UnexpectedTokenInOperand {
                                found: self.text_of(token).to_string(),
                            },
                            token.span,
                        );
                    }
                    self.skip_to_next_operand();
                    if self.peek().map(|token| token.kind) == Some(TokenKind::Comma) {
                        self.bump();
                    } else {
                        break;
                    }
                }
            }
        }
        operands
    }

    /// Step over everything up to the next `,` of the Operand list, which is
    /// where a broken Operand is given up on.
    fn skip_to_next_operand(&mut self) {
        while let Some(token) = self.peek() {
            if token.kind == TokenKind::Comma {
                break;
            }
            self.bump();
        }
    }

    /// `operand` (2.5): one Operand, whatever it turns out to be.
    fn parse_operand(&mut self) -> Option<Operand> {
        let first = self.peek()?;
        self.operand_start = first.span.start;
        match first.kind {
            TokenKind::Comma => {
                self.raise(DiagnosticKind::OperandExpected, first.span);
                None
            }
            TokenKind::Hash => self.parse_immediate(first),
            TokenKind::LeftParen => self.parse_parenthesised_operand(first),
            TokenKind::Minus if self.looks_like_predecrement() => self.parse_predecrement(first),
            TokenKind::Identifier => {
                let text = self.text_of(first);
                if let Some(register) = Register::parse(text, first.span) {
                    self.bump();
                    return self.parse_register_operand(register, first);
                }
                if let Some(special) = SpecialRegister::parse(text) {
                    self.bump();
                    return Some(Operand::SpecialRegister {
                        register: special,
                        span: first.span,
                    });
                }
                self.parse_expression_operand()
            }
            _ => self.parse_expression_operand(),
        }
    }

    /// `immediate = "#" expression` (2.5).
    fn parse_immediate(&mut self, hash: Token) -> Option<Operand> {
        self.bump();
        let value = match self.parse_expression() {
            Ok(value) => value,
            Err(TermError::Reported) => return None,
            Err(TermError::Absent) => {
                self.raise(
                    DiagnosticKind::ExpressionExpected {
                        after: "#".to_string(),
                    },
                    Span::empty(hash.span.end),
                );
                return None;
            }
        };
        let mut end = value.span().end;
        if let Some(suffix) = self.take_size_suffix(value.span()) {
            end = suffix.span.end;
            if suffix.size.is_some() {
                self.malformed(
                    OperandShape::Immediate,
                    "an immediate carries no size: the size goes on the operation, as in `move.w #5,d0`",
                    None,
                );
                return None;
            }
        }
        Some(Operand::Immediate {
            value,
            span: Span::new(hash.span.start, end),
        })
    }

    /// Whether a `-` opens a `predecrement` (2.5): only when a `(` and a
    /// register follow it. Everywhere else a `-` is a unary minus, so `-4(a6)`
    /// and `-(a1)` both land where they should.
    ///
    /// A *data* register counts, although the mode takes an address one, so
    /// that `-(d1)` is answered with the shape it tried to be rather than with
    /// "an expression holds no registers" (2.5, ADR 0003).
    fn looks_like_predecrement(&self) -> bool {
        self.peek_at(1).map(|token| token.kind) == Some(TokenKind::LeftParen)
            && self.register_at(2).is_some()
    }

    /// `predecrement = "-" "(" address_register ")"` (2.5).
    fn parse_predecrement(&mut self, minus: Token) -> Option<Operand> {
        self.bump();
        let open = self.bump().expect("a `(`");
        let register = self.register_at(0).expect("a register");
        let register_token = self.bump().expect("a register");
        if register.kind != RegisterKind::Address {
            let name = self.text_of(register_token).to_string();
            // No related `(`: it is closed, and the register is the mistake.
            self.malformed(
                OperandShape::Predecrement,
                format!("`{name}` is not an address register"),
                None,
            );
            return None;
        }
        let close = self.expect_close(open, OperandShape::Predecrement)?;
        Some(Operand::Predecrement {
            register,
            span: Span::new(minus.span.start, close.span.end),
        })
    }

    /// A lone register: a register direct Operand, or the first item of a
    /// `register_list` when a `-` or a `/` follows it (1.12).
    fn parse_register_operand(&mut self, register: Register, token: Token) -> Option<Operand> {
        match self.peek().map(|token| token.kind) {
            Some(TokenKind::Minus) | Some(TokenKind::Slash) => {
                self.parse_register_list(register, token)
            }
            _ => Some(match register.kind {
                RegisterKind::Data => Operand::DataRegisterDirect {
                    register,
                    span: token.span,
                },
                RegisterKind::Address => Operand::AddressRegisterDirect {
                    register,
                    span: token.span,
                },
            }),
        }
    }

    /// `register_list = register_list_item { "/" register_list_item }` (1.12).
    ///
    /// A range may cross from `d7` into `a0`, which is one contiguous field of
    /// a `movem` mask; what it may not do is run backwards.
    fn parse_register_list(&mut self, first: Register, first_token: Token) -> Option<Operand> {
        let start = first_token.span.start;
        let mut items = Vec::new();
        let mut register = first;
        let mut register_span = first_token.span;
        let mut end = first_token.span.end;
        loop {
            if self.peek().map(|token| token.kind) == Some(TokenKind::Minus) {
                self.bump();
                let to = match self.register_at(0) {
                    Some(to) => to,
                    None => {
                        self.register_expected_in_list('-');
                        return None;
                    }
                };
                let token = self.bump().expect("a register");
                end = token.span.end;
                let span = Span::new(register_span.start, end);
                if to.mask_index() < register.mask_index() {
                    self.raise(
                        DiagnosticKind::RegisterRangeOutOfOrder {
                            from: register.name().to_string(),
                            to: to.name().to_string(),
                        },
                        span,
                    );
                }
                items.push(RegisterListItem::Range {
                    from: register,
                    to,
                    span,
                });
            } else {
                items.push(RegisterListItem::Single {
                    register,
                    span: register_span,
                });
            }
            if self.peek().map(|token| token.kind) != Some(TokenKind::Slash) {
                break;
            }
            self.bump();
            match self.register_at(0) {
                Some(next) => {
                    let token = self.bump().expect("a register");
                    register = next;
                    register_span = token.span;
                    end = token.span.end;
                }
                None => {
                    self.register_expected_in_list('/');
                    return None;
                }
            }
        }
        Some(Operand::RegisterList {
            items,
            span: Span::new(start, end),
        })
    }

    /// "a register was expected after `/`", pointing at what was written
    /// instead.
    fn register_expected_in_list(&mut self, after: char) {
        let span = match self.peek() {
            Some(token) => token.span,
            None => Span::empty(self.last_end()),
        };
        self.raise(
            DiagnosticKind::RegisterExpectedInRegisterList { after },
            span,
        );
    }

    /// `parenthesised_operand` (2.5): the decision procedure for an Operand
    /// that starts with `(`.
    ///
    /// Rule 1 is the two-token lookahead that separates `(a0)` from
    /// `(640-COLS*SCALE)/2`; rule 2 is everything else, and it is what decides
    /// whether a missing `)` is a `malformed_operand` or an
    /// `unclosed_parenthesis`.
    fn parse_parenthesised_operand(&mut self, open: Token) -> Option<Operand> {
        self.bump();
        // Any register, not just an address one: `(d0)` is a mistake with a
        // name — the base of an indirect operand is an address register — and
        // `parse_base_and_index` says so, where reading it as a grouped
        // Expression would answer `register_in_expression` and offer nothing
        // (2.5, ADR 0003).
        let is_register = self.register_at(0).is_some() || self.program_counter_at(0);
        if is_register {
            let after = self.peek_at(1).map(|token| token.kind);
            // The `None` arm is s68k's, not the document's: a field that ends
            // right after `(a0` has recognised the mode well enough to say the
            // `)` is missing, which beats telling the author that `a0` is a
            // register.
            if matches!(
                after,
                Some(TokenKind::RightParen) | Some(TokenKind::Comma) | None
            ) {
                return self.parse_base_and_index(open, None);
            }
        }

        let expression = match self.parse_expression() {
            Ok(expression) => expression,
            Err(TermError::Reported) => return None,
            Err(TermError::Absent) => {
                self.raise(
                    DiagnosticKind::ExpressionExpected {
                        after: "(".to_string(),
                    },
                    Span::empty(open.span.end),
                );
                return None;
            }
        };
        match self.peek() {
            Some(token) if token.kind == TokenKind::Comma => {
                self.bump();
                self.parse_base_and_index(open, Some(expression))
            }
            Some(token) if token.kind == TokenKind::RightParen => {
                self.bump();
                match self.peek() {
                    // `(640-COLS*SCALE)/2`: the parentheses were a grouped
                    // Expression and the Pratt loop carries on with it.
                    Some(next) if BinaryOperator::from_token_kind(next.kind).is_some() => {
                        let span = Span::new(open.span.start, token.span.end);
                        let grouped = with_span(expression, span);
                        let whole = self.parse_expression_from(grouped, 0);
                        self.finish_expression_operand(whole)
                    }
                    // `(4)(a0)`: the parentheses were the displacement.
                    Some(next) if next.kind == TokenKind::LeftParen => {
                        self.parse_displacement_tail(expression)
                    }
                    _ => {
                        let span = Span::new(open.span.start, token.span.end);
                        self.finish_expression_operand(with_span(expression, span))
                    }
                }
            }
            Some(token) => {
                // As in `parse_operand_list` and `parse_prefix`, an `Error`
                // token has already been reported by the tokenizer.
                if token.kind != TokenKind::Error {
                    self.raise(
                        DiagnosticKind::UnexpectedTokenInOperand {
                            found: self.text_of(token).to_string(),
                        },
                        token.span,
                    );
                }
                None
            }
            None => {
                self.raise(DiagnosticKind::UnclosedParenthesis, open.span);
                None
            }
        }
    }

    /// The `( base [ , index ] )` tail shared by every displaced mode: the
    /// cursor sits on the base register and `open` is the `(` it is inside.
    ///
    /// With no `displacement` this is `(a0)`, `(a0)+`, `(a0,d1.w)` and
    /// `(pc,d1.w)`; with one it is both spellings of `displacement`, `index`,
    /// `pc_displacement` and `pc_index`, which is why the four productions of
    /// 2.5 are one function here.
    fn parse_base_and_index(&mut self, open: Token, displacement: Option<Expr>) -> Option<Operand> {
        let start = self.operand_start;
        if let Some(base) = self.address_register_at(0) {
            self.bump();
            match self.peek().map(|token| token.kind) {
                Some(TokenKind::RightParen) => {
                    let close = self.bump().expect("a `)`");
                    match displacement {
                        Some(displacement) => Some(Operand::Displacement {
                            displacement,
                            base,
                            span: Span::new(start, close.span.end),
                        }),
                        None => {
                            if self.peek().map(|token| token.kind) == Some(TokenKind::Plus) {
                                let plus = self.bump().expect("a `+`");
                                Some(Operand::Postincrement {
                                    register: base,
                                    span: Span::new(start, plus.span.end),
                                })
                            } else {
                                Some(Operand::Indirect {
                                    register: base,
                                    span: Span::new(start, close.span.end),
                                })
                            }
                        }
                    }
                }
                Some(TokenKind::Comma) => {
                    self.bump();
                    let index = self.parse_index_register(OperandShape::Index)?;
                    let close = self.expect_close(open, OperandShape::Index)?;
                    Some(Operand::Index {
                        displacement,
                        base,
                        index,
                        span: Span::new(start, close.span.end),
                    })
                }
                _ => {
                    let shape = match displacement {
                        Some(_) => OperandShape::Displacement,
                        None => OperandShape::Indirect,
                    };
                    self.malformed(shape, "the `)` is missing", Some(open));
                    None
                }
            }
        } else if self.program_counter_at(0) {
            self.bump();
            match self.peek().map(|token| token.kind) {
                Some(TokenKind::RightParen) => {
                    let close = self.bump().expect("a `)`");
                    match displacement {
                        Some(displacement) => Some(Operand::PcDisplacement {
                            displacement,
                            span: Span::new(start, close.span.end),
                        }),
                        None => {
                            self.malformed(
                                OperandShape::PcDisplacement,
                                "the displacement is missing",
                                None,
                            );
                            None
                        }
                    }
                }
                Some(TokenKind::Comma) => {
                    self.bump();
                    let index = self.parse_index_register(OperandShape::PcIndex)?;
                    let close = self.expect_close(open, OperandShape::PcIndex)?;
                    Some(Operand::PcIndex {
                        displacement,
                        index,
                        span: Span::new(start, close.span.end),
                    })
                }
                _ => {
                    self.malformed(
                        OperandShape::PcDisplacement,
                        "the `)` is missing",
                        Some(open),
                    );
                    None
                }
            }
        } else {
            let shape = match displacement {
                Some(_) => OperandShape::Displacement,
                None => OperandShape::Indirect,
            };
            let (problem, unclosed) = match self.peek() {
                Some(token) => {
                    // The message names this token, so it is stepped over
                    // before the Diagnostic is built: `malformed` spans up to
                    // the last token *read*, and a Diagnostic points at the
                    // characters it is about (ADR 0003). The `(` is not
                    // related here: it is closed, and the base is the mistake.
                    let problem = format!("`{}` is not an address register", self.text_of(token));
                    self.bump();
                    (problem, None)
                }
                None => ("the base register is missing".to_string(), Some(open)),
            };
            self.malformed(shape, problem, unclosed);
            None
        }
    }

    /// `index_register = register [ size_suffix ]` (2.5). No suffix means `.w`,
    /// the 68000's default.
    fn parse_index_register(&mut self, shape: OperandShape) -> Option<IndexRegister> {
        let register = match self.register_at(0) {
            Some(register) => register,
            None => {
                // Whatever stands where the index register should be is
                // stepped over, so that the span of the Diagnostic covers it
                // rather than stopping at the `,` before it (ADR 0003).
                self.bump();
                self.malformed(shape, "the index register is missing", None);
                return None;
            }
        };
        let token = self.bump().expect("a register");
        let suffix = self.take_size_suffix(token.span);
        let end = suffix.map_or(token.span.end, |suffix| suffix.span.end);
        Some(IndexRegister {
            register,
            size: suffix.and_then(|suffix| suffix.size),
            span: Span::new(token.span.start, end),
        })
    }

    /// An Operand that begins with an Expression: `absolute`, or the `x(An)`
    /// spelling of a displaced mode.
    fn parse_expression_operand(&mut self) -> Option<Operand> {
        let expression = match self.parse_expression() {
            Ok(expression) => expression,
            Err(TermError::Reported) => return None,
            Err(TermError::Absent) => {
                match self.peek() {
                    Some(token) => {
                        self.raise(
                            DiagnosticKind::UnexpectedTokenInOperand {
                                found: self.text_of(token).to_string(),
                            },
                            token.span,
                        );
                    }
                    None => self.raise(
                        DiagnosticKind::OperandExpected,
                        Span::empty(self.operand_start),
                    ),
                }
                return None;
            }
        };
        if self.peek().map(|token| token.kind) == Some(TokenKind::LeftParen) {
            return self.parse_displacement_tail(expression);
        }
        self.finish_expression_operand(expression)
    }

    /// The `(An)`, `(An,Xn)`, `(pc)` and `(pc,Xn)` tail of `x(An)` and its
    /// relatives: the cursor sits on the `(`.
    fn parse_displacement_tail(&mut self, displacement: Expr) -> Option<Operand> {
        let open = self.bump().expect("a `(`");
        self.parse_base_and_index(open, Some(displacement))
    }

    /// `absolute = expression [ size_suffix ]` (2.5), the forced widths
    /// `label.w` and `label.l` included.
    fn finish_expression_operand(&mut self, value: Expr) -> Option<Operand> {
        let start = self.operand_start;
        let suffix = self.take_size_suffix(value.span());
        let end = suffix.map_or(value.span().end, |suffix| suffix.span.end);
        Some(Operand::Absolute {
            value,
            size: suffix.and_then(|suffix| suffix.size),
            span: Span::new(start, end),
        })
    }

    /// The `size_suffix` at the cursor, read as `docs/grammar.md` 1.11 reads it
    /// in the Operand field: one character is a size, two or more are a dot
    /// inside a name, and none at all is "`.` is not a size".
    fn take_size_suffix(&mut self, preceding: Span) -> Option<ParsedSizeSuffix> {
        let token = self.peek()?;
        if token.kind != TokenKind::SizeSuffix {
            return None;
        }
        self.bump();
        let run = self.suffix_run(token.span);
        let size = if run.chars().count() >= 2 {
            let name = Span::new(preceding.start, token.span.end)
                .text(self.text)
                .to_string();
            self.raise(DiagnosticKind::DotInName { name }, token.span);
            None
        } else {
            match SizeSuffix::from_run(run) {
                Some(size) => Some(size),
                None => {
                    self.raise(
                        DiagnosticKind::UnknownSizeSuffix {
                            suffix: run.to_string(),
                        },
                        token.span,
                    );
                    None
                }
            }
        };
        Some(ParsedSizeSuffix {
            size,
            span: token.span,
        })
    }

    /// The `)` an Addressing mode needs, or the `malformed_operand` that names
    /// the shape it was trying to be.
    fn expect_close(&mut self, open: Token, shape: OperandShape) -> Option<Token> {
        match self.peek() {
            Some(token) if token.kind == TokenKind::RightParen => {
                self.bump();
                Some(token)
            }
            _ => {
                self.malformed(shape, "the `)` is missing", Some(open));
                None
            }
        }
    }

    // -- the Expressions --------------------------------------------------

    /// `expression` (2.7), as a Pratt loop over
    /// [`BinaryOperator::precedence`].
    fn parse_expression(&mut self) -> Result<Expr, TermError> {
        let left = self.parse_prefix()?;
        Ok(self.parse_expression_from(left, 0))
    }

    /// The loop of `parse_expression`, entered with a left term already read.
    ///
    /// `parenthesised_operand` needs that entry point: when a `(` … `)` turns
    /// out to have been a grouped Expression, the loop has to carry on with it
    /// as its left term (2.5, rule 2).
    fn parse_expression_from(&mut self, mut left: Expr, minimum: u8) -> Expr {
        while let Some(token) = self.peek() {
            let operator = match BinaryOperator::from_token_kind(token.kind) {
                Some(operator) => operator,
                None => break,
            };
            if operator.precedence() < minimum {
                break;
            }
            self.bump();
            let right = match self.parse_prefix() {
                Ok(right) => self.parse_expression_from(right, operator.precedence() + 1),
                Err(TermError::Reported) => break,
                Err(TermError::Absent) => {
                    self.raise(
                        DiagnosticKind::ExpressionExpected {
                            after: self.text_of(token).to_string(),
                        },
                        Span::empty(token.span.end),
                    );
                    break;
                }
            };
            let span = Span::new(left.span().start, right.span().end);
            left = Expr::Binary {
                operator,
                left: Box::new(left),
                right: Box::new(right),
                span,
            };
        }
        left
    }

    /// `unary_expression` and `primary_expression` (2.7), with the depth
    /// backstop of [`MAX_NESTING_DEPTH`] under them.
    ///
    /// Every recursion of the Expression parser passes through here — a `(`
    /// descends through [`parse_expression`](Self::parse_expression), a chained
    /// unary operator calls this directly — so counting the frames here counts
    /// them all. At the limit the descent stops with one Diagnostic and
    /// `TermError::Reported`, which every caller answers by staying quiet, so
    /// a line of three thousand `(` costs one message rather than the stack.
    fn parse_prefix(&mut self) -> Result<Expr, TermError> {
        if self.depth >= MAX_NESTING_DEPTH {
            let span = self
                .peek()
                .map_or_else(|| Span::empty(self.last_end()), |token| token.span);
            self.raise(
                DiagnosticKind::NestingTooDeep {
                    limit: MAX_NESTING_DEPTH,
                },
                span,
            );
            return Err(TermError::Reported);
        }
        self.depth += 1;
        let term = self.parse_prefix_term();
        self.depth -= 1;
        term
    }

    /// The body of [`parse_prefix`](Self::parse_prefix), which is where the
    /// terms of 2.7 are actually read.
    fn parse_prefix_term(&mut self) -> Result<Expr, TermError> {
        let token = match self.peek() {
            Some(token) => token,
            None => return Err(TermError::Absent),
        };
        match token.kind {
            TokenKind::Minus | TokenKind::Tilde => {
                self.bump();
                let operator =
                    UnaryOperator::from_token_kind(token.kind).expect("a unary operator");
                let operand = match self.parse_prefix() {
                    Ok(operand) => operand,
                    Err(TermError::Absent) => {
                        self.raise(
                            DiagnosticKind::ExpressionExpected {
                                after: operator.symbol().to_string(),
                            },
                            Span::empty(token.span.end),
                        );
                        return Err(TermError::Reported);
                    }
                    Err(error) => return Err(error),
                };
                let span = Span::new(token.span.start, operand.span().end);
                Ok(Expr::Unary {
                    operator,
                    operand: Box::new(operand),
                    span,
                })
            }
            TokenKind::Plus => {
                // EASy68K has no unary `+` and neither does s68k (1.13); the
                // term is parsed all the same, so that one Diagnostic is enough.
                self.bump();
                let term = self.parse_prefix();
                let written = match &term {
                    Ok(expression) => expression.span().text(self.text).to_string(),
                    Err(_) => String::new(),
                };
                self.raise(
                    DiagnosticKind::PlusIsNotAUnaryOperator { term: written },
                    token.span,
                );
                match term {
                    Ok(expression) => Ok(expression),
                    Err(_) => Err(TermError::Reported),
                }
            }
            TokenKind::Star => {
                self.bump();
                Ok(Expr::CurrentAddress { span: token.span })
            }
            TokenKind::Number(base) => {
                self.bump();
                Ok(self.number(token, base))
            }
            TokenKind::StringLiteral(quote) => {
                self.bump();
                Ok(Expr::CharacterLiteral {
                    bytes: token::string_literal_bytes(self.text_of(token)),
                    quote,
                    span: token.span,
                })
            }
            TokenKind::Identifier => {
                let name = self.text_of(token).to_string();
                self.bump();
                if is_reserved_name(&name) {
                    // `symbol_reference` excludes the twenty-one reserved names
                    // (2.7), so `#a0+4` and `4+d0` are this and not a Symbol.
                    self.raise(
                        DiagnosticKind::RegisterInExpression { register: name },
                        token.span,
                    );
                    return Err(TermError::Reported);
                }
                Ok(Expr::Symbol {
                    name,
                    span: token.span,
                })
            }
            TokenKind::LocalIdentifier => {
                self.bump();
                Ok(Expr::Symbol {
                    name: self.text_of(token).to_string(),
                    span: token.span,
                })
            }
            TokenKind::LeftParen => {
                self.bump();
                let inner = match self.parse_expression() {
                    Ok(inner) => inner,
                    Err(TermError::Absent) => {
                        self.raise(
                            DiagnosticKind::ExpressionExpected {
                                after: "(".to_string(),
                            },
                            Span::empty(token.span.end),
                        );
                        return Err(TermError::Reported);
                    }
                    Err(error) => return Err(error),
                };
                match self.peek() {
                    Some(close) if close.kind == TokenKind::RightParen => {
                        self.bump();
                        Ok(with_span(
                            inner,
                            Span::new(token.span.start, close.span.end),
                        ))
                    }
                    _ => {
                        self.raise(DiagnosticKind::UnclosedParenthesis, token.span);
                        Err(TermError::Reported)
                    }
                }
            }
            // The tokenizer has already said what is wrong with it; stepping
            // over it is what keeps the line from cascading.
            TokenKind::Error => {
                self.bump();
                Err(TermError::Reported)
            }
            _ => Err(TermError::Absent),
        }
    }

    /// A `number` token as a value (1.8).
    fn number(&mut self, token: Token, base: NumberBase) -> Expr {
        let text = self.text_of(token);
        let digits = &text[base.prefix().len()..];
        let value = match token::parse_digits(base, digits) {
            Some(value) => value,
            None => {
                if !digits.is_empty() && digits.chars().all(|digit| base.is_digit(digit)) {
                    // Every digit belongs to the base, so what failed is the
                    // width: `invalid_number` is the tokenizer's and this is
                    // not it.
                    self.raise(
                        DiagnosticKind::NumberTooLarge {
                            text: text.to_string(),
                        },
                        token.span,
                    );
                }
                0
            }
        };
        Expr::Number {
            value,
            base,
            span: token.span,
        }
    }

    // -- the cursor over the Operand field's tokens ------------------------

    /// The token at the cursor.
    fn peek(&self) -> Option<Token> {
        self.tokens.get(self.index).copied()
    }

    /// The token `ahead` places past the cursor.
    fn peek_at(&self, ahead: usize) -> Option<Token> {
        self.tokens.get(self.index + ahead).copied()
    }

    /// Take the token at the cursor and step over it.
    fn bump(&mut self) -> Option<Token> {
        let token = self.peek();
        if token.is_some() {
            self.index += 1;
        }
        token
    }

    /// Whether the Operand field's tokens have run out.
    fn at_end(&self) -> bool {
        self.index >= self.tokens.len()
    }

    /// Where the last token read ended, which is where a Diagnostic about
    /// something missing points.
    fn last_end(&self) -> usize {
        match self.index.checked_sub(1).and_then(|at| self.tokens.get(at)) {
            Some(token) => token.span.end,
            None => self.operand_start,
        }
    }

    /// The register `ahead` places past the cursor, if that token is one.
    fn register_at(&self, ahead: usize) -> Option<Register> {
        let token = self.peek_at(ahead)?;
        if token.kind != TokenKind::Identifier {
            return None;
        }
        Register::parse(self.text_of(token), token.span)
    }

    /// The address register `ahead` places past the cursor, if that token is
    /// one. A data register is not one: `predecrement` and every displaced mode
    /// take an address register as their base (2.5).
    fn address_register_at(&self, ahead: usize) -> Option<Register> {
        self.register_at(ahead)
            .filter(|register| register.kind == RegisterKind::Address)
    }

    /// Whether the token `ahead` places past the cursor is `pc`.
    fn program_counter_at(&self, ahead: usize) -> bool {
        match self.peek_at(ahead) {
            Some(token) if token.kind == TokenKind::Identifier => {
                super::ast::is_program_counter(self.text_of(token))
            }
            _ => false,
        }
    }

    // -- odds and ends ------------------------------------------------------

    /// The text a token covers.
    fn text_of(&self, token: Token) -> &'a str {
        token.span.text(self.text)
    }

    /// The run of a `size_suffix` token, without its dot.
    fn suffix_run(&self, span: Span) -> &'a str {
        Span::new(span.start + 1, span.end).text(self.text)
    }

    /// Raise a Diagnostic about `span` of this line.
    fn raise(&mut self, kind: DiagnosticKind, span: Span) {
        let location = self.location(span);
        self.diagnostics.push(Diagnostic::new(kind, location));
    }

    /// `malformed_operand`: an Operand that started as a known Addressing mode
    /// and did not finish, named as the shape it was trying to be and never as
    /// "invalid syntax" ([ADR 0003](../../../docs/adr/0003-operands-are-parsed-independently-of-the-instruction.md)).
    fn malformed(&mut self, shape: OperandShape, problem: impl Into<String>, open: Option<Token>) {
        let span = Span::new(self.operand_start, self.last_end());
        let location = self.location(span);
        let mut diagnostic = Diagnostic::new(
            DiagnosticKind::MalformedOperand {
                shape,
                problem: problem.into(),
            },
            location,
        );
        if let Some(open) = open {
            let opened = self.location(open.span);
            diagnostic = diagnostic.with_related(opened, "this `(` is never closed");
        }
        self.diagnostics.push(diagnostic);
    }

    /// The Location of `span` on this line.
    fn location(&self, span: Span) -> Location {
        location(&self.file, self.line_index, self.text, span)
    }
}

/// The same Expression, carrying another span.
///
/// A grouped Expression has no node of its own — `(1+2)` *is* `1+2` — so the
/// parentheses reach the tree as the span of the Expression they hold, which is
/// what an underline in the editor needs.
fn with_span(expression: Expr, span: Span) -> Expr {
    match expression {
        Expr::Number { value, base, .. } => Expr::Number { value, base, span },
        Expr::CharacterLiteral { bytes, quote, .. } => {
            Expr::CharacterLiteral { bytes, quote, span }
        }
        Expr::Symbol { name, .. } => Expr::Symbol { name, span },
        Expr::CurrentAddress { .. } => Expr::CurrentAddress { span },
        Expr::Unary {
            operator, operand, ..
        } => Expr::Unary {
            operator,
            operand,
            span,
        },
        Expr::Binary {
            operator,
            left,
            right,
            ..
        } => Expr::Binary {
            operator,
            left,
            right,
            span,
        },
    }
}

/// Which kind of Comment field begins at `start` (1.6).
///
/// A `;` or a `*` there is the marker of an `explicit_comment`; anything else
/// is EASy68K's `bare_comment`, which is what the once-per-File suggestion of
/// [`parse_file`] is about.
fn comment_kind(text: &str, start: usize) -> CommentKind {
    match text[start..].chars().next() {
        Some(';') | Some('*') => CommentKind::Explicit,
        _ => CommentKind::Bare,
    }
}

/// Where `text[start..end]` ends once its trailing whitespace is dropped.
fn trimmed_end(text: &str, start: usize, end: usize) -> usize {
    let end = end.min(text.len());
    if end <= start {
        return start;
    }
    start + text[start..end].trim_end_matches(is_whitespace).len()
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- helpers ----------------------------------------------------------

    /// Parse one line of the test File.
    fn parse(text: &str) -> (Line, Vec<Diagnostic>) {
        parse_line(text, "main.m68k", 0)
    }

    /// The codes a line raises, in source order.
    fn codes(text: &str) -> Vec<&'static str> {
        parse(text)
            .1
            .iter()
            .map(|diagnostic| diagnostic.code())
            .collect()
    }

    /// The messages a line raises.
    fn messages(text: &str) -> Vec<String> {
        parse(text)
            .1
            .iter()
            .map(|diagnostic| diagnostic.message())
            .collect()
    }

    /// The hint of the one Diagnostic a line raises.
    fn hint(text: &str) -> Option<String> {
        let (_, diagnostics) = parse(text);
        assert_eq!(diagnostics.len(), 1, "{text:?} raised {:?}", codes(text));
        diagnostics[0].hint()
    }

    /// Parse a line that must raise nothing at all.
    fn clean(text: &str) -> Line {
        let (line, diagnostics) = parse(text);
        assert!(
            diagnostics.is_empty(),
            "{text:?} raised {:?}",
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.message())
                .collect::<Vec<_>>()
        );
        line
    }

    /// The Operation of a line that must raise nothing.
    fn operation(text: &str) -> Operation {
        clean(text)
            .operation
            .unwrap_or_else(|| panic!("{text:?} has no operation"))
    }

    /// The Operands of a line that must raise nothing, as
    /// [`describe`] writes them.
    fn shapes(text: &str) -> Vec<String> {
        operation(text).operands.iter().map(describe).collect()
    }

    /// The Operands of a line that may well raise something.
    fn shapes_of(text: &str) -> Vec<String> {
        parse(text)
            .0
            .operation
            .map(|operation| operation.operands.iter().map(describe).collect())
            .unwrap_or_default()
    }

    /// The one Operand of a line that must raise nothing.
    fn only(text: &str) -> String {
        let shapes = shapes(text);
        assert_eq!(shapes.len(), 1, "{text:?} has {} operands", shapes.len());
        shapes.into_iter().next().expect("one operand")
    }

    /// An Operand written back out, in the canonical form of
    /// `tests/corpus/README.md`: the shape and nothing of the spelling, so that
    /// `(4,a6)` and `4(a6)` read the same here because they *are* the same.
    fn describe(operand: &Operand) -> String {
        match operand {
            Operand::Immediate { value, .. } => format!("#{}", render(value)),
            Operand::DataRegisterDirect { register, .. }
            | Operand::AddressRegisterDirect { register, .. } => register.name().to_string(),
            Operand::SpecialRegister { register, .. } => register.name().to_string(),
            Operand::Indirect { register, .. } => format!("({})", register.name()),
            Operand::Postincrement { register, .. } => format!("({})+", register.name()),
            Operand::Predecrement { register, .. } => format!("-({})", register.name()),
            Operand::Displacement {
                displacement, base, ..
            } => format!("{}({})", render(displacement), base.name()),
            Operand::Index {
                displacement,
                base,
                index,
                ..
            } => format!(
                "{}({},{})",
                displacement.as_ref().map_or("0".to_string(), render),
                base.name(),
                describe_index(index)
            ),
            Operand::PcDisplacement { displacement, .. } => {
                format!("{}(pc)", render(displacement))
            }
            Operand::PcIndex {
                displacement,
                index,
                ..
            } => format!(
                "{}(pc,{})",
                displacement.as_ref().map_or("0".to_string(), render),
                describe_index(index)
            ),
            Operand::Absolute { value, size, .. } => format!(
                "{}{}",
                render(value),
                size.map(|size| size.suffix().to_string())
                    .unwrap_or_default()
            ),
            Operand::RegisterList { items, .. } => items
                .iter()
                .map(|item| match item {
                    RegisterListItem::Single { register, .. } => register.name().to_string(),
                    RegisterListItem::Range { from, to, .. } => {
                        format!("{}-{}", from.name(), to.name())
                    }
                })
                .collect::<Vec<_>>()
                .join("/"),
        }
    }

    /// An index register, always with its size, the `.w` default included.
    fn describe_index(index: &IndexRegister) -> String {
        format!(
            "{}{}",
            index.register.name(),
            index.size.unwrap_or(SizeSuffix::Word).suffix()
        )
    }

    /// An Expression, fully parenthesised, so that a precedence test reads as
    /// the tree it is asserting.
    fn render(expression: &Expr) -> String {
        match expression {
            Expr::Number { value, .. } => value.to_string(),
            Expr::CharacterLiteral { bytes, .. } => format!(
                "'{}'",
                bytes.iter().map(|byte| *byte as char).collect::<String>()
            ),
            Expr::Symbol { name, .. } => name.clone(),
            Expr::CurrentAddress { .. } => "*".to_string(),
            Expr::Unary {
                operator, operand, ..
            } => format!("({}{})", operator.symbol(), render(operand)),
            Expr::Binary {
                operator,
                left,
                right,
                ..
            } => format!("({} {} {})", render(left), operator.symbol(), render(right)),
        }
    }

    /// One Expression, written as the Operand of an `org`.
    fn expression(text: &str) -> String {
        only(&format!("    org {text}"))
    }

    /// Every program of one half of `tests/corpus`, by file name.
    fn corpus(directory: &str) -> Vec<(String, String)> {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/corpus");
        let mut paths: Vec<std::path::PathBuf> = std::fs::read_dir(root.join(directory))
            .unwrap_or_else(|error| panic!("cannot read {directory}: {error}"))
            .map(|entry| entry.expect("a corpus entry").path())
            .filter(|path| path.is_file())
            .collect();
        paths.sort();
        paths
            .into_iter()
            .map(|path| {
                let name = path
                    .file_name()
                    .expect("a file name")
                    .to_string_lossy()
                    .to_string();
                let text = std::fs::read_to_string(&path)
                    .unwrap_or_else(|error| panic!("cannot read {name}: {error}"));
                (name, text)
            })
            .collect()
    }

    // -- section 1, the lexical rules -------------------------------------

    mod lexical_rules {
        use super::*;

        #[test]
        fn case_rule_lowers_operations_and_keeps_symbols() {
            assert_eq!(operation("    MOVE.B d0,d1").lowercase_name(), "move");
            assert_eq!(operation("    MOVE.B d0,d1").size, Some(SizeSuffix::Byte));
            assert_eq!(operation("    move.B D0,d1").operands.len(), 2);
            assert_eq!(shapes("    move.l Count,d0"), ["Count", "d0"]);
            assert_eq!(shapes("    move.l count,d0"), ["count", "d0"]);
        }

        #[test]
        fn label_rule_reads_the_table_of_one_four() {
            // The nine rows of `docs/grammar.md` 1.4, in order.
            let rows: [(&str, Option<&str>, Option<&str>); 9] = [
                ("loop:  move.l d0,d1", Some("loop"), Some("move")),
                ("end:   move.l d0,d1", Some("end"), Some("move")),
                (".retry: move.l d0,d1", Some(".retry"), Some("move")),
                ("loop   move.l d0,d1", Some("loop"), Some("move")),
                ("clr    d0", None, Some("clr")),
                ("dc.b   1", None, Some("dc")),
                ("foo.b  d0", None, Some("foo")),
                ("   loop", None, Some("loop")),
                ("   move.l d0,d1", None, Some("move")),
            ];
            for (text, label, operation) in rows {
                let line = clean(text);
                assert_eq!(
                    line.label.as_ref().map(|label| label.name.as_str()),
                    label,
                    "the label of {text:?}"
                );
                assert_eq!(
                    line.operation
                        .as_ref()
                        .map(|operation| operation.lowercase_name()),
                    operation.map(|name| name.to_string()),
                    "the operation of {text:?}"
                );
            }
        }

        #[test]
        fn label_rule_refuses_a_register_name() {
            assert_eq!(codes("d0:  move.l d0,d1"), vec!["reserved_name_as_symbol"]);
            assert_eq!(codes("pc:"), vec!["reserved_name_as_symbol"]);
            assert_eq!(codes("sp:"), vec!["reserved_name_as_symbol"]);
            // The Label is still recorded, so the rest of the line is read as
            // it was written.
            assert_eq!(
                parse("d0:").0.label.map(|label| label.name),
                Some("d0".into())
            );
        }

        #[test]
        fn label_rule_takes_one_label_a_line() {
            assert_eq!(codes("foo: bar: nop"), vec!["two_labels_on_one_line"]);
            let line = parse("foo: bar: nop").0;
            assert_eq!(line.label.map(|label| label.name), Some("foo".into()));
            assert_eq!(
                line.operation.map(|operation| operation.lowercase_name()),
                Some("nop".into()),
                "the operation after the second label is still read"
            );
        }

        #[test]
        fn a_colon_follows_its_name_directly() {
            // A field holds no whitespace (2.1), so `loop :` is a column 1
            // Label and then a `:` with no name before it.
            assert_eq!(codes("loop : nop"), ["empty_label"]);
            assert_eq!(parse("loop : nop").0.label.unwrap().name, "loop");
            assert_eq!(
                parse("loop : nop").0.operation.unwrap().lowercase_name(),
                "nop"
            );
        }

        #[test]
        fn local_identifier_opens_with_a_dot() {
            assert_eq!(clean(".retry:").label.unwrap().name, ".retry");
            assert!(clean(".retry:").label.unwrap().is_local());
            assert_eq!(only("    bra .loop"), ".loop");
            // A `.` in column 1 is a Local label and not a bare size suffix.
            assert_eq!(clean(".l").label.unwrap().name, ".l");
        }

        #[test]
        fn operand_field_extent_ends_at_whitespace_not_beside_a_comma() {
            assert_eq!(shapes("    move.b #23,d0   trap task 23"), ["#23", "d0"]);
            assert_eq!(
                clean("    move.b #23,d0   trap task 23")
                    .comment
                    .unwrap()
                    .text,
                "trap task 23"
            );
            assert_eq!(shapes("    lea greeting, a1"), ["greeting", "a1"]);
            assert_eq!(shapes("    move.w (a0, d5), d6"), ["0(a0,d5.w)", "d6"]);
        }

        #[test]
        fn operand_field_extent_keeps_whitespace_inside_a_quoted_literal() {
            assert_eq!(
                shapes("    dc.b '    X     Y   Left  Rght',13,10"),
                ["'    X     Y   Left  Rght'", "13", "10"]
            );
        }

        #[test]
        fn operand_field_extent_ends_at_a_semicolon_at_any_depth() {
            // Rule 2 outweighs the open parenthesis, so the report is about the
            // parenthesis and not about the Comment.
            assert_eq!(codes("    move.l (a0,d1  ; save it"), ["malformed_operand"]);
            assert_eq!(
                clean("    move.l d0,d1 ; copy").comment.unwrap().text,
                "; copy"
            );
        }

        #[test]
        fn comment_rule_reads_the_three_kinds() {
            assert_eq!(
                clean("* a whole line").comment.unwrap().kind,
                CommentKind::Line
            );
            assert_eq!(
                clean("  ; indented").comment.unwrap().kind,
                CommentKind::Line
            );
            assert_eq!(
                clean("    nop ; done").comment.unwrap().kind,
                CommentKind::Explicit
            );
            assert_eq!(
                clean("    move.l d0,d1  * copy").comment.unwrap().kind,
                CommentKind::Explicit
            );
            let bare = clean("    move.b #23,d0   trap task 23");
            assert_eq!(bare.comment.as_ref().unwrap().kind, CommentKind::Bare);
            assert!(bare.bare_comment, "the flag mirrors the kind");
        }

        #[test]
        fn comment_rule_reads_a_star_by_where_it_sits() {
            // The table of 1.6, row by row.
            assert!(clean("* comment").is_comment_line());
            assert_eq!(
                clean("    move.l d0,d1 * comment").comment.unwrap().kind,
                CommentKind::Explicit
            );
            assert_eq!(
                clean("    nop * comment").comment.unwrap().kind,
                CommentKind::Bare,
                "the `*` opened the operand field, so the comment field starts after it"
            );
            assert_eq!(expression("*"), "*");
            assert_eq!(expression("(*+1)&-2"), "((* + 1) & (-2))");
            assert_eq!(expression("12*60*60*100"), "(((12 * 60) * 60) * 100)");
        }

        #[test]
        fn comment_rule_reads_a_star_after_a_label() {
            // Row 2 of the table of 1.6 on a line with no Operation field
            // (2.2): the `*` is the first non-blank character of the Comment
            // field, so it marks it, exactly as EASy68K writes it.
            let cases = [
                ("start: * entry point", "start", "* entry point"),
                ("start * entry point", "start", "* entry point"),
                ("loop: *", "loop", "*"),
            ];
            for (text, label, comment) in cases {
                let line = clean(text);
                assert_eq!(line.label.as_ref().unwrap().name, label, "for {text:?}");
                assert!(line.operation.is_none(), "for {text:?}");
                let read = line.comment.as_ref().unwrap();
                assert_eq!(read.kind, CommentKind::Explicit, "for {text:?}");
                assert_eq!(read.text, comment, "for {text:?}");
                assert!(!line.bare_comment, "for {text:?}");
            }
            assert_eq!(
                clean("start: ; entry point").comment.unwrap().text,
                "; entry point",
                "the `;` spelling of the same line is unchanged"
            );
        }

        #[test]
        fn number_reads_the_four_bases() {
            assert_eq!(expression("10"), "10");
            assert_eq!(expression("$ff"), "255");
            assert_eq!(expression("%1010"), "10");
            assert_eq!(expression("@17"), "15");
            assert_eq!(expression("010"), "10", "a leading zero means nothing");
        }

        #[test]
        fn number_too_large_is_reported_by_the_parser() {
            assert_eq!(
                codes("    move.l #$ffffffffffffffffff,d0"),
                ["number_too_large"]
            );
            assert_eq!(
                codes("    move.l #$7fffffffffffffff,d0"),
                Vec::<&str>::new(),
                "the largest value that fits is not too large"
            );
        }

        #[test]
        fn character_literal_reads_a_doubled_quote() {
            assert_eq!(expression("'A'"), "'A'");
            assert_eq!(expression("'ab'"), "'ab'");
            assert_eq!(expression("''''"), "'''");
            assert_eq!(expression("''"), "''");
        }

        #[test]
        fn string_literal_may_be_double_quoted() {
            // Accepted, and the quote is kept: the `double_quoted_string`
            // suggestion is once per File and so is `parse_file`'s (1.9).
            let line = clean("    dc.b \"Hi\",0");
            let operands = line.operation.unwrap().operands;
            match &operands[0] {
                Operand::Absolute {
                    value: Expr::CharacterLiteral { quote, bytes, .. },
                    ..
                } => {
                    assert_eq!(*quote, QuoteKind::Double);
                    assert_eq!(bytes, b"Hi");
                }
                other => panic!("expected a literal, found {other:?}"),
            }
        }

        #[test]
        fn register_reads_sp_as_a7_and_pc_only_in_a_mode() {
            assert_eq!(shapes("    move.l sp,d0"), ["a7", "d0"]);
            assert_eq!(shapes("    move.l -(sp),d0"), ["-(a7)", "d0"]);
            assert_eq!(codes("    move.l pc,d0"), ["register_in_expression"]);
        }

        #[test]
        fn special_register_stands_at_any_position() {
            assert_eq!(shapes("    andi.w #$00,SR"), ["#0", "sr"]);
            assert_eq!(shapes("    move sr,d2"), ["sr", "d2"]);
            assert_eq!(shapes("    move.b ccr,d0"), ["ccr", "d0"]);
            assert_eq!(shapes("    move.l usp,a0"), ["usp", "a0"]);
        }

        #[test]
        fn size_suffix_is_read_by_the_field_it_stands_in() {
            assert_eq!(operation("    move.l d0,d1").size, Some(SizeSuffix::Long));
            assert_eq!(operation("    bra.s target").size, Some(SizeSuffix::Short));
            assert_eq!(codes("    move.q d0,d1"), ["unknown_size_suffix"]);
            assert_eq!(codes("    move.ll d0,d1"), ["unknown_size_suffix"]);
            assert_eq!(codes("    move. d0,d1"), ["unknown_size_suffix"]);
            assert_eq!(messages("    move. d0,d1"), ["`.` is not a size"]);
            // In the Operand field one letter is a size and a longer run is a
            // dot inside a name (1.11).
            assert_eq!(shapes("    move.l label.w,d0"), ["label.w", "d0"]);
            assert_eq!(codes("    move.l array.length,d0"), ["dot_in_name"]);
            assert_eq!(codes("    move.l label.q,d0"), ["unknown_size_suffix"]);
        }

        #[test]
        fn register_list_reads_ranges_and_slashes() {
            assert_eq!(
                shapes("    movem.l d0-d3/a0-a2,-(sp)"),
                ["d0-d3/a0-a2", "-(a7)"]
            );
            assert_eq!(shapes("    movem.l d1/d3/d5,-(sp)"), ["d1/d3/d5", "-(a7)"]);
            assert_eq!(
                shapes("    movem.l d1,-(a7)")[0],
                "d1",
                "a lone register is a register direct Operand, not a list of one"
            );
        }

        #[test]
        fn register_range_may_cross_from_d7_into_a0() {
            assert_eq!(shapes("    movem.l d0-a6,-(sp)"), ["d0-a6", "-(a7)"]);
        }

        #[test]
        fn register_range_out_of_order_is_an_error() {
            assert_eq!(
                codes("    movem.l d5-d2,-(sp)"),
                ["register_range_out_of_order"]
            );
            assert_eq!(
                codes("    movem.l a2-d5,-(sp)"),
                ["register_range_out_of_order"]
            );
            assert_eq!(
                hint("    movem.l d5-d2,-(sp)"),
                Some("write `d2-d5`".to_string())
            );
        }
    }

    // -- section 2, the grammar -------------------------------------------

    mod the_grammar {
        use super::*;

        #[test]
        fn source_line_reads_a_blank_line_and_a_comment_line() {
            assert!(clean("").is_blank());
            assert!(clean("      ").is_blank());
            assert!(clean("* comment").is_comment_line());
            assert!(clean("   ; comment").is_comment_line());
        }

        #[test]
        fn code_line_holds_up_to_four_fields() {
            let line = clean("start:  move.l #10,d0   set the counter");
            assert_eq!(line.label.unwrap().name, "start");
            let operation = line.operation.unwrap();
            assert_eq!(operation.name, "move");
            assert_eq!(operation.size, Some(SizeSuffix::Long));
            assert_eq!(operation.operands.len(), 2);
            assert_eq!(line.comment.unwrap().text, "set the counter");
        }

        #[test]
        fn label_field_reads_both_spellings() {
            assert!(clean("loop:  nop").label.unwrap().colon);
            assert!(!clean("loop   nop").label.unwrap().colon);
            assert_eq!(
                clean("loop:").label.unwrap().span,
                Span::new(0, 4),
                "the colon is not part of the name"
            );
        }

        #[test]
        fn operation_field_reads_a_name_a_size_and_no_operands() {
            let operation = operation("    rts");
            assert_eq!(operation.name, "rts");
            assert_eq!(operation.size, None);
            assert!(operation.operands.is_empty());
            assert_eq!(operation.operand_field_span, None);
        }

        #[test]
        fn whitespace_before_the_operand_field_is_optional() {
            // s68k is lenient here where the help is not (2.4, and the note
            // under it): the tokenizer has already separated the fields, so
            // nothing is ambiguous and there is nothing to diagnose.
            assert_eq!(shapes("    move.l#5,d0"), ["#5", "d0"]);
            assert_eq!(shapes("    dc.b'x'"), ["'x'"]);
            assert_eq!(shapes("loop:move.l d0,d1"), ["d0", "d1"]);
        }

        #[test]
        fn operand_list_separates_at_commas() {
            assert_eq!(shapes("    dc.b 1,2,3"), ["1", "2", "3"]);
            assert_eq!(shapes_of("    dc.b 1, 2 ,3"), ["1", "2", "3"]);
        }

        #[test]
        fn immediate_is_a_hash_and_an_expression() {
            assert_eq!(only("    move.l #5"), "#5");
            assert_eq!(shapes("    move.l #$1f,d0"), ["#31", "d0"]);
            assert_eq!(shapes("    move.l #'A',d0"), ["#'A'", "d0"]);
        }

        #[test]
        fn register_direct_reads_both_banks() {
            assert_eq!(shapes("    move.l d3,a6"), ["d3", "a6"]);
        }

        #[test]
        fn indirect_postincrement_and_predecrement() {
            assert_eq!(shapes("    move.l (a0),(a1)"), ["(a0)", "(a1)"]);
            assert_eq!(shapes("    move.l (a0)+,-(a1)"), ["(a0)+", "-(a1)"]);
        }

        #[test]
        fn displacement_reads_both_spellings() {
            assert_eq!(shapes("    move.l 4(a6),d0"), ["4(a6)", "d0"]);
            assert_eq!(shapes("    move.l (4,a6),d0"), ["4(a6)", "d0"]);
            assert_eq!(shapes("    move.l -4(a6),d0"), ["(-4)(a6)", "d0"]);
        }

        #[test]
        fn index_reads_all_three_spellings() {
            assert_eq!(shapes("    move.l 4(a6,d1.w),d0"), ["4(a6,d1.w)", "d0"]);
            assert_eq!(shapes("    move.l (4,a6,d1.w),d0"), ["4(a6,d1.w)", "d0"]);
            assert_eq!(
                shapes("    move.l (a6,d1),d0"),
                ["0(a6,d1.w)", "d0"],
                "the displacement-free form is a displacement of zero"
            );
        }

        #[test]
        fn pc_displacement_and_pc_index_read_both_spellings() {
            assert_eq!(shapes("    lea label(pc),a0"), ["label(pc)", "a0"]);
            assert_eq!(shapes("    lea (label,pc),a0"), ["label(pc)", "a0"]);
            assert_eq!(
                shapes("    lea label(pc,d1.w),a0"),
                ["label(pc,d1.w)", "a0"]
            );
            assert_eq!(
                shapes("    lea (label,pc,d1.l),a0"),
                ["label(pc,d1.l)", "a0"]
            );
            assert_eq!(shapes("    lea (pc,d1.w),a0"), ["0(pc,d1.w)", "a0"]);
        }

        #[test]
        fn absolute_reads_a_bare_expression_and_a_forced_width() {
            assert_eq!(shapes("    move.l $1000,d0"), ["4096", "d0"]);
            assert_eq!(shapes("    move.l ($1000),d0"), ["4096", "d0"]);
            assert_eq!(shapes("    move.l label.w,d0"), ["label.w", "d0"]);
            assert_eq!(shapes("    move.l label.l,d0"), ["label.l", "d0"]);
            assert_eq!(shapes("    bra done"), ["done"]);
        }

        #[test]
        fn index_register_defaults_to_word() {
            let operand = &operation("    move.l 4(a6,d1),d0").operands[0];
            match operand {
                Operand::Index { index, .. } => assert_eq!(index.size, None),
                other => panic!("expected an index operand, found {other:?}"),
            }
            assert_eq!(describe(operand), "4(a6,d1.w)");
        }

        #[test]
        fn parenthesised_operand_is_decided_after_the_close() {
            // Rule 1: a register and then `)` or `,`.
            assert_eq!(only("    jmp (a0)"), "(a0)");
            // Rule 2, a `)` then a binary operator: a grouped Expression.
            assert_eq!(expression("(640-2)/2"), "((640 - 2) / 2)");
            // Rule 2, a `)` then a `(`: the displacement of a displaced mode.
            assert_eq!(shapes("    move.l (4)(a0),d1"), ["4(a0)", "d1"]);
            // Rule 2, a `)` and nothing else: an absolute.
            assert_eq!(shapes("    move.l ($1000).w,d0"), ["4096.w", "d0"]);
        }

        #[test]
        fn text_operand_field_is_never_tokenized() {
            let operation = operation("    include io.x68");
            assert!(operation.operands.is_empty());
            assert_eq!(operation.text.unwrap().text, "io.x68");
        }

        #[test]
        fn file_specification_keeps_its_spaces_when_quoted() {
            assert_eq!(
                operation("    include 'input output macros.x68'")
                    .text
                    .unwrap()
                    .text,
                "'input output macros.x68'"
            );
            assert_eq!(
                operation("    incbin ..\\lib\\data.bin").text.unwrap().text,
                "..\\lib\\data.bin",
                "a backslash is a character and a dot is no size suffix"
            );
            let line = clean("    include io.x68   ; the io macros");
            assert_eq!(line.operation.unwrap().text.unwrap().text, "io.x68");
            assert_eq!(line.comment.unwrap().text, "; the io macros");

            // The field ends by all four rules of 1.5, so what follows can be
            // EASy68K's bare Comment field and is read as one.
            let line = clean("    include io.x68  load it");
            assert_eq!(
                line.operation.as_ref().unwrap().text.as_ref().unwrap().text,
                "io.x68"
            );
            let comment = line.comment.as_ref().unwrap();
            assert_eq!(comment.kind, CommentKind::Bare);
            assert_eq!(comment.text, "load it");
            assert!(line.bare_comment, "the flag mirrors the kind");
        }

        #[test]
        fn message_text_keeps_its_commas() {
            assert_eq!(
                operation("    fail ERROR, Argument missing in call to foo macro.")
                    .text
                    .unwrap()
                    .text,
                "ERROR, Argument missing in call to foo macro."
            );
            let line = clean("    fail out of range ; and a comment");
            assert_eq!(line.operation.unwrap().text.unwrap().text, "out of range");
            assert_eq!(line.comment.unwrap().text, "; and a comment");
        }

        #[test]
        fn raw_operand_field_keeps_a_structured_control_line_whole() {
            let operation = operation("    if.l d1 <hs> #NOON then.s");
            assert_eq!(operation.lowercase_name(), "if");
            assert_eq!(operation.size, Some(SizeSuffix::Long));
            assert_eq!(operation.text.unwrap().text, "d1 <hs> #NOON then.s");
            let line = clean("    if <cs> then.s                ; if set");
            assert_eq!(line.operation.unwrap().text.unwrap().text, "<cs> then.s");
            assert_eq!(line.comment.unwrap().text, "; if set");
        }

        #[test]
        fn refused_operation_is_an_operation_in_column_one() {
            assert_eq!(clean("endm").operation.unwrap().lowercase_name(), "endm");
            assert!(clean("endm").label.is_none());
        }

        #[test]
        fn macro_definition_is_skipped_whole() {
            let text = "DELAY   MACRO\n    move.l  #\\1,d1\n    ENDM\n    nop\n";
            let parsed = parse_file("main.m68k", text);
            assert!(
                parsed.diagnostics.is_empty(),
                "the body raised {:?}",
                parsed
                    .diagnostics
                    .iter()
                    .map(|diagnostic| diagnostic.code())
                    .collect::<Vec<_>>()
            );
            assert_eq!(parsed.lines.len(), 4);
            assert_eq!(
                parsed.lines[0].label.as_ref().unwrap().name,
                "DELAY",
                "the `macro` line itself is read"
            );
            assert!(parsed.lines[1].is_blank(), "the body is not tokenized");
            assert!(parsed.lines[2].is_blank(), "nor is the `endm` line");
            assert_eq!(
                parsed.lines[3].operation.as_ref().unwrap().lowercase_name(),
                "nop",
                "the File carries on after the definition"
            );
        }

        #[test]
        fn macro_definition_ends_on_an_operation_named_endm() {
            // 2.6 ends the skip on a line whose *Operation* is `endm`, which is
            // not the same as a line holding the word: `bra endm` branches to a
            // Label called `endm` and the definition runs on.
            let text = "DELAY   MACRO\n    bra endm\n    move.l #1,d1\n    ENDM\n    nop\n";
            let parsed = parse_file("main.m68k", text);
            assert!(parsed.diagnostics.is_empty(), "the body was read");
            assert!(
                parsed.lines[1].is_blank(),
                "`bra endm` is body, not the end"
            );
            assert!(parsed.lines[2].is_blank());
            assert!(parsed.lines[3].is_blank(), "the `ENDM` line closes it");
            assert_eq!(
                parsed.lines[4].operation.as_ref().unwrap().lowercase_name(),
                "nop"
            );

            // A Label may sit in front of the `endm`, in either spelling, and a
            // Comment line never ends the skip.
            for closing in ["    endm", "done: endm", "done endm", "ENDM"] {
                let text = format!("DELAY MACRO\n* endm in a comment\n{closing}\n    nop\n");
                let parsed = parse_file("main.m68k", &text);
                assert!(parsed.lines[1].is_blank(), "for {closing:?}");
                assert!(parsed.lines[2].is_blank(), "for {closing:?}");
                assert_eq!(
                    parsed.lines[3].operation.as_ref().unwrap().lowercase_name(),
                    "nop",
                    "{closing:?} should close the definition"
                );
            }
        }

        #[test]
        fn unterminated_macro_definition_is_reported_against_the_macro_line() {
            let parsed = parse_file("main.m68k", "DELAY   MACRO\n    move.l #1,d1\n");
            assert_eq!(
                parsed
                    .diagnostics
                    .iter()
                    .map(|diagnostic| diagnostic.code())
                    .collect::<Vec<_>>(),
                ["unterminated_macro_definition"]
            );
            assert_eq!(parsed.diagnostics[0].location.line, 0);
        }

        #[test]
        fn expression_follows_the_easy68k_precedence_table() {
            // `>> <<`, then `& ! | ^`, then `* / \`, then `+ -`.
            assert_eq!(expression("1<<2+3"), "((1 << 2) + 3)");
            assert_eq!(expression("1+2<<3"), "(1 + (2 << 3))");
            assert_eq!(
                expression("1!2&3"),
                "((1 | 2) & 3)",
                "`!` and `|` are one operator, and `|` is how a message writes it"
            );
            assert_eq!(expression("1&2*3"), "((1 & 2) * 3)");
            assert_eq!(expression("1*2+3"), "((1 * 2) + 3)");
            assert_eq!(expression("(800<<16+600)"), "((800 << 16) + 600)");
            assert_eq!(expression("COLS*2"), "(COLS * 2)");
        }

        #[test]
        fn additive_and_multiplicative_expressions_associate_left() {
            assert_eq!(expression("1-2-3"), "((1 - 2) - 3)");
            assert_eq!(expression("8/2\\3"), "((8 / 2) \\ 3)");
        }

        #[test]
        fn unary_expression_reads_minus_and_tilde() {
            assert_eq!(expression("-2"), "(-2)");
            assert_eq!(expression("~5"), "(~5)");
            assert_eq!(
                expression("-2*3"),
                "((-2) * 3)",
                "a unary operator binds tighter than any binary one"
            );
        }

        #[test]
        fn a_unary_operator_may_be_chained() {
            // The layered grammar writes one `[ unary_operator ]`; accepting
            // more is the lenient direction and costs nothing (2.7's note).
            assert_eq!(expression("--5"), "(-(-5))");
            assert_eq!(expression("~-5"), "(~(-5))");
        }

        #[test]
        fn primary_expression_reads_every_term() {
            assert_eq!(expression("$10"), "16");
            assert_eq!(expression("'A'"), "'A'");
            assert_eq!(expression("count"), "count");
            assert_eq!(expression(".local"), ".local");
            assert_eq!(expression("*"), "*");
            assert_eq!(expression("(1+2)"), "(1 + 2)");
        }

        #[test]
        fn grouped_expression_keeps_its_parentheses_in_the_span() {
            let operand = &operation("    org (1+2)").operands[0];
            assert_eq!(operand.span(), Span::new(8, 13));
            match operand {
                Operand::Absolute { value, .. } => assert_eq!(value.span(), Span::new(8, 13)),
                other => panic!("expected an absolute operand, found {other:?}"),
            }
        }

        #[test]
        fn symbol_reference_excludes_the_reserved_names() {
            assert_eq!(codes("    move.l #a0+4,d0"), ["register_in_expression"]);
            assert_eq!(codes("    add.l 4+d0,d1"), ["register_in_expression"]);
            assert_eq!(
                hint("    move.l #a0+4,d0"),
                Some(
                    "an expression is computed while assembling, when no register has a value yet"
                        .to_string()
                )
            );
        }
    }

    // -- section 3, the ambiguities ---------------------------------------

    mod ambiguities {
        use super::*;

        #[test]
        fn a_label_in_column_one_needs_no_colon() {
            // 3.1
            let line = clean("loop move.l d0,d1");
            assert_eq!(line.label.unwrap().name, "loop");
            assert_eq!(line.operation.unwrap().lowercase_name(), "move");
            // Indent it and it is an Operation instead.
            let indented = clean("    loop move.l d0,d1");
            assert!(indented.label.is_none());
            assert_eq!(indented.operation.unwrap().lowercase_name(), "loop");
        }

        #[test]
        fn a_mnemonic_in_column_one_is_the_instruction() {
            // 3.2
            assert!(clean("clr d0").label.is_none());
            assert_eq!(clean("clr d0").operation.unwrap().lowercase_name(), "clr");
            assert_eq!(clean("clr: d0").label.unwrap().name, "clr");
        }

        #[test]
        fn a_misspelt_mnemonic_keeps_its_operands() {
            // 3.3
            let operation = operation("mvoe.l d0,d1");
            assert_eq!(operation.lowercase_name(), "mvoe");
            assert_eq!(operation.size, Some(SizeSuffix::Long));
            assert_eq!(operation.operands.len(), 2);
        }

        #[test]
        fn a_star_opening_the_operand_field_is_the_current_address() {
            // 3.4
            assert_eq!(shapes("    lea *,a0"), ["*", "a0"]);
            assert_eq!(only("    org *"), "*");
            // The cost: `nop * done here` reads the `*` as an Operand, which is
            // what the analyzer is given a sentence for.
            let line = clean("    nop * done here");
            assert_eq!(describe(&line.operation.unwrap().operands[0]), "*");
            assert_eq!(line.comment.unwrap().text, "done here");
        }

        #[test]
        fn an_expression_split_by_a_space_is_explained() {
            // 3.5, the six rows of the table.
            let fires = [
                "    move.l #2 * 3",
                "    move.l #2 * 3,d0",
                "NOON equ 12*60  * 60",
            ];
            for text in fires {
                assert!(
                    codes(text).contains(&"expression_split_by_space"),
                    "{text:?} raised {:?}",
                    codes(text)
                );
            }
            let quiet = [
                "    move.l d0,d1  * copy",
                "    dc.b 1  * one byte",
                "    movem.l d0-d2,-(a7)  * 3 registers saved",
            ];
            for text in quiet {
                assert_eq!(codes(text), Vec::<&str>::new(), "{text:?} should be quiet");
            }
        }

        #[test]
        fn expression_split_by_space_names_the_semicolon_only_for_a_bare_comment() {
            assert_eq!(
                hint("    move.l #2 * 3"),
                Some("write `#2*3`".to_string()),
                "the comment field already carries its `*` marker"
            );
            assert_eq!(
                hint("    move.l #2 - 3"),
                Some("write `#2-3`, or start a comment with `;`".to_string())
            );
        }

        #[test]
        fn a_space_before_a_comma_keeps_the_operand_field_open() {
            // 3.6
            assert_eq!(shapes_of("    move.l d0 ,d1"), ["d0", "d1"]);
            assert_eq!(codes("    move.l d0 ,d1"), ["space_before_comma"]);
            assert_eq!(
                shapes_of("    trap #15   , display Y"),
                ["#15", "display"],
                "which is the mistake the suggestion is really about"
            );
            assert_eq!(
                codes("    lea greeting, a1"),
                Vec::<&str>::new(),
                "a space after a comma is not the same thing"
            );
        }

        #[test]
        fn an_unmarked_comment_field_is_accepted() {
            // 3.7
            let line = clean("    trap #15   draw line from X1,Y1 to X2,Y2");
            assert_eq!(describe(&line.operation.unwrap().operands[0]), "#15");
            assert_eq!(
                line.comment.unwrap().text,
                "draw line from X1,Y1 to X2,Y2",
                "the field ended before `draw`, commas and all"
            );
        }

        #[test]
        fn a_parenthesis_is_decided_by_what_follows_it() {
            // 3.8
            assert_eq!(only("    jmp (a0)"), "(a0)");
            assert_eq!(shapes("    move.l (label),d0"), ["label", "d0"]);
            assert_eq!(
                expression("(640-COLS*SCALE)/2"),
                "((640 - (COLS * SCALE)) / 2)"
            );
        }

        #[test]
        fn a_minus_between_two_registers_is_a_range() {
            // 3.9
            assert_eq!(shapes("    movem.l d0-d3,-(a7)"), ["d0-d3", "-(a7)"]);
            assert_eq!(
                codes("    movem.l d0-3,-(a7)"),
                ["register_expected_in_register_list"]
            );
        }

        #[test]
        fn a_word_with_a_size_suffix_is_never_a_label() {
            // 3.10
            assert!(clean("dc.b 1,2,3").label.is_none());
            assert_eq!(
                clean("dc.b 1,2,3").operation.unwrap().lowercase_name(),
                "dc"
            );
            assert_eq!(clean("END:").label.unwrap().name, "END");
        }

        #[test]
        fn a_semicolon_beats_an_unclosed_parenthesis() {
            // 3.11, one of each.
            let (_, diagnostics) = parse("    move.l (a0,d1  ; save it");
            assert_eq!(diagnostics[0].code(), "malformed_operand");
            assert_eq!(
                diagnostics[0].message(),
                "this looks like an indexed operand, `4(a0,d1.w)`, but the `)` is missing"
            );
            assert_eq!(
                diagnostics[0].related.len(),
                1,
                "the `(` it was opened with is a related location"
            );
            assert_eq!(
                codes("    org (*+1&-2   ; word align"),
                ["unclosed_parenthesis"]
            );
        }
    }

    // -- section 4, the parser's Diagnostics -------------------------------

    mod parser_diagnostics {
        use super::*;
        use crate::assembler::diagnostics::ALL_CODES;

        /// One line per Diagnostic the parser raises: the table of
        /// `docs/grammar.md` section 4 minus the five the tokenizer raises and
        /// the three `parse_file` raises, in the order that table lists them.
        /// `the_table_covers_every_diagnostic_the_parser_raises` derives that
        /// list from [`ALL_CODES`] rather than restating it, so a new kind
        /// cannot be added without either a case here or a line in one of the
        /// exclusion lists there.
        const CASES: &[(&str, &str)] = &[
            ("number_too_large", "    move.l #$ffffffffffffffffff,d0"),
            ("unknown_size_suffix", "    move.q d0,d1"),
            ("dot_in_name", "    move.l array.length,d0"),
            ("reserved_name_as_symbol", "d0: nop"),
            ("two_labels_on_one_line", "foo: bar: nop"),
            ("operation_expected", "loop: 5"),
            ("empty_label", ": nop"),
            ("operand_expected", "    move.l d0,"),
            ("unclosed_parenthesis", "    org (1+2"),
            ("nesting_too_deep", TOO_DEEP),
            ("malformed_operand", "    move.l 4(d0),d1"),
            ("unexpected_token_in_operand", "    move.l d0),d1"),
            (
                "register_expected_in_register_list",
                "    movem.l d0-3,-(a7)",
            ),
            ("register_range_out_of_order", "    movem.l d5-d2,-(a7)"),
            ("register_in_expression", "    move.l #a0+4,d0"),
            ("expression_expected", "    move.l #,d0"),
            ("plus_is_not_a_unary_operator", "    move.l #+5,d0"),
            ("expression_split_by_space", "    move.l #2 * 3"),
            ("space_before_comma", "    move.l d0 ,d1"),
        ];

        /// The `nesting_too_deep` line of [`CASES`]: one `(` more than
        /// [`MAX_NESTING_DEPTH`], counted by the test below rather than by eye.
        const TOO_DEEP: &str =
            "    dc.l (((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((((1";

        #[test]
        fn every_case_raises_the_code_it_names() {
            assert_eq!(
                TOO_DEEP.matches('(').count(),
                MAX_NESTING_DEPTH + 1,
                "the `nesting_too_deep` case has to nest one past the limit"
            );
            for (code, text) in CASES {
                assert!(
                    codes(text).contains(code),
                    "{text:?} should raise `{code}`, raised {:?}",
                    codes(text)
                );
            }
        }

        #[test]
        fn the_table_covers_every_diagnostic_the_parser_raises() {
            // Derived from the one place every code is written down, so that
            // the table and section 4 cannot drift apart: everything but the
            // five the tokenizer raises, the three `parse_file` raises and the
            // analyzer's own, which no line of the parser can reach.
            const TOKENIZERS: [&str; 5] = [
                "character_above_latin1",
                "non_breaking_space",
                "unexpected_character",
                "unterminated_string",
                "invalid_number",
            ];
            const OVER_A_FILE: [&str; 3] = [
                "unterminated_macro_definition",
                "bare_comment",
                "double_quoted_string",
            ];
            // "Later" is the phase order of the design record, not the
            // calendar: the analyzer, the Directives and the Layout all raise
            // Diagnostics of their own, and none of them is the parser's.
            const LATER_PHASES: [&str; 37] = [
                "unknown_mnemonic",
                "mnemonic_used_as_label",
                "wrong_operand_count",
                "missing_comma_between_operands",
                "invalid_addressing_mode",
                "invalid_operand_pair",
                "both_operands_in_memory",
                "address_register_byte_size",
                "unimplemented_addressing_mode",
                "invalid_address_width",
                "value_out_of_range",
                "bare_number_as_address",
                "star_is_the_current_address",
                "invalid_size",
                "immediate_out_of_range",
                "unimplemented_operation",
                "symbol_already_defined",
                "undefined_symbol",
                "forward_reference_not_allowed",
                "division_by_zero",
                "character_literal_too_long",
                "constant_above_32_bits",
                "register_list_in_expression",
                "odd_origin",
                "address_used_twice",
                "code_after_end",
                "entry_point_case_mismatch",
                "end_without_an_address",
                "directive_needs_a_label",
                "label_not_allowed",
                "register_list_expected",
                "not_a_register_list",
                "register_list_not_defined_yet",
                "user_defined_error",
                "no_bytes_in_an_offset_region",
                "value_expected",
                "unreadable_file",
            ];
            let expected: Vec<&str> = ALL_CODES
                .iter()
                .copied()
                .filter(|code| {
                    !TOKENIZERS.contains(code)
                        && !OVER_A_FILE.contains(code)
                        && !LATER_PHASES.contains(code)
                })
                .collect();
            let covered: Vec<&str> = CASES.iter().map(|(code, _)| *code).collect();
            assert_eq!(covered, expected);
        }

        #[test]
        fn malformed_operand_names_the_shape_it_tried_to_be() {
            let cases = [
                (
                    "    move.l -(a7",
                    "this looks like a predecrement operand, `-(a0)`, but the `)` is missing",
                ),
                (
                    "    move.l (a0",
                    "this looks like an indirect operand, `(a0)`, but the `)` is missing",
                ),
                (
                    "    move.l (a0,",
                    "this looks like an indexed operand, `4(a0,d1.w)`, but the index register is missing",
                ),
                (
                    "    move.l 4(d0),d1",
                    "this looks like a displacement operand, `4(a0)`, but `d0` is not an address register",
                ),
                (
                    "    move.l (pc),d0",
                    "this looks like a PC-relative operand, `label(pc)`, but the displacement is missing",
                ),
                (
                    "    move.l #5.w,d0",
                    "this looks like an immediate operand, `#5`, but an immediate carries no size: the size goes on the operation, as in `move.w #5,d0`",
                ),
            ];
            for (text, message) in cases {
                assert_eq!(messages(text), [message.to_string()], "for {text:?}");
            }
        }

        #[test]
        fn unexpected_token_in_operand_answers_a_stray_parenthesis() {
            assert_eq!(
                hint("    move.l d0),d1"),
                Some("there is no `(` for this `)`".to_string())
            );
            assert_eq!(
                messages("    move.l 2**3,d0"),
                ["`3` was not expected here".to_string()],
                "`2**3` has no derivation: `*` is the current address (1.13)"
            );
        }

        #[test]
        fn plus_is_not_a_unary_operator_says_what_to_write() {
            assert_eq!(
                hint("    move.l #+5,d0"),
                Some("write `5`; the unary operators are `-` and `~`".to_string())
            );
        }

        #[test]
        fn dot_in_name_says_what_to_write() {
            assert_eq!(
                hint("    move.l array.length,d0"),
                Some(
                    "write `array_length`; a dot after a name is a size, `.b`, `.w`, `.l` or `.s`"
                        .to_string()
                )
            );
        }

        #[test]
        fn one_bad_character_costs_one_message() {
            // The tokenizer has already said what is wrong with the character
            // (step 3's rule), so no arm of the Operand parser says it again —
            // not the one that opens an Operand, and not the two that meet the
            // token after a complete one.
            assert_eq!(codes("    move.l ?,d0"), ["unexpected_character"]);
            assert_eq!(codes("    move.l #1<2,d0"), ["unexpected_character"]);
            assert_eq!(
                shapes_of("    move.l #1<2,d0"),
                ["#1", "d0"],
                "the Operands beside it survive"
            );
            assert_eq!(codes("    move.l (1<2),d0"), ["unexpected_character"]);
            assert_eq!(
                codes("    move.l d0,d\u{2019}1"),
                ["character_above_latin1"]
            );
        }

        #[test]
        fn operand_expected_is_raised_once_for_one_missing_operand() {
            assert_eq!(codes("    dc.b ,"), ["operand_expected"]);
            assert_eq!(codes("    move.l  ,d0"), ["operand_expected"]);
            assert_eq!(codes("    move.l d0,"), ["operand_expected"]);
            assert_eq!(
                codes("    dc.b ,,"),
                ["operand_expected", "operand_expected"],
                "two missing operands are two messages"
            );
        }

        #[test]
        fn nesting_too_deep_stops_the_descent_rather_than_the_stack() {
            // wasm's stack is a megabyte and its overflow is a trap, not a
            // Diagnostic, so this runs on a thread of exactly that size: the
            // assertion is as much that it comes back at all.
            let text = format!("    dc.l {}1", "(".repeat(5000));
            let read = std::thread::Builder::new()
                .stack_size(1 << 20)
                .spawn(move || {
                    let (line, diagnostics) = parse_line(&text, "main.m68k", 0);
                    let codes: Vec<&str> = diagnostics
                        .iter()
                        .map(|diagnostic| diagnostic.code())
                        .collect();
                    (line.operation.is_some(), codes.join(","))
                })
                .expect("a thread with wasm's stack")
                .join()
                .expect("the parser came back");
            assert_eq!(read, (true, "nesting_too_deep".to_string()));

            // What fits still parses, and one `(` too many is the limit.
            let deep = format!(
                "    dc.l {}1{}",
                "(".repeat(MAX_NESTING_DEPTH - 8),
                ")".repeat(MAX_NESTING_DEPTH - 8)
            );
            assert_eq!(only(&deep), "1");
        }

        #[test]
        fn a_diagnostic_points_at_the_characters_it_is_about() {
            let (_, diagnostics) = parse("    move.q d0,d1");
            assert_eq!(diagnostics[0].location.line, 0);
            assert_eq!(diagnostics[0].location.column, 8);
            assert_eq!(diagnostics[0].location.end_column, 10);
            assert_eq!(diagnostics[0].location.file, "main.m68k");

            // A `malformed_operand` covers the token its message names, which
            // has only been peeked at the moment the message is built.
            let (_, diagnostics) = parse("    move.l 4(d0),d1");
            assert_eq!(diagnostics[0].location.column, 11, "`4(d0`, not `4(`");
            assert_eq!(diagnostics[0].location.end_column, 15);
            let (_, diagnostics) = parse("    move.l (label,d0),d1");
            assert_eq!(diagnostics[0].location.column, 11);
            assert_eq!(diagnostics[0].location.end_column, 20, "`(label,d0`");
        }

        #[test]
        fn the_diagnostics_of_a_line_come_out_in_source_order() {
            let (_, diagnostics) = parse("    move.q array.length,d0");
            let codes: Vec<&str> = diagnostics
                .iter()
                .map(|diagnostic| diagnostic.code())
                .collect();
            assert_eq!(codes, ["unknown_size_suffix", "dot_in_name"]);
            assert!(diagnostics[0].location.column < diagnostics[1].location.column);
        }

        #[test]
        fn a_broken_operand_does_not_lose_the_rest_of_the_line() {
            let line = parse("loop:  move.l 4(d0),d1   ; a comment").0;
            assert_eq!(line.label.unwrap().name, "loop");
            let operation = line.operation.unwrap();
            assert_eq!(operation.lowercase_name(), "move");
            assert_eq!(operation.operands.len(), 1, "the second Operand survives");
            assert_eq!(line.comment.unwrap().text, "; a comment");
        }

        #[test]
        fn bare_comment_and_double_quoted_string_are_raised_once_a_file() {
            let text = "    move.b #23,d0   trap task 23\n    move.b #24,d0   another one\n";
            let parsed = parse_file("main.m68k", text);
            assert_eq!(
                parsed
                    .diagnostics
                    .iter()
                    .map(|diagnostic| diagnostic.code())
                    .collect::<Vec<_>>(),
                ["bare_comment"]
            );
            assert_eq!(parsed.diagnostics[0].location.line, 0);

            // A `file_specification` ends by the same four rules of 1.5, so a
            // bare Comment after one is a bare Comment and is the first of the
            // File when it comes first.
            let text = "    include io.x68  load it\n    move.l d0,d1  bare here\n";
            let parsed = parse_file("main.m68k", text);
            assert_eq!(
                parsed
                    .diagnostics
                    .iter()
                    .map(|diagnostic| diagnostic.code())
                    .collect::<Vec<_>>(),
                ["bare_comment"]
            );
            assert_eq!(parsed.diagnostics[0].location.line, 0);

            let text = "    dc.b \"one\",0\n    dc.b \"two\",0\n";
            let parsed = parse_file("main.m68k", text);
            assert_eq!(
                parsed
                    .diagnostics
                    .iter()
                    .map(|diagnostic| diagnostic.code())
                    .collect::<Vec<_>>(),
                ["double_quoted_string"]
            );
            assert_eq!(parsed.diagnostics[0].location.line, 0);
            assert_eq!(parsed.diagnostics[0].location.column, 9);
        }

        #[test]
        fn a_file_specification_never_suggests_a_single_quote() {
            let parsed = parse_file("main.m68k", "    include \"io.x68\"\n");
            assert!(
                parsed.diagnostics.is_empty(),
                "the help documents both quotes for a filename (1.9)"
            );
        }
    }

    // -- section 6, the corpus --------------------------------------------

    mod corpus_pass {
        use super::*;

        #[test]
        fn the_thirty_editor_programs_parse_with_no_diagnostic() {
            let programs = corpus("editor");
            assert_eq!(programs.len(), 30);
            let mut lines = 0;
            for (name, text) in programs {
                for (index, span) in split_lines(&text).into_iter().enumerate() {
                    let text_of_line = span.text(&text);
                    let (_, diagnostics) = parse_line(text_of_line, &name, index);
                    assert!(
                        diagnostics.is_empty(),
                        "{name}:{} {text_of_line:?} raised {:?}",
                        index + 1,
                        diagnostics
                            .iter()
                            .map(|diagnostic| diagnostic.message())
                            .collect::<Vec<_>>()
                    );
                    lines += 1;
                }
            }
            assert_eq!(
                lines, 8957,
                "the 30 editor programs are 8957 of the corpus's 9672 lines"
            );
        }

        #[test]
        fn the_label_rule_turns_on_column_one_and_nothing_else() {
            // Every indented line of the corpus, read again with its
            // indentation removed: the Operation of such a line is a Mnemonic
            // or a Directive name, so `label_rule` must read it the same way in
            // column 1 as it does indented. Anything else would mean the rule
            // was resting on the indentation.
            for (name, text) in corpus("editor") {
                for (index, span) in split_lines(&text).into_iter().enumerate() {
                    let line = span.text(&text);
                    let trimmed = line.trim_start_matches(is_whitespace);
                    if trimmed == line {
                        continue;
                    }
                    let (before, _) = parse_line(line, &name, index);
                    let (after, diagnostics) = parse_line(trimmed, &name, index);
                    assert!(
                        diagnostics.is_empty(),
                        "{name}:{} {trimmed:?} raised {:?} once unindented",
                        index + 1,
                        diagnostics
                            .iter()
                            .map(|diagnostic| diagnostic.code())
                            .collect::<Vec<_>>()
                    );
                    assert_eq!(
                        shape_of(&before),
                        shape_of(&after),
                        "{name}:{} {line:?} reads differently in column 1",
                        index + 1
                    );
                }
            }
        }

        /// A line's fields, without any of its spans, so that two readings of
        /// the same text at different columns can be compared.
        fn shape_of(line: &Line) -> String {
            format!(
                "{}|{}|{}",
                line.label
                    .as_ref()
                    .map(|label| label.name.clone())
                    .unwrap_or_default(),
                line.operation
                    .as_ref()
                    .map(|operation| format!(
                        "{}{}({})",
                        operation.lowercase_name(),
                        operation
                            .size
                            .map(|size| size.suffix().to_string())
                            .unwrap_or_default(),
                        operation
                            .operands
                            .iter()
                            .map(describe)
                            .collect::<Vec<_>>()
                            .join(",")
                    ))
                    .unwrap_or_default(),
                line.comment
                    .as_ref()
                    .map(|comment| comment.text.clone())
                    .unwrap_or_default()
            )
        }

        #[test]
        fn every_line_of_the_corpus_serialises() {
            // The hover API crosses into TypeScript as JSON, so every shape the
            // tree can take has to survive `Serialize`. The corpus is the
            // widest set of shapes there is. `bad-apple.x68` is skipped by
            // size, as `tests/corpus/README.md` allows: its 6526 `dc.b` lines
            // are one shape and cost ten seconds to serialise.
            for directory in ["editor", "easy68k"] {
                for (name, text) in corpus(directory) {
                    if text.len() > 1_000_000 {
                        continue;
                    }
                    for (index, span) in split_lines(&text).into_iter().enumerate() {
                        let (line, _) = parse_line(span.text(&text), &name, index);
                        serde_json::to_value(&line)
                            .unwrap_or_else(|error| panic!("{name}:{} {error}", index + 1));
                    }
                }
            }
        }

        #[test]
        fn the_easy68k_originals_only_suggest_the_bare_comment_field() {
            // The three EASy68K originals hold everything the rewrite refuses —
            // a Macro definition, conditional assembly, structured control — and
            // none of it is a parser Diagnostic: the Macro body is skipped and
            // the refused keywords take their Operand field as raw text (2.6).
            // What is left is EASy68K's own Comment style.
            let expected: [(&str, Vec<(usize, &str)>); 3] = [
                ("clockDigital.X68", vec![(16, "bare_comment")]),
                ("graphicSound.X68", vec![(15, "bare_comment")]),
                ("mouseWindowSize.X68", vec![]),
            ];
            let programs = corpus("easy68k");
            assert_eq!(programs.len(), 3);
            for ((name, text), (expected_name, expected_diagnostics)) in
                programs.iter().zip(expected)
            {
                assert_eq!(name, expected_name);
                let parsed = parse_file(name, text);
                let found: Vec<(usize, &str)> = parsed
                    .diagnostics
                    .iter()
                    .map(|diagnostic| (diagnostic.location.line, diagnostic.code()))
                    .collect();
                assert_eq!(found, expected_diagnostics, "in {name}");
            }
        }

        #[test]
        fn only_a_macro_body_of_the_easy68k_originals_fails_line_by_line() {
            // The same three programs read line by line, without the Macro
            // skip, which is the measure of what that skip buys: one line, the
            // `move.l #\1,d1` of `clockDigital.X68`, whose `\1` is a Macro
            // parameter and which no grammar here describes.
            let mut found = Vec::new();
            for (name, text) in corpus("easy68k") {
                for (index, span) in split_lines(&text).into_iter().enumerate() {
                    let (_, diagnostics) = parse_line(span.text(&text), &name, index);
                    for diagnostic in diagnostics {
                        found.push(format!("{name}:{} {}", index + 1, diagnostic.code()));
                    }
                }
            }
            assert_eq!(found, ["clockDigital.X68:22 expression_expected"]);
        }
    }

    // -- the hover API -----------------------------------------------------

    #[test]
    fn a_line_serialises_as_plain_objects_for_the_editor() {
        let line = crate::assembler::parse_line("loop: move.l #5,(a0)+ ; go");
        let value = serde_json::to_value(&line).expect("a Line serialises");
        assert_eq!(value["label"]["name"], "loop");
        assert_eq!(value["label"]["colon"], true);
        assert_eq!(value["operation"]["name"], "move");
        assert_eq!(value["operation"]["size"], "long");
        assert_eq!(value["operation"]["operands"][0]["kind"], "immediate");
        assert_eq!(value["operation"]["operands"][0]["value"]["kind"], "number");
        assert_eq!(value["operation"]["operands"][0]["value"]["value"], 5);
        assert_eq!(value["operation"]["operands"][1]["kind"], "postincrement");
        assert_eq!(
            value["operation"]["operands"][1]["register"]["kind"],
            "address"
        );
        assert_eq!(value["operation"]["operands"][1]["register"]["number"], 0);
        assert_eq!(value["operation"]["operands"][1]["span"]["start"], 16);
        assert_eq!(value["operation"]["operands"][1]["span"]["end"], 21);
        assert_eq!(value["comment"]["kind"], "explicit");
        assert_eq!(value["comment"]["text"], "; go");
        assert_eq!(value["bare_comment"], false);
    }
}
