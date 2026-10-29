//! The instruction table: the single source of truth about every Mnemonic.
//!
//! [ADR
//! 0003](../../../../docs/adr/0003-operands-are-parsed-independently-of-the-instruction.md)
//! puts every question about an instruction here: which Addressing modes it
//! takes at each position, how many Operands, which sizes and which default,
//! which range its own count or vector has, and the [`Family`] the lowering
//! encodes it as. The parser asks the table one question only — "is this word a
//! Mnemonic?" ([`super::super::names::is_operation_name`]) — and the analyzer
//! asks it all the others.
//!
//! # Reading a row
//!
//! One [`InstructionSpec`] a Mnemonic. Its [`forms`](InstructionSpec::forms)
//! are the shapes it may be written in, one [`Form`] a shape: `asl` has two,
//! `asl #1,d0` and `asl (a0)`, and the analyzer picks the Form by the number of
//! Operands and then by how well they fit. A [`Form`] carries one [`Modes`] set
//! per Operand position, the sizes it accepts, the size it means when the
//! source writes none, and whether the 68000 reserves it for supervisor mode.
//!
//! A real 68000 Mnemonic s68k does not implement is in the table too, marked
//! [`Implementation::NotImplemented`] with the reason, so that writing it gets
//! "`movep` is not implemented: …" and never "unknown instruction" (the design
//! record, "Scope").
//!
//! # Processors
//!
//! Every [`Form`] names the first [`Processor`] that has it
//! ([`Form::processor`]), and the analyzer refuses one above the Program's
//! processor, which `opt p=` chooses (ADR 0011). The 68010's are complete:
//! `move ccr,<ea>`, `rtd`, `bkpt`, `movec` and `moves`. So are the 68020's:
//! its own instructions and the shapes it gave a 68000 Mnemonic, `muls.l`,
//! `chk.l` and `tst a0` among them, as Forms of their own, but `callm` and
//! `rtm`, which are [`Implementation::NotImplemented`]. Its eight bit-field
//! instructions are [`Form::is_bit_field`], which EASy68K's `opt bit` gives a
//! 68000 or a 68010 Program too.
//!
//! A Form also names the first processor that runs it only in supervisor
//! mode ([`Form::privileged`]): `move sr,<ea>` is the 68000's in either mode
//! and privileged from the 68010 on.

use bitflags::bitflags;

use super::encoded::{BitFieldOperation, Condition, Sign, Size};
use crate::assembler::ast::{Operand, SizeSuffix, SpecialRegister};
use crate::machine::Processor;

bitflags! {
    /// The Addressing modes an Operand position accepts.
    ///
    /// The names are the ones a Diagnostic prints ([`Modes::names`]), which are
    /// the notation the old checker used and the one the asm-editor's
    /// documentation writes: `Dn`, `An`, `(An)`, `(An)+`, `-(An)`, `d(An)`,
    /// `d(An,Xn)`, `Ea/<label>`, `d(PC)`, `d(PC,Xn)`, `Im`.
    ///
    /// Every Addressing mode of the language is here since phase 3 (CONTEXT.md,
    /// "Addressing mode"): the
    /// PC-relative pair were deliberately absent while the analyzer answered
    /// them with "not implemented yet", and they are ordinary modes now, in
    /// the groups the reference puts them in — data, memory and control, and
    /// never alterable, because nothing is written through the program
    /// counter. `sr`, `ccr` and `usp` are modes of their own, because an
    /// instruction that takes one takes nothing else in that position; `usp`
    /// is `move usp,An` and `move An,usp`'s alone.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub struct Modes: u32 {
        /// `d0` to `d7`.
        const DN = 1 << 0;
        /// `a0` to `a7`, `sp` included.
        const AN = 1 << 1;
        /// `(a0)`
        const INDIRECT = 1 << 2;
        /// `(a0)+`
        const POSTINCREMENT = 1 << 3;
        /// `-(a0)`
        const PREDECREMENT = 1 << 4;
        /// `4(a0)`
        const DISPLACEMENT = 1 << 5;
        /// `4(a0,d1.w)`
        const INDEX = 1 << 6;
        /// `$2000`, and a Label, which is one.
        const ABSOLUTE = 1 << 7;
        /// `#5`
        const IMMEDIATE = 1 << 8;
        /// `d0-d3/a0-a2`, which only `movem` takes.
        const REGISTER_LIST = 1 << 9;
        /// `sr`, the status register.
        const SR = 1 << 10;
        /// `ccr`, the condition codes.
        const CCR = 1 << 11;
        /// `label(pc)`
        const PC_DISPLACEMENT = 1 << 12;
        /// `label(pc,d1.w)`
        const PC_INDEX = 1 << 13;
        /// `usp`, the user stack pointer.
        const USP = 1 << 14;
        /// `sfc`, `dfc` or `vbr`, a control register of the 68010 that is no
        /// stack pointer, or one of the 68020's, `cacr`, `caar`, `msp` or
        /// `isp`, which only `movec` reaches; it reaches `usp` too.
        const CONTROL_REGISTER = 1 << 15;
        /// `d1:d2`, two data registers, the 68020's: `Dh:Dl` of a long
        /// multiply, `Dr:Dq` of a long divide, `cas2`'s compare and update
        /// pairs.
        const REGISTER_PAIR = 1 << 16;
        /// `(a0):(d1)`, `cas2`'s two addresses.
        const INDIRECT_PAIR = 1 << 17;
        /// The `{offset:width}` of a 68020 bit field, after its Operand: a
        /// position that takes one takes it with the modes beside it, and an
        /// Operand written with one is of its target's mode and this.
        const BIT_FIELD = 1 << 18;
    }
}

impl Modes {
    /// Every mode an ordinary Operand may be: everything but a register list.
    pub const ALL: Modes = Modes::DN
        .union(Modes::AN)
        .union(Modes::MEMORY)
        .union(Modes::IMMEDIATE);
    /// The two PC-relative modes, which are read and never written.
    pub const PC_RELATIVE: Modes = Modes::PC_DISPLACEMENT.union(Modes::PC_INDEX);
    /// The eight modes that name a place in memory.
    pub const MEMORY: Modes = Modes::INDIRECT
        .union(Modes::POSTINCREMENT)
        .union(Modes::PREDECREMENT)
        .union(Modes::DISPLACEMENT)
        .union(Modes::INDEX)
        .union(Modes::ABSOLUTE)
        .union(Modes::PC_RELATIVE);
    /// The 68000's "data addressing modes": everything but an address register.
    pub const DATA: Modes = Modes::ALL.difference(Modes::AN);
    /// The 68000's "alterable addressing modes": everything that can be
    /// written, which is everything but an immediate and the two PC-relative
    /// modes — the program counter is read, and a program does not write
    /// through it.
    pub const ALTERABLE: Modes = Modes::ALL
        .difference(Modes::IMMEDIATE)
        .difference(Modes::PC_RELATIVE);
    /// Data and alterable at once: the destination of `clr`, `neg`, `not`,
    /// `Scc` and the immediate instructions.
    pub const DATA_ALTERABLE: Modes = Modes::DATA.intersection(Modes::ALTERABLE);
    /// In memory and alterable: the destination of a memory shift, and of the
    /// `<ea>` half of `add`, `and` and `or`.
    pub const MEMORY_ALTERABLE: Modes = Modes::MEMORY.difference(Modes::PC_RELATIVE);
    /// The "control addressing modes": a place in memory that is not walked
    /// over, so neither `(An)+` nor `-(An)`. `lea`, `pea`, `jmp` and `jsr` take
    /// these, the PC-relative pair included.
    pub const CONTROL: Modes = Modes::INDIRECT
        .union(Modes::DISPLACEMENT)
        .union(Modes::INDEX)
        .union(Modes::ABSOLUTE)
        .union(Modes::PC_RELATIVE);
    /// Control and alterable: control without the two modes nothing writes
    /// through.
    pub const CONTROL_ALTERABLE: Modes = Modes::CONTROL.difference(Modes::PC_RELATIVE);
    /// Where `movem` may put registers: control alterable, plus `-(An)`. The
    /// PC-relative modes are not among them, which is the one place the two
    /// directions of `movem` differ by more than the side the list is on.
    pub const MOVEM_TO_MEMORY: Modes = Modes::CONTROL_ALTERABLE.union(Modes::PREDECREMENT);
    /// Where `movem` may read them back from: control, plus `(An)+`.
    pub const MOVEM_FROM_MEMORY: Modes = Modes::CONTROL.union(Modes::POSTINCREMENT);
    /// A register list, or the single register that stands for a list of one.
    pub const REGISTER_LIST_OR_REGISTER: Modes =
        Modes::REGISTER_LIST.union(Modes::DN).union(Modes::AN);
    /// Where a shift count or a bit number may come from.
    pub const COUNT: Modes = Modes::DN.union(Modes::IMMEDIATE);
    /// Either bank of general registers, which is what `exg` swaps.
    pub const ANY_REGISTER: Modes = Modes::DN.union(Modes::AN);
    /// The two halves of the status register, which are the only Operands a
    /// position holding one accepts: a Form that names either of them names no
    /// ordinary Addressing mode beside it, which is what
    /// `Analyzer::choose_form` reads.
    pub const STATUS: Modes = Modes::SR.union(Modes::CCR);
    /// The special registers: the two halves of the status register, the
    /// user stack pointer and the 68010's other control registers. A position
    /// that takes one takes nothing else but another of them.
    pub const SPECIAL: Modes = Modes::STATUS
        .union(Modes::USP)
        .union(Modes::CONTROL_REGISTER);
    /// What `movec` reads or writes beside a general register: a control
    /// register, the user stack pointer among them.
    pub const MOVEC_CONTROL: Modes = Modes::USP.union(Modes::CONTROL_REGISTER);

    /// The mode one parsed Operand is. Every Operand of the language is one of
    /// the table's modes; the `Option` is kept for a mode a later processor
    /// adds that the table cannot talk about yet.
    pub fn of(operand: &Operand) -> Option<Modes> {
        Some(match operand {
            Operand::Immediate { .. } => Modes::IMMEDIATE,
            Operand::DataRegisterDirect { .. } => Modes::DN,
            Operand::AddressRegisterDirect { .. } => Modes::AN,
            Operand::Indirect { .. } => Modes::INDIRECT,
            Operand::Postincrement { .. } => Modes::POSTINCREMENT,
            Operand::Predecrement { .. } => Modes::PREDECREMENT,
            Operand::Displacement { .. } => Modes::DISPLACEMENT,
            Operand::Index { .. } => Modes::INDEX,
            Operand::Absolute { .. } => Modes::ABSOLUTE,
            Operand::RegisterList { .. } => Modes::REGISTER_LIST,
            Operand::SpecialRegister {
                register: SpecialRegister::Sr,
                ..
            } => Modes::SR,
            Operand::SpecialRegister {
                register: SpecialRegister::Ccr,
                ..
            } => Modes::CCR,
            Operand::SpecialRegister {
                register: SpecialRegister::Usp,
                ..
            } => Modes::USP,
            Operand::SpecialRegister {
                register:
                    SpecialRegister::Sfc
                    | SpecialRegister::Dfc
                    | SpecialRegister::Vbr
                    | SpecialRegister::Cacr
                    | SpecialRegister::Caar
                    | SpecialRegister::Msp
                    | SpecialRegister::Isp,
                ..
            } => Modes::CONTROL_REGISTER,
            Operand::PcDisplacement { .. } => Modes::PC_DISPLACEMENT,
            Operand::PcIndex { .. } => Modes::PC_INDEX,
            Operand::RegisterPair { .. } => Modes::REGISTER_PAIR,
            Operand::IndirectPair { .. } => Modes::INDIRECT_PAIR,
            Operand::BitField { target, .. } => {
                return Modes::of(target).map(|mode| mode | Modes::BIT_FIELD)
            }
            // The 68020's forms are encoded in the modes of `d8(An,Xn)` and
            // `d8(PC,Xn)`, so the positions that take one take them all.
            Operand::FullIndex { base, .. } => match base.is_program_counter() {
                true => Modes::PC_INDEX,
                false => Modes::INDEX,
            },
        })
    }

