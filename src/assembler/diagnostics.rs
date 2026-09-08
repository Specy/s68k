//! The Diagnostics of every phase of the Assembler.
//!
//! A [`Diagnostic`] is a finding about the source: a [`Severity`], a
//! [`DiagnosticKind`] carrying the data its message needs, a
//! [`Location`], and any number of related Locations. The kind has a stable
//! snake_case [`code`](DiagnosticKind::code) that the asm-editor and the tests
//! key on, so a message may be reworded freely and a code may not.
//!
//! The messages follow [ADR
//! 0003](../../../docs/adr/0003-operands-are-parsed-independently-of-the-instruction.md):
//! say what was found, what is allowed, and what was probably meant. The
//! message says what is wrong and the [`hint`](DiagnosticKind::hint) says what
//! to do about it (CONTEXT.md, "Hint"), both short enough to read in an editor
//! gutter.
//!
//! The parser's kinds are the table of `docs/grammar.md` section 4, code for
//! code; the analyzer's are the first set of the design record's "Diagnostics",
//! and phases 2 and 3 add to them. A kind is listed here before anything raises
//! it, so that the message can be written once and reviewed against the help.
//!
//! Serialisation is one-way and flat: a Diagnostic serialises as
//! `{ severity, code, message, hint, location, related }`, with the message and
//! the hint already rendered, so the TypeScript side receives plain objects and
//! never a Rust enum's shape.

use serde::ser::{SerializeSeq, SerializeStruct};
use serde::{Serialize, Serializer};

use super::source::Location;
use super::token::{NumberBase, QuoteKind};

/// How much a Diagnostic matters.
///
/// `Error` stops the Program from being built; `Warning` and `Suggestion` are
/// reported while it still builds (CONTEXT.md, "Diagnostic").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    /// The Program cannot be built.
    Error,
    /// The Program is built, and something in it is probably not meant.
    Warning,
    /// The Program is built, and there is a better way to write it.
    Suggestion,
}

impl Severity {
    /// The name the serialised form uses.
    pub fn as_str(&self) -> &'static str {
        match self {
            Severity::Error => "error",
            Severity::Warning => "warning",
            Severity::Suggestion => "suggestion",
        }
    }
}

/// The Addressing mode shape a malformed Operand was trying to be.
///
/// [`DiagnosticKind::MalformedOperand`] names it, so that a broken Operand is
/// reported as what it tried to be and never as "invalid syntax" (ADR 0003).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OperandShape {
    /// `#5`
    Immediate,
    /// `(a0)`
    Indirect,
    /// `(a0)+`
    Postincrement,
    /// `-(a0)`
    Predecrement,
    /// `4(a0)`
    Displacement,
    /// `4(a0,d1.w)`
    Index,
    /// `label(pc)`
    PcDisplacement,
    /// `label(pc,d1.w)`
    PcIndex,
    /// `$1000`
    Absolute,
    /// `d0-d3/a0-a2`
    RegisterList,
    /// `(640-2)/2`
    GroupedExpression,
}

impl OperandShape {
    /// How a message names the shape ("an indexed operand").
    pub fn description(&self) -> &'static str {
        match self {
            OperandShape::Immediate => "an immediate operand",
            OperandShape::Indirect => "an indirect operand",
            OperandShape::Postincrement => "a postincrement operand",
            OperandShape::Predecrement => "a predecrement operand",
            OperandShape::Displacement => "a displacement operand",
            OperandShape::Index => "an indexed operand",
            OperandShape::PcDisplacement => "a PC-relative operand",
            OperandShape::PcIndex => "a PC-relative indexed operand",
            OperandShape::Absolute => "an absolute operand",
            OperandShape::RegisterList => "a register list",
            OperandShape::GroupedExpression => "a parenthesised expression",
        }
    }

    /// A written example of the shape, which is the half of the message a
    /// student can copy.
    pub fn example(&self) -> &'static str {
        match self {
            OperandShape::Immediate => "#5",
            OperandShape::Indirect => "(a0)",
            OperandShape::Postincrement => "(a0)+",
            OperandShape::Predecrement => "-(a0)",
            OperandShape::Displacement => "4(a0)",
            OperandShape::Index => "4(a0,d1.w)",
            OperandShape::PcDisplacement => "label(pc)",
            OperandShape::PcIndex => "label(pc,d1.w)",
            OperandShape::Absolute => "$1000",
            OperandShape::RegisterList => "d0-d3/a0-a2",
            OperandShape::GroupedExpression => "(640-2)/2",
        }
    }
}

