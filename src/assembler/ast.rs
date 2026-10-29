//! The tree one Source line parses into.
//!
//! One [`Line`] holds the four fields of `docs/grammar.md` 1.3 — a Label, an
//! Operation, a Comment, and whether that Comment is EASy68K's bare one — and
//! every node carries the [`Span`] it covers, so a Diagnostic about any part of
//! it lands on the right columns without the text being read twice.
//!
//! The tree is the *shape* of the line and says nothing about its meaning
//! ([ADR
//! 0003](../../../docs/adr/0003-operands-are-parsed-independently-of-the-instruction.md)):
//! every well-formed [`Operand`] may stand at any position of any Operation,
//! `.s` is a size like any other, and a bare identifier is an
//! [`Operand::Absolute`] even when the Symbol turns out to be a Register list.
//! The analyzer is what compares this against the instruction table.

use serde::Serialize;

use super::source::Span;
use super::token::{NumberBase, QuoteKind, TokenKind};
use crate::machine::Processor;

/// The size an Operation or an Operand is written with (`docs/grammar.md`
/// 1.11).
///
/// `.s` is a branch displacement size; the parser accepts it anywhere and the
/// analyzer is what says "`.s` is not a size for `move`".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SizeSuffix {
    /// `.b`, one byte.
    Byte,
    /// `.w`, one word.
    Word,
    /// `.l`, one long.
    Long,
    /// `.s`, a short branch displacement.
    Short,
}

impl SizeSuffix {
    /// The size a suffix letter names, case insensitively.
    pub fn from_letter(letter: char) -> Option<SizeSuffix> {
        match letter.to_ascii_lowercase() {
            'b' => Some(SizeSuffix::Byte),
            'w' => Some(SizeSuffix::Word),
            'l' => Some(SizeSuffix::Long),
            's' => Some(SizeSuffix::Short),
            _ => None,
        }
    }

    /// The size a whole suffix run names, `None` for anything else.
    pub fn from_run(run: &str) -> Option<SizeSuffix> {
        let mut characters = run.chars();
        let letter = characters.next()?;
        if characters.next().is_some() {
            return None;
        }
        SizeSuffix::from_letter(letter)
    }

    /// The letter the size is written with.
    pub fn letter(&self) -> char {
        match self {
            SizeSuffix::Byte => 'b',
            SizeSuffix::Word => 'w',
            SizeSuffix::Long => 'l',
            SizeSuffix::Short => 's',
        }
    }

    /// How the size is written, dot included: `.b`.
    pub fn suffix(&self) -> &'static str {
        match self {
            SizeSuffix::Byte => ".b",
            SizeSuffix::Word => ".w",
            SizeSuffix::Long => ".l",
            SizeSuffix::Short => ".s",
        }
    }

    /// The name a message calls the size ("byte").
    pub fn name(&self) -> &'static str {
        match self {
            SizeSuffix::Byte => "byte",
            SizeSuffix::Word => "word",
            SizeSuffix::Long => "long",
            SizeSuffix::Short => "short",
        }
    }
}

/// Which bank a [`Register`] is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RegisterKind {
    /// `d0` to `d7`.
    Data,
    /// `a0` to `a7`, `sp` being `a7`.
    Address,
}

/// One of the sixteen general registers.
///
/// `sp` is `a7` and nothing else: the name is not kept, because the printer
/// never writes it back out (`tests/corpus/README.md`, "Registers").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub struct Register {
    /// Which bank it is in.
    pub kind: RegisterKind,
    /// Which register of the bank, 0 to 7.
    pub number: u8,
    /// Where it was written.
    pub span: Span,
}

/// The eight data register names, in order.
const DATA_REGISTER_NAMES: [&str; 8] = ["d0", "d1", "d2", "d3", "d4", "d5", "d6", "d7"];
/// The eight address register names, in order. `sp` is a ninth spelling of `a7`.
const ADDRESS_REGISTER_NAMES: [&str; 8] = ["a0", "a1", "a2", "a3", "a4", "a5", "a6", "a7"];

impl Register {
    /// The register a name stands for, case insensitively, or `None` when the
    /// name is not a register at all.
    pub fn parse(name: &str, span: Span) -> Option<Register> {
        let name = name.to_ascii_lowercase();
        if name == "sp" {
            return Some(Register {
                kind: RegisterKind::Address,
                number: 7,
                span,
            });
        }
        let mut characters = name.chars();
        let bank = match characters.next()? {
            'd' => RegisterKind::Data,
            'a' => RegisterKind::Address,
            _ => return None,
        };
        let digit = characters.next()?;
        if characters.next().is_some() {
            return None;
        }
        let number = digit.to_digit(8)? as u8;
        Some(Register {
            kind: bank,
            number,
            span,
        })
    }

    /// The canonical name of the register, `a7` for `sp`.
    pub fn name(&self) -> &'static str {
        let index = (self.number & 7) as usize;
        match self.kind {
            RegisterKind::Data => DATA_REGISTER_NAMES[index],
            RegisterKind::Address => ADDRESS_REGISTER_NAMES[index],
        }
    }

    /// Where the register sits in a `movem` mask: `d0` to `d7` are 0 to 7 and
    /// `a0` to `a7` are 8 to 15. This is the order a
    /// [`RegisterRange`](RegisterListItem::Range) has to run in.
    pub fn mask_index(&self) -> u8 {
        match self.kind {
            RegisterKind::Data => self.number & 7,
            RegisterKind::Address => 8 + (self.number & 7),
        }
    }
}