    /// How a message names each mode of the set, in the table's own order.
    ///
    /// This is the "what is allowed" half of an ADR 0003 message:
    /// `["Dn", "(An)", "Ea/<label>"]`.
    pub fn names(&self) -> Vec<String> {
        MODE_NAMES
            .iter()
            .filter(|(mode, _)| self.contains(*mode))
            .map(|(_, name)| (*name).to_string())
            .collect()
    }
}

/// Every mode with the name a message gives it, in the order a message lists
/// them: registers, then the ways of reaching memory, then an immediate.
const MODE_NAMES: [(Modes, &str); 19] = [
    (Modes::DN, "Dn"),
    (Modes::AN, "An"),
    (Modes::INDIRECT, "(An)"),
    (Modes::POSTINCREMENT, "(An)+"),
    (Modes::PREDECREMENT, "-(An)"),
    (Modes::DISPLACEMENT, "d(An)"),
    (Modes::INDEX, "d(An,Xn)"),
    (Modes::ABSOLUTE, "Ea/<label>"),
    (Modes::PC_DISPLACEMENT, "d(PC)"),
    (Modes::PC_INDEX, "d(PC,Xn)"),
    (Modes::IMMEDIATE, "Im"),
    (Modes::REGISTER_LIST, "<register list>"),
    (Modes::SR, "sr"),
    (Modes::CCR, "ccr"),
    (Modes::USP, "usp"),
    (Modes::CONTROL_REGISTER, "Rc"),
    (Modes::REGISTER_PAIR, "Dx:Dy"),
    (Modes::INDIRECT_PAIR, "(Rx):(Ry)"),
    (Modes::BIT_FIELD, "{offset:width}"),
];

/// The sizes a [`Form`] accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SizeRule {
    /// The instruction carries no size at all: `moveq`, `lea`, `jsr`, `nop`.
    Unsized,
    /// `.b`, `.w` and `.l`.
    Any,
    /// `.w` and `.l`: everything whose destination is an address register, and
    /// `movem` and `ext`.
    WordOrLong,
    /// `.l` only: `extb`.
    LongOnly,
    /// `.w` only: the memory form of a shift, which shifts one word by one bit,
    /// and the instructions the help gives a data length of "Word" — `swap`,
    /// `divs`, `divu`, `muls`, `mulu` and every `DBcc`.
    WordOnly,
    /// `.b` only: every `Scc`, which writes one byte.
    ByteOnly,
    /// `.b` or `.l`: the bit instructions, whose width is decided by where the
    /// destination is — a byte in memory, a long in a data register
    /// (the help's "DATA LENGTH: Byte, longword").
    ByteOrLong,
    /// `.b`, `.s`, `.w` and `.l` on a branch, which is a displacement width and
    /// not an operand size: the size of the branch the lowering stores
    /// ([`choice::branch_size`](super::choice::branch_size)), whose distance the
    /// analyzer checks against it at the addresses the Layout gave the
    /// instructions, their real sizes. `.b` is
    /// `.s`: EASy68K "will accept .B or .S to force 1-byte offsets and .W or
    /// .L to force 2-byte offsets" (`Reference/68ks9b.htm`), so a program that
    /// writes `bra.b` assembles here as it does there.
    Branch,
}

impl SizeRule {
    /// The sizes the rule accepts, in the order a message lists them.
    pub fn allowed(&self) -> &'static [SizeSuffix] {
        match self {
            SizeRule::Unsized => &[],
            SizeRule::Any => &[SizeSuffix::Byte, SizeSuffix::Word, SizeSuffix::Long],
            SizeRule::WordOrLong => &[SizeSuffix::Word, SizeSuffix::Long],
            SizeRule::LongOnly => &[SizeSuffix::Long],
            SizeRule::WordOnly => &[SizeSuffix::Word],
            SizeRule::ByteOnly => &[SizeSuffix::Byte],
            SizeRule::ByteOrLong => &[SizeSuffix::Byte, SizeSuffix::Long],
            SizeRule::Branch => &[
                SizeSuffix::Byte,
                SizeSuffix::Short,
                SizeSuffix::Word,
                SizeSuffix::Long,
            ],
        }
    }

    /// Whether the rule accepts `size`.
    pub fn accepts(&self, size: SizeSuffix) -> bool {
        self.allowed().contains(&size)
    }

    /// The size an instruction of this rule means when the source writes none.
    ///
    /// Word wherever there is a choice, which is the 68000's own default and
    /// what 1.4.2 stored; `None` where the instruction carries no size, a
    /// branch included (`tests/corpus/README.md` prints no size for one), and
    /// for a bit instruction, where the destination decides the width and the
    /// encoded instruction holds no size at all.
    pub fn default_size(&self) -> Option<Size> {
        match self {
            SizeRule::Unsized | SizeRule::Branch | SizeRule::ByteOrLong => None,
            SizeRule::Any | SizeRule::WordOrLong | SizeRule::WordOnly => Some(Size::Word),
            SizeRule::LongOnly => Some(Size::Long),
            SizeRule::ByteOnly => Some(Size::Byte),
        }
    }
}

/// A rule about the two Operands together, which no per-position rule can say.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Combination {
    /// Nothing beyond the per-position rules.
    None,
    /// At most one Operand may be in memory: the 68000 has no memory-to-memory
    /// `add`, `sub`, `and` or `or`.
    AtMostOneMemoryOperand,
}

/// One shape an instruction may be written in.
///
/// Most instructions have one; `asl` and its kind have two (a count and a
/// register, or one memory Operand) and `movem` has two (registers out,
/// registers back).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Form {
    /// One rule per Operand position; its length is the Operand count.
    pub operands: &'static [Modes],
    /// The sizes this shape accepts.
    pub sizes: SizeRule,
    /// What the Operands may not be together.
    pub combination: Combination,
    /// The first processor that runs this shape only in supervisor mode, or
    /// `None` when every processor runs it in either: the 68000 for
    /// `move <ea>,sr`, `andi`, `ori` and `eori` into `sr`, `rte`, `stop`,
    /// `reset` and `move usp`, and the 68010 for `move sr,<ea>`, which the
    /// 68010 made privileged, and for its own `movec` and `moves`. No `ccr`
    /// shape ever was.
    ///
    /// A privileged Form assembles like any other: a program decides its mode
    /// as it runs, and the Interpreter raises a privilege violation, vector 8,
    /// when one runs in user mode (ADR 0008).
    pub privileged: Option<Processor>,
    /// The first processor that has the shape: the 68000 for all but
    /// `move ccr,<ea>`, the 68010's, and `extb.l`, the 68020's. The analyzer
    /// refuses a Form above the Program's processor.
    pub processor: Processor,
}

impl Form {
    /// Whether `processor` runs the shape only in supervisor mode.
    pub fn is_privileged_on(&self, processor: Processor) -> bool {
        self.privileged
            .is_some_and(|first| processor.has(first) && processor.has(self.processor))
    }

    /// How many Operands the shape takes.
    pub fn arity(&self) -> usize {
        self.operands.len()
    }

    /// Whether these Operands fit the shape, position by position: each of
    /// a mode the position takes, with a bit field exactly where it takes
    /// one.
    ///
    /// A mode this phase does not assemble ([`Modes::of`] answers `None` for
    /// one) decides nothing here: the analyzer has already reported it.
    pub fn fits(&self, operands: &[Operand]) -> bool {
        self.arity() == operands.len()
            && operands
                .iter()
                .enumerate()
                .all(|(index, operand)| match Modes::of(operand) {
                    Some(mode) => {
                        let allowed = self.operands[index];
                        allowed.contains(mode)
                            && allowed.contains(Modes::BIT_FIELD) == mode.contains(Modes::BIT_FIELD)
                    }
                    None => true,
                })
    }

    /// Whether it is one of the 68020's bit-field instructions', which
    /// `opt bit` gives a 68000 Program.
    pub fn is_bit_field(&self) -> bool {
        self.operands
            .iter()
            .any(|modes| modes.contains(Modes::BIT_FIELD))
    }
}

/// A form with no rule about the Operands together.
const fn form(operands: &'static [Modes], sizes: SizeRule) -> Form {
    dialect_form(Processor::M68000, operands, sizes)
}

/// A form a later processor than the 68000 introduced ([`Form::processor`]).
const fn dialect_form(processor: Processor, operands: &'static [Modes], sizes: SizeRule) -> Form {
    Form {
        operands,
        sizes,
        combination: Combination::None,
        privileged: None,
        processor,
    }
}

/// A form a later processor introduced and runs only in supervisor mode.
const fn privileged_dialect_form(
    processor: Processor,
    operands: &'static [Modes],
    sizes: SizeRule,
) -> Form {
    Form {
        operands,
        sizes,
        combination: Combination::None,
        privileged: Some(processor),
        processor,
    }
}

/// A form of an instruction that cannot read and write memory at once.
const fn register_form(operands: &'static [Modes], sizes: SizeRule) -> Form {
    Form {
        operands,
        sizes,
        combination: Combination::AtMostOneMemoryOperand,
        privileged: None,
        processor: Processor::M68000,
    }
}

/// A form the 68000 runs only in supervisor mode ([`Form::privileged`]).
const fn privileged_form(operands: &'static [Modes], sizes: SizeRule) -> Form {
    Form {
        operands,
        sizes,
        combination: Combination::None,
        privileged: Some(Processor::M68000),
        processor: Processor::M68000,
    }
}

/// The range an instruction's own count or vector has to be in.
///
/// This is not the range check against an operand size — that one follows from
/// the size and is the analyzer's `immediate_out_of_range`. This is the field
/// the instruction itself encodes: `addq` adds 1 to 8 and nothing else,
/// `trap` has sixteen vectors, and a shift shifts 1 to 8 places. A static bit
/// number has none: EASy68K encodes any, and the 68000 takes it modulo the
/// destination's width.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ValueRule {
    /// How a message names the value: "the count of `addq`".
    pub subject: &'static str,
    /// The lowest value the field holds.
    pub min: i64,
    /// The highest value it holds.
    pub max: i64,
    /// What to do about it, used as the Diagnostic's hint word for word, when
    /// there is something short to say.
    pub hint: Option<&'static str>,
}

/// A value rule.
const fn value(subject: &'static str, min: i64, max: i64, hint: Option<&'static str>) -> ValueRule {
    ValueRule {
        subject,
        min,
        max,
        hint,
    }
}