/// What a Diagnostic is about.
///
/// One variant per kind, carrying exactly what its message needs. The
/// [`code`](DiagnosticKind::code) is the stable name; the message and the hint
/// are rendered from the variant's data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiagnosticKind {
    // ---- the tokenizer's, `docs/grammar.md` 1.1, 1.8, 1.9 ----
    /// A source character above code 255, which has no byte (ADR 0004).
    CharacterAboveLatin1 {
        /// The character as it was written.
        character: char,
    },
    /// A no-break space (`$A0`) outside a quoted literal: invisible, and what a
    /// paste from a web page leaves behind.
    NonBreakingSpace,
    /// A character that starts no token.
    UnexpectedCharacter {
        /// The character as it was written.
        character: char,
    },
    /// A quoted literal that reaches the end of its line.
    UnterminatedString {
        /// The quote the literal opened with.
        quote: QuoteKind,
    },
    /// A digit that does not belong to the number's base, or a base prefix with
    /// no digits after it at all.
    InvalidNumber {
        /// The base the prefix asked for.
        base: NumberBase,
        /// The offending digit, or `None` when there were no digits at all.
        digit: Option<char>,
    },
    /// A number that does not fit in the 64 bits values are computed in.
    ///
    /// The help has no equivalent — EASy68K works in 32 bits and warns above
    /// them — so this is s68k's, and it is an error because there is no value
    /// to carry on with (see the implementation notes, phase 1 step 3).
    NumberTooLarge {
        /// The number as it was written, prefix included.
        text: String,
    },

    // ---- the parser's, `docs/grammar.md` section 4 ----
    /// A `.` suffix whose run is not `b`, `w`, `l` or `s`.
    UnknownSizeSuffix {
        /// The run after the dot, without the dot.
        suffix: String,
    },
    /// A `.` inside a name, in the Operand field (`docs/grammar.md` 1.11).
    DotInName {
        /// The name as it was written, dot included.
        name: String,
    },
    /// A Symbol named like one of the twenty-one reserved register names.
    ReservedNameAsSymbol {
        /// The name that was written.
        name: String,
    },
    /// A second colon-terminated identifier where the operation should be.
    TwoLabelsOnOneLine {
        /// The second Label's name.
        name: String,
    },
    /// A Label followed by something that cannot be an operation name.
    OperationExpected {
        /// What was found instead, as it was written.
        found: String,
    },
    /// A `:` with no identifier before it.
    EmptyLabel,
    /// A `,` with no Operand after it, or an empty Operand before one.
    OperandExpected,
    /// A `(` that opened a grouped Expression and is never closed.
    UnclosedParenthesis,
    /// An Operand that nests deeper than the parser will descend.
    ///
    /// A backstop and not a rule of the language: the Expression parser is
    /// recursive and the crate ships as WebAssembly, whose stack is a megabyte,
    /// so a pasted line of three thousand `(` has to end in a Diagnostic rather
    /// than in a trap (see the implementation notes, phase 1 step 5).
    NestingTooDeep {
        /// How deep the parser descends before it stops.
        limit: usize,
    },
    /// An Operand that started as an Addressing mode and did not finish.
    MalformedOperand {
        /// The shape the Operand was trying to be.
        shape: OperandShape,
        /// What is missing, as a short phrase ("the `)` is missing").
        problem: String,
    },
    /// A token left over after a complete Operand.
    UnexpectedTokenInOperand {
        /// The leftover token, as it was written.
        found: String,
    },
    /// A `-` or `/` in a register list with no register after it.
    RegisterExpectedInRegisterList {
        /// The separator that was written, `-` or `/`.
        after: char,
    },
    /// A register range whose ends are the wrong way round.
    RegisterRangeOutOfOrder {
        /// The register written first.
        from: String,
        /// The register written second.
        to: String,
    },
    /// A register name where an Expression term must stand.
    RegisterInExpression {
        /// The register that was written.
        register: String,
    },
    /// A `macro` with no `endm` before the end of the File.
    UnterminatedMacroDefinition,
    /// A `#`, an operator or a `(` with no term after it.
    ExpressionExpected {
        /// What the missing term would have followed, as it was written.
        after: String,
    },
    /// A `+` where a term must begin. EASy68K has no unary `+` either.
    PlusIsNotAUnaryOperator {
        /// The term the `+` was written in front of, empty when there is none.
        term: String,
    },
    /// The comment field reads as the continuation of the Operand's Expression,
    /// which an Expression cannot be: it holds no whitespace.
    ExpressionSplitBySpace {
        /// The operator the comment field starts with.
        operator: String,
        /// The Operand and the comment field written as one Expression.
        joined: String,
        /// Whether the comment field is a bare one, in which case the hint also
        /// names `;`.
        bare_comment: bool,
    },
    /// The first bare Comment field in a File.
    BareComment,
    /// The Operand field is continued across whitespace because a `,` follows.
    SpaceBeforeComma,
    /// The first double-quoted string or character literal in a File.
    DoubleQuotedString,

    // ---- the analyzer's, the design record's "Diagnostics" ----
    /// An operation name that is in no instruction table.
    UnknownMnemonic {
        /// The name that was written.
        name: String,
        /// The closest known Mnemonic or Directive name, when there is one.
        suggestion: Option<String>,
        /// Whether the word could have been meant as a Label — an indented word
        /// that is nothing else (`label_rule`, `docs/grammar.md` 1.4).
        could_be_label: bool,
    },
    /// A word in column 1 that names an instruction, where a Label was probably
    /// meant (`label_rule` row 5).
    MnemonicUsedAsLabel {
        /// The name that was written.
        name: String,
    },
    /// An Operation given the wrong number of Operands.
    WrongOperandCount {
        /// The instruction that was written.
        mnemonic: String,
        /// How many Operands the line has.
        found: usize,
        /// The counts the instruction takes, lowest first.
        expected: Vec<usize>,
        /// Whether `expected` is a *minimum* rather than the exact counts,
        /// which is what a Directive taking a list of values needs: `dc.b`
        /// wants at least one item and `dc.b 1,2,3` is three of them.
        at_least: bool,
    },
    /// An Addressing mode the instruction does not take at that position.
    InvalidAddressingMode {
        /// The instruction that was written.
        mnemonic: String,
        /// Which Operand, counting from 1.
        position: usize,
        /// The mode that was found, named as a message names it.
        found: String,
        /// The modes the instruction takes there, named the same way.
        allowed: Vec<String>,
        /// What was probably meant, when the mistake is one with a name:
        /// `clr a0` wants `suba.l a0,a0`.
        suggestion: Option<String>,
    },
    /// Neither shape of an instruction that has two whole shapes rather than
    /// two positions: `addx`, `subx`, `abcd` and `sbcd` take two data
    /// registers or two predecrement Operands, and `addx d0,-(a1)` is wrong in
    /// neither Operand on its own.
    ///
    /// It is one Diagnostic over both Operands, because the position of the
    /// mistake is not where the mistake is (`Reference/68ks5e.htm`, "ADDRESS
    /// METHODS: Dn, -(An)").
    InvalidOperandPair {
        /// The instruction that was written.
        mnemonic: String,
        /// What the line has instead, as prose: "an immediate and a data
        /// register".
        found: String,
        /// What to write instead, when there is something short to say.
        advice: Option<String>,
    },
    /// An Operand short, and what stands where the missing one would is a bare
    /// Comment field that reads as an Operand: `move.l  d0 d1`.
    ///
    /// The Operand field ends at whitespace that is not beside a comma
    /// (`docs/grammar.md` 1.5), so the `d1` is EASy68K's bare comment and the
    /// line really does have one Operand. The count error says so; this says
    /// why.
    MissingCommaBetweenOperands {
        /// The word that was read as a Comment.
        operand: String,
        /// The Operand before it, when there is one, so that the hint can write
        /// the pair out.
        previous: Option<String>,
    },
    /// Both Operands of an instruction that has only one memory access are in
    /// memory.
    BothOperandsInMemory {
        /// The instruction that was written.
        mnemonic: String,
    },
    /// A byte-sized instruction with an address register in it. An address
    /// register is never used a byte at a time, whether it is the operand read
    /// or the one written (`Reference/68ks5g.htm`, `Reference/68ks4d.htm`).
    AddressRegisterByteSize {
        /// The instruction that was written.
        mnemonic: String,
    },
    /// An Operand the parser reads and the Assembler does not assemble.
    ///
    /// One is left, `usp`, and it is not an Addressing mode at all: every
    /// Addressing mode of the language is assembled since phase 3, `sr` and
    /// `ccr` included. The code is what the asm-editor keys on and does not
    /// change with the list.
    UnimplementedAddressingMode {
        /// The Operand as it was written.
        operand: String,
        /// What it is, as a message names it ("the user stack pointer").
        description: String,
        /// What to do about it, used as the hint word for word.
        advice: Option<String>,
    },
    /// A size suffix on an absolute address that is not `.w` or `.l`.
    ///
    /// `move.l table.b,d0` forces the *address* to a byte, which no address is
    /// ever forced to; the size of the instruction goes on the mnemonic
    /// (`Reference/68ks1e.htm`, "Forcing Absolute Short Addressing").
    InvalidAddressWidth {
        /// The address as it was written, suffix excluded.
        address: String,
        /// The suffix that was written, dot included.
        size: String,
    },
    /// A value outside the range of the field the instruction encodes it in:
    /// the count of a quick form, the vector of `trap`, a shift count, a bit
    /// number, the displacement of an Addressing mode.
    ValueOutOfRange {
        /// What the value is, as a message names it ("the count of `addq`").
        subject: String,
        /// The value that was written.
        value: i64,
        /// The lowest value the field holds.
        min: i64,
        /// The highest value it holds.
        max: i64,
        /// What to do about it, used as the hint word for word.
        advice: Option<String>,
    },
    /// A bare number below the program's origin where an immediate was almost
    /// certainly meant: `move.l 5,d0` reads the long at address 5.
    BareNumberAsAddress {
        /// The number that was written.
        value: i64,
    },
    /// A `*` given to an Operation that takes no Operands, which is EASy68K's
    /// comment marker read as the current address (`docs/grammar.md` 3.4).
    StarIsTheCurrentAddress {
        /// The Operation the `*` follows.
        mnemonic: String,
    },
    /// A size suffix the instruction does not carry.
    InvalidSize {
        /// The instruction that was written.
        mnemonic: String,
        /// The size that was written, dot included.
        size: String,
        /// The sizes the instruction takes, dots included; empty when it is
        /// not sized at all.
        allowed: Vec<String>,
    },
    /// An Immediate whose value does not fit the size it is used at.
    ImmediateOutOfRange {
        /// The value that was written.
        value: i64,
        /// The size it has to fit, named as a message names it ("byte").
        size: String,
        /// The lowest value that fits.
        min: i64,
        /// The highest value that fits.
        max: i64,
    },
    /// An Operation s68k knows about and does not implement, named with the
    /// reason (the design record, "Scope").
    UnimplementedOperation {
        /// The operation name that was written.
        name: String,
        /// Why it is not implemented, as a short phrase.
        reason: String,
        /// What to write instead, when there is something.
        alternative: Option<String>,
    },
    /// A Symbol defined twice. The first definition is a related Location.
    SymbolAlreadyDefined {
        /// The name that was defined twice.
        name: String,
    },
    /// A Symbol used and never defined.
    UndefinedSymbol {
        /// The name that was used.
        name: String,
        /// The closest defined Symbol, when there is one.
        suggestion: Option<String>,
    },
    /// A Symbol used above its definition where the value decides the Layout.
    ForwardReferenceNotAllowed {
        /// The Symbol that was used.
        name: String,
        /// The Directive that cannot wait for it.
        directive: String,
    },
    /// An Expression that divides by zero.
    DivisionByZero,
    /// A character literal of more than four characters: `#'abcdefgh'`.
    ///
    /// EASy68K's "ASCII constant exceeds 4 characters" (`errors.htm`), and a
    /// warning like its own: four characters are a long, the value is the last
    /// four bytes, and the line still assembles.
    CharacterLiteralTooLong {
        /// The literal as it was written, quotes included.
        text: String,
        /// How many characters it holds.
        characters: usize,
    },
    /// A written number above `$ffffffff`: `big equ $1234567890`.
    ///
    /// EASy68K's "Numeric constant exceeds 32 bits" (`errors.htm`), and a
    /// warning like its own: the value is kept in the 64 bits an Expression is
    /// computed in and checked against the size it is used at.
    ConstantAbove32Bits {
        /// The number as it was written, in the base it was written in.
        text: String,
        /// Its value.
        value: i64,
    },
    /// A `reg` Symbol where an Expression term must stand. EASy68K's own
    /// "Register list symbol used in an expression"; the parser's neighbour of
    /// it, about a register rather than a Symbol, is
    /// [`RegisterInExpression`](DiagnosticKind::RegisterInExpression).
    RegisterListInExpression {
        /// The name that stands for the Register list.
        name: String,
    },

    // ---- the Directives and the Layout ----
    /// An `org` to an odd address, which is rounded up (the design record,
    /// "Layout").
    OddOrigin {
        /// The address that was written.
        address: i64,
    },
    /// Two lines laid out over the same address. The earlier one is a related
    /// Location.
    AddressUsedTwice {
        /// The first address the two share.
        address: i64,
    },
    /// A line carrying a Label or an Operation after the `end` Directive.
    CodeAfterEnd,
    /// `end`'s Operand names a Label that is defined with different letter
    /// case. The Entry point is taken from the Label that is defined, which is
    /// what EASy68K does (its own `mouseWindowSize.X68` writes `start` and
    /// `END START`); the warning is here because s68k's Symbols are case
    /// sensitive everywhere else (CONTEXT.md, "Symbol").
    EntryPointCase {
        /// The name `end` was given.
        written: String,
        /// The Label that was found, as it is written.
        found: String,
    },
    /// An `end` with no Operand, which names no Entry point.
    EndWithoutAnAddress,
    /// A Directive that gives a name to something, written with no name.
    DirectiveNeedsALabel {
        /// The Directive that was written.
        directive: String,
    },
    /// A Label on a Directive that takes none: `page`, and the
    /// conditional-assembly Directives (`Directives/page.htm`,
    /// `Directives/conditional.htm`). EASy68K's "Label is not allowed".
    LabelNotAllowed {
        /// The Directive that was written.
        directive: String,
        /// The name that was written in the Label field.
        name: String,
    },
    /// `reg` given something that is not a register list: `regs reg 5`.
    RegisterListExpected {
        /// What was found, as a message names it ("an immediate operand").
        found: String,
    },
    /// A name where `movem` expects a register list that is a Symbol of another
    /// kind. EASy68K's "Symbol is not a register list symbol".
    NotARegisterList {
        /// The name that was written.
        name: String,
        /// What it is instead, as a message names it ("a constant").
        kind: String,
    },
    /// A `reg` Symbol used above the `reg` line that defines it. EASy68K's
    /// "Register list symbol not previously defined"; unlike every other
    /// forward reference this one is refused in an instruction Operand,
    /// because the list is not a value and the Layout has nothing to put off.
    RegisterListNotDefinedYet {
        /// The name that was written.
        name: String,
    },
    /// The `fail` Directive: the program's own error, whose message is the text
    /// after the Directive (`Directives/fail.htm`).
    UserDefinedError {
        /// The message the source wrote, or EASy68K's default when it wrote
        /// none.
        message: String,
        /// Whether the source wrote one at all, which is what the hint turns
        /// on.
        written: bool,
    },
    /// A line that would produce bytes inside an `offset` region, where "no
    /// machine code is generated by instructions or directives"
    /// (`Directives/offset.htm`): an instruction, a `simhalt` or a `dc`. `ds`
    /// is what a region is made of and raises nothing.
    NoBytesInAnOffsetRegion {
        /// What the line would have produced, as a message names it: "an
        /// instruction", "`dc.b`".
        item: String,
    },
    /// A Directive given an Operand that is an Addressing mode rather than a
    /// value: `dc.b (a0)`.
    ValueExpected {
        /// The Directive that was written, size included.
        directive: String,
        /// What was found, as a message names it ("a data register").
        found: String,
        /// How to rewrite this particular Operand, when it is one of the two
        /// shapes 1.4.2 accepted and [ADR
        /// 0001](../../../docs/adr/0001-easy68k-is-the-reference-dialect.md)
        /// drops — `ten equ #10` and `reg equ d1`. Used as the hint word for
        /// word; without one the hint says what a value is.
        advice: Option<String>,
    },
    /// A File the Assembler was told to read and cannot.
    UnreadableFile {
        /// The path that was asked for.
        path: String,
        /// The closest path the Project does hold, when there is one.
        suggestion: Option<String>,
        /// Whether the Project holds a *binary* File at that path, in which
        /// case it is there and holds no source.
        binary: bool,
    },
}