/// A register that is an Operand in its own right rather than an Addressing
/// mode (`docs/grammar.md` 1.10).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SpecialRegister {
    /// The status register. Its low byte is the CCR.
    Sr,
    /// The condition code register, the low byte of SR.
    Ccr,
    /// The user stack pointer.
    Usp,
    /// The source function code, a control register of the 68010.
    Sfc,
    /// The destination function code, a control register of the 68010.
    Dfc,
    /// The vector base register, a control register of the 68010.
    Vbr,
    /// The cache control register, a control register of the 68020.
    Cacr,
    /// The cache address register, a control register of the 68020.
    Caar,
    /// The master stack pointer, a control register of the 68020.
    Msp,
    /// The interrupt stack pointer, a control register of the 68020.
    Isp,
}

impl SpecialRegister {
    /// The special register a name stands for, case insensitively.
    pub fn parse(name: &str) -> Option<SpecialRegister> {
        match name.to_ascii_lowercase().as_str() {
            "sr" => Some(SpecialRegister::Sr),
            "ccr" => Some(SpecialRegister::Ccr),
            "usp" => Some(SpecialRegister::Usp),
            "sfc" => Some(SpecialRegister::Sfc),
            "dfc" => Some(SpecialRegister::Dfc),
            "vbr" => Some(SpecialRegister::Vbr),
            "cacr" => Some(SpecialRegister::Cacr),
            "caar" => Some(SpecialRegister::Caar),
            "msp" => Some(SpecialRegister::Msp),
            "isp" => Some(SpecialRegister::Isp),
            _ => None,
        }
    }

    /// The canonical name of the register.
    pub fn name(&self) -> &'static str {
        match self {
            SpecialRegister::Sr => "sr",
            SpecialRegister::Ccr => "ccr",
            SpecialRegister::Usp => "usp",
            SpecialRegister::Sfc => "sfc",
            SpecialRegister::Dfc => "dfc",
            SpecialRegister::Vbr => "vbr",
            SpecialRegister::Cacr => "cacr",
            SpecialRegister::Caar => "caar",
            SpecialRegister::Msp => "msp",
            SpecialRegister::Isp => "isp",
        }
    }

    /// The first processor that has it: the 68000 for `sr`, `ccr` and
    /// `usp`, the 68010 for its control registers and the 68020 for its own.
    pub fn processor(&self) -> Processor {
        match self {
            SpecialRegister::Sr | SpecialRegister::Ccr | SpecialRegister::Usp => Processor::M68000,
            SpecialRegister::Sfc | SpecialRegister::Dfc | SpecialRegister::Vbr => Processor::M68010,
            SpecialRegister::Cacr
            | SpecialRegister::Caar
            | SpecialRegister::Msp
            | SpecialRegister::Isp => Processor::M68020,
        }
    }
}

/// The twenty-eight names no Symbol may carry (`docs/grammar.md` 1.10): the
/// sixteen general registers, `sp`, `pc`, `sr`, `ccr` and `usp`, the 68010's
/// control registers `sfc`, `dfc` and `vbr`, and the 68020's `cacr`, `caar`,
/// `msp` and `isp`, which `movec` names in every Program, whatever its
/// processor.
pub const RESERVED_NAMES: [&str; 28] = [
    "d0", "d1", "d2", "d3", "d4", "d5", "d6", "d7", "a0", "a1", "a2", "a3", "a4", "a5", "a6", "a7",
    "sp", "pc", "sr", "ccr", "usp", "sfc", "dfc", "vbr", "cacr", "caar", "msp", "isp",
];

/// Whether `name` is one of the [`RESERVED_NAMES`], case insensitively.
pub fn is_reserved_name(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    RESERVED_NAMES.contains(&name.as_str())
}

/// Whether `name` is the program counter, which is a register only inside the
/// PC-relative modes.
pub fn is_program_counter(name: &str) -> bool {
    name.eq_ignore_ascii_case("pc")
}

/// The name of a suppressed register of the 68020's indexed Operands, `zd0`
/// to `zd7`, `za0` to `za7` and `zpc` (`docs/grammar.md` 2.5): a register
/// whose place is kept in the encoding and whose value is not added. It is a
/// register only there, as `pc` is, so a Symbol may still be called so.
pub fn suppressed_register(name: &str) -> Option<Suppressed> {
    let name = name.to_ascii_lowercase();
    if name == "zpc" {
        return Some(Suppressed::ProgramCounter);
    }
    let register = Register::parse(name.strip_prefix('z')?, Span::empty(0))?;
    Some(match register.kind {
        RegisterKind::Data => Suppressed::Data(register.number),
        RegisterKind::Address => Suppressed::Address(register.number),
    })
}

/// A suppressed register, as [`suppressed_register`] reads its name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(tag = "kind", content = "number", rename_all = "snake_case")]
pub enum Suppressed {
    /// `zd0` to `zd7`.
    Data(u8),
    /// `za0` to `za7`.
    Address(u8),
    /// `zpc`.
    ProgramCounter,
}