/// Which encoding an instruction belongs to, and so which function of
/// [`super::lowering`] builds it.
///
/// A Family is not a Mnemonic: several Mnemonics share one
/// ([`Family::Immediate`] is all six of `addi`, `subi`, `andi`, `ori`, `eori`
/// and `cmpi`), and one Mnemonic may encode as several instructions, which is
/// what the normalisations of `tests/corpus/README.md` are — `add #1,d0` is an
/// `addi` and `move.l d0,a0` is a `movea`. The Family is where each of those
/// choices is written down once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    /// `move`, which becomes `movea` when the destination is an address
    /// register.
    Move,
    /// `movea`, written out.
    Movea,
    /// `movem`, either direction.
    Movem,
    /// `moveq`.
    Moveq,
    /// `add` and `sub`, which become `adda`/`suba` into an address register
    /// and `addi`/`subi` from an immediate into anything else, and
    /// `addq`/`subq` when the immediate is 1 to 8. The flag says which of the
    /// two.
    AddSub {
        /// `true` for `sub`, `false` for `add`.
        subtract: bool,
    },
    /// `addx` and `subx`, which carry the extend flag in as well.
    AddSubExtended {
        /// `true` for `subx`, `false` for `addx`.
        subtract: bool,
    },
    /// `abcd` and `sbcd`, the same two shapes in decimal.
    AddSubDecimal {
        /// `true` for `sbcd`, `false` for `abcd`.
        subtract: bool,
    },
    /// `adda` and `suba`, written out.
    AddSubAddress {
        /// `true` for `suba`, `false` for `adda`.
        subtract: bool,
    },
    /// `addq` and `subq`.
    AddSubQuick {
        /// `true` for `subq`, `false` for `addq`.
        subtract: bool,
    },
    /// `addi`, `subi`, `andi`, `ori`, `eori` and `cmpi`, one variant each.
    Immediate(ImmediateKind),
    /// `cmp`, which becomes `cmpa` into an address register, `cmpi` from an
    /// immediate into memory and `cmpm` between two postincrements; an
    /// immediate compared with a data register stays a `cmp`.
    Cmp,
    /// `cmpa`, written out.
    Cmpa,
    /// `cmpm`, written out.
    Cmpm,
    /// `and`, `or` and `eor`, which become `andi`, `ori` and `eori` from an
    /// immediate into anything but a data register, and `eor` from an
    /// immediate into anything at all.
    Logical(LogicalKind),
    /// `divs` and `divu`.
    Divide(Sign),
    /// `muls` and `mulu`, and their long forms, the 68020's.
    Multiply(Sign),
    /// `divsl` and `divul`, the 68020's: 32 bits by 32 into `Dr:Dq`.
    DivideLong(Sign),
    /// `clr`.
    Clr,
    /// `neg`.
    Neg,
    /// `negx`: `neg` with the extend flag subtracted as well.
    NegExtended,
    /// `nbcd`: the tens complement, which is `negx` in decimal.
    NegDecimal,
    /// `not`.
    Not,
    /// `tst`.
    Tst,
    /// `ext`: byte to word with `.w`, word to long with `.l`.
    Ext,
    /// `extb`: byte to long, the one form `extb.l`.
    ExtByteToLong,
    /// `swap`.
    Swap,
    /// `exg`.
    Exg,
    /// `lea`.
    Lea,
    /// `pea`.
    Pea,
    /// `link`.
    Link,
    /// `unlk`.
    Unlk,
    /// `jmp`.
    Jmp,
    /// `jsr`.
    Jsr,
    /// `bra`.
    Bra,
    /// `bsr`.
    Bsr,
    /// Every `Bcc`, with the condition it tests.
    Bcc(Condition),
    /// Every `Scc`.
    Scc(Condition),
    /// Every `DBcc`, `dbra` being `dbf`.
    DBcc(Condition),
    /// `asl`, `asr`, `lsl`, `lsr`, `rol` and `ror`.
    Shift(ShiftKind, ShiftWay),
    /// `btst`, `bset`, `bclr` and `bchg`.
    Bit(BitOperation),
    /// `trap`.
    Trap,
    /// `rts`.
    Rts,
    /// `nop`.
    Nop,
    /// `movep`, either direction.
    Movep,
    /// `tas`.
    Tas,
    /// `rtr`.
    Rtr,
    /// `rte`.
    Rte,
    /// `stop #n`.
    Stop,
    /// `reset`.
    Reset,
    /// `chk`.
    Chk,
    /// `trapv`.
    Trapv,
    /// `illegal`.
    Illegal,
    /// `rtd #d`, the 68010's.
    Rtd,
    /// `bkpt #n`, the 68010's.
    Bkpt,
    /// `movec`, either direction, the 68010's.
    Movec,
    /// Every `trapcc`, the 68020's, with the condition it tests.
    TrapCc(Condition),
    /// `pack` and `unpk`, the 68020's.
    Pack {
        /// `true` for `unpk`.
        unpack: bool,
    },
    /// `chk2` and `cmp2`, the 68020's.
    BoundsCheck {
        /// `true` for `chk2`, which traps out of bounds.
        trap: bool,
    },
    /// `cas`, the 68020's.
    Cas,
    /// `cas2`, the 68020's.
    Cas2,
    /// The 68020's eight bit-field instructions, which `opt bit` gives a
    /// 68000 Program.
    BitField(BitFieldOperation),
    /// `moves`, either direction, the 68010's.
    Moves,
}

/// Which of the six immediate instructions a [`Family::Immediate`] is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImmediateKind {
    /// `addi`
    Add,
    /// `subi`
    Sub,
    /// `andi`
    And,
    /// `ori`
    Or,
    /// `eori`
    Eor,
    /// `cmpi`
    Cmp,
}

/// Which of the three register-to-register logical instructions this is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogicalKind {
    /// `and`
    And,
    /// `or`
    Or,
    /// `eor`
    Eor,
}

/// Which of the three shifts this is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShiftKind {
    /// `asl`, `asr`: the sign is kept.
    Arithmetic,
    /// `lsl`, `lsr`: zeros come in.
    Logical,
    /// `rol`, `ror`: the bits come round.
    Rotate,
    /// `roxl`, `roxr`: the bits come round through the extend flag, which makes
    /// the rotation 9, 17 or 33 bits wide (`Reference/68ks7g.htm`).
    RotateExtend,
}

/// Which way a shift moves its bits. The same thing as
/// [`ShiftDirection`](super::encoded::ShiftDirection), kept apart so that the
/// table names a direction without depending on the encoded types' spelling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShiftWay {
    /// `asl`, `lsl`, `rol`.
    Left,
    /// `asr`, `lsr`, `ror`.
    Right,
}

/// Which of the four bit instructions this is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BitOperation {
    /// `btst`: read the bit.
    Test,
    /// `bset`: read it and set it.
    Set,
    /// `bclr`: read it and clear it.
    Clear,
    /// `bchg`: read it and flip it.
    Change,
}

/// Whether s68k assembles a Mnemonic, and what to say when it does not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Implementation {
    /// s68k assembles and runs it, as the given [`Family`].
    Implemented(Family),
    /// A real 68000 instruction s68k does not assemble. Writing it is
    /// `unimplemented_operation` and never `unknown_mnemonic`, so that a
    /// student is told the difference between a typo and a missing feature
    /// (the design record, "Scope").
    NotImplemented {
        /// Why, as the clause after "`movep` is not implemented: ".
        reason: &'static str,
        /// What to write instead, when there is something.
        alternative: Option<&'static str>,
    },
}

/// One row of the table: everything that is known about one Mnemonic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InstructionSpec {
    /// The Mnemonic, in lower case. Lookup is case insensitive.
    pub mnemonic: &'static str,
    /// Whether s68k assembles it, and as what.
    pub implementation: Implementation,
    /// The shapes it may be written in. Empty when it is not implemented.
    pub forms: &'static [Form],
    /// The range of the instruction's own count, vector or bit number, when it
    /// has one. It is checked against the first Operand, and only when that
    /// Operand is an immediate.
    pub value_rule: Option<ValueRule>,
}

impl InstructionSpec {
    /// The [`Family`] that encodes it, or `None` when it is not implemented.
    pub fn family(&self) -> Option<Family> {
        match self.implementation {
            Implementation::Implemented(family) => Some(family),
            Implementation::NotImplemented { .. } => None,
        }
    }

    /// Whether s68k assembles it.
    pub fn is_implemented(&self) -> bool {
        matches!(self.implementation, Implementation::Implemented(_))
    }

    /// The Operand counts the instruction accepts, lowest first.
    pub fn arities(&self) -> Vec<usize> {
        let mut arities: Vec<usize> = self.forms.iter().map(Form::arity).collect();
        arities.sort_unstable();
        arities.dedup();
        arities
    }

    /// Whether some Form of the instruction fits these Operands as they stand.
    ///
    /// The Layout asks it before it reads a bare name as a `reg` Symbol: in
    /// `movem.l table,d0-d2` the first Operand is an address and the *second*
    /// is the register list, so a `table` that is no register list is no
    /// mistake. Only when nothing fits is a name in a register-list position
    /// answered by name.
    pub fn has_a_form_that_fits(&self, operands: &[Operand]) -> bool {
        self.forms.iter().any(|form| form.fits(operands))
    }

    /// Whether some Form of the instruction accepts a register list at
    /// `position`.
    ///
    /// It is what tells a bare name that stands for a `reg` Symbol from one
    /// that stands for an address: `movem AllRegs,-(a7)` reads position 0 as a
    /// register list, `move AllRegs,d0` reads it as an Expression and gets
    /// EASy68K's "Register list symbol used in an expression". Today only
    /// `movem` answers `true`, and it answers it for both of its positions,
    /// because either of them holds the list depending on the direction.
    ///
    /// The Operand count is deliberately not part of the question: a `movem`
    /// with the wrong number of Operands should be told about the count and not
    /// also about a name it would have read as a list.
    pub fn takes_a_register_list(&self, position: usize) -> bool {
        self.forms.iter().any(|form| {
            form.operands
                .get(position)
                .is_some_and(|modes| modes.contains(Modes::REGISTER_LIST))
        })
    }

    /// Every size any of its forms accepts, in the order a message lists them.
    pub fn sizes(&self) -> Vec<SizeSuffix> {
        let mut sizes: Vec<SizeSuffix> = Vec::new();
        for form in self.forms {
            for size in form.sizes.allowed() {
                if !sizes.contains(size) {
                    sizes.push(*size);
                }
            }
        }
        sizes
    }
}

/// An implemented row.
const fn implemented(
    mnemonic: &'static str,
    family: Family,
    forms: &'static [Form],
) -> InstructionSpec {
    InstructionSpec {
        mnemonic,
        implementation: Implementation::Implemented(family),
        forms,
        value_rule: None,
    }
}

/// An implemented row whose first Operand is a field of the instruction with a
/// range of its own.
const fn implemented_with_value(
    mnemonic: &'static str,
    family: Family,
    forms: &'static [Form],
    value_rule: ValueRule,
) -> InstructionSpec {
    InstructionSpec {
        mnemonic,
        implementation: Implementation::Implemented(family),
        forms,
        value_rule: Some(value_rule),
    }
}

/// A row for a real 68000 instruction s68k does not assemble.
const fn not_implemented(
    mnemonic: &'static str,
    reason: &'static str,
    alternative: Option<&'static str>,
) -> InstructionSpec {
    InstructionSpec {
        mnemonic,
        implementation: Implementation::NotImplemented {
            reason,
            alternative,
        },
        forms: &[],
        value_rule: None,
    }
}