/// Every code this module can produce, in the order the variants are declared.
///
/// A new variant is added here as well as to
/// [`code`](DiagnosticKind::code) — the `code` match has no catch-all arm, so
/// the compiler asks for the code, and `every_kind_has_a_stable_code` asks for
/// this list.
pub const ALL_CODES: &[&str] = &[
    "character_above_latin1",
    "non_breaking_space",
    "unexpected_character",
    "unterminated_string",
    "invalid_number",
    "number_too_large",
    "unknown_size_suffix",
    "dot_in_name",
    "reserved_name_as_symbol",
    "two_labels_on_one_line",
    "operation_expected",
    "empty_label",
    "operand_expected",
    "unclosed_parenthesis",
    "nesting_too_deep",
    "malformed_operand",
    "unexpected_token_in_operand",
    "register_expected_in_register_list",
    "register_range_out_of_order",
    "register_in_expression",
    "unterminated_macro_definition",
    "expression_expected",
    "plus_is_not_a_unary_operator",
    "expression_split_by_space",
    "bare_comment",
    "space_before_comma",
    "double_quoted_string",
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

impl DiagnosticKind {
    /// The stable snake_case name of the kind.
    ///
    /// This is what the asm-editor keys on and what the tests name, so it never
    /// changes once it has shipped; the message above it may be reworded at
    /// will.
    pub fn code(&self) -> &'static str {
        match self {
            DiagnosticKind::CharacterAboveLatin1 { .. } => "character_above_latin1",
            DiagnosticKind::NonBreakingSpace => "non_breaking_space",
            DiagnosticKind::UnexpectedCharacter { .. } => "unexpected_character",
            DiagnosticKind::UnterminatedString { .. } => "unterminated_string",
            DiagnosticKind::InvalidNumber { .. } => "invalid_number",
            DiagnosticKind::NumberTooLarge { .. } => "number_too_large",
            DiagnosticKind::UnknownSizeSuffix { .. } => "unknown_size_suffix",
            DiagnosticKind::DotInName { .. } => "dot_in_name",
            DiagnosticKind::ReservedNameAsSymbol { .. } => "reserved_name_as_symbol",
            DiagnosticKind::TwoLabelsOnOneLine { .. } => "two_labels_on_one_line",
            DiagnosticKind::OperationExpected { .. } => "operation_expected",
            DiagnosticKind::EmptyLabel => "empty_label",
            DiagnosticKind::OperandExpected => "operand_expected",
            DiagnosticKind::UnclosedParenthesis => "unclosed_parenthesis",
            DiagnosticKind::NestingTooDeep { .. } => "nesting_too_deep",
            DiagnosticKind::MalformedOperand { .. } => "malformed_operand",
            DiagnosticKind::UnexpectedTokenInOperand { .. } => "unexpected_token_in_operand",
            DiagnosticKind::RegisterExpectedInRegisterList { .. } => {
                "register_expected_in_register_list"
            }
            DiagnosticKind::RegisterRangeOutOfOrder { .. } => "register_range_out_of_order",
            DiagnosticKind::RegisterInExpression { .. } => "register_in_expression",
            DiagnosticKind::UnterminatedMacroDefinition => "unterminated_macro_definition",
            DiagnosticKind::ExpressionExpected { .. } => "expression_expected",
            DiagnosticKind::PlusIsNotAUnaryOperator { .. } => "plus_is_not_a_unary_operator",
            DiagnosticKind::ExpressionSplitBySpace { .. } => "expression_split_by_space",
            DiagnosticKind::BareComment => "bare_comment",
            DiagnosticKind::SpaceBeforeComma => "space_before_comma",
            DiagnosticKind::DoubleQuotedString => "double_quoted_string",
            DiagnosticKind::UnknownMnemonic { .. } => "unknown_mnemonic",
            DiagnosticKind::MnemonicUsedAsLabel { .. } => "mnemonic_used_as_label",
            DiagnosticKind::WrongOperandCount { .. } => "wrong_operand_count",
            DiagnosticKind::MissingCommaBetweenOperands { .. } => "missing_comma_between_operands",
            DiagnosticKind::InvalidAddressingMode { .. } => "invalid_addressing_mode",
            DiagnosticKind::InvalidOperandPair { .. } => "invalid_operand_pair",
            DiagnosticKind::BothOperandsInMemory { .. } => "both_operands_in_memory",
            DiagnosticKind::AddressRegisterByteSize { .. } => "address_register_byte_size",
            DiagnosticKind::UnimplementedAddressingMode { .. } => "unimplemented_addressing_mode",
            DiagnosticKind::InvalidAddressWidth { .. } => "invalid_address_width",
            DiagnosticKind::ValueOutOfRange { .. } => "value_out_of_range",
            DiagnosticKind::BareNumberAsAddress { .. } => "bare_number_as_address",
            DiagnosticKind::StarIsTheCurrentAddress { .. } => "star_is_the_current_address",
            DiagnosticKind::InvalidSize { .. } => "invalid_size",
            DiagnosticKind::ImmediateOutOfRange { .. } => "immediate_out_of_range",
            DiagnosticKind::UnimplementedOperation { .. } => "unimplemented_operation",
            DiagnosticKind::SymbolAlreadyDefined { .. } => "symbol_already_defined",
            DiagnosticKind::UndefinedSymbol { .. } => "undefined_symbol",
            DiagnosticKind::ForwardReferenceNotAllowed { .. } => "forward_reference_not_allowed",
            DiagnosticKind::DivisionByZero => "division_by_zero",
            DiagnosticKind::CharacterLiteralTooLong { .. } => "character_literal_too_long",
            DiagnosticKind::ConstantAbove32Bits { .. } => "constant_above_32_bits",
            DiagnosticKind::RegisterListInExpression { .. } => "register_list_in_expression",
            DiagnosticKind::OddOrigin { .. } => "odd_origin",
            DiagnosticKind::AddressUsedTwice { .. } => "address_used_twice",
            DiagnosticKind::CodeAfterEnd => "code_after_end",
            DiagnosticKind::EntryPointCase { .. } => "entry_point_case_mismatch",
            DiagnosticKind::EndWithoutAnAddress => "end_without_an_address",
            DiagnosticKind::DirectiveNeedsALabel { .. } => "directive_needs_a_label",
            DiagnosticKind::LabelNotAllowed { .. } => "label_not_allowed",
            DiagnosticKind::RegisterListExpected { .. } => "register_list_expected",
            DiagnosticKind::NotARegisterList { .. } => "not_a_register_list",
            DiagnosticKind::RegisterListNotDefinedYet { .. } => "register_list_not_defined_yet",
            DiagnosticKind::UserDefinedError { .. } => "user_defined_error",
            DiagnosticKind::NoBytesInAnOffsetRegion { .. } => "no_bytes_in_an_offset_region",
            DiagnosticKind::ValueExpected { .. } => "value_expected",
            DiagnosticKind::UnreadableFile { .. } => "unreadable_file",
        }
    }

    /// How much the kind matters.
    ///
    /// The severity belongs to the kind and not to the place that raises it, so
    /// that the table of `docs/grammar.md` section 4 is true of every
    /// Diagnostic with that code.
    pub fn severity(&self) -> Severity {
        match self {
            DiagnosticKind::ExpressionSplitBySpace { .. }
            | DiagnosticKind::CharacterLiteralTooLong { .. }
            | DiagnosticKind::ConstantAbove32Bits { .. }
            | DiagnosticKind::MissingCommaBetweenOperands { .. }
            | DiagnosticKind::StarIsTheCurrentAddress { .. }
            | DiagnosticKind::OddOrigin { .. }
            | DiagnosticKind::CodeAfterEnd
            | DiagnosticKind::EntryPointCase { .. }
            | DiagnosticKind::EndWithoutAnAddress => Severity::Warning,
            DiagnosticKind::BareComment
            | DiagnosticKind::SpaceBeforeComma
            | DiagnosticKind::DoubleQuotedString
            | DiagnosticKind::BareNumberAsAddress { .. }
            | DiagnosticKind::MnemonicUsedAsLabel { .. } => Severity::Suggestion,
            _ => Severity::Error,
        }
    }

    /// What is wrong, in one sentence.
    pub fn message(&self) -> String {
        match self {
            DiagnosticKind::CharacterAboveLatin1 { character } => {
                format!("{} cannot be stored: a character is one byte", show(*character))
            }
            DiagnosticKind::NonBreakingSpace => "this is a no-break space, not a space".to_string(),
            DiagnosticKind::UnexpectedCharacter { character } => {
                format!("{} cannot start anything here", show(*character))
            }
            DiagnosticKind::UnterminatedString { .. } => {
                "this string is not closed before the end of the line".to_string()
            }
            DiagnosticKind::InvalidNumber { base, digit } => match digit {
                Some(digit) => format!("{} is not a {} digit", show(*digit), base.name()),
                None => format!("`{}` has no digits after it", base.prefix()),
            },
            DiagnosticKind::NumberTooLarge { text } => {
                format!("`{text}` does not fit in the 64 bits a value is computed in")
            }
            DiagnosticKind::UnknownSizeSuffix { suffix } => format!("`.{suffix}` is not a size"),
            DiagnosticKind::DotInName { name } => {
                format!("`{name}` holds a dot: a name has none after its first character")
            }
            DiagnosticKind::ReservedNameAsSymbol { name } => {
                format!("`{name}` is a register name and cannot be a symbol")
            }
            DiagnosticKind::TwoLabelsOnOneLine { name } => {
                format!("`{name}` is a second label on this line")
            }
            DiagnosticKind::OperationExpected { found } => {
                format!("`{found}` is not a mnemonic or a directive")
            }
            DiagnosticKind::EmptyLabel => "there is no name before this `:`".to_string(),
            DiagnosticKind::OperandExpected => {
                "an operand was expected after this comma".to_string()
            }
            DiagnosticKind::UnclosedParenthesis => "this `(` is never closed".to_string(),
            DiagnosticKind::NestingTooDeep { limit } => {
                format!("this operand nests more than {limit} levels deep and is not read further")
            }
            DiagnosticKind::MalformedOperand { shape, problem } => format!(
                "this looks like {}, `{}`, but {problem}",
                shape.description(),
                shape.example()
            ),
            DiagnosticKind::UnexpectedTokenInOperand { found } => {
                format!("`{found}` was not expected here")
            }
            DiagnosticKind::RegisterExpectedInRegisterList { after } => {
                format!("a register was expected after `{after}`")
            }
            DiagnosticKind::RegisterRangeOutOfOrder { from, to } => format!(
                "`{from}-{to}` runs backwards: a range goes from the lower register to the higher one, in the order `d0`-`d7`, `a0`-`a7`"
            ),
            DiagnosticKind::RegisterInExpression { register } => {
                format!("`{register}` is a register; an expression holds no registers")
            }
            DiagnosticKind::UnterminatedMacroDefinition => {
                "this macro definition is never closed".to_string()
            }
            DiagnosticKind::ExpressionExpected { after } => {
                format!("an expression was expected after `{after}`")
            }
            DiagnosticKind::PlusIsNotAUnaryOperator { .. } => {
                "`+` is not a unary operator".to_string()
            }
            DiagnosticKind::ExpressionSplitBySpace { operator, .. } => format!(
                "the operand field ended at the space before `{operator}`; an expression contains no whitespace"
            ),
            DiagnosticKind::BareComment => {
                "this is EASy68K's comment field; s68k reads it as a comment".to_string()
            }
            DiagnosticKind::SpaceBeforeComma => {
                "the operand field continues past this space because a comma follows it".to_string()
            }
            DiagnosticKind::DoubleQuotedString => {
                "EASy68K writes strings in single quotes".to_string()
            }
            DiagnosticKind::UnknownMnemonic { name, .. } => {
                format!("`{name}` is not an instruction or a directive")
            }
            DiagnosticKind::MnemonicUsedAsLabel { name } => {
                format!("`{name}` in column 1 is the instruction `{name}`, not a label")
            }
            DiagnosticKind::WrongOperandCount {
                mnemonic,
                found,
                expected,
                at_least,
            } => match at_least {
                true => format!(
                    "`{mnemonic}` takes at least {}, and this line has {}",
                    value_count(expected.first().copied().unwrap_or(1)),
                    count(*found)
                ),
                false => format!(
                    "`{mnemonic}` takes {}, and this line has {}",
                    operand_count(expected),
                    count(*found)
                ),
            },
            DiagnosticKind::MissingCommaBetweenOperands { operand, .. } => format!(
                "the operand field ended at the space before `{operand}`, and `{operand}` was read as a comment"
            ),
            DiagnosticKind::InvalidAddressingMode {
                mnemonic,
                position,
                found,
                ..
            } => format!(
                "the {} operand of `{mnemonic}` cannot be {found}",
                ordinal(*position)
            ),
            DiagnosticKind::InvalidOperandPair {
                mnemonic, found, ..
            } => format!(
                "`{mnemonic}` takes two data registers or two predecrement operands, and this \
                 line has {found}"
            ),
            DiagnosticKind::BothOperandsInMemory { mnemonic } => {
                format!("`{mnemonic}` cannot read and write memory in one instruction")
            }
            DiagnosticKind::AddressRegisterByteSize { mnemonic } => format!(
                "`{mnemonic}.b` uses an address register, and an address register is never used one byte at a time"
            ),
            DiagnosticKind::UnimplementedAddressingMode {
                operand,
                description,
                ..
            } => format!("`{operand}` is {description}, which s68k does not assemble"),
            DiagnosticKind::InvalidAddressWidth { address, size } => format!(
                "`{size}` after `{address}` forces the width of the address, and an address is \
                 forced to `.w` or `.l`"
            ),
            DiagnosticKind::ValueOutOfRange {
                subject,
                value,
                min,
                max,
                ..
            } => format!("{subject} is {min} to {max}, and `{value}` is outside it"),
            DiagnosticKind::BareNumberAsAddress { value } => format!(
                "`{value}` here means the contents of address {value}, not the number {value}"
            ),
            DiagnosticKind::StarIsTheCurrentAddress { mnemonic } => format!(
                "the `*` after `{mnemonic}` is the current address, not the start of a comment"
            ),
            DiagnosticKind::InvalidSize {
                mnemonic, size, ..
            } => format!("`{size}` is not a size for `{mnemonic}`"),
            DiagnosticKind::ImmediateOutOfRange { value, size, .. } => {
                format!("`#{value}` does not fit in a {size}")
            }
            DiagnosticKind::UnimplementedOperation { name, reason, .. } => {
                format!("`{name}` is not implemented: {reason}")
            }
            DiagnosticKind::SymbolAlreadyDefined { name } => {
                format!("`{name}` is already defined")
            }
            DiagnosticKind::UndefinedSymbol { name, .. } => format!("`{name}` is not defined"),
            DiagnosticKind::ForwardReferenceNotAllowed { name, directive } => format!(
                "`{directive}` cannot use `{name}`, which is defined further down"
            ),
            DiagnosticKind::DivisionByZero => "this expression divides by zero".to_string(),
            DiagnosticKind::CharacterLiteralTooLong { text, characters } => format!(
                "`{text}` is {characters} characters, and a character literal holds at most four"
            ),
            DiagnosticKind::ConstantAbove32Bits { text, .. } => {
                format!("`{text}` does not fit in the 32 bits this machine works in")
            }
            DiagnosticKind::RegisterListInExpression { name } => format!(
                "`{name}` stands for a register list, and an expression holds no register list"
            ),
            DiagnosticKind::OddOrigin { address } => format!(
                "`${address:x}` is an odd address, and an instruction or a word starts on an even one"
            ),
            DiagnosticKind::AddressUsedTwice { address } => {
                format!("this line is laid out over `${address:x}`, which is already used")
            }
            DiagnosticKind::CodeAfterEnd => {
                "this line comes after `end` and is not assembled".to_string()
            }
            DiagnosticKind::EntryPointCase { written, found } => format!(
                "`{written}` is not defined, `{found}` is, and the program starts at `{found}`"
            ),
            DiagnosticKind::EndWithoutAnAddress => {
                "`end` says where the program starts, and this one says no address".to_string()
            }
            DiagnosticKind::DirectiveNeedsALabel { directive } => {
                format!("`{directive}` gives a name to something, and this line has no name")
            }
            DiagnosticKind::LabelNotAllowed { directive, name } => {
                format!("`{directive}` takes no label, and this line declares `{name}`")
            }
            DiagnosticKind::RegisterListExpected { found } => {
                format!("`reg` names a register list, and this is {found}")
            }
            DiagnosticKind::NotARegisterList { name, kind } => {
                format!("`{name}` is {kind}, not a register list")
            }
            DiagnosticKind::RegisterListNotDefinedYet { name } => format!(
                "`{name}` is a register list defined further down, and a register list has to be \
                 defined before it is used"
            ),
            DiagnosticKind::UserDefinedError { message, .. } => message.clone(),
            DiagnosticKind::NoBytesInAnOffsetRegion { item } => {
                format!("an `offset` region produces no bytes, and {item} produces some")
            }
            DiagnosticKind::ValueExpected {
                directive,
                found,
                ..
            } => {
                format!("`{directive}` takes a value, and this is {found}")
            }
            DiagnosticKind::UnreadableFile { path, binary, .. } => match binary {
                true => format!("`{path}` holds bytes, not source"),
                false => format!("there is no file named `{path}` in this project"),
            },
        }
    }

    /// What to do about it, when there is something short to say.
    pub fn hint(&self) -> Option<String> {
        match self {
            DiagnosticKind::CharacterAboveLatin1 { character } => {
                Some(match look_alike(*character) {
                    Some(plain) => format!("write `{plain}`"),
                    None => {
                        "a character is one Latin-1 byte, so its code is 255 at most".to_string()
                    }
                })
            }
            DiagnosticKind::NonBreakingSpace => {
                Some("replace it with a space; a paste from a web page leaves it".to_string())
            }
            DiagnosticKind::UnterminatedString { quote } => {
                let quote = quote.character();
                Some(format!(
                    "add the closing `{quote}`; `{quote}{quote}` writes a quote inside a string"
                ))
            }
            DiagnosticKind::InvalidNumber { base, .. } => {
                Some(format!("{} digits are {}", base.name(), base.digits()))
            }
            DiagnosticKind::NumberTooLarge { .. } => Some(
                "a value is computed in 64 bits and checked against the operand's size".to_string(),
            ),
            DiagnosticKind::UnknownSizeSuffix { .. } => {
                Some("sizes are `.b`, `.w`, `.l`, and `.s` on branches".to_string())
            }
            DiagnosticKind::DotInName { name } => Some(format!(
                "write `{}`; a dot after a name is a size, `.b`, `.w`, `.l` or `.s`",
                name.replace('.', "_")
            )),
            DiagnosticKind::ReservedNameAsSymbol { name } => {
                Some(format!("pick another name, such as `{name}_value`"))
            }
            DiagnosticKind::TwoLabelsOnOneLine { name } => {
                Some(format!("one label to a line; put `{name}:` on its own"))
            }
            DiagnosticKind::EmptyLabel => Some("a label is a name, then the colon".to_string()),
            DiagnosticKind::OperandExpected => {
                Some("remove the comma if the operand list ends here".to_string())
            }
            DiagnosticKind::UnclosedParenthesis => Some("add the `)`".to_string()),
            DiagnosticKind::NestingTooDeep { .. } => Some(
                "no expression needs to nest that deep; check for a `)` that is missing"
                    .to_string(),
            ),
            DiagnosticKind::UnexpectedTokenInOperand { found } => Some(if found == ")" {
                "there is no `(` for this `)`".to_string()
            } else {
                "an operand ends at a comma or at the end of the operand field".to_string()
            }),
            DiagnosticKind::RegisterExpectedInRegisterList { .. } => {
                Some("a register list reads `d0-d3/a0-a2`".to_string())
            }
            DiagnosticKind::RegisterRangeOutOfOrder { from, to } => {
                Some(format!("write `{to}-{from}`"))
            }
            DiagnosticKind::RegisterInExpression { .. } => Some(
                "an expression is computed while assembling, when no register has a value yet"
                    .to_string(),
            ),
            DiagnosticKind::UnterminatedMacroDefinition => {
                Some("macro definitions end with `endm`".to_string())
            }
            DiagnosticKind::PlusIsNotAUnaryOperator { term } => Some(if term.is_empty() {
                "the unary operators are `-` and `~`".to_string()
            } else {
                format!("write `{term}`; the unary operators are `-` and `~`")
            }),
            DiagnosticKind::ExpressionSplitBySpace {
                joined,
                bare_comment,
                ..
            } => Some(if *bare_comment {
                format!("write `{joined}`, or start a comment with `;`")
            } else {
                format!("write `{joined}`")
            }),
            DiagnosticKind::BareComment => Some("start comments with `;` to say so".to_string()),
            DiagnosticKind::SpaceBeforeComma => {
                Some("remove the space; if the comma starts a comment, write `;` first".to_string())
            }
            DiagnosticKind::DoubleQuotedString => Some("`'Hello'` assembles in both".to_string()),
            DiagnosticKind::UnknownMnemonic {
                name,
                suggestion,
                could_be_label,
            } => {
                let label_hint =
                    format!("start it in column 1, or end it with a colon, if `{name}` is a label");
                match (suggestion, could_be_label) {
                    (Some(suggestion), true) => {
                        Some(format!("did you mean `{suggestion}`? {label_hint}"))
                    }
                    (Some(suggestion), false) => Some(format!("did you mean `{suggestion}`?")),
                    (None, true) => Some(label_hint),
                    (None, false) => None,
                }
            }
            DiagnosticKind::MnemonicUsedAsLabel { name } => {
                Some(format!("write `{name}:` if `{name}` is a label"))
            }
            DiagnosticKind::InvalidAddressingMode {
                allowed, suggestion, ..
            } => match (suggestion, allowed.is_empty()) {
                (Some(suggestion), true) => Some(suggestion.clone()),
                (Some(suggestion), false) => {
                    Some(format!("{suggestion}; there it takes {}", list(allowed)))
                }
                (None, true) => None,
                (None, false) => Some(format!("there it takes {}", list(allowed))),
            },
            DiagnosticKind::MissingCommaBetweenOperands { operand, previous } => {
                Some(match previous {
                    Some(previous) => format!("write `{previous},{operand}`"),
                    None => "operands are written as one list, separated by commas".to_string(),
                })
            }
            DiagnosticKind::InvalidOperandPair {
                mnemonic, advice, ..
            } => Some(match advice {
                Some(advice) => format!(
                    "write `{mnemonic} d0,d1` or `{mnemonic} -(a0),-(a1)`; {advice}"
                ),
                None => format!("write `{mnemonic} d0,d1` or `{mnemonic} -(a0),-(a1)`"),
            }),
            DiagnosticKind::BothOperandsInMemory { .. } => Some(
                "one of the two operands has to be a register; load one of them first".to_string(),
            ),
            DiagnosticKind::AddressRegisterByteSize { .. } => {
                Some("use `.w` or `.l`".to_string())
            }
            DiagnosticKind::UnimplementedAddressingMode { advice, .. } => advice.clone(),
            DiagnosticKind::InvalidAddressWidth { address, .. } => Some(format!(
                "write `{address}.w` or `{address}.l`, or `{address}` on its own; the size the \
                 instruction works at goes after the mnemonic"
            )),
            DiagnosticKind::ValueOutOfRange { advice, .. } => advice.clone(),
            DiagnosticKind::BareNumberAsAddress { value } => {
                Some(format!("write `#{value}` for the number itself"))
            }
            DiagnosticKind::StarIsTheCurrentAddress { mnemonic } => Some(format!(
                "`{mnemonic}` takes no operands, so the `*` is ignored; write `;` to start a comment"
            )),
            DiagnosticKind::InvalidSize {
                mnemonic, allowed, ..
            } => Some(if allowed.is_empty() {
                format!("`{mnemonic}` carries no size")
            } else {
                format!("`{mnemonic}` takes {}", list(allowed))
            }),
            DiagnosticKind::ImmediateOutOfRange { size, min, max, .. } => {
                Some(format!("a {size} immediate holds {min} to {max}"))
            }
            DiagnosticKind::UnimplementedOperation { alternative, .. } => alternative
                .as_ref()
                .map(|alternative| format!("write {alternative} instead")),
            DiagnosticKind::SymbolAlreadyDefined { .. } => Some(
                "a name is defined once; `set` defines a symbol that may be redefined".to_string(),
            ),
            DiagnosticKind::UndefinedSymbol { name, suggestion } => Some(match suggestion {
                // A miscounted register is not a symbol anyone meant to
                // define, so it is answered before the "did you mean" of the
                // symbol table, which cannot reach a register name in any case.
                _ if numbered_register(name) => {
                    "the data registers are `d0` to `d7` and the address registers `a0` to `a7`"
                        .to_string()
                }
                Some(suggestion) => format!("did you mean `{suggestion}`?"),
                None => format!("define `{name}` with a label or with `equ`"),
            }),
            DiagnosticKind::ForwardReferenceNotAllowed { directive, .. } => Some(format!(
                "`{directive}` decides where the program goes, so move the definition above it"
            )),
            DiagnosticKind::CharacterLiteralTooLong { text, .. } => Some(format!(
                "four characters are a long, and this one is worth its last four; write \
                 `dc.b {text}` and use its address for the whole of it"
            )),
            DiagnosticKind::ConstantAbove32Bits { .. } => Some(
                "a 32-bit value goes up to 4294967295, `$ffffffff`; the value is kept whole and \
                 checked against the size it is used at"
                    .to_string(),
            ),
            DiagnosticKind::RegisterListInExpression { name } => Some(format!(
                "`{name}` is the register list operand of `movem` and nothing else"
            )),
            DiagnosticKind::OddOrigin { address } => Some(format!(
                "the next item is laid out at `${:x}`",
                address.wrapping_add(1)
            )),
            DiagnosticKind::AddressUsedTwice { .. } => Some(
                "move one of the two with `org`, or make what comes before it shorter".to_string(),
            ),
            DiagnosticKind::CodeAfterEnd => {
                Some("`end` is the last line the assembler reads".to_string())
            }
            DiagnosticKind::EntryPointCase { found, .. } => Some(format!(
                "symbols are case sensitive here, so write `end {found}`"
            )),
            DiagnosticKind::EndWithoutAnAddress => Some(
                "write the label the program starts at, `end START`; without one it starts at a \
                 label named `START`, or at the first instruction"
                    .to_string(),
            ),
            DiagnosticKind::DirectiveNeedsALabel { directive } => Some(format!(
                "write the name in the first column, `count {directive} …`"
            )),
            DiagnosticKind::LabelNotAllowed { name, .. } => Some(format!(
                "put `{name}:` on a line of its own if it is meant to name the address here"
            )),
            DiagnosticKind::RegisterListExpected { .. } => {
                Some("write the registers themselves, `regs reg d0-d3/a0-a2`".to_string())
            }
            DiagnosticKind::NotARegisterList { name, .. } => Some(format!(
                "`movem` reads a name here only when `reg` defined it, as in \
                 `{name} reg d0-d3/a0-a2`"
            )),
            DiagnosticKind::RegisterListNotDefinedYet { name } => Some(format!(
                "move the `{name} reg …` line above this one"
            )),
            DiagnosticKind::UserDefinedError { written, .. } => Some(match written {
                true => "this is the message of the `fail` directive on this line, and not \
                         something the assembler found"
                    .to_string(),
                false => "`fail` reports the rest of its line as an error; write the message \
                          after it, as in `fail the buffer is too small`"
                    .to_string(),
            }),
            DiagnosticKind::NoBytesInAnOffsetRegion { .. } => Some(
                "an `offset` region names the fields of a structure with `ds`; write `org *` \
                 below them to end it and assemble code again"
                    .to_string(),
            ),
            DiagnosticKind::ValueExpected { advice, .. } => Some(advice.clone().unwrap_or_else(
                || {
                    "a value is a number, a character literal, a symbol or an expression of them"
                        .to_string()
                },
            )),
            DiagnosticKind::UnreadableFile {
                suggestion, binary, ..
            } => match (binary, suggestion) {
                (true, _) => Some("`incbin` reads a binary file; source is read with `include`".to_string()),
                (false, Some(suggestion)) => Some(format!("did you mean `{suggestion}`?")),
                (false, None) => None,
            },
            DiagnosticKind::WrongOperandCount {
                mnemonic,
                at_least: true,
                ..
            } => Some(format!("write the values after it, `{mnemonic} 1,2,3`")),
            DiagnosticKind::WrongOperandCount { .. }
            | DiagnosticKind::UnexpectedCharacter { .. }
            | DiagnosticKind::OperationExpected { .. }
            | DiagnosticKind::MalformedOperand { .. }
            | DiagnosticKind::ExpressionExpected { .. }
            | DiagnosticKind::DivisionByZero => None,
        }
    }
}