/// The index register of an indexed Operand, with the size it is used at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct IndexRegister {
    /// The register itself.
    pub register: Register,
    /// The size written after it. `None` is `.w`, the 68000's default and what
    /// the fixture printer writes out.
    pub size: Option<SizeSuffix>,
    /// The scale written after it, `*2`, `*4` or `*8` (or `*1`), the 68020's;
    /// `None` when none is written, which is 1.
    pub scale: Option<u8>,
    /// Whether it was written suppressed, `zd1` or `za1`, the 68020's: an
    /// index whose value is not added.
    pub suppressed: bool,
    /// Where the whole `d1.w*4` was written.
    pub span: Span,
}

/// The base of one of the 68020's indexed Operands, [`Operand::FullIndex`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BaseRegister {
    /// `a0` to `a7`.
    Address {
        /// The register.
        register: Register,
    },
    /// `pc`.
    ProgramCounter,
    /// `za0` to `za7`: an address register suppressed.
    SuppressedAddress {
        /// Its number.
        number: u8,
    },
    /// `zpc`: the program counter suppressed, an access in program space
    /// with nothing added for the base.
    SuppressedProgramCounter,
    /// No base written: suppressed, as `za0`.
    Omitted,
}

impl BaseRegister {
    /// Whether it is the program counter, suppressed or not.
    pub fn is_program_counter(&self) -> bool {
        matches!(
            self,
            BaseRegister::ProgramCounter | BaseRegister::SuppressedProgramCounter
        )
    }
}

/// A base or an outer displacement of a 68020 indexed Operand, with the
/// width written after it: `1000.w`, `label.l`, or none, which the Assembler
/// chooses (`instructions::choice::displacement_size`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SizedDisplacement {
    /// The displacement.
    pub value: Expr,
    /// `.w` or `.l` when the source forced one.
    pub size: Option<SizeSuffix>,
}

/// Whether a 68020 indexed Operand reads its address from memory, and where
/// its index is added (`docs/grammar.md` 2.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Indirection {
    /// `(bd,An,Xn)`: no brackets.
    None,
    /// `([bd,An,Xn],od)`: the index inside the brackets, or none at all,
    /// `([bd,An],od)`.
    Preindexed,
    /// `([bd,An],Xn,od)`: the index after the brackets.
    Postindexed,
}

/// One item of a register list: a single register, or a range.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RegisterListItem {
    /// `d3`
    Single {
        /// The register.
        register: Register,
        /// Where it was written.
        span: Span,
    },
    /// `d0-d3`, and it may cross from `d7` into `a0` (`docs/grammar.md` 1.12).
    Range {
        /// The lower end, written first.
        from: Register,
        /// The higher end, written second.
        to: Register,
        /// Where the whole range was written.
        span: Span,
    },
}

impl RegisterListItem {
    /// Where the item was written.
    pub fn span(&self) -> Span {
        match self {
            RegisterListItem::Single { span, .. } => *span,
            RegisterListItem::Range { span, .. } => *span,
        }
    }
}

/// A unary operator of an Expression (`docs/grammar.md` 1.13).
///
/// There are two. EASy68K has no unary `+`, and neither does s68k:
/// `plus_is_not_a_unary_operator` says so.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UnaryOperator {
    /// `-`, negation.
    Minus,
    /// `~`, one's complement.
    Not,
}

impl UnaryOperator {
    /// The operator a token stands for, `None` when it is not one.
    pub fn from_token_kind(kind: TokenKind) -> Option<UnaryOperator> {
        match kind {
            TokenKind::Minus => Some(UnaryOperator::Minus),
            TokenKind::Tilde => Some(UnaryOperator::Not),
            _ => None,
        }
    }

    /// How the operator is written.
    pub fn symbol(&self) -> &'static str {
        match self {
            UnaryOperator::Minus => "-",
            UnaryOperator::Not => "~",
        }
    }
}

/// A binary operator of an Expression, EASy68K's set verbatim
/// (`Directives/operators.htm`).
///
/// `!` and `|` are one operator, logical OR, written two ways; the spelling is
/// recovered from the token's span when a message needs it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BinaryOperator {
    /// `+`
    Add,
    /// `-`
    Subtract,
    /// `*`
    Multiply,
    /// `/`
    Divide,
    /// `\`, the modulus.
    Modulo,
    /// `&`, logical AND.
    And,
    /// `!` or `|`, logical OR.
    Or,
    /// `^`, exclusive OR.
    ExclusiveOr,
    /// `<<`
    ShiftLeft,
    /// `>>`
    ShiftRight,
}

impl BinaryOperator {
    /// The operator a token stands for, `None` when it is not one.
    pub fn from_token_kind(kind: TokenKind) -> Option<BinaryOperator> {
        match kind {
            TokenKind::Plus => Some(BinaryOperator::Add),
            TokenKind::Minus => Some(BinaryOperator::Subtract),
            TokenKind::Star => Some(BinaryOperator::Multiply),
            TokenKind::Slash => Some(BinaryOperator::Divide),
            TokenKind::Backslash => Some(BinaryOperator::Modulo),
            TokenKind::Ampersand => Some(BinaryOperator::And),
            TokenKind::Bang | TokenKind::Pipe => Some(BinaryOperator::Or),
            TokenKind::Caret => Some(BinaryOperator::ExclusiveOr),
            TokenKind::ShiftLeft => Some(BinaryOperator::ShiftLeft),
            TokenKind::ShiftRight => Some(BinaryOperator::ShiftRight),
            _ => None,
        }
    }