/// The one form every `Bcc`, `bra` and `bsr` shares: a Label, and a branch
/// size suffix that is accepted and not range checked.
const BRANCH_FORM: &[Form] = &[form(&[Modes::ABSOLUTE], SizeRule::Branch)];
/// The one form every `DBcc` shares: a counting data register and a Label.
const DECREMENT_BRANCH_FORM: &[Form] = &[form(&[Modes::DN, Modes::ABSOLUTE], SizeRule::WordOnly)];
/// The one form every `Scc` shares: one data-alterable Operand, set to all ones
/// or all zeros.
const SET_CONDITIONALLY_FORM: &[Form] = &[form(&[Modes::DATA_ALTERABLE], SizeRule::ByteOnly)];
/// The two forms of a shift or rotate: a count and a data register, or one
/// memory Operand, which shifts one word by one place.
const SHIFT_FORMS: &[Form] = &[
    form(&[Modes::COUNT, Modes::DN], SizeRule::Any),
    form(&[Modes::MEMORY_ALTERABLE], SizeRule::WordOnly),
];
/// The form of `btst`, which only reads its destination and so takes any data
/// mode.
const BIT_TEST_FORM: &[Form] = &[form(&[Modes::COUNT, Modes::DATA], SizeRule::ByteOrLong)];
/// The form of `bset`, `bclr` and `bchg`, which write their destination.
const BIT_WRITE_FORM: &[Form] = &[form(
    &[Modes::COUNT, Modes::DATA_ALTERABLE],
    SizeRule::ByteOrLong,
)];

/// The five Forms of `move`: the general one first, so that a `move` that fits
/// none is judged against it, then the four that name a half of the status
/// register.
///
/// `move <ea>,ccr` and `move <ea>,sr` take any **data** Addressing mode, an
/// immediate included, and `move sr,<ea>` writes any **data alterable** one;
/// all four are a word (`Reference/68ks4d.htm`). `move ccr,<ea>` is the one the
/// 68000 does not have: it is the 68010's, and assembles in a Program written
/// for the 68010 or the 68020 (ADR 0011). On a 68000, `move sr,<ea>` reads
/// the condition codes in its low byte.
///
/// `move <ea>,sr` is privileged: it can clear the supervisor bit. Reading `sr`
/// is not privileged on a 68000, and is from the 68010 on, which gave
/// `move ccr,<ea>` to user mode instead. The last two Forms are `move usp,An` and
/// `move An,usp`, a long between an address register and the user stack
/// pointer, which only supervisor mode reaches (`Reference/68ks4f.htm`).
const MOVE_FORMS: &[Form] = &[
    form(&[Modes::ALL, Modes::ALTERABLE], SizeRule::Any),
    form(&[Modes::DATA, Modes::CCR], SizeRule::WordOnly),
    privileged_form(&[Modes::DATA, Modes::SR], SizeRule::WordOnly),
    Form {
        operands: &[Modes::SR, Modes::DATA_ALTERABLE],
        sizes: SizeRule::WordOnly,
        combination: Combination::None,
        privileged: Some(Processor::M68010),
        processor: Processor::M68000,
    },
    dialect_form(
        Processor::M68010,
        &[Modes::CCR, Modes::DATA_ALTERABLE],
        SizeRule::WordOnly,
    ),
    privileged_form(&[Modes::USP, Modes::AN], SizeRule::LongOnly),
    privileged_form(&[Modes::AN, Modes::USP], SizeRule::LongOnly),
];
/// The three Forms of `andi`, `ori` and `eori`: the ordinary one, then the two
/// the help adds — "Operations that uses the status register (SR) and the flag
/// register (CCR) can only work with word and byte" (`Reference/68ks6d.htm`,
/// the `EORI` page, which is the one that carries that sentence;
/// `Reference/68ks6b.htm` is where the address-method list is) — a byte into
/// `ccr` and a word into `sr`. The one into `sr` is privileged, as
/// `move <ea>,sr` is.
const IMMEDIATE_LOGICAL_FORMS: &[Form] = &[
    form(&[Modes::IMMEDIATE, Modes::DATA_ALTERABLE], SizeRule::Any),
    form(&[Modes::IMMEDIATE, Modes::CCR], SizeRule::ByteOnly),
    privileged_form(&[Modes::IMMEDIATE, Modes::SR], SizeRule::WordOnly),
];
/// The two Forms of `addx` and `subx`, which are the whole of what they take:
/// two data registers, or two predecrement Operands and nothing else
/// ("ADDRESS METHODS: Dn, -(An)", `Reference/68ks5e.htm` and `68ks5v.htm`).
///
/// The register form adds the two registers; the memory form is the one that
/// walks a multi-precision number down through memory, which is why the 68000
/// offers `-(An)` and no other way of reaching it. A mixture of the two is not
/// an instruction, and the analyzer answers one by naming both shapes and
/// `add`.
const EXTENDED_PAIR_FORMS: &[Form] = &[
    form(&[Modes::DN, Modes::DN], SizeRule::Any),
    form(&[Modes::PREDECREMENT, Modes::PREDECREMENT], SizeRule::Any),
];
/// The same two Forms for `abcd` and `sbcd`, which work on one byte and take no
/// other size ("DATA LENGTH: Byte", `Reference/68ks8e.htm` and `68ks8g.htm`).
const DECIMAL_PAIR_FORMS: &[Form] = &[
    form(&[Modes::DN, Modes::DN], SizeRule::ByteOnly),
    form(
        &[Modes::PREDECREMENT, Modes::PREDECREMENT],
        SizeRule::ByteOnly,
    ),
];
/// The two Forms of `movep`, one a direction: a data register and a
/// displacement Operand, which is the only Addressing mode it has
/// (`Reference/68ks4g.htm`), or `(An)`, which EASy68K takes as a displacement
/// of 0 (`movepfl` in its `INSTTABL.CPP`, `movep` in its `BUILD.CPP`).
const MOVEP_FORMS: &[Form] = &[
    form(&[Modes::DN, MOVEP_MEMORY], SizeRule::WordOrLong),
    form(&[MOVEP_MEMORY, Modes::DN], SizeRule::WordOrLong),
];

/// The Forms of `muls` and `mulu`: a word by a word into a long, the 68000's,
/// and the 68020's long, into `Dl` or into `Dh:Dl`.
const LONG_MULTIPLY_FORMS: &[Form] = &[
    form(&[Modes::DATA, Modes::DN], SizeRule::WordOnly),
    dialect_form(
        Processor::M68020,
        &[Modes::DATA, Modes::DN.union(Modes::REGISTER_PAIR)],
        SizeRule::LongOnly,
    ),
];
/// The Forms of `divs` and `divu`: a long by a word, the 68000's, and the
/// 68020's long, into `Dq` or of 64 bits in `Dr:Dq`.
const LONG_DIVIDE_FORMS: &[Form] = LONG_MULTIPLY_FORMS;
/// The one Form of `divsl` and `divul`, the 68020's: 32 bits by 32, the
/// remainder into `Dr` and the quotient into `Dq`.
const DIVIDE_WITH_REMAINDER_FORM: &[Form] = &[dialect_form(
    Processor::M68020,
    &[Modes::DATA, Modes::REGISTER_PAIR],
    SizeRule::LongOnly,
)];

/// The two Forms of `pack` and `unpk`, the 68020's: two data registers, or
/// two predecrements, and the adjustment, a word, as an immediate.
const PACK_FORMS: &[Form] = &[
    dialect_form(
        Processor::M68020,
        &[Modes::DN, Modes::DN, Modes::IMMEDIATE],
        SizeRule::Unsized,
    ),
    dialect_form(
        Processor::M68020,
        &[Modes::PREDECREMENT, Modes::PREDECREMENT, Modes::IMMEDIATE],
        SizeRule::Unsized,
    ),
];

/// The bit field a bit-field instruction reads: in a data register or in a
/// control mode, the PC-relative pair included.
const BIT_FIELD_READ: Modes = Modes::DN.union(Modes::CONTROL).union(Modes::BIT_FIELD);
/// The bit field one writes: in a data register or a control alterable mode.
const BIT_FIELD_WRITTEN: Modes = Modes::DN
    .union(Modes::CONTROL_ALTERABLE)
    .union(Modes::BIT_FIELD);

/// A bit-field instruction's row, the 68020's.
const fn bit_field(
    mnemonic: &'static str,
    operation: BitFieldOperation,
    forms: &'static [Form],
) -> InstructionSpec {
    implemented(mnemonic, Family::BitField(operation), forms)
}

/// The Form of `bfextu`, `bfexts` and `bfffo`: a bit field read, and the
/// data register written.
const BIT_FIELD_INTO_A_REGISTER: &[Form] = &[dialect_form(
    Processor::M68020,
    &[BIT_FIELD_READ, Modes::DN],
    SizeRule::Unsized,
)];
/// The Form of `bfchg`, `bfclr` and `bfset`: a bit field written.
const BIT_FIELD_CHANGED: &[Form] = &[dialect_form(
    Processor::M68020,
    &[BIT_FIELD_WRITTEN],
    SizeRule::Unsized,
)];

/// The memory Operand of `movep`: `d(An)`, and `(An)` as `0(An)`.
const MOVEP_MEMORY: Modes = Modes::DISPLACEMENT.union(Modes::INDIRECT);

/// A `Bcc` row.
const fn branch(mnemonic: &'static str, condition: Condition) -> InstructionSpec {
    implemented(mnemonic, Family::Bcc(condition), BRANCH_FORM)
}

/// The two Forms every `trapcc` shares, the 68020's: no operand, or an
/// immediate of a word or a long, which only the handler reads.
const TRAP_CONDITIONALLY_FORMS: &[Form] = &[
    dialect_form(Processor::M68020, &[], SizeRule::Unsized),
    dialect_form(Processor::M68020, &[Modes::IMMEDIATE], SizeRule::WordOrLong),
];

/// A `trapcc` row.
const fn trap_conditionally(mnemonic: &'static str, condition: Condition) -> InstructionSpec {
    implemented(
        mnemonic,
        Family::TrapCc(condition),
        TRAP_CONDITIONALLY_FORMS,
    )
}

/// A `DBcc` row.
const fn decrement_branch(mnemonic: &'static str, condition: Condition) -> InstructionSpec {
    implemented(mnemonic, Family::DBcc(condition), DECREMENT_BRANCH_FORM)
}

/// An `Scc` row.
const fn set_conditionally(mnemonic: &'static str, condition: Condition) -> InstructionSpec {
    implemented(mnemonic, Family::Scc(condition), SET_CONDITIONALLY_FORM)
}

/// A shift or rotate row.
const fn shift(mnemonic: &'static str, kind: ShiftKind, way: ShiftWay) -> InstructionSpec {
    implemented_with_value(
        mnemonic,
        Family::Shift(kind, way),
        SHIFT_FORMS,
        value(
            "the count of a shift",
            1,
            8,
            Some("put the count in a data register: `asl d1,d0` shifts by whatever `d1` holds"),
        ),
    )
}

/// A bit instruction row. A static bit number has no value rule: EASy68K
/// encodes its low byte whatever it is (`staticBit` in its `BUILD.CPP`), and
/// the 68000 takes it modulo the destination's width, which the analyzer
/// warns of (`bit_number_wraps`).
const fn bit(
    mnemonic: &'static str,
    operation: BitOperation,
    forms: &'static [Form],
) -> InstructionSpec {
    implemented(mnemonic, Family::Bit(operation), forms)
}