/// Whether `name` is a register that does not exist: `d8`, `a9`, `d10`.
///
/// Such a name is a legal Symbol and is read as one (`docs/grammar.md` 1.7), so
/// it reaches the evaluator as `undefined_symbol`; what the student did is
/// miscount the registers, and "define `d8` with a label or with `equ`" would
/// answer that with nonsense. No single register is offered instead, because
/// `d8` is one edit from every one of `d0` to `d7` and naming one of them would
/// be a guess; the hint names the whole range, which is the fact that was
/// missed.
///
/// The number has to be a plausible miscount (below 32) so that a Label named
/// `A320` keeps the ordinary "did you mean" of the symbol table.
fn numbered_register(name: &str) -> bool {
    let mut characters = name.chars();
    let kind = characters.next();
    if !matches!(kind, Some('d') | Some('D') | Some('a') | Some('A')) {
        return false;
    }
    let digits: String = characters.collect();
    if digits.is_empty() || !digits.chars().all(|digit| digit.is_ascii_digit()) {
        return false;
    }
    // `d0` to `d7` and `a0` to `a7` are reserved names and never reach here, so
    // every number that does is one the 68000 has no register for. The bound
    // keeps it to a plausible miscount — a machine with 32 registers is one a
    // student may have been taught — so that a Label named `A320` is answered
    // as the name it is.
    digits
        .parse::<u32>()
        .map(|number| number < 32)
        .unwrap_or(false)
}