    /// How the operator is written, `|` for the OR that may also be written
    /// `!`.
    pub fn symbol(&self) -> &'static str {
        match self {
            BinaryOperator::Add => "+",
            BinaryOperator::Subtract => "-",
            BinaryOperator::Multiply => "*",
            BinaryOperator::Divide => "/",
            BinaryOperator::Modulo => "\\",
            BinaryOperator::And => "&",
            BinaryOperator::Or => "|",
            BinaryOperator::ExclusiveOr => "^",
            BinaryOperator::ShiftLeft => "<<",
            BinaryOperator::ShiftRight => ">>",
        }
    }

    /// How tightly the operator binds; the higher the tighter.
    ///
    /// EASy68K's table (`Directives/operators.htm`), highest first: `>> <<`,
    /// then `& ! | ^`, then `* / \`, then `+ -`. Equal precedence associates
    /// left. These are the binding powers the Pratt loop runs on, and the
    /// layered rules of `docs/grammar.md` 2.7 are the same thing written out.
    pub fn precedence(&self) -> u8 {
        match self {
            BinaryOperator::ShiftLeft | BinaryOperator::ShiftRight => 4,
            BinaryOperator::And | BinaryOperator::Or | BinaryOperator::ExclusiveOr => 3,
            BinaryOperator::Multiply | BinaryOperator::Divide | BinaryOperator::Modulo => 2,
            BinaryOperator::Add | BinaryOperator::Subtract => 1,
        }
    }
}

/// An Expression, as a tree.
///
/// It is evaluated in 64 bits against the symbol table and the current address
/// (`src/assembler/expr.rs`, still to be written); nothing here holds a value
/// other than what the source wrote.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Expr {
    /// A written number, with the base it was written in.
    Number {
        /// Its value.
        value: i64,
        /// The base it was written in, which a message quotes back.
        base: NumberBase,
        /// Where it was written.
        span: Span,
    },
    /// A quoted literal: a character literal in an Expression (`'A'` is 65) and
    /// a string in a `dc` item, which is one token with two readings
    /// (`docs/grammar.md` 1.9). The bytes are Windows-1252 and `''` has already
    /// been read as one quote.
    CharacterLiteral {
        /// The Windows-1252 bytes the literal stands for.
        bytes: Vec<u8>,
        /// Which quote it was written with.
        quote: QuoteKind,
        /// Where it was written.
        span: Span,
    },
    /// A Symbol: a Label, a Constant, a Variable, or a name that turns out to be
    /// none of them. A leading `.` makes it a Local label.
    Symbol {
        /// The name, case sensitive.
        name: String,
        /// Where it was written.
        span: Span,
    },
    /// `*`, the address the next byte will be placed at.
    CurrentAddress {
        /// Where it was written.
        span: Span,
    },
    /// `-x` or `~x`.
    Unary {
        /// Which operator.
        operator: UnaryOperator,
        /// What it applies to.
        operand: Box<Expr>,
        /// Where the whole term was written, operator included.
        span: Span,
    },
    /// `x + y`, and the nine other binary operators.
    Binary {
        /// Which operator.
        operator: BinaryOperator,
        /// The left side.
        left: Box<Expr>,
        /// The right side.
        right: Box<Expr>,
        /// Where the whole Expression was written.
        span: Span,
    },
}

impl Expr {
    /// Where the Expression was written.
    pub fn span(&self) -> Span {
        match self {
            Expr::Number { span, .. }
            | Expr::CharacterLiteral { span, .. }
            | Expr::Symbol { span, .. }
            | Expr::CurrentAddress { span }
            | Expr::Unary { span, .. }
            | Expr::Binary { span, .. } => *span,
        }
    }

    /// Whether this is one Symbol and nothing else, which is what a `dc` item,
    /// a branch target and a Register list reference all start out as.
    pub fn as_symbol(&self) -> Option<&str> {
        match self {
            Expr::Symbol { name, .. } => Some(name),
            _ => None,
        }
    }
}