/// Every Mnemonic s68k reads, implemented or not.
///
/// The order is the design record's grouping — data movement, arithmetic,
/// binary coded decimal, logic, shifts, bits, program control — so that a
/// reader looking for a neighbour of an instruction finds it beside it. Lookup
/// is [`lookup`] and does not depend on the order.
pub const TABLE: &[InstructionSpec] = &[
    // ---- data movement ----
    implemented("move", Family::Move, MOVE_FORMS),
    implemented(
        "movea",
        Family::Movea,
        &[form(&[Modes::ALL, Modes::AN], SizeRule::WordOrLong)],
    ),
    implemented(
        "movem",
        Family::Movem,
        &[
            form(
                &[Modes::REGISTER_LIST_OR_REGISTER, Modes::MOVEM_TO_MEMORY],
                SizeRule::WordOrLong,
            ),
            form(
                &[Modes::MOVEM_FROM_MEMORY, Modes::REGISTER_LIST_OR_REGISTER],
                SizeRule::WordOrLong,
            ),
        ],
    ),
    implemented("movep", Family::Movep, MOVEP_FORMS),
    implemented_with_value(
        "moveq",
        Family::Moveq,
        &[form(&[Modes::IMMEDIATE, Modes::DN], SizeRule::LongOnly)],
        value(
            "the value of `moveq`",
            -128,
            255,
            Some("`move.l #n,d0` takes any value"),
        ),
    ),
    implemented(
        "exg",
        Family::Exg,
        &[form(
            &[Modes::ANY_REGISTER, Modes::ANY_REGISTER],
            SizeRule::LongOnly,
        )],
    ),
    implemented(
        "lea",
        Family::Lea,
        &[form(&[Modes::CONTROL, Modes::AN], SizeRule::LongOnly)],
    ),
    implemented(
        "pea",
        Family::Pea,
        &[form(&[Modes::CONTROL], SizeRule::LongOnly)],
    ),
    // `link.l`, a long displacement, is the 68020's
    implemented(
        "link",
        Family::Link,
        &[
            form(&[Modes::AN, Modes::IMMEDIATE], SizeRule::Unsized),
            dialect_form(
                Processor::M68020,
                &[Modes::AN, Modes::IMMEDIATE],
                SizeRule::LongOnly,
            ),
        ],
    ),
    implemented(
        "unlk",
        Family::Unlk,
        &[form(&[Modes::AN], SizeRule::Unsized)],
    ),
    implemented(
        "swap",
        Family::Swap,
        &[form(&[Modes::DN], SizeRule::WordOnly)],
    ),
    // ---- integer arithmetic ----
    implemented(
        "add",
        Family::AddSub { subtract: false },
        &[register_form(
            &[Modes::ALL, Modes::ALTERABLE],
            SizeRule::Any,
        )],
    ),
    implemented(
        "sub",
        Family::AddSub { subtract: true },
        &[register_form(
            &[Modes::ALL, Modes::ALTERABLE],
            SizeRule::Any,
        )],
    ),
    implemented(
        "adda",
        Family::AddSubAddress { subtract: false },
        &[form(&[Modes::ALL, Modes::AN], SizeRule::WordOrLong)],
    ),
    implemented(
        "suba",
        Family::AddSubAddress { subtract: true },
        &[form(&[Modes::ALL, Modes::AN], SizeRule::WordOrLong)],
    ),
    implemented(
        "addi",
        Family::Immediate(ImmediateKind::Add),
        &[form(
            &[Modes::IMMEDIATE, Modes::DATA_ALTERABLE],
            SizeRule::Any,
        )],
    ),
    implemented(
        "subi",
        Family::Immediate(ImmediateKind::Sub),
        &[form(
            &[Modes::IMMEDIATE, Modes::DATA_ALTERABLE],
            SizeRule::Any,
        )],
    ),
    implemented_with_value(
        "addq",
        Family::AddSubQuick { subtract: false },
        &[form(&[Modes::IMMEDIATE, Modes::ALTERABLE], SizeRule::Any)],
        value(
            "the count of `addq`",
            1,
            8,
            Some("`add #n,<ea>` has no such limit"),
        ),
    ),
    implemented_with_value(
        "subq",
        Family::AddSubQuick { subtract: true },
        &[form(&[Modes::IMMEDIATE, Modes::ALTERABLE], SizeRule::Any)],
        value(
            "the count of `subq`",
            1,
            8,
            Some("`sub #n,<ea>` has no such limit"),
        ),
    ),
    implemented(
        "addx",
        Family::AddSubExtended { subtract: false },
        EXTENDED_PAIR_FORMS,
    ),
    implemented(
        "subx",
        Family::AddSubExtended { subtract: true },
        EXTENDED_PAIR_FORMS,
    ),
    implemented(
        "negx",
        Family::NegExtended,
        &[form(&[Modes::DATA_ALTERABLE], SizeRule::Any)],
    ),
    implemented(
        "clr",
        Family::Clr,
        &[form(&[Modes::DATA_ALTERABLE], SizeRule::Any)],
    ),
    implemented(
        "cmp",
        Family::Cmp,
        // Three forms of two Operands, and the order decides: the general
        // one first, so that a `cmp` that fits none is judged against it; the
        // `cmpm` shape second, which is the only way two postincrements are
        // legal (`tests/corpus/README.md`, "Normalisations"); and an
        // immediate compared with memory third, which is `cmpi`, as EASy68K's
        // `cmpfl` has it. An immediate compared with a register is the first
        // form's, and is a `cmp`.
        &[
            form(&[Modes::ALL, Modes::ANY_REGISTER], SizeRule::Any),
            form(&[Modes::POSTINCREMENT, Modes::POSTINCREMENT], SizeRule::Any),
            form(&[Modes::IMMEDIATE, Modes::DATA_ALTERABLE], SizeRule::Any),
            dialect_form(
                Processor::M68020,
                &[Modes::IMMEDIATE, Modes::PC_RELATIVE],
                SizeRule::Any,
            ),
        ],
    ),
    implemented(
        "cmpa",
        Family::Cmpa,
        &[form(&[Modes::ALL, Modes::AN], SizeRule::WordOrLong)],
    ),
    // against the program counter on the 68020
    implemented(
        "cmpi",
        Family::Immediate(ImmediateKind::Cmp),
        &[
            form(&[Modes::IMMEDIATE, Modes::DATA_ALTERABLE], SizeRule::Any),
            dialect_form(
                Processor::M68020,
                &[Modes::IMMEDIATE, Modes::PC_RELATIVE],
                SizeRule::Any,
            ),
        ],
    ),
    implemented(
        "cmpm",
        Family::Cmpm,
        &[form(
            &[Modes::POSTINCREMENT, Modes::POSTINCREMENT],
            SizeRule::Any,
        )],
    ),
    implemented("divs", Family::Divide(Sign::Signed), LONG_DIVIDE_FORMS),
    implemented("divu", Family::Divide(Sign::Unsigned), LONG_DIVIDE_FORMS),
    implemented("muls", Family::Multiply(Sign::Signed), LONG_MULTIPLY_FORMS),
    implemented(
        "mulu",
        Family::Multiply(Sign::Unsigned),
        LONG_MULTIPLY_FORMS,
    ),
    implemented(
        "divsl",
        Family::DivideLong(Sign::Signed),
        DIVIDE_WITH_REMAINDER_FORM,
    ),
    implemented(
        "divul",
        Family::DivideLong(Sign::Unsigned),
        DIVIDE_WITH_REMAINDER_FORM,
    ),
    implemented(
        "ext",
        Family::Ext,
        &[form(&[Modes::DN], SizeRule::WordOrLong)],
    ),
    // The 68020's, and the first member of its dialect (ADR 0011).
    implemented(
        "extb",
        Family::ExtByteToLong,
        &[dialect_form(
            Processor::M68020,
            &[Modes::DN],
            SizeRule::LongOnly,
        )],
    ),
    implemented(
        "neg",
        Family::Neg,
        &[form(&[Modes::DATA_ALTERABLE], SizeRule::Any)],
    ),
    // the 68020 tests an address register, a word or a long, a PC-relative
    // operand and an immediate too
    implemented(
        "tst",
        Family::Tst,
        &[
            form(&[Modes::DATA_ALTERABLE], SizeRule::Any),
            dialect_form(Processor::M68020, &[Modes::ALL], SizeRule::Any),
        ],
    ),
    // ---- binary coded decimal ----
    implemented(
        "abcd",
        Family::AddSubDecimal { subtract: false },
        DECIMAL_PAIR_FORMS,
    ),
    implemented(
        "sbcd",
        Family::AddSubDecimal { subtract: true },
        DECIMAL_PAIR_FORMS,
    ),
    implemented(
        "nbcd",
        Family::NegDecimal,
        &[form(&[Modes::DATA_ALTERABLE], SizeRule::ByteOnly)],
    ),
    // ---- logical ----
    implemented(
        "and",
        Family::Logical(LogicalKind::And),
        &[register_form(
            &[Modes::DATA, Modes::DATA_ALTERABLE],
            SizeRule::Any,
        )],
    ),
    implemented(
        "or",
        Family::Logical(LogicalKind::Or),
        &[register_form(
            &[Modes::DATA, Modes::DATA_ALTERABLE],
            SizeRule::Any,
        )],
    ),
    implemented(
        "eor",
        Family::Logical(LogicalKind::Eor),
        // A data register, and an immediate, which is `eori`: EASy68K's
        // `eorfl` has both, and `eor` has no `<ea>,dn` form for an immediate
        // to be read by. The register form stays first, so that a line that
        // fits neither is judged against it.
        &[
            form(&[Modes::DN, Modes::DATA_ALTERABLE], SizeRule::Any),
            form(&[Modes::IMMEDIATE, Modes::DATA_ALTERABLE], SizeRule::Any),
        ],
    ),
    implemented(
        "andi",
        Family::Immediate(ImmediateKind::And),
        IMMEDIATE_LOGICAL_FORMS,
    ),
    implemented(
        "ori",
        Family::Immediate(ImmediateKind::Or),
        IMMEDIATE_LOGICAL_FORMS,
    ),
    implemented(
        "eori",
        Family::Immediate(ImmediateKind::Eor),
        IMMEDIATE_LOGICAL_FORMS,
    ),
    implemented(
        "not",
        Family::Not,
        &[form(&[Modes::DATA_ALTERABLE], SizeRule::Any)],
    ),
    // ---- shift and rotate ----
    shift("asl", ShiftKind::Arithmetic, ShiftWay::Left),
    shift("asr", ShiftKind::Arithmetic, ShiftWay::Right),
    shift("lsl", ShiftKind::Logical, ShiftWay::Left),
    shift("lsr", ShiftKind::Logical, ShiftWay::Right),
    shift("rol", ShiftKind::Rotate, ShiftWay::Left),
    shift("ror", ShiftKind::Rotate, ShiftWay::Right),
    shift("roxl", ShiftKind::RotateExtend, ShiftWay::Left),
    shift("roxr", ShiftKind::RotateExtend, ShiftWay::Right),
    // ---- bit manipulation ----
    bit("btst", BitOperation::Test, BIT_TEST_FORM),
    bit("bset", BitOperation::Set, BIT_WRITE_FORM),
    bit("bclr", BitOperation::Clear, BIT_WRITE_FORM),
    bit("bchg", BitOperation::Change, BIT_WRITE_FORM),
    // ---- program control ----
    implemented(
        "bra",
        Family::Bra,
        &[form(&[Modes::ABSOLUTE], SizeRule::Branch)],
    ),
    implemented(
        "bsr",
        Family::Bsr,
        &[form(&[Modes::ABSOLUTE], SizeRule::Branch)],
    ),
    implemented(
        "jmp",
        Family::Jmp,
        &[form(&[Modes::CONTROL], SizeRule::Unsized)],
    ),
    implemented(
        "jsr",
        Family::Jsr,
        &[form(&[Modes::CONTROL], SizeRule::Unsized)],
    ),
    implemented("rts", Family::Rts, &[form(&[], SizeRule::Unsized)]),
    implemented("nop", Family::Nop, &[form(&[], SizeRule::Unsized)]),
    implemented_with_value(
        "trap",
        Family::Trap,
        &[form(&[Modes::IMMEDIATE], SizeRule::Unsized)],
        value(
            "the vector of `trap`",
            0,
            15,
            Some(
                "`trap #0` to `#14` go through vectors 32 to 46, and `trap #15` is EASy68K's \
                 input and output",
            ),
        ),
    ),
    implemented("rtr", Family::Rtr, &[form(&[], SizeRule::Unsized)]),
    implemented(
        "rte",
        Family::Rte,
        &[privileged_form(&[], SizeRule::Unsized)],
    ),
    implemented(
        "tas",
        Family::Tas,
        &[form(&[Modes::DATA_ALTERABLE], SizeRule::ByteOnly)],
    ),
    implemented("trapv", Family::Trapv, &[form(&[], SizeRule::Unsized)]),
    // `chk.l` is the 68020's
    implemented(
        "chk",
        Family::Chk,
        &[
            form(&[Modes::DATA, Modes::DN], SizeRule::WordOnly),
            dialect_form(
                Processor::M68020,
                &[Modes::DATA, Modes::DN],
                SizeRule::LongOnly,
            ),
        ],
    ),
    implemented("illegal", Family::Illegal, &[form(&[], SizeRule::Unsized)]),
    // `stop #n` loads the whole status register from its immediate word and
    // waits for an interrupt request above the new mask; `reset` asserts the
    // reset line to devices s68k does not have, and only takes its time
    implemented(
        "stop",
        Family::Stop,
        &[privileged_form(&[Modes::IMMEDIATE], SizeRule::Unsized)],
    ),
    implemented(
        "reset",
        Family::Reset,
        &[privileged_form(&[], SizeRule::Unsized)],
    ),
    // ---- Bcc ----
    branch("bcc", Condition::CarryClear),
    branch("bhs", Condition::CarryClear),
    branch("bcs", Condition::CarrySet),
    branch("blo", Condition::CarrySet),
    branch("beq", Condition::Equal),
    branch("bne", Condition::NotEqual),
    branch("bge", Condition::GreaterThanOrEqual),
    branch("bgt", Condition::GreaterThan),
    branch("bhi", Condition::High),
    branch("ble", Condition::LessThanOrEqual),
    branch("bls", Condition::LowOrSame),
    branch("blt", Condition::LessThan),
    branch("bmi", Condition::Minus),
    branch("bpl", Condition::Plus),
    branch("bvc", Condition::OverflowClear),
    branch("bvs", Condition::OverflowSet),
    // ---- DBcc ----
    decrement_branch("dbcc", Condition::CarryClear),
    decrement_branch("dbhs", Condition::CarryClear),
    decrement_branch("dbcs", Condition::CarrySet),
    decrement_branch("dblo", Condition::CarrySet),
    decrement_branch("dbeq", Condition::Equal),
    decrement_branch("dbne", Condition::NotEqual),
    decrement_branch("dbge", Condition::GreaterThanOrEqual),
    decrement_branch("dbgt", Condition::GreaterThan),
    decrement_branch("dbhi", Condition::High),
    decrement_branch("dble", Condition::LessThanOrEqual),
    decrement_branch("dbls", Condition::LowOrSame),
    decrement_branch("dblt", Condition::LessThan),
    decrement_branch("dbmi", Condition::Minus),
    decrement_branch("dbpl", Condition::Plus),
    decrement_branch("dbvc", Condition::OverflowClear),
    decrement_branch("dbvs", Condition::OverflowSet),
    decrement_branch("dbt", Condition::True),
    decrement_branch("dbf", Condition::False),
    decrement_branch("dbra", Condition::False),
    // ---- Scc ----
    set_conditionally("scc", Condition::CarryClear),
    set_conditionally("shs", Condition::CarryClear),
    set_conditionally("scs", Condition::CarrySet),
    set_conditionally("slo", Condition::CarrySet),
    set_conditionally("seq", Condition::Equal),
    set_conditionally("sne", Condition::NotEqual),
    set_conditionally("sge", Condition::GreaterThanOrEqual),
    set_conditionally("sgt", Condition::GreaterThan),
    set_conditionally("shi", Condition::High),
    set_conditionally("sle", Condition::LessThanOrEqual),
    set_conditionally("sls", Condition::LowOrSame),
    set_conditionally("slt", Condition::LessThan),
    set_conditionally("smi", Condition::Minus),
    set_conditionally("spl", Condition::Plus),
    set_conditionally("svc", Condition::OverflowClear),
    set_conditionally("svs", Condition::OverflowSet),
    set_conditionally("st", Condition::True),
    set_conditionally("sf", Condition::False),
    // ---- the 68010's ----
    implemented_with_value(
        "rtd",
        Family::Rtd,
        &[dialect_form(
            Processor::M68010,
            &[Modes::IMMEDIATE],
            SizeRule::Unsized,
        )],
        value(
            "the displacement of `rtd`",
            -32768,
            32767,
            Some("it is a word, sign extended, added to the stack pointer after the return"),
        ),
    ),
    implemented_with_value(
        "bkpt",
        Family::Bkpt,
        &[dialect_form(
            Processor::M68010,
            &[Modes::IMMEDIATE],
            SizeRule::Unsized,
        )],
        value("the vector of `bkpt`", 0, 7, None),
    ),
    // a long between a general register and a control register, `usp`
    // included, in either direction
    implemented(
        "movec",
        Family::Movec,
        &[
            privileged_dialect_form(
                Processor::M68010,
                &[Modes::MOVEC_CONTROL, Modes::ANY_REGISTER],
                SizeRule::LongOnly,
            ),
            privileged_dialect_form(
                Processor::M68010,
                &[Modes::ANY_REGISTER, Modes::MOVEC_CONTROL],
                SizeRule::LongOnly,
            ),
        ],
    ),
    // a general register to memory or back, through the function codes
    implemented(
        "moves",
        Family::Moves,
        &[
            privileged_dialect_form(
                Processor::M68010,
                &[Modes::ANY_REGISTER, Modes::MEMORY_ALTERABLE],
                SizeRule::Any,
            ),
            privileged_dialect_form(
                Processor::M68010,
                &[Modes::MEMORY_ALTERABLE, Modes::ANY_REGISTER],
                SizeRule::Any,
            ),
        ],
    ),
    // ---- the 68020's ----
    implemented("pack", Family::Pack { unpack: false }, PACK_FORMS),
    implemented("unpk", Family::Pack { unpack: true }, PACK_FORMS),
    // the bounds pair in memory, against a data or an address register
    implemented(
        "chk2",
        Family::BoundsCheck { trap: true },
        &[dialect_form(
            Processor::M68020,
            &[Modes::CONTROL, Modes::ANY_REGISTER],
            SizeRule::Any,
        )],
    ),
    implemented(
        "cmp2",
        Family::BoundsCheck { trap: false },
        &[dialect_form(
            Processor::M68020,
            &[Modes::CONTROL, Modes::ANY_REGISTER],
            SizeRule::Any,
        )],
    ),
    implemented(
        "cas",
        Family::Cas,
        &[dialect_form(
            Processor::M68020,
            &[Modes::DN, Modes::DN, Modes::MEMORY_ALTERABLE],
            SizeRule::Any,
        )],
    ),
    implemented(
        "cas2",
        Family::Cas2,
        &[dialect_form(
            Processor::M68020,
            &[
                Modes::REGISTER_PAIR,
                Modes::REGISTER_PAIR,
                Modes::INDIRECT_PAIR,
            ],
            SizeRule::WordOrLong,
        )],
    ),
    trap_conditionally("trapt", Condition::True),
    trap_conditionally("trapf", Condition::False),
    trap_conditionally("traphi", Condition::High),
    trap_conditionally("trapls", Condition::LowOrSame),
    trap_conditionally("trapcc", Condition::CarryClear),
    trap_conditionally("traphs", Condition::CarryClear),
    trap_conditionally("trapcs", Condition::CarrySet),
    trap_conditionally("traplo", Condition::CarrySet),
    trap_conditionally("trapne", Condition::NotEqual),
    trap_conditionally("trapeq", Condition::Equal),
    trap_conditionally("trapvc", Condition::OverflowClear),
    trap_conditionally("trapvs", Condition::OverflowSet),
    trap_conditionally("trappl", Condition::Plus),
    trap_conditionally("trapmi", Condition::Minus),
    trap_conditionally("trapge", Condition::GreaterThanOrEqual),
    trap_conditionally("traplt", Condition::LessThan),
    trap_conditionally("trapgt", Condition::GreaterThan),
    trap_conditionally("traple", Condition::LessThanOrEqual),
    // ---- the 68020's bit fields, which `opt bit` gives a 68000 Program ----
    bit_field(
        "bftst",
        BitFieldOperation::Test,
        &[dialect_form(
            Processor::M68020,
            &[BIT_FIELD_READ],
            SizeRule::Unsized,
        )],
    ),
    bit_field(
        "bfextu",
        BitFieldOperation::ExtractUnsigned,
        BIT_FIELD_INTO_A_REGISTER,
    ),
    bit_field("bfchg", BitFieldOperation::Change, BIT_FIELD_CHANGED),
    bit_field(
        "bfexts",
        BitFieldOperation::ExtractSigned,
        BIT_FIELD_INTO_A_REGISTER,
    ),
    bit_field("bfclr", BitFieldOperation::Clear, BIT_FIELD_CHANGED),
    bit_field(
        "bfffo",
        BitFieldOperation::FindFirstOne,
        BIT_FIELD_INTO_A_REGISTER,
    ),
    bit_field("bfset", BitFieldOperation::Set, BIT_FIELD_CHANGED),
    bit_field(
        "bfins",
        BitFieldOperation::Insert,
        &[dialect_form(
            Processor::M68020,
            &[Modes::DN, BIT_FIELD_WRITTEN],
            SizeRule::Unsized,
        )],
    ),
    // The module call of the 68020 alone, which s68k's 68020, a 68EC020,
    // leaves out for good (the asm-editor's `docs/design/m68k-machine.md`,
    // decision 3).
    not_implemented(
        "callm",
        "s68k's 68020 leaves out the module call, which no later processor kept",
        Some("`jsr`"),
    ),
    not_implemented(
        "rtm",
        "s68k's 68020 leaves out the module call, which no later processor kept",
        Some("`rts`"),
    ),
];