/// A character as a message writes it: itself when it can be seen, its code
/// when it cannot.
fn show(character: char) -> String {
    if character.is_control() || character == '\u{A0}' {
        format!("the character ${:02x}", character as u32)
    } else {
        format!("`{character}`")
    }
}

/// The plain character a typographic look-alike stands for, which is nearly
/// always what a paste from a web page or a word processor meant
/// (`docs/grammar.md` 1.1).
fn look_alike(character: char) -> Option<char> {
    match character {
        '\u{2018}' | '\u{2019}' => Some('\''),
        '\u{201C}' | '\u{201D}' => Some('"'),
        '\u{2013}' | '\u{2014}' => Some('-'),
        _ => None,
    }
}

/// `1` as "first", and so on, for the Operand a message is about.
fn ordinal(position: usize) -> String {
    match position {
        1 => "first".to_string(),
        2 => "second".to_string(),
        3 => "third".to_string(),
        4 => "fourth".to_string(),
        other => format!("{other}th"),
    }
}

/// How many Operands, in words: "no operands", "one operand", "two operands".
fn count(number: usize) -> String {
    match number {
        0 => "none".to_string(),
        1 => "one".to_string(),
        2 => "two".to_string(),
        3 => "three".to_string(),
        other => other.to_string(),
    }
}