/// One Operand of an Operation.
///
/// Every Addressing mode of the glossary, plus a Register list and the three
/// special registers. Each carries the Span of the whole Operand, which is what
/// an "invalid addressing mode" Diagnostic underlines.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Operand {
    /// `#5`, and `#5.l`
    Immediate {
        /// The value.
        value: Expr,
        /// Whether it was written `#n.l`, which is EASy68K's way of keeping the
        /// long form of an instruction: a `move.l #1,d0` or an `add.l #1,d0`
        /// whose immediate is written so is not made a `moveq` or an `addq`
        /// (`instructions::choice`). It changes nothing else: the size the
        /// immediate is encoded at is the instruction's.
        long: bool,
        /// Where the whole Operand was written.
        span: Span,
    },
    /// `d3`
    DataRegisterDirect {
        /// The register.
        register: Register,
        /// Where the whole Operand was written.
        span: Span,
    },
    /// `a6`, `sp`
    AddressRegisterDirect {
        /// The register.
        register: Register,
        /// Where the whole Operand was written.
        span: Span,
    },
    /// `sr`, `ccr`, `usp`
    SpecialRegister {
        /// Which one.
        register: SpecialRegister,
        /// Where the whole Operand was written.
        span: Span,
    },
    /// `(a0)`
    Indirect {
        /// The base register.
        register: Register,
        /// Where the whole Operand was written.
        span: Span,
    },
    /// `(a0)+`
    Postincrement {
        /// The base register.
        register: Register,
        /// Where the whole Operand was written.
        span: Span,
    },
    /// `-(a0)`
    Predecrement {
        /// The base register.
        register: Register,
        /// Where the whole Operand was written.
        span: Span,
    },
    /// `4(a0)` and `(4,a0)`
    Displacement {
        /// The displacement.
        displacement: Expr,
        /// The base register.
        base: Register,
        /// Where the whole Operand was written.
        span: Span,
    },
    /// `4(a0,d1.w)`, `(4,a0,d1.w)` and `(a0,d1.w)`
    Index {
        /// The displacement. `None` is the displacement-free `(a0,d1.w)` form,
        /// which is a displacement of zero.
        displacement: Option<Expr>,
        /// The base register.
        base: Register,
        /// The index register and its size.
        index: IndexRegister,
        /// Where the whole Operand was written.
        span: Span,
    },
    /// `label(pc)` and `(label,pc)`
    PcDisplacement {
        /// The displacement.
        displacement: Expr,
        /// Where the whole Operand was written.
        span: Span,
    },
    /// `label(pc,d1.w)`, `(label,pc,d1.w)` and `(pc,d1.w)`
    PcIndex {
        /// The displacement, `None` for the displacement-free form.
        displacement: Option<Expr>,
        /// The index register and its size.
        index: IndexRegister,
        /// Where the whole Operand was written.
        span: Span,
    },
    /// The 68020's indexed forms beyond `d(An,Xn)` and `d(PC,Xn)`: a
    /// displacement written with its width, `(1000.w,a0,d1.l)`; a base or an
    /// index suppressed, `(d0.l*4)`, `(4,za0,d1)`, `(a0,zd0)`, `(zpc,d0)`; and
    /// memory indirection, `([4,a0],d1.w*4,8)` and `([4,a0,d1.w*4],8)`
    /// (`docs/grammar.md` 2.5). A scale alone is the 68000's
    /// [`Index`](Operand::Index) or [`PcIndex`](Operand::PcIndex) with one.
    FullIndex {
        /// The base displacement, `None` when none is written.
        base_displacement: Option<SizedDisplacement>,
        /// The base register.
        base: BaseRegister,
        /// The index register, `None` when none is written.
        index: Option<IndexRegister>,
        /// Whether, and how, the address is read from memory.
        indirection: Indirection,
        /// The outer displacement, after the brackets, `None` when none is
        /// written.
        outer_displacement: Option<SizedDisplacement>,
        /// Where the whole Operand was written.
        span: Span,
    },
    /// `$1000`, `label`, `($1000)`, `label.w`: a bare Expression, with the
    /// width it is forced to when one is written.
    Absolute {
        /// The address.
        value: Expr,
        /// `.w` or `.l` when the source forced one.
        size: Option<SizeSuffix>,
        /// Where the whole Operand was written.
        span: Span,
    },
    /// `d0-d3/a0-a2`. A lone register is **not** this: it parses as a register
    /// direct Operand and the analyzer reads it as a list of one
    /// (`docs/grammar.md` 1.12).
    RegisterList {
        /// The items, in the order they were written.
        items: Vec<RegisterListItem>,
        /// Where the whole Operand was written.
        span: Span,
    },
    /// `d1:d2`, two data registers, the 68020's: the 64-bit product `Dh:Dl`
    /// of `muls.l` and `mulu.l`, the remainder and the quotient `Dr:Dq` of
    /// `divs.l` and its kind, and `cas2`'s two compare and two update
    /// registers (`docs/grammar.md` 2.5).
    RegisterPair {
        /// The register before the colon.
        first: Register,
        /// The register after it.
        second: Register,
        /// Where the whole Operand was written.
        span: Span,
    },
    /// `d0{4:8}` and `(a0){d1:d2}`, an Operand with the `{offset:width}` of
    /// a 68020 bit-field instruction after it (`docs/grammar.md` 2.5).
    BitField {
        /// The Operand the field is in.
        target: Box<Operand>,
        /// The bit it starts at.
        offset: FieldValue,
        /// How many bits it holds.
        width: FieldValue,
        /// Where the whole Operand, field included, was written.
        span: Span,
    },
    /// `(a0):(d1)`, two registers in parentheses, the 68020's: the two
    /// addresses `cas2` compares and swaps through, each in an address or a
    /// data register (`docs/grammar.md` 2.5).
    IndirectPair {
        /// The register of the first parentheses.
        first: Register,
        /// The register of the second.
        second: Register,
        /// Where the whole Operand was written.
        span: Span,
    },
}

/// The offset or the width of a bit field, `{offset:width}`: a number, with
/// or without a `#`, or a data register that holds it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FieldValue {
    /// `4`, `#4`, `width-1`.
    Value {
        /// The number.
        value: Expr,
    },
    /// `d1`.
    Register {
        /// The data register.
        register: Register,
    },
}

impl Operand {
    /// Where the Operand was written.
    pub fn span(&self) -> Span {
        match self {
            Operand::Immediate { span, .. }
            | Operand::DataRegisterDirect { span, .. }
            | Operand::AddressRegisterDirect { span, .. }
            | Operand::SpecialRegister { span, .. }
            | Operand::Indirect { span, .. }
            | Operand::Postincrement { span, .. }
            | Operand::Predecrement { span, .. }
            | Operand::Displacement { span, .. }
            | Operand::Index { span, .. }
            | Operand::PcDisplacement { span, .. }
            | Operand::PcIndex { span, .. }
            | Operand::FullIndex { span, .. }
            | Operand::Absolute { span, .. }
            | Operand::RegisterList { span, .. }
            | Operand::RegisterPair { span, .. }
            | Operand::IndirectPair { span, .. }
            | Operand::BitField { span, .. } => *span,
        }
    }