/// The row for `mnemonic`, case insensitively, or `None` when no Mnemonic has
/// that name.
///
/// This is the whole of what the parser asks the table (`label_rule`,
/// `docs/grammar.md` 1.4) and the first thing the analyzer asks it.
pub fn lookup(mnemonic: &str) -> Option<&'static InstructionSpec> {
    TABLE
        .iter()
        .find(|spec| spec.mnemonic.eq_ignore_ascii_case(mnemonic))
}

/// Whether `name` is a Mnemonic, case insensitively.
pub fn is_mnemonic(name: &str) -> bool {
    lookup(name).is_some()
}

/// Every Mnemonic in the table, in the table's order.
pub fn mnemonics() -> impl Iterator<Item = &'static str> {
    TABLE.iter().map(|spec| spec.mnemonic)
}

/// The Mnemonics worth offering as a "did you mean": every one.
pub fn suggestible_mnemonics() -> impl Iterator<Item = &'static str> {
    mnemonics()
}

/// The name in `candidates` closest to `name`, when one is close enough to be
/// worth offering as a "did you mean".
///
/// Close enough is one edit for a word of three characters or fewer and two
/// beyond that, which catches `mvoe` (a transposition is two edits), `movei`
/// and `mov` without offering a Mnemonic for every short word a student writes.
pub fn closest_name<'a>(name: &str, candidates: impl Iterator<Item = &'a str>) -> Option<&'a str> {
    let name = name.to_ascii_lowercase();
    let limit = if name.len() <= 3 { 1 } else { 2 };
    let mut best: Option<(usize, &'a str)> = None;
    for candidate in candidates {
        let distance = edit_distance(&name, candidate);
        if distance > limit {
            continue;
        }
        match best {
            Some((best_distance, _)) if best_distance <= distance => {}
            _ => best = Some((distance, candidate)),
        }
    }
    best.map(|(_, candidate)| candidate)
}