/// The Operand counts an instruction accepts, as a message reads them out:
/// "no operands", "one or two operands".
fn operand_count(expected: &[usize]) -> String {
    match expected {
        [] | [0] => "no operands".to_string(),
        [1] => "one operand".to_string(),
        counts => {
            let words: Vec<String> = counts.iter().map(|number| count(*number)).collect();
            format!("{} operands", list(&words))
        }
    }
}

/// How many values a list Directive wants at least, in words: "one value".
fn value_count(number: usize) -> String {
    match number {
        1 => "one value".to_string(),
        other => format!("{} values", count(other)),
    }
}

/// A list a message can read out: "a, b or c".
fn list(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [only] => only.clone(),
        [rest @ .., last] => format!("{} or {last}", rest.join(", ")),
    }
}

/// One finding about the source.
///
/// Build it with [`Diagnostic::new`], which takes the severity from the kind,
/// and add related Locations with
/// [`with_related`](Diagnostic::with_related) — the second definition of a
/// Symbol, the `(` a `)` is missing for, the `include` line a File was reached
/// through.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    /// How much it matters. Set from the kind.
    pub severity: Severity,
    /// What it is about.
    pub kind: DiagnosticKind,
    /// Where it is.
    pub location: Location,
    /// Other places worth looking at, each with a sentence saying why.
    pub related: Vec<(Location, String)>,
}

impl Diagnostic {
    /// A Diagnostic of `kind` at `location`, with the severity the kind carries.
    pub fn new(kind: DiagnosticKind, location: Location) -> Self {
        Self {
            severity: kind.severity(),
            kind,
            location,
            related: Vec::new(),
        }
    }

    /// The same Diagnostic with one more related Location.
    pub fn with_related(mut self, location: Location, message: impl Into<String>) -> Self {
        self.related.push((location, message.into()));
        self
    }

    /// The stable snake_case name of the kind.
    pub fn code(&self) -> &'static str {
        self.kind.code()
    }

    /// What is wrong, in one sentence.
    pub fn message(&self) -> String {
        self.kind.message()
    }

    /// What to do about it, when there is something short to say.
    pub fn hint(&self) -> Option<String> {
        self.kind.hint()
    }

    /// Whether this stops the Program from being built.
    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }
}