    /// The name of the Addressing mode, as the serialised form writes it.
    ///
    /// It is the `kind` tag of this enum, spelled once as a `&'static str` so
    /// that the editor's `parseLine` can name the mode of the Operand under the
    /// cursor without serialising the whole Operand. A test asserts the two
    /// stay the same word.
    pub fn mode_name(&self) -> &'static str {
        match self {
            Operand::Immediate { .. } => "immediate",
            Operand::DataRegisterDirect { .. } => "data_register_direct",
            Operand::AddressRegisterDirect { .. } => "address_register_direct",
            Operand::SpecialRegister { .. } => "special_register",
            Operand::Indirect { .. } => "indirect",
            Operand::Postincrement { .. } => "postincrement",
            Operand::Predecrement { .. } => "predecrement",
            Operand::Displacement { .. } => "displacement",
            Operand::Index { .. } => "index",
            Operand::PcDisplacement { .. } => "pc_displacement",
            Operand::PcIndex { .. } => "pc_index",
            Operand::FullIndex { .. } => "full_index",
            Operand::Absolute { .. } => "absolute",
            Operand::RegisterList { .. } => "register_list",
            Operand::RegisterPair { .. } => "register_pair",
            Operand::IndirectPair { .. } => "indirect_pair",
            Operand::BitField { .. } => "bit_field",
        }
    }

    /// How a Diagnostic names the Operand ("an immediate"), which is the
    /// "what was found" half of ADR 0003's message.
    pub fn description(&self) -> &'static str {
        match self {
            Operand::Immediate { .. } => "an immediate",
            Operand::DataRegisterDirect { .. } => "a data register",
            Operand::AddressRegisterDirect { .. } => "an address register",
            Operand::SpecialRegister {
                register: SpecialRegister::Sr,
                ..
            } => "the status register",
            Operand::SpecialRegister {
                register: SpecialRegister::Ccr,
                ..
            } => "the condition codes",
            Operand::SpecialRegister {
                register: SpecialRegister::Usp,
                ..
            } => "the user stack pointer",
            Operand::SpecialRegister {
                register: SpecialRegister::Sfc,
                ..
            } => "the source function code",
            Operand::SpecialRegister {
                register: SpecialRegister::Dfc,
                ..
            } => "the destination function code",
            Operand::SpecialRegister {
                register: SpecialRegister::Vbr,
                ..
            } => "the vector base register",
            Operand::SpecialRegister {
                register: SpecialRegister::Cacr,
                ..
            } => "the cache control register",
            Operand::SpecialRegister {
                register: SpecialRegister::Caar,
                ..
            } => "the cache address register",
            Operand::SpecialRegister {
                register: SpecialRegister::Msp,
                ..
            } => "the master stack pointer",
            Operand::SpecialRegister {
                register: SpecialRegister::Isp,
                ..
            } => "the interrupt stack pointer",
            Operand::Indirect { .. } => "an indirect operand",
            Operand::Postincrement { .. } => "a postincrement operand",
            Operand::Predecrement { .. } => "a predecrement operand",
            Operand::Displacement { .. } => "a displacement operand",
            Operand::Index { .. } => "an indexed operand",
            Operand::PcDisplacement { .. } => "a PC-relative operand",
            Operand::PcIndex { .. } => "a PC-relative indexed operand",
            Operand::FullIndex {
                base, indirection, ..
            } => match (base.is_program_counter(), indirection) {
                (false, Indirection::None) => "an indexed operand",
                (true, Indirection::None) => "a PC-relative indexed operand",
                (false, _) => "a memory indirect operand",
                (true, _) => "a PC-relative memory indirect operand",
            },
            Operand::Absolute { .. } => "an absolute operand",
            Operand::RegisterList { .. } => "a register list",
            Operand::RegisterPair { .. } => "a register pair",
            Operand::IndirectPair { .. } => "a pair of indirect operands",
            Operand::BitField { target, .. } => target.description(),
        }
    }
}

/// The Label field of a line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Label {
    /// The name, without the colon. Case sensitive; a leading `.` makes it a
    /// Local label.
    pub name: String,
    /// Whether it was written with a colon. A Label in column 1 needs none.
    pub colon: bool,
    /// Where the name was written, colon excluded.
    pub span: Span,
}

impl Label {
    /// Whether this is a Local label, scoped to the Global label above it.
    pub fn is_local(&self) -> bool {
        self.name.starts_with('.')
    }
}

/// Which of the three kinds of Comment a line carries (`docs/grammar.md` 1.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CommentKind {
    /// The whole line is a Comment: its first non-blank character is `*` or `;`.
    Line,
    /// A `;` anywhere, or a `*` opening the comment field.
    Explicit,
    /// EASy68K's own comment field, with no marker at all.
    Bare,
}

/// The Comment field of a line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Comment {
    /// Which kind it is.
    pub kind: CommentKind,
    /// The text, marker included, without the line terminator.
    pub text: String,
    /// Where it was written.
    pub span: Span,
}

/// An Operand field that is read as raw text rather than as Operands: the
/// filename of `include` and `incbin`, the message of `fail`, the options of
/// `opt`, and everything on a refused keyword's line (`docs/grammar.md` 2.5,
/// 2.6).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TextOperandField {
    /// The text, exactly as it was written, without the Comment.
    pub text: String,
    /// Where it was written.
    pub span: Span,
}