/// The Levenshtein distance between two words: how many single-character
/// insertions, deletions or substitutions turn one into the other.
///
/// It is public because a path is compared the same way: the closest existing
/// paths of a missing `include` are found with it
/// ([`include::closest_paths`](super::super::include::closest_paths)).
pub fn edit_distance(left: &str, right: &str) -> usize {
    let left: Vec<char> = left.chars().collect();
    let right: Vec<char> = right.chars().collect();
    let mut previous: Vec<usize> = (0..=right.len()).collect();
    let mut current = vec![0; right.len() + 1];
    for (i, left_character) in left.iter().enumerate() {
        current[0] = i + 1;
        for (j, right_character) in right.iter().enumerate() {
            let substitution = usize::from(left_character != right_character);
            current[j + 1] = (previous[j] + substitution)
                .min(previous[j + 1] + 1)
                .min(current[j] + 1);
        }
        std::mem::swap(&mut previous, &mut current);
    }
    previous[right.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_mnemonic_is_written_once_and_in_lower_case() {
        let mut names: Vec<&str> = mnemonics().collect();
        let count = names.len();
        for name in &names {
            assert_eq!(
                *name,
                name.to_ascii_lowercase(),
                "`{name}` is written in lower case, because lookup lowers the other side"
            );
        }
        names.sort_unstable();
        names.dedup();
        assert_eq!(count, names.len(), "a mnemonic has one row");
    }

    #[test]
    fn the_table_holds_every_mnemonic_the_old_checker_accepted() {
        // The list of `src/semantic_checker.rs`, plus `extb`, which only
        // `src/compiler.rs` knew (`tests/corpus/README.md`, "will change").
        for mnemonic in [
            "add", "sub", "move", "adda", "suba", "divs", "divu", "muls", "mulu", "swap", "clr",
            "exg", "neg", "ext", "extb", "tst", "cmp", "bcc", "bcs", "beq", "bne", "blt", "ble",
            "bgt", "bge", "bls", "bhi", "bpl", "bmi", "blo", "bhs", "bvc", "bvs", "bsr", "bra",
            "scc", "scs", "seq", "sne", "sge", "sgt", "sle", "sls", "slt", "shi", "smi", "spl",
            "svc", "svs", "slo", "shs", "sf", "st", "dbcc", "dbcs", "dbeq", "dbne", "dbge", "dbgt",
            "dble", "dbls", "dblt", "dbhi", "dbmi", "dbpl", "dbvc", "dbvs", "dbf", "dbt", "dbhs",
            "dblo", "dbra", "link", "unlk", "not", "addi", "andi", "ori", "eori", "subi", "cmpi",
            "movea", "cmpa", "cmpm", "or", "and", "eor", "lea", "pea", "addq", "subq", "moveq",
            "movem", "jmp", "jsr", "trap", "rts", "nop", "lsl", "lsr", "asr", "asl", "rol", "ror",
            "btst", "bclr", "bchg", "bset",
        ] {
            let spec = lookup(mnemonic).unwrap_or_else(|| panic!("`{mnemonic}` is in the table"));
            assert!(
                spec.is_implemented(),
                "`{mnemonic}` assembled on 1.4.2 and still does"
            );
        }
    }

    #[test]
    fn a_refused_instruction_is_in_the_table_with_its_reason() {
        // The 68020's module call, left out for good. The extend-flag and
        // binary-coded-decimal group was here until phase 3's second half
        // implemented it, `rte` until exceptions (C6), and `stop` and `reset`
        // until interrupts (C8).
        for mnemonic in ["callm", "rtm"] {
            let spec = lookup(mnemonic).unwrap_or_else(|| panic!("`{mnemonic}` is in the table"));
            match spec.implementation {
                Implementation::NotImplemented { reason, .. } => {
                    assert!(!reason.is_empty(), "`{mnemonic}` says why");
                }
                Implementation::Implemented(_) => {
                    panic!("`{mnemonic}` is not implemented")
                }
            }
            assert!(spec.forms.is_empty(), "`{mnemonic}` is never form checked");
        }
    }

    /// The Forms of a later processor are its own instructions and the
    /// shapes it gave a 68000 Mnemonic (ADR 0011): the 68010's `move ccr,<ea>`,
    /// `rtd`, `bkpt`, `movec` and `moves`; the 68020's `extb.l`, `chk.l`,
    /// `link.l`, `tst` and `cmpi` of what the 68000 cannot read, and every
    /// `trapcc`, of no operand or one.
    #[test]
    fn every_form_names_its_first_processor() {
        let later: Vec<(&str, usize, Processor)> = TABLE
            .iter()
            .filter(|spec| !matches!(spec.family(), Some(Family::TrapCc(_))))
            .flat_map(|spec| {
                spec.forms
                    .iter()
                    .enumerate()
                    .filter(|(_, form)| form.processor != Processor::M68000)
                    .map(|(index, form)| (spec.mnemonic, index, form.processor))
            })
            .collect();
        assert_eq!(
            later,
            vec![
                ("move", 4, Processor::M68010),
                ("link", 1, Processor::M68020),
                ("cmp", 3, Processor::M68020),
                ("cmpi", 1, Processor::M68020),
                ("divs", 1, Processor::M68020),
                ("divu", 1, Processor::M68020),
                ("muls", 1, Processor::M68020),
                ("mulu", 1, Processor::M68020),
                ("divsl", 0, Processor::M68020),
                ("divul", 0, Processor::M68020),
                ("extb", 0, Processor::M68020),
                ("tst", 1, Processor::M68020),
                ("chk", 1, Processor::M68020),
                ("rtd", 0, Processor::M68010),
                ("bkpt", 0, Processor::M68010),
                ("movec", 0, Processor::M68010),
                ("movec", 1, Processor::M68010),
                ("moves", 0, Processor::M68010),
                ("moves", 1, Processor::M68010),
                ("pack", 0, Processor::M68020),
                ("pack", 1, Processor::M68020),
                ("unpk", 0, Processor::M68020),
                ("unpk", 1, Processor::M68020),
                ("chk2", 0, Processor::M68020),
                ("cmp2", 0, Processor::M68020),
                ("cas", 0, Processor::M68020),
                ("cas2", 0, Processor::M68020),
                ("bftst", 0, Processor::M68020),
                ("bfextu", 0, Processor::M68020),
                ("bfchg", 0, Processor::M68020),
                ("bfexts", 0, Processor::M68020),
                ("bfclr", 0, Processor::M68020),
                ("bfffo", 0, Processor::M68020),
                ("bfset", 0, Processor::M68020),
                ("bfins", 0, Processor::M68020),
            ]
        );
        assert_eq!(
            lookup("move").expect("a row").forms[4].operands,
            &[Modes::CCR, Modes::DATA_ALTERABLE]
        );
        let traps: Vec<&InstructionSpec> = TABLE
            .iter()
            .filter(|spec| matches!(spec.family(), Some(Family::TrapCc(_))))
            .collect();
        assert_eq!(traps.len(), 18, "sixteen conditions, `hs` and `lo`");
        for spec in traps {
            assert_eq!(spec.forms, TRAP_CONDITIONALLY_FORMS, "`{}`", spec.mnemonic);
        }
    }

    /// Every instruction of the 68010 and the 68020 is in, the bit fields
    /// as the 68020's Forms; the module call is left out for good.
    #[test]
    fn the_later_instructions_are_all_in() {
        for mnemonic in ["rtd", "bkpt", "movec", "moves"] {
            let spec = lookup(mnemonic).expect("a row");
            assert!(spec.is_implemented(), "`{mnemonic}`");
            assert!(spec
                .forms
                .iter()
                .all(|form| form.processor == Processor::M68010));
        }
        let bit_fields: Vec<&str> = TABLE
            .iter()
            .filter(|spec| spec.forms.iter().any(Form::is_bit_field))
            .map(|spec| spec.mnemonic)
            .collect();
        assert_eq!(
            bit_fields,
            ["bftst", "bfextu", "bfchg", "bfexts", "bfclr", "bfffo", "bfset", "bfins"]
        );
        for spec in TABLE {
            assert!(
                spec.is_implemented() || ["callm", "rtm"].contains(&spec.mnemonic),
                "`{}`",
                spec.mnemonic
            );
        }
        for mnemonic in ["callm", "rtm"] {
            assert!(matches!(
                lookup(mnemonic).expect("a row").implementation,
                Implementation::NotImplemented { .. }
            ));
        }
        assert!(suggestible_mnemonics().any(|name| name == "bftst"));
        assert!(suggestible_mnemonics().any(|name| name == "callm"));
    }

    /// The shapes of the extend-flag and binary-coded-decimal group, read
    /// straight off the reference pages.
    ///
    /// `addx`, `subx`, `abcd` and `sbcd` take two data registers or two
    /// predecrements and nothing else; `negx` and `nbcd` take one data
    /// alterable Operand; `roxl` and `roxr` take the three shapes of the other
    /// shifts. The sizes are byte, word and long except for the three decimal
    /// ones, which are a byte, and for the memory form of a rotate, which is a
    /// word (`Reference/68ks5e.htm`, `68ks5v.htm`, `68ks5q.htm`, `68ks8e.htm`,
    /// `68ks8g.htm`, `68ks8f.htm`, `68ks7g.htm`, `68ks7h.htm`).
    #[test]
    fn the_extend_flag_group_takes_what_the_reference_gives_it() {
        for mnemonic in ["addx", "subx", "abcd", "sbcd"] {
            let spec = lookup(mnemonic).expect("a row");
            assert!(spec.is_implemented(), "`{mnemonic}` is implemented");
            let shapes: Vec<&[Modes]> = spec.forms.iter().map(|form| form.operands).collect();
            assert_eq!(
                shapes,
                vec![
                    &[Modes::DN, Modes::DN][..],
                    &[Modes::PREDECREMENT, Modes::PREDECREMENT][..]
                ],
                "`{mnemonic}` takes Dy,Dx or -(Ay),-(Ax) and nothing else"
            );
        }
        for (mnemonic, sizes) in [
            ("addx", SizeRule::Any),
            ("subx", SizeRule::Any),
            ("negx", SizeRule::Any),
            ("abcd", SizeRule::ByteOnly),
            ("sbcd", SizeRule::ByteOnly),
            ("nbcd", SizeRule::ByteOnly),
        ] {
            let spec = lookup(mnemonic).expect("a row");
            for form in spec.forms {
                assert_eq!(form.sizes, sizes, "the sizes of `{mnemonic}`");
            }
        }
        for mnemonic in ["negx", "nbcd"] {
            let spec = lookup(mnemonic).expect("a row");
            assert_eq!(
                spec.forms.iter().map(Form::arity).collect::<Vec<usize>>(),
                vec![1],
                "`{mnemonic}` takes one operand"
            );
            assert_eq!(spec.forms[0].operands[0], Modes::DATA_ALTERABLE);
        }
        for mnemonic in ["roxl", "roxr"] {
            let spec = lookup(mnemonic).expect("a row");
            assert_eq!(
                spec.forms, SHIFT_FORMS,
                "`{mnemonic}` has the shapes of the other shifts"
            );
            assert!(
                spec.value_rule.is_some(),
                "`{mnemonic}` counts from 1 to 8 like them"
            );
        }
    }

    #[test]
    fn lookup_is_case_insensitive() {
        assert_eq!(lookup("MOVE").map(|spec| spec.mnemonic), Some("move"));
        assert_eq!(lookup("Dbra").map(|spec| spec.mnemonic), Some("dbra"));
        assert!(lookup("frobnicate").is_none());
        assert!(lookup("dc").is_none(), "a directive is not a mnemonic");
    }

    #[test]
    fn the_sizes_are_the_helps_data_lengths() {
        // `Reference/68ks*.htm`, "DATA LENGTH", for the instructions 1.4.2
        // refused a size on altogether. The help's own `MOVEQ` example writes
        // `MOVEQ.L`, and ADR 0001 says an EASy68K program assembles unchanged.
        for (mnemonic, size, accepted) in [
            ("moveq", SizeSuffix::Long, true),
            ("moveq", SizeSuffix::Word, false),
            ("lea", SizeSuffix::Long, true),
            ("pea", SizeSuffix::Long, true),
            ("exg", SizeSuffix::Long, true),
            ("swap", SizeSuffix::Word, true),
            ("divu", SizeSuffix::Word, true),
            ("divu", SizeSuffix::Long, true),
            ("divu", SizeSuffix::Byte, false),
            ("dbra", SizeSuffix::Word, true),
            ("seq", SizeSuffix::Byte, true),
            ("btst", SizeSuffix::Byte, true),
            ("btst", SizeSuffix::Long, true),
            ("btst", SizeSuffix::Word, false),
            ("jsr", SizeSuffix::Long, false),
            ("trap", SizeSuffix::Word, false),
        ] {
            let spec = lookup(mnemonic).expect("a row");
            assert_eq!(
                spec.forms.iter().any(|form| form.sizes.accepts(size)),
                accepted,
                "`{mnemonic}{}`",
                size.suffix()
            );
        }
    }

    #[test]
    fn a_form_names_its_sizes_and_its_default() {
        assert_eq!(SizeRule::Any.default_size(), Some(Size::Word));
        assert_eq!(SizeRule::LongOnly.default_size(), Some(Size::Long));
        assert_eq!(SizeRule::Unsized.default_size(), None);
        assert_eq!(SizeRule::Branch.default_size(), None);
        assert!(SizeRule::Branch.accepts(SizeSuffix::Short));
        // `.b` is `.s` on a branch (`Reference/68ks9b.htm`), and neither is an
        // operand size, so a branch still stores none.
        assert!(SizeRule::Branch.accepts(SizeSuffix::Byte));
        assert!(!SizeRule::Any.accepts(SizeSuffix::Short));
        assert!(!SizeRule::WordOrLong.accepts(SizeSuffix::Byte));
    }

    #[test]
    fn the_mode_sets_are_the_manuals() {
        assert!(!Modes::DATA.contains(Modes::AN));
        assert!(!Modes::ALTERABLE.contains(Modes::IMMEDIATE));
        assert!(!Modes::DATA_ALTERABLE.contains(Modes::AN));
        assert!(!Modes::DATA_ALTERABLE.contains(Modes::IMMEDIATE));
        assert!(Modes::DATA_ALTERABLE.contains(Modes::DN));
        assert!(!Modes::CONTROL.contains(Modes::POSTINCREMENT));
        assert!(!Modes::CONTROL.contains(Modes::PREDECREMENT));
        assert!(Modes::MOVEM_TO_MEMORY.contains(Modes::PREDECREMENT));
        assert!(!Modes::MOVEM_TO_MEMORY.contains(Modes::POSTINCREMENT));
        assert!(Modes::MOVEM_FROM_MEMORY.contains(Modes::POSTINCREMENT));
        assert!(!Modes::MOVEM_FROM_MEMORY.contains(Modes::PREDECREMENT));
    }

    /// The PC-relative modes belong to data, memory and control, and to no
    /// group that is written to (`Reference/68ks1e.htm`, and the manual's
    /// four groups). Every row of the table takes them exactly where its
    /// groups do, which is why implementing them changed no row.
    #[test]
    fn the_pc_relative_modes_are_read_and_never_written() {
        assert!(Modes::DATA.contains(Modes::PC_RELATIVE));
        assert!(Modes::MEMORY.contains(Modes::PC_RELATIVE));
        assert!(Modes::CONTROL.contains(Modes::PC_RELATIVE));
        assert!(Modes::ALL.contains(Modes::PC_RELATIVE));
        assert!(!Modes::ALTERABLE.intersects(Modes::PC_RELATIVE));
        assert!(!Modes::DATA_ALTERABLE.intersects(Modes::PC_RELATIVE));
        assert!(!Modes::MEMORY_ALTERABLE.intersects(Modes::PC_RELATIVE));
        assert!(!Modes::CONTROL_ALTERABLE.intersects(Modes::PC_RELATIVE));
        // `movem` reads registers back through a PC-relative operand and never
        // writes them out through one, which is the one place its two
        // directions differ by more than the side the list is on.
        assert!(Modes::MOVEM_FROM_MEMORY.contains(Modes::PC_RELATIVE));
        assert!(!Modes::MOVEM_TO_MEMORY.intersects(Modes::PC_RELATIVE));
        // The four that take a control operand take them: `lea`, `pea`, `jmp`
        // and `jsr` are the reason the group exists.
        for mnemonic in ["lea", "pea", "jmp", "jsr"] {
            let spec = lookup(mnemonic).expect("a row");
            assert!(
                spec.forms[0].operands[0].contains(Modes::PC_RELATIVE),
                "`{mnemonic}` takes a PC-relative operand"
            );
        }
    }

    #[test]
    fn a_mode_set_names_itself_the_way_the_old_checker_did() {
        assert_eq!(Modes::DN.names(), vec!["Dn"]);
        assert_eq!(
            Modes::CONTROL.names(),
            vec![
                "(An)",
                "d(An)",
                "d(An,Xn)",
                "Ea/<label>",
                "d(PC)",
                "d(PC,Xn)"
            ]
        );
        assert_eq!(
            Modes::ALL.names(),
            vec![
                "Dn",
                "An",
                "(An)",
                "(An)+",
                "-(An)",
                "d(An)",
                "d(An,Xn)",
                "Ea/<label>",
                "d(PC)",
                "d(PC,Xn)",
                "Im"
            ]
        );
    }

    /// The 68000 reserves nine shapes for supervisor mode: the four that
    /// write `sr` (`move <ea>,sr` and `andi`, `ori` and `eori` into it),
    /// `rte`, `stop`, `reset`, and the two of `move usp`. `move sr,<ea>`
    /// became privileged on the 68010, whose `movec` and `moves` are too, and
    /// the `ccr` shapes never were.
    #[test]
    fn only_the_forms_that_write_sr_or_reach_the_user_stack_pointer_are_privileged() {
        let privileged: Vec<(&str, usize, Processor)> = TABLE
            .iter()
            .flat_map(|spec| {
                spec.forms.iter().enumerate().filter_map(|(index, form)| {
                    form.privileged.map(|first| (spec.mnemonic, index, first))
                })
            })
            .collect();
        let m68000 = Processor::M68000;
        let m68010 = Processor::M68010;
        assert_eq!(
            privileged,
            vec![
                ("move", 2, m68000),
                ("move", 3, m68010),
                ("move", 5, m68000),
                ("move", 6, m68000),
                ("andi", 2, m68000),
                ("ori", 2, m68000),
                ("eori", 2, m68000),
                ("rte", 0, m68000),
                ("stop", 0, m68000),
                ("reset", 0, m68000),
                ("movec", 0, m68010),
                ("movec", 1, m68010),
                ("moves", 0, m68010),
                ("moves", 1, m68010),
            ]
        );
        for spec in TABLE {
            for form in spec.forms {
                assert_eq!(
                    form.is_privileged_on(Processor::M68000),
                    form.operands.last() == Some(&Modes::SR)
                        || form.operands.contains(&Modes::USP)
                        || ["rte", "stop", "reset"].contains(&spec.mnemonic),
                    "`{}`: a form is privileged on the 68000 exactly when it writes `sr`, \
                     reaches `usp` or is `rte`, `stop` or `reset`",
                    spec.mnemonic
                );
            }
        }
        let move_from_sr = lookup("move").expect("a row").forms[3];
        assert_eq!(move_from_sr.operands, &[Modes::SR, Modes::DATA_ALTERABLE]);
        assert!(
            !move_from_sr.is_privileged_on(Processor::M68000),
            "a 68000 reads `sr` in user mode"
        );
        assert!(move_from_sr.is_privileged_on(Processor::M68010));
        assert!(move_from_sr.is_privileged_on(Processor::M68020));
    }

    #[test]
    fn did_you_mean_offers_a_near_miss_and_nothing_further() {
        assert_eq!(closest_name("mvoe", mnemonics()), Some("move"));
        assert_eq!(closest_name("MOVEE", mnemonics()), Some("move"));
        assert_eq!(closest_name("jump", mnemonics()), Some("jmp"));
        assert_eq!(closest_name("frobnicate", mnemonics()), None);
        assert_eq!(closest_name("xyz", mnemonics()), None);
    }

    #[test]
    fn every_implemented_row_has_a_form_and_every_form_a_rule_per_operand() {
        for spec in TABLE {
            if !spec.is_implemented() {
                continue;
            }
            assert!(
                !spec.forms.is_empty(),
                "`{}` needs at least one form",
                spec.mnemonic
            );
            for form in spec.forms {
                assert!(
                    form.operands.len() <= 3,
                    "`{}` has no instruction of more than three operands, as `cas` has",
                    spec.mnemonic
                );
                if form.combination == Combination::AtMostOneMemoryOperand {
                    assert_eq!(
                        form.operands.len(),
                        2,
                        "`{}`: the memory rule is about two operands",
                        spec.mnemonic
                    );
                }
            }
            // Two forms of the same arity are told apart by *all* their
            // Operands (`Analyzer::choose_form`), so the order matters: the
            // one an Operation that fits neither is judged against comes
            // first. `cmp` and `movem` are the only two, and both are written
            // that way round.
            let arities = spec.arities();
            if arities.len() != spec.forms.len() {
                assert!(
                    matches!(
                        spec.mnemonic,
                        "cmp"
                            | "cmpi"
                            | "divs"
                            | "divu"
                            | "muls"
                            | "mulu"
                            | "tst"
                            | "chk"
                            | "link"
                            | "eor"
                            | "movem"
                            | "move"
                            | "movep"
                            | "andi"
                            | "ori"
                            | "eori"
                            | "addx"
                            | "subx"
                            | "abcd"
                            | "sbcd"
                            | "movec"
                            | "moves"
                            | "pack"
                            | "unpk"
                    ),
                    "`{}` has two forms of one arity; `Analyzer::choose_form` picks the \
                     first that fits and falls back to the first, so the order has to be \
                     deliberate",
                    spec.mnemonic
                );
            }
            // A Form that names `sr`, `ccr`, `usp` or a control register names
            // no ordinary Addressing mode in that position, which is what
            // `Analyzer::choose_form` rests on when it keeps the Forms that
            // agree with the special register that was written.
            for form in spec.forms {
                for modes in form.operands {
                    assert!(
                        !modes.intersects(Modes::SPECIAL) || Modes::SPECIAL.contains(*modes),
                        "`{}` has a position that takes a special register beside an \
                         addressing mode; `Analyzer::choose_form` cannot tell its Forms apart",
                        spec.mnemonic
                    );
                }
            }
        }
    }
}