/// One related Location, as the serialised form writes it.
#[derive(Serialize)]
struct SerializedRelated<'a> {
    location: &'a Location,
    message: &'a str,
}

/// The related Locations of a Diagnostic, as an array of objects.
struct SerializedRelatedList<'a>(&'a [(Location, String)]);

impl Serialize for SerializedRelatedList<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut sequence = serializer.serialize_seq(Some(self.0.len()))?;
        for (location, message) in self.0 {
            sequence.serialize_element(&SerializedRelated { location, message })?;
        }
        sequence.end()
    }
}

impl Serialize for Diagnostic {
    /// `{ severity, code, message, hint, location, related }`, with the message
    /// and the hint already rendered: the TypeScript side receives plain
    /// objects and never has to know what a [`DiagnosticKind`] is.
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut diagnostic = serializer.serialize_struct("Diagnostic", 6)?;
        diagnostic.serialize_field("severity", &self.severity)?;
        diagnostic.serialize_field("code", self.code())?;
        diagnostic.serialize_field("message", &self.message())?;
        diagnostic.serialize_field("hint", &self.hint())?;
        diagnostic.serialize_field("location", &self.location)?;
        diagnostic.serialize_field("related", &SerializedRelatedList(&self.related))?;
        diagnostic.end()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One sample of every variant, with the code it must carry. A new variant
    /// is added here, and `every_kind_has_a_stable_code` checks the list
    /// against [`ALL_CODES`].
    fn every_kind() -> Vec<(DiagnosticKind, &'static str)> {
        vec![
            (
                DiagnosticKind::CharacterAboveLatin1 {
                    character: '\u{2019}',
                },
                "character_above_latin1",
            ),
            (DiagnosticKind::NonBreakingSpace, "non_breaking_space"),
            (
                DiagnosticKind::UnexpectedCharacter { character: '?' },
                "unexpected_character",
            ),
            (
                DiagnosticKind::UnterminatedString {
                    quote: QuoteKind::Single,
                },
                "unterminated_string",
            ),
            (
                DiagnosticKind::InvalidNumber {
                    base: NumberBase::Hexadecimal,
                    digit: Some('g'),
                },
                "invalid_number",
            ),
            (
                DiagnosticKind::NumberTooLarge {
                    text: "$ffffffffffffffffff".to_string(),
                },
                "number_too_large",
            ),
            (
                DiagnosticKind::UnknownSizeSuffix {
                    suffix: "q".to_string(),
                },
                "unknown_size_suffix",
            ),
            (
                DiagnosticKind::DotInName {
                    name: "array.length".to_string(),
                },
                "dot_in_name",
            ),
            (
                DiagnosticKind::ReservedNameAsSymbol {
                    name: "d0".to_string(),
                },
                "reserved_name_as_symbol",
            ),
            (
                DiagnosticKind::TwoLabelsOnOneLine {
                    name: "bar".to_string(),
                },
                "two_labels_on_one_line",
            ),
            (
                DiagnosticKind::OperationExpected {
                    found: "5".to_string(),
                },
                "operation_expected",
            ),
            (DiagnosticKind::EmptyLabel, "empty_label"),
            (DiagnosticKind::OperandExpected, "operand_expected"),
            (DiagnosticKind::UnclosedParenthesis, "unclosed_parenthesis"),
            (
                DiagnosticKind::NestingTooDeep { limit: 64 },
                "nesting_too_deep",
            ),
            (
                DiagnosticKind::MalformedOperand {
                    shape: OperandShape::Index,
                    problem: "the `)` is missing".to_string(),
                },
                "malformed_operand",
            ),
            (
                DiagnosticKind::UnexpectedTokenInOperand {
                    found: ")".to_string(),
                },
                "unexpected_token_in_operand",
            ),
            (
                DiagnosticKind::RegisterExpectedInRegisterList { after: '/' },
                "register_expected_in_register_list",
            ),
            (
                DiagnosticKind::RegisterRangeOutOfOrder {
                    from: "d5".to_string(),
                    to: "d2".to_string(),
                },
                "register_range_out_of_order",
            ),
            (
                DiagnosticKind::RegisterInExpression {
                    register: "a0".to_string(),
                },
                "register_in_expression",
            ),
            (
                DiagnosticKind::UnterminatedMacroDefinition,
                "unterminated_macro_definition",
            ),
            (
                DiagnosticKind::ExpressionExpected {
                    after: "+".to_string(),
                },
                "expression_expected",
            ),
            (
                DiagnosticKind::PlusIsNotAUnaryOperator {
                    term: "5".to_string(),
                },
                "plus_is_not_a_unary_operator",
            ),
            (
                DiagnosticKind::ExpressionSplitBySpace {
                    operator: "*".to_string(),
                    joined: "#2*3".to_string(),
                    bare_comment: true,
                },
                "expression_split_by_space",
            ),
            (DiagnosticKind::BareComment, "bare_comment"),
            (DiagnosticKind::SpaceBeforeComma, "space_before_comma"),
            (DiagnosticKind::DoubleQuotedString, "double_quoted_string"),
            (
                DiagnosticKind::UnknownMnemonic {
                    name: "mvoe".to_string(),
                    suggestion: Some("move".to_string()),
                    could_be_label: false,
                },
                "unknown_mnemonic",
            ),
            (
                DiagnosticKind::MnemonicUsedAsLabel {
                    name: "clr".to_string(),
                },
                "mnemonic_used_as_label",
            ),
            (
                DiagnosticKind::WrongOperandCount {
                    mnemonic: "move".to_string(),
                    found: 1,
                    expected: vec![2],
                    at_least: false,
                },
                "wrong_operand_count",
            ),
            (
                DiagnosticKind::MissingCommaBetweenOperands {
                    operand: "d1".to_string(),
                    previous: Some("d0".to_string()),
                },
                "missing_comma_between_operands",
            ),
            (
                DiagnosticKind::InvalidAddressingMode {
                    mnemonic: "divu".to_string(),
                    position: 2,
                    found: "an immediate".to_string(),
                    allowed: vec!["Dn".to_string()],
                    suggestion: None,
                },
                "invalid_addressing_mode",
            ),
            (
                DiagnosticKind::InvalidOperandPair {
                    mnemonic: "addx".to_string(),
                    found: "an immediate and a data register".to_string(),
                    advice: None,
                },
                "invalid_operand_pair",
            ),
            (
                DiagnosticKind::BothOperandsInMemory {
                    mnemonic: "add".to_string(),
                },
                "both_operands_in_memory",
            ),
            (
                DiagnosticKind::AddressRegisterByteSize {
                    mnemonic: "move".to_string(),
                },
                "address_register_byte_size",
            ),
            (
                DiagnosticKind::UnimplementedAddressingMode {
                    operand: "usp".to_string(),
                    description: "the user stack pointer".to_string(),
                    advice: Some("s68k has one stack pointer, `a7`".to_string()),
                },
                "unimplemented_addressing_mode",
            ),
            (
                DiagnosticKind::InvalidAddressWidth {
                    address: "table".to_string(),
                    size: ".b".to_string(),
                },
                "invalid_address_width",
            ),
            (
                DiagnosticKind::ValueOutOfRange {
                    subject: "the count of `addq`".to_string(),
                    value: 9,
                    min: 1,
                    max: 8,
                    advice: Some("`add #n,<ea>` has no such limit".to_string()),
                },
                "value_out_of_range",
            ),
            (
                DiagnosticKind::BareNumberAsAddress { value: 5 },
                "bare_number_as_address",
            ),
            (
                DiagnosticKind::StarIsTheCurrentAddress {
                    mnemonic: "nop".to_string(),
                },
                "star_is_the_current_address",
            ),
            (
                DiagnosticKind::InvalidSize {
                    mnemonic: "move".to_string(),
                    size: ".s".to_string(),
                    allowed: vec![".b".to_string(), ".w".to_string(), ".l".to_string()],
                },
                "invalid_size",
            ),
            (
                DiagnosticKind::ImmediateOutOfRange {
                    value: 256,
                    size: "byte".to_string(),
                    min: -128,
                    max: 255,
                },
                "immediate_out_of_range",
            ),
            (
                DiagnosticKind::UnimplementedOperation {
                    name: "macro".to_string(),
                    reason: "macros are not assembled here".to_string(),
                    alternative: None,
                },
                "unimplemented_operation",
            ),
            (
                DiagnosticKind::SymbolAlreadyDefined {
                    name: "count".to_string(),
                },
                "symbol_already_defined",
            ),
            (
                DiagnosticKind::UndefinedSymbol {
                    name: "cont".to_string(),
                    suggestion: Some("count".to_string()),
                },
                "undefined_symbol",
            ),
            (
                DiagnosticKind::ForwardReferenceNotAllowed {
                    name: "end_of_data".to_string(),
                    directive: "org".to_string(),
                },
                "forward_reference_not_allowed",
            ),
            (DiagnosticKind::DivisionByZero, "division_by_zero"),
            (
                DiagnosticKind::CharacterLiteralTooLong {
                    text: "'abcdefgh'".to_string(),
                    characters: 8,
                },
                "character_literal_too_long",
            ),
            (
                DiagnosticKind::ConstantAbove32Bits {
                    text: "$1234567890".to_string(),
                    value: 0x1234567890,
                },
                "constant_above_32_bits",
            ),
            (
                DiagnosticKind::RegisterListInExpression {
                    name: "AllRegs".to_string(),
                },
                "register_list_in_expression",
            ),
            (DiagnosticKind::OddOrigin { address: 0x2001 }, "odd_origin"),
            (
                DiagnosticKind::AddressUsedTwice { address: 0x2000 },
                "address_used_twice",
            ),
            (DiagnosticKind::CodeAfterEnd, "code_after_end"),
            (
                DiagnosticKind::EntryPointCase {
                    written: "START".to_string(),
                    found: "start".to_string(),
                },
                "entry_point_case_mismatch",
            ),
            (
                DiagnosticKind::EndWithoutAnAddress,
                "end_without_an_address",
            ),
            (
                DiagnosticKind::DirectiveNeedsALabel {
                    directive: "equ".to_string(),
                },
                "directive_needs_a_label",
            ),
            (
                DiagnosticKind::LabelNotAllowed {
                    directive: "page".to_string(),
                    name: "heading".to_string(),
                },
                "label_not_allowed",
            ),
            (
                DiagnosticKind::RegisterListExpected {
                    found: "an immediate operand".to_string(),
                },
                "register_list_expected",
            ),
            (
                DiagnosticKind::NotARegisterList {
                    name: "count".to_string(),
                    kind: "a constant".to_string(),
                },
                "not_a_register_list",
            ),
            (
                DiagnosticKind::RegisterListNotDefinedYet {
                    name: "AllRegs".to_string(),
                },
                "register_list_not_defined_yet",
            ),
            (
                DiagnosticKind::UserDefinedError {
                    message: "the buffer is too small".to_string(),
                    written: true,
                },
                "user_defined_error",
            ),
            (
                DiagnosticKind::NoBytesInAnOffsetRegion {
                    item: "an instruction".to_string(),
                },
                "no_bytes_in_an_offset_region",
            ),
            (
                DiagnosticKind::ValueExpected {
                    directive: "dc.b".to_string(),
                    found: "a data register".to_string(),
                    advice: None,
                },
                "value_expected",
            ),
            (
                DiagnosticKind::UnreadableFile {
                    path: "lib/io.x68".to_string(),
                    suggestion: None,
                    binary: false,
                },
                "unreadable_file",
            ),
        ]
    }