/// The Operation field of a line: what the line does.
///
/// The name is not classified here — the parser hands it to the analyzer, which
/// owns the one instruction table (ADR 0003). A `text_operation` fills
/// [`text`](Operation::text) and leaves [`operands`](Operation::operands)
/// empty; every other Operation does the reverse, but for `memory`, which
/// fills both: the word naming its kind of range, and its addresses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Operation {
    /// The Mnemonic or Directive name, as it was written. Case insensitive
    /// when it is looked up.
    pub name: String,
    /// Where the name was written.
    pub name_span: Span,
    /// The size suffix, when the source wrote a valid one.
    pub size: Option<SizeSuffix>,
    /// Where the size suffix was written, dot included.
    pub size_span: Option<Span>,
    /// The Operands, in the order they were written.
    pub operands: Vec<Operand>,
    /// The raw Operand field of a `text_operation`, when this is one; or the
    /// `memory_kind` word of a `memory` line (`docs/grammar.md` 2.6).
    pub text: Option<TextOperandField>,
    /// Where the whole Operand field was written, `None` when there is none.
    pub operand_field_span: Option<Span>,
    /// Where the whole Operation was written, Operand field included.
    pub span: Span,
}

impl Operation {
    /// The name in lower case, which is how the instruction table is keyed.
    pub fn lowercase_name(&self) -> String {
        self.name.to_ascii_lowercase()
    }
}

/// One Source line, read into its four fields.
///
/// A line with none of them is a blank line; a line whose Comment is
/// [`CommentKind::Line`] is a Comment line. Every other line has an Operation,
/// a Label, or both.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Line {
    /// The Label the line declares, if any.
    pub label: Option<Label>,
    /// What the line does, if anything.
    pub operation: Option<Operation>,
    /// The Comment field, if any.
    pub comment: Option<Comment>,
    /// Whether that Comment is EASy68K's bare one. It mirrors
    /// `comment.kind == CommentKind::Bare` and is kept beside it because the
    /// `bare_comment` suggestion is raised once per File and the Assembler
    /// reads this field to decide.
    pub bare_comment: bool,
}

impl Line {
    /// Whether the line holds nothing at all.
    pub fn is_blank(&self) -> bool {
        self.label.is_none() && self.operation.is_none() && self.comment.is_none()
    }

    /// Whether the whole line is a Comment.
    pub fn is_comment_line(&self) -> bool {
        self.label.is_none()
            && self.operation.is_none()
            && matches!(
                self.comment,
                Some(Comment {
                    kind: CommentKind::Line,
                    ..
                })
            )
    }

    /// Whether the line puts something in the Program: an Operation, whatever
    /// it turns out to be. A line with a Label and nothing else marks an
    /// address and produces no bytes.
    pub fn has_operation(&self) -> bool {
        self.operation.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn span() -> Span {
        Span::new(0, 2)
    }

    #[test]
    fn register_reads_every_spelling() {
        assert_eq!(
            Register::parse("d0", span()),
            Some(Register {
                kind: RegisterKind::Data,
                number: 0,
                span: span()
            })
        );
        assert_eq!(Register::parse("A7", span()).map(|r| r.name()), Some("a7"));
        assert_eq!(Register::parse("sp", span()).map(|r| r.name()), Some("a7"));
        assert_eq!(
            Register::parse("SP", span()).map(|r| r.kind),
            Some(RegisterKind::Address)
        );
        assert_eq!(Register::parse("d8", span()), None);
        assert_eq!(Register::parse("pc", span()), None);
        assert_eq!(Register::parse("d10", span()), None);
        assert_eq!(Register::parse("count", span()), None);
    }

    #[test]
    fn register_list_order_runs_from_d0_to_a7() {
        let d7 = Register::parse("d7", span()).expect("d7");
        let a0 = Register::parse("a0", span()).expect("a0");
        assert_eq!(d7.mask_index(), 7);
        assert_eq!(a0.mask_index(), 8);
        assert!(
            d7.mask_index() < a0.mask_index(),
            "a range may cross d7 into a0"
        );
    }

    #[test]
    fn the_twenty_eight_reserved_names_are_reserved() {
        assert_eq!(RESERVED_NAMES.len(), 28);
        for name in RESERVED_NAMES {
            assert!(is_reserved_name(name));
            assert!(is_reserved_name(&name.to_ascii_uppercase()));
        }
        assert!(!is_reserved_name("count"));
        assert!(!is_reserved_name("d8"));
        assert!(is_program_counter("PC"));
        assert!(!is_program_counter("pcx"));
    }

    #[test]
    fn every_addressing_mode_names_itself_as_it_serialises() {
        let data = Register::parse("d0", span()).expect("d0");
        let address = Register::parse("a0", span()).expect("a0");
        let index = IndexRegister {
            register: data,
            size: None,
            scale: None,
            suppressed: false,
            span: span(),
        };
        let value = Expr::Number {
            value: 1,
            base: crate::assembler::token::NumberBase::Decimal,
            span: span(),
        };
        let operands = vec![
            Operand::Immediate {
                value: value.clone(),
                long: false,
                span: span(),
            },
            Operand::DataRegisterDirect {
                register: data,
                span: span(),
            },
            Operand::AddressRegisterDirect {
                register: address,
                span: span(),
            },
            Operand::SpecialRegister {
                register: SpecialRegister::Ccr,
                span: span(),
            },
            Operand::Indirect {
                register: address,
                span: span(),
            },
            Operand::Postincrement {
                register: address,
                span: span(),
            },
            Operand::Predecrement {
                register: address,
                span: span(),
            },
            Operand::Displacement {
                displacement: value.clone(),
                base: address,
                span: span(),
            },
            Operand::Index {
                displacement: None,
                base: address,
                index,
                span: span(),
            },
            Operand::PcDisplacement {
                displacement: value.clone(),
                span: span(),
            },
            Operand::PcIndex {
                displacement: None,
                index,
                span: span(),
            },
            Operand::Absolute {
                value,
                size: None,
                span: span(),
            },
            Operand::RegisterList {
                items: Vec::new(),
                span: span(),
            },
        ];
        assert_eq!(operands.len(), 13, "one case per Addressing mode");
        for operand in &operands {
            let serialised = serde_json::to_value(operand).expect("an Operand serialises");
            assert_eq!(
                serialised["kind"],
                operand.mode_name(),
                "`mode_name` is the `kind` tag: {operand:?}"
            );
        }
    }

    #[test]
    fn special_register_reads_its_three_names() {
        assert_eq!(SpecialRegister::parse("SR"), Some(SpecialRegister::Sr));
        assert_eq!(SpecialRegister::parse("ccr"), Some(SpecialRegister::Ccr));
        assert_eq!(SpecialRegister::parse("Usp"), Some(SpecialRegister::Usp));
        assert_eq!(SpecialRegister::parse("pc"), None);
    }

    #[test]
    fn size_suffix_reads_one_letter_and_no_more() {
        assert_eq!(SizeSuffix::from_run("b"), Some(SizeSuffix::Byte));
        assert_eq!(SizeSuffix::from_run("L"), Some(SizeSuffix::Long));
        assert_eq!(SizeSuffix::from_run("s"), Some(SizeSuffix::Short));
        assert_eq!(SizeSuffix::from_run("q"), None);
        assert_eq!(SizeSuffix::from_run("ll"), None);
        assert_eq!(SizeSuffix::from_run(""), None);
        assert_eq!(SizeSuffix::Word.suffix(), ".w");
        assert_eq!(SizeSuffix::Word.name(), "word");
    }

    #[test]
    fn shift_expression_binds_tighter_than_the_rest() {
        // EASy68K's table: `>> <<`, then `& ! | ^`, then `* / \`, then `+ -`.
        let shift = BinaryOperator::ShiftLeft.precedence();
        let bitwise = BinaryOperator::And.precedence();
        let multiplicative = BinaryOperator::Multiply.precedence();
        let additive = BinaryOperator::Add.precedence();
        assert!(shift > bitwise && bitwise > multiplicative && multiplicative > additive);
        assert_eq!(
            BinaryOperator::Or.precedence(),
            BinaryOperator::ExclusiveOr.precedence()
        );
        assert_eq!(BinaryOperator::Modulo.precedence(), multiplicative);
    }

    #[test]
    fn both_spellings_of_or_are_one_operator() {
        assert_eq!(
            BinaryOperator::from_token_kind(TokenKind::Bang),
            Some(BinaryOperator::Or)
        );
        assert_eq!(
            BinaryOperator::from_token_kind(TokenKind::Pipe),
            Some(BinaryOperator::Or)
        );
        assert_eq!(BinaryOperator::from_token_kind(TokenKind::Hash), None);
        assert_eq!(
            UnaryOperator::from_token_kind(TokenKind::Minus),
            Some(UnaryOperator::Minus)
        );
        assert_eq!(UnaryOperator::from_token_kind(TokenKind::Plus), None);
    }

    #[test]
    fn every_node_carries_its_span() {
        let value = Expr::Number {
            value: 2,
            base: NumberBase::Decimal,
            span: Span::new(1, 2),
        };
        let expression = Expr::Binary {
            operator: BinaryOperator::Multiply,
            left: Box::new(value.clone()),
            right: Box::new(Expr::CurrentAddress {
                span: Span::new(3, 4),
            }),
            span: Span::new(1, 4),
        };
        assert_eq!(expression.span(), Span::new(1, 4));
        let operand = Operand::Immediate {
            value: expression,
            long: false,
            span: Span::new(0, 4),
        };
        assert_eq!(operand.span(), Span::new(0, 4));
        assert_eq!(operand.description(), "an immediate");
    }

    #[test]
    fn a_line_knows_what_it_holds() {
        let blank = Line::default();
        assert!(blank.is_blank());
        assert!(!blank.is_comment_line());

        let comment_line = Line {
            comment: Some(Comment {
                kind: CommentKind::Line,
                text: "* a comment".to_string(),
                span: Span::new(0, 11),
            }),
            ..Line::default()
        };
        assert!(comment_line.is_comment_line());
        assert!(!comment_line.is_blank());
        assert!(!comment_line.has_operation());

        let label_only = Line {
            label: Some(Label {
                name: "start".to_string(),
                colon: true,
                span: Span::new(0, 5),
            }),
            ..Line::default()
        };
        assert!(!label_only.is_blank());
        assert!(!label_only.has_operation());
        assert!(!label_only.label.as_ref().expect("a label").is_local());
    }

    #[test]
    fn a_local_label_starts_with_a_dot() {
        let label = Label {
            name: ".retry".to_string(),
            colon: false,
            span: Span::new(0, 6),
        };
        assert!(label.is_local());
    }
}