    fn location() -> Location {
        Location::new("main.m68k", 3, 8, 12)
    }

    #[test]
    fn every_kind_has_a_stable_code() {
        let kinds = every_kind();
        for (kind, code) in &kinds {
            assert_eq!(kind.code(), *code, "the code of {kind:?} changed");
        }
        let codes: Vec<&str> = kinds.iter().map(|(_, code)| *code).collect();
        assert_eq!(
            codes, ALL_CODES,
            "every_kind() and ALL_CODES must list the same codes, in the same order"
        );
    }

    #[test]
    fn every_code_is_unique_and_snake_case() {
        let mut seen = std::collections::BTreeSet::new();
        for code in ALL_CODES {
            assert!(seen.insert(*code), "`{code}` is used twice");
            assert!(
                code.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'),
                "`{code}` is not snake_case"
            );
            assert!(!code.starts_with('_') && !code.ends_with('_'), "`{code}`");
        }
    }

    #[test]
    fn every_kind_has_a_message() {
        for (kind, code) in every_kind() {
            let message = kind.message();
            assert!(!message.is_empty(), "`{code}` has an empty message");
            assert!(
                !message.contains("{"),
                "`{code}` has an unrendered placeholder: {message}"
            );
            if let Some(hint) = kind.hint() {
                assert!(!hint.is_empty(), "`{code}` has an empty hint");
                assert!(
                    !hint.contains("{"),
                    "`{code}` has an unrendered hint: {hint}"
                );
            }
        }
    }

    #[test]
    fn severity_follows_the_grammar_table() {
        for (kind, code) in every_kind() {
            let expected = match code {
                "missing_comma_between_operands"
                | "character_literal_too_long"
                | "constant_above_32_bits"
                | "expression_split_by_space"
                | "star_is_the_current_address"
                | "odd_origin"
                | "code_after_end"
                | "entry_point_case_mismatch"
                | "end_without_an_address" => Severity::Warning,
                "bare_comment"
                | "space_before_comma"
                | "double_quoted_string"
                | "bare_number_as_address"
                | "mnemonic_used_as_label" => Severity::Suggestion,
                _ => Severity::Error,
            };
            assert_eq!(kind.severity(), expected, "the severity of `{code}`");
            assert_eq!(Diagnostic::new(kind, location()).severity, expected);
        }
    }

    #[test]
    fn messages_are_written_in_the_voice_of_adr_0003() {
        let found = DiagnosticKind::InvalidAddressingMode {
            mnemonic: "divu".to_string(),
            position: 2,
            found: "an immediate".to_string(),
            allowed: vec![
                "a data register".to_string(),
                "an indirect operand".to_string(),
            ],
            suggestion: None,
        };
        assert_eq!(
            found.message(),
            "the second operand of `divu` cannot be an immediate"
        );
        assert_eq!(
            found.hint(),
            Some("there it takes a data register or an indirect operand".to_string())
        );

        let malformed = DiagnosticKind::MalformedOperand {
            shape: OperandShape::Index,
            problem: "the `)` is missing".to_string(),
        };
        assert_eq!(
            malformed.message(),
            "this looks like an indexed operand, `4(a0,d1.w)`, but the `)` is missing"
        );

        let look_alike = DiagnosticKind::CharacterAboveLatin1 {
            character: '\u{2019}',
        };
        assert_eq!(look_alike.hint(), Some("write `'`".to_string()));

        let unknown = DiagnosticKind::UnknownMnemonic {
            name: "loop".to_string(),
            suggestion: None,
            could_be_label: true,
        };
        assert_eq!(
            unknown.hint(),
            Some("start it in column 1, or end it with a colon, if `loop` is a label".to_string())
        );
    }

    /// A register the 68000 does not have is a legal Symbol name and is read as
    /// one, so the hint answers the mistake that was made rather than offering
    /// to define it.
    #[test]
    fn a_register_that_does_not_exist_is_answered_with_the_registers_there_are() {
        for name in ["d8", "a8", "D9", "a15", "d31"] {
            assert!(numbered_register(name), "`{name}`");
            let kind = DiagnosticKind::UndefinedSymbol {
                name: name.to_string(),
                suggestion: Some("count".to_string()),
            };
            assert_eq!(
                kind.hint(),
                Some(
                    "the data registers are `d0` to `d7` and the address registers `a0` to `a7`"
                        .to_string()
                ),
                "`{name}` is a miscounted register, whatever else is defined"
            );
        }
        // A name that only looks like one keeps the symbol table's answer.
        for name in ["data", "a1x", "A320", "count", "arr2d"] {
            assert!(!numbered_register(name), "`{name}`");
        }
        let ordinary = DiagnosticKind::UndefinedSymbol {
            name: "cont".to_string(),
            suggestion: Some("count".to_string()),
        };
        assert_eq!(ordinary.hint(), Some("did you mean `count`?".to_string()));
    }

    #[test]
    fn a_control_character_is_named_by_its_code() {
        let kind = DiagnosticKind::UnexpectedCharacter { character: '\u{1}' };
        assert_eq!(
            kind.message(),
            "the character $01 cannot start anything here"
        );
    }

    #[test]
    fn the_serialised_shape_is_flat() {
        let diagnostic = Diagnostic::new(
            DiagnosticKind::SymbolAlreadyDefined {
                name: "count".to_string(),
            },
            Location::new("main.m68k", 12, 0, 5),
        )
        .with_related(Location::new("lib/io.x68", 2, 0, 5), "first defined here");
        let json = serde_json::to_value(&diagnostic).expect("a diagnostic serialises");
        assert_eq!(
            json,
            serde_json::json!({
                "severity": "error",
                "code": "symbol_already_defined",
                "message": "`count` is already defined",
                "hint": "a name is defined once; `set` defines a symbol that may be redefined",
                "location": {
                    "file": "main.m68k",
                    "line": 12,
                    "column": 0,
                    "endColumn": 5
                },
                "related": [
                    {
                        "location": {
                            "file": "lib/io.x68",
                            "line": 2,
                            "column": 0,
                            "endColumn": 5
                        },
                        "message": "first defined here"
                    }
                ]
            })
        );
    }

    #[test]
    fn a_diagnostic_without_a_hint_serialises_a_null() {
        let diagnostic = Diagnostic::new(DiagnosticKind::DivisionByZero, location());
        let json = serde_json::to_value(&diagnostic).expect("a diagnostic serialises");
        assert_eq!(json["hint"], serde_json::Value::Null);
        assert_eq!(json["related"], serde_json::json!([]));
        assert_eq!(json["severity"], "error");
    }

    #[test]
    fn every_kind_serialises() {
        for (kind, code) in every_kind() {
            let diagnostic = Diagnostic::new(kind, location());
            let json = serde_json::to_value(&diagnostic).expect("a diagnostic serialises");
            assert_eq!(json["code"], code);
            assert_eq!(json["message"], diagnostic.message());
        }
    }
}
