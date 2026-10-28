//! The encoded instruction: what the Assembler stores and the Interpreter runs.
//!
//! These types were `src/instructions.rs`'s and are moved here because they are
//! the *output* of the front end: the analyzer checks an [`ast::Operand`] and
//! the lowering ([`super::lowering`]) turns it into the [`Operand`] of this
//! module, which the Interpreter executes without ever reading source again
//! (CONTEXT.md, "Interpreter"). `src/instructions.rs` re-exports every one of
//! them, so the old pipeline and the Interpreter still name them where they
//! always did.
//!
//! Nothing here carries a Span or a Location: an [`Instruction`] is a value,
//! and where it came from is the Program's business
//! (`src/assembler/program.rs`, still to be written).
//!
//! [`ast::Operand`]: super::super::ast::Operand

use std::str::FromStr;

use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::wasm_bindgen;

/// The width an instruction works at, in bytes.
///
/// The numbers are the widths themselves, so `Size::Long as usize` is 4.
#[wasm_bindgen]
#[derive(Debug, Clone, Copy, Serialize, Eq, PartialEq)]
pub enum Size {
    /// One byte, `.b`.
    Byte = 1,
    /// Two bytes, `.w`.
    Word = 2,
    /// Four bytes, `.l`.
    Long = 4,
}

impl Size {
    /// How many bytes the size covers.
    #[inline(always)]
    pub fn to_bytes(&self) -> usize {
        *self as usize
    }
    /// How many bits the size covers.
    #[inline(always)]
    pub fn to_bits(&self) -> usize {
        *self as usize * 8
    }
}

/// Which way a `movem` moves its registers.
#[wasm_bindgen]
#[derive(Debug, Clone, Copy, Serialize, Eq, PartialEq)]
pub enum TargetDirection {
    /// The registers are written to memory (`movem.l d0-d2,-(a7)`).
    ToMemory,
    /// The registers are read back from memory (`movem.l (a7)+,d0-d2`).
    FromMemory,
}

/// One of the sixteen general registers, as an encoded operand.
#[derive(Debug, Clone, Serialize, Deserialize, Copy, PartialEq, Eq)]
#[serde(tag = "type", content = "value")]
pub enum RegisterOperand {
    /// `a0` to `a7`.
    Address(u8),
    /// `d0` to `d7`.
    Data(u8),
}

impl RegisterOperand {
    /// Where the register sits in a `movem` mask: `d0` to `d7` are 0 to 7 and
    /// `a0` to `a7` are 8 to 15.
    pub fn to_index(&self) -> u16 {
        match self {
            RegisterOperand::Address(index) => *index as u16 + 8,
            RegisterOperand::Data(index) => *index as u16,
        }
    }
    /// The register's name as it is written in source and as the editor spells
    /// it: `d0` to `d7`, `a0` to `a7`.
    pub fn name(&self) -> String {
        match self {
            RegisterOperand::Address(index) => format!("a{index}"),
            RegisterOperand::Data(index) => format!("d{index}"),
        }
    }
}

/// The index register of an indexed Operand, with the width it is read at.
#[derive(Debug, Clone, Serialize, Copy, PartialEq, Eq)]
pub struct IndexRegister {
    /// The register itself.
    pub register: RegisterOperand,
    //pub scale: u8,
    /// `Size::Word` or `Size::Long`; a byte index does not exist.
    pub size: Size,
}

/// An Addressing mode with its registers and its value already worked out.
///
/// Every Expression has been evaluated by the time an Operand is built, so a
/// displacement is a number and a Label is the address it stands for
/// (`tests/corpus/README.md`, "Branch and jump targets").
#[derive(Debug, Clone, Serialize, Copy, PartialEq, Eq)]
pub enum Operand {
    /// `#5`, kept at the size the instruction reads it at, zero extended:
    /// `move.b #-1,d0` holds `$FF`, `move.w #-1,d0` `$FFFF`. A `movem`
    /// register list is one too, as its mask, until it is lowered into the
    /// [`Instruction::MOVEM`] that reads it.
    Immediate(u32),
    /// `d3`, `a6`.
    Register(RegisterOperand),
    /// `(a0)`, by address register number.
    Indirect(u8),
    /// `(a0)+`, by address register number.
    PostIndirect(u8),
    /// `-(a0)`, by address register number.
    PreIndirect(u8),
    /// `4(a6)`.
    IndirectDisplacement {
        /// The displacement, sign extended from the width it was written at.
        offset: i32,
        /// The base register.
        base: RegisterOperand,
    },
    /// `4(a6,d1.w)`.
    IndirectIndex {
        /// The base register.
        base: RegisterOperand,
        /// The displacement, sign extended from the width it was written at.
        offset: i32,
        /// The index register and the width it is read at.
        index: IndexRegister,
    },

    /// `label(pc)`, as the address it reaches.
    ///
    /// The 68000 encodes a displacement from the extension word that holds it,
    /// and the source writes the address it wants; this keeps the address. The
    /// displacement depends on where that extension word sits — two bytes
    /// past the instruction for most of them, four for `movem` and
    /// `btst #n,d(pc)`, whose mask or bit number comes first — and that is
    /// the business of whatever reads or writes the words, which knows the
    /// instruction's address ([`crate::machine::decode`]). The Interpreter uses
    /// the address as it is, wherever the instruction has been placed.
    PcDisplacement {
        /// The address the operand names.
        target: u32,
    },
    /// `label(pc,d1.w)`, as the address the displacement reaches.
    PcIndex {
        /// The address `d8(PC)` reaches, which the index register is added
        /// to when the instruction runs. `(pc,d1.w)` writes no displacement,
        /// so its target is the extension word itself.
        target: u32,
        /// The index register and the width it is read at.
        index: IndexRegister,
    },

    /// `$2000`, and a Label, which is its address by the time it gets here.
    Absolute {
        /// The address. An absolute short one is the address its word means,
        /// sign extended: `$8000.w` is `$FFFF8000`.
        address: usize,
        /// Whether the address is encoded in a word or a long.
        width: AddressWidth,
    },
}

/// How wide an absolute address is encoded: the 68000's absolute short and
/// absolute long modes, `(xxx).W` and `(xxx).L`.
///
/// Both name an address of 32 bits; the short one holds 16 of them and is
/// sign extended, so it reaches `$0000` to `$7FFF` and `$FFFF8000` to
/// `$FFFFFFFF` (`Reference/68ks1e.htm`, "Forcing Absolute Short Addressing").
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
pub enum AddressWidth {
    /// `label.w`: one extension word.
    Word,
    /// `label.l`: two extension words.
    Long,
}

/// How a `Bcc`, `bra` or `bsr` encodes its displacement.
///
/// The three are distinct encodings of the same branch: the short one keeps
/// its displacement in the operation word, so it is two bytes; the word one
/// writes `$00` there and adds a word; the long one, which only the 68020 has,
/// writes `$FF` and adds a long. On a 68000 and a 68010 that `$FF` is a short
/// displacement of −1 (`docs/research/m68k-cpu-variants.md` §2.1 in the
/// asm-editor).
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
pub enum BranchSize {
    /// `.s`, also written `.b`: a displacement of −128 to 127, never 0.
    Short,
    /// `.w`: a displacement of −32768 to 32767.
    Word,
    /// `.l`, the 68020's: a displacement of 32 bits.
    Long,
}

/// How far the extension word of a PC-relative Operand usually sits from the
/// instruction's own address: the word after the operation word.
///
/// On a 68000 the displacement of `d16(PC)` is added to the address of the
/// extension word that holds it. That is this word for every instruction but
/// `movem` and `btst #n,d(pc)`, whose mask or bit number comes first and
/// puts it two bytes further, which
/// [`lowering::extension_word_offset`](super::lowering::extension_word_offset)
/// says per instruction. A PC-relative [`Operand`] keeps the address it
/// reaches, so the encoder ([`crate::machine::encode`]) finds the word as it
/// writes it, and only the Assembler reads the number: to check that the
/// distance fits its field (`Analyzer::check_pc_distance`) and to give
/// `(pc,d1.w)`, which writes no displacement, the address of its extension
/// word.
pub const EXTENSION_WORD_OFFSET: i32 = 2;

/*
Thanks to:  https://github.com/transistorfet/moa/blob/main/emulator/cpus/m68k/src/instructions.rs
for the Conditions and inspiration
 */

/// The condition a `Bcc`, `DBcc` or `Scc` tests.
///
/// The aliases have no variant of their own: `hs` is [`Condition::CarryClear`]
/// and `lo` is [`Condition::CarrySet`], which is why `bhs` prints back as `bcc`
/// (`tests/corpus/README.md`, "Condition codes").
#[wasm_bindgen]
#[derive(Copy, Clone, Debug, Serialize, PartialEq, Eq)]
pub enum Condition {
    /// `t`, always.
    True,
    /// `f`, never. `dbra` is `dbf`.
    False,
    /// `hi`, unsigned greater than.
    High,
    /// `ls`, unsigned less than or equal.
    LowOrSame,
    /// `cc`, also written `hs`: carry clear.
    CarryClear,
    /// `cs`, also written `lo`: carry set.
    CarrySet,
    /// `ne`, not equal.
    NotEqual,
    /// `eq`, equal.
    Equal,
    /// `vc`, overflow clear.
    OverflowClear,
    /// `vs`, overflow set.
    OverflowSet,
    /// `pl`, positive.
    Plus,
    /// `mi`, negative.
    Minus,
    /// `ge`, signed greater than or equal.
    GreaterThanOrEqual,
    /// `lt`, signed less than.
    LessThan,
    /// `gt`, signed greater than.
    GreaterThan,
    /// `le`, signed less than or equal.
    LessThanOrEqual,
}

impl FromStr for Condition {
    type Err = String;
    fn from_str(s: &str) -> Result<Condition, Self::Err> {
        let s = s.to_lowercase();
        Ok(match s.as_str() {
            "t" => Condition::True,
            "f" => Condition::False,
            "hi" => Condition::High,
            "ls" => Condition::LowOrSame,
            "cc" | "hs" => Condition::CarryClear,
            "cs" | "lo" => Condition::CarrySet,
            "ne" => Condition::NotEqual,
            "eq" => Condition::Equal,
            "vc" => Condition::OverflowClear,
            "vs" => Condition::OverflowSet,
            "pl" => Condition::Plus,
            "mi" => Condition::Minus,
            "ge" => Condition::GreaterThanOrEqual,
            "lt" => Condition::LessThan,
            "gt" => Condition::GreaterThan,
            "le" => Condition::LessThanOrEqual,
            _ => return Err(format!("Invalid condition: {}", s)),
        })
    }
}

impl Condition {
    /// Every condition in the order of its four-bit code, which is the order
    /// of the variants: `t` is `%0000` and `le` is `%1111`.
    pub const ALL: [Condition; 16] = [
        Condition::True,
        Condition::False,
        Condition::High,
        Condition::LowOrSame,
        Condition::CarryClear,
        Condition::CarrySet,
        Condition::NotEqual,
        Condition::Equal,
        Condition::OverflowClear,
        Condition::OverflowSet,
        Condition::Plus,
        Condition::Minus,
        Condition::GreaterThanOrEqual,
        Condition::LessThan,
        Condition::GreaterThan,
        Condition::LessThanOrEqual,
    ];

    /// The condition a four-bit code names, bits 11–8 of a `Bcc`, `DBcc` or
    /// `Scc`; only the low four bits of `code` are read.
    pub fn from_code(code: u16) -> Condition {
        Condition::ALL[(code & 0xF) as usize]
    }

    /// The four-bit code of the condition, the inverse of
    /// [`from_code`](Condition::from_code).
    pub fn code(self) -> u16 {
        self as u16
    }

    /// The canonical suffix, lowercase: `t`, `f`, `hi`, … `le`. The aliases
    /// `hs` and `lo` are `cc` and `cs`.
    pub fn suffix(self) -> &'static str {
        match self {
            Condition::True => "t",
            Condition::False => "f",
            Condition::High => "hi",
            Condition::LowOrSame => "ls",
            Condition::CarryClear => "cc",
            Condition::CarrySet => "cs",
            Condition::NotEqual => "ne",
            Condition::Equal => "eq",
            Condition::OverflowClear => "vc",
            Condition::OverflowSet => "vs",
            Condition::Plus => "pl",
            Condition::Minus => "mi",
            Condition::GreaterThanOrEqual => "ge",
            Condition::LessThan => "lt",
            Condition::GreaterThan => "gt",
            Condition::LessThanOrEqual => "le",
        }
    }
}

/// Which way a shift or rotate moves its bits.
#[derive(Copy, Clone, Debug, Serialize, PartialEq, Eq)]
pub enum ShiftDirection {
    /// `asr`, `lsr`, `ror`.
    Right,
    /// `asl`, `lsl`, `rol`.
    Left,
}

/// Whether a multiply or divide reads its operands as signed or unsigned.
#[derive(Copy, Clone, Debug, Serialize, PartialEq, Eq)]
pub enum Sign {
    /// `muls`, `divs`.
    Signed,
    /// `mulu`, `divu`.
    Unsigned,
}

/// One instruction, ready to run: what the Assembler stores and what the
/// decoder ([`crate::machine::decode`]) reads back from words.
///
/// **One value per encoding.** Every 68000 encoding has a value of its own, so
/// that the encoder and the decoder are inverses over this enum. Where the
/// 68000 has two encodings of one operation, they are told apart by variant or
/// by Operand: [`ADDQ`](Instruction::ADDQ), [`ADDI`](Instruction::ADDI) and
/// [`ADD`](Instruction::ADD) with an [`Operand::Immediate`] source are the
/// three encodings of `add #1,d0`; [`MOVEQ`](Instruction::MOVEQ) and a long
/// [`MOVE`](Instruction::MOVE) of an immediate are two of `move.l #1,d0`;
/// [`CMP`](Instruction::CMP) and [`CMPI`](Instruction::CMPI), and
/// [`AND`](Instruction::AND)/[`OR`](Instruction::OR) and their immediate
/// forms, likewise. A shift by one in memory and a register shift by `#1` are
/// told apart by their target: the memory form shifts a place in memory and
/// the register forms a data register. A branch carries its [`BranchSize`]
/// and an absolute address its [`AddressWidth`].
///
/// What is kept is what the 68000 reads. The bits it ignores are not: bits
/// 10–8 of a brief index word, and the high byte of the word that holds a byte
/// immediate, an `andi`/`ori`/`eori` to `ccr` or a static bit number. An
/// immediate is kept as the value at the size it is read at, zero extended
/// (`$FF` for `move.b #-1,d0`), and the displacement of `link` sign extended.
/// So `encode(decode(words)) == words` holds for every sequence whose ignored
/// bits are clear. Assemblers do not all clear them — GNU `as` writes
/// `move.b #-1,d0` as `$103C $FFFF` — and such words decode to the same value
/// as their clear form.
///
/// **The Assembler's choices.** The Mnemonic a student wrote and the variant
/// here are not one to one: the Assembler chooses the encoding EASy68K
/// chooses (`tests/corpus/README.md`, "Normalisations stay visible"), so
/// `add #1,d0` is [`Instruction::ADDQ`], `add #100,d0`
/// [`Instruction::ADDI`], `cmp #5,d0` [`Instruction::CMP`] and `move.l d0,a0`
/// [`Instruction::MOVEA`]. It chooses a branch's [`BranchSize`] and an
/// address's [`AddressWidth`] by EASy68K's back-reference rule as well.
/// [`super::lowering`] is where that happens and [`super::choice`] holds the
/// rules; [`crate::machine::encode`] writes the words.
#[derive(Clone, Debug, Serialize, Copy, PartialEq, Eq)]
pub enum Instruction {
    /// `adda.<size> <ea>,An`
    ADDA(Operand, RegisterOperand, Size),
    /// `suba.<size> <ea>,An`
    SUBA(Operand, RegisterOperand, Size),
    /// `cmpa.<size> <ea>,An`
    CMPA(Operand, RegisterOperand, Size),
    /// `movea.<size> <ea>,An`
    MOVEA(Operand, RegisterOperand, Size), //add TAS()
    /// `movem.<size>`, in either direction.
    MOVEM {
        /// Which way the registers move.
        direction: TargetDirection,
        /// `.w` or `.l`.
        size: Size,
        /// One bit a register, `d0` lowest; reversed for a predecrement target.
        registers_mask: u16,
        /// The memory operand.
        target: Operand,
    },
    /// `move.<size> <ea>,<ea>`
    MOVE(Operand, Operand, Size),
    /// `add.<size> <ea>,Dn` and `add.<size> Dn,<ea>`
    ADD(Operand, Operand, Size),
    /// `sub.<size> <ea>,Dn` and `sub.<size> Dn,<ea>`
    SUB(Operand, Operand, Size),
    /// `addx.<size> Dy,Dx` and `addx.<size> -(Ay),-(Ax)`: the source, the
    /// destination and the size. The extend flag is added in as well
    /// (`Reference/68ks5e.htm`).
    ADDX(Operand, Operand, Size),
    /// `subx.<size> Dy,Dx` and `subx.<size> -(Ay),-(Ax)`, the same shape
    /// (`Reference/68ks5v.htm`).
    SUBX(Operand, Operand, Size),
    /// `addq.<size> #1-8,<ea>`
    ADDQ(u8, Operand, Size),
    /// `moveq #-128-127,Dn`
    MOVEQ(u8, RegisterOperand),
    /// `subq.<size> #1-8,<ea>`
    SUBQ(u8, Operand, Size),
    /// `addi.<size> #n,<ea>`
    ADDI(u32, Operand, Size),
    /// `subi.<size> #n,<ea>`
    SUBI(u32, Operand, Size),
    /// `andi.<size> #n,<ea>`
    ANDI(u32, Operand, Size),
    /// `ori.<size> #n,<ea>`
    ORI(u32, Operand, Size),
    /// `eori.<size> #n,<ea>`
    EORI(u32, Operand, Size),
    /// `cmpi.<size> #n,<ea>`
    CMPI(u32, Operand, Size),
    /// `cmpm.<size> (An)+,(An)+`
    CMPM(Operand, Operand, Size),
    /// `divs`/`divu <ea>,Dn`
    DIVx(Operand, RegisterOperand, Sign),
    /// `muls`/`mulu <ea>,Dn`
    MULx(Operand, RegisterOperand, Sign),
    /// `swap Dn`
    SWAP(RegisterOperand),
    /// `clr.<size> <ea>`
    CLR(Operand, Size),
    /// `exg Rn,Rn`
    EXG(RegisterOperand, RegisterOperand),
    /// `lea <ea>,An`
    LEA(Operand, RegisterOperand),
    /// `pea <ea>`
    PEA(Operand),
    /// `neg.<size> <ea>`
    NEG(Operand, Size),
    /// `negx.<size> <ea>`: zero minus the operand minus the extend flag
    /// (`Reference/68ks5q.htm`).
    NEGX(Operand, Size),
    /// `abcd Dy,Dx` and `abcd -(Ay),-(Ax)`: one byte of binary coded decimal,
    /// plus the extend flag. It carries no size, because a byte is the only one
    /// it has (`Reference/68ks8e.htm`).
    ABCD(Operand, Operand),
    /// `sbcd Dy,Dx` and `sbcd -(Ay),-(Ax)`, the same in subtraction
    /// (`Reference/68ks8g.htm`).
    SBCD(Operand, Operand),
    /// `nbcd <ea>`: the tens complement of one byte, less the extend flag
    /// (`Reference/68ks8f.htm`).
    NBCD(Operand),
    /// `ext.w`, `ext.l` and `extb.l`: the register, the width read and the
    /// width written. `extb.l`, a byte read and a long written, is the 68020's,
    /// and the decoder reads it only for a 68020.
    EXT(RegisterOperand, Size, Size),
    /// `tst.<size> <ea>`
    TST(Operand, Size),
    /// `cmp.<size> <ea>,Dn`
    CMP(Operand, RegisterOperand, Size),
    /// `Bcc <address>`: the address it branches to, the condition and the
    /// size of its displacement. The decoder never makes `t` or `f` here,
    /// because those two codes are [`BRA`](Instruction::BRA) and
    /// [`BSR`](Instruction::BSR).
    Bcc(u32, Condition, BranchSize),
    /// `Scc <ea>`
    Scc(Operand, Condition),
    /// `DBcc Dn,<address>`
    DBcc(RegisterOperand, u32, Condition),
    /// `bra <address>`, with the size of its displacement.
    BRA(u32, BranchSize),
    /// `link An,#n`
    LINK(RegisterOperand, u32),
    /// `unlk An`
    UNLK(RegisterOperand),
    /// `not.<size> <ea>`
    NOT(Operand, Size),
    /// `or.<size> <ea>,Dn` and `or.<size> Dn,<ea>`
    OR(Operand, Operand, Size),
    /// `and.<size> <ea>,Dn` and `and.<size> Dn,<ea>`
    AND(Operand, Operand, Size),
    /// `eor.<size> Dn,<ea>`
    EOR(Operand, Operand, Size),
    /// `jsr <ea>`
    JSR(Operand),
    /// `asl`/`asr`: the count, what is shifted, the direction and the size.
    ASd(Operand, Operand, ShiftDirection, Size),
    /// `rol`/`ror`, the same shape.
    ROd(Operand, Operand, ShiftDirection, Size),
    /// `lsl`/`lsr`, the same shape.
    LSd(Operand, Operand, ShiftDirection, Size),
    /// `roxl`/`roxr`, the same shape again: the rotation goes through the
    /// extend flag, which makes it 9, 17 or 33 bits wide
    /// (`Reference/68ks7g.htm`).
    ROXd(Operand, Operand, ShiftDirection, Size),
    /// `btst <bit>,<ea>`
    BTST(Operand, Operand),
    /// `bclr <bit>,<ea>`
    BCLR(Operand, Operand),
    /// `bset <bit>,<ea>`
    BSET(Operand, Operand),
    /// `bchg <bit>,<ea>`
    BCHG(Operand, Operand),
    /// `jmp <ea>`
    JMP(Operand),
    /// `bsr <address>`, with the size of its displacement.
    BSR(u32, BranchSize),
    /// `trap #0-15`; only `#15`, the I/O trap, is simulated.
    TRAP(u8),
    /// `rte`: pop the status register and the program counter an exception
    /// pushed. Privileged. Not run yet: there are no exceptions to return
    /// from.
    RTE,
    /// `stop #n`: load the status register and wait for an interrupt.
    /// Privileged. Not run yet.
    STOP(u16),
    /// `reset`: assert the reset line to the devices. Privileged. Not run yet.
    RESET,
    /// `move usp,An` and `move An,usp`: the user stack pointer, which only
    /// supervisor mode can reach. Privileged. Not run yet.
    MOVEUSP {
        /// `true` for `move An,usp`, `false` for `move usp,An`.
        to_usp: bool,
        /// The address register on the other side.
        register: RegisterOperand,
    },
    /// A word of the form `$Axxx`, the line 1010 emulator: it raises an
    /// exception of its own (vector 10), so that software can give it a
    /// meaning. The whole word is kept. Not run yet.
    LINEA(u16),
    /// A word of the form `$Fxxx` other than SIMHALT's `$FFFF $FFFF`, the line
    /// 1111 emulator (vector 11), where the 68020 finds its coprocessors. The
    /// whole word is kept. Not run yet.
    LINEF(u16),
    /// `movep.<size>`, a byte-interleaved transfer between a data register and
    /// every second byte of memory (`Reference/68ks4g.htm`).
    MOVEP {
        /// Which way the bytes go: `ToMemory` is `movep dx,d16(ay)`.
        direction: TargetDirection,
        /// `.w` (two bytes) or `.l` (four).
        size: Size,
        /// The data register the bytes come from or go to.
        register: RegisterOperand,
        /// The memory operand, always a displacement one.
        target: Operand,
    },
    /// `move <ea>,ccr`: the low byte of a word sets the condition codes.
    MOVEtoCCR(Operand),
    /// `move ccr,<ea>`: the condition codes as a word, zero extended.
    ///
    /// The 68000 has no such instruction — it is the 68010's — and s68k
    /// assembles it because the design record asks for `move` "to and from SR
    /// and CCR" (the implementation notes, phase 3). The decoder reads its
    /// words only for a 68010 or later; on a 68000 they are illegal.
    MOVEfromCCR(Operand),
    /// `move <ea>,sr`: a word sets the whole status register.
    MOVEtoSR(Operand),
    /// `move sr,<ea>`: the whole status register as a word.
    MOVEfromSR(Operand),
    /// `andi #n,ccr`, a byte.
    ANDItoCCR(u8),
    /// `ori #n,ccr`, a byte.
    ORItoCCR(u8),
    /// `eori #n,ccr`, a byte.
    EORItoCCR(u8),
    /// `andi #n,sr`, a word.
    ANDItoSR(u16),
    /// `ori #n,sr`, a word.
    ORItoSR(u16),
    /// `eori #n,sr`, a word.
    EORItoSR(u16),
    /// `tas <ea>`: test a byte and set its top bit.
    TAS(Operand),
    /// `rtr`: pop the condition codes, then the return address.
    RTR,
    /// `chk <ea>,Dn`: end the run when the register is outside 0 to `<ea>`.
    CHK(Operand, RegisterOperand),
    /// `trapv`: end the run when the overflow flag is set.
    TRAPV,
    /// `illegal`, `$4AFC`: end the run, always.
    ILLEGAL,
    /// A word that is the first word of no instruction of the processor it was
    /// decoded for. The 68000 treats it as [`ILLEGAL`](Instruction::ILLEGAL)
    /// (an illegal instruction exception, vector 4), and so does s68k; the word
    /// is kept so that it can be shown, as `dc.w`, and written back. The
    /// Assembler never makes one.
    IllegalWord(u16),
    /// `rts`
    RTS,
    /// `nop`
    NOP,
    /// `simhalt`, the Directive that pauses the run.
    ///
    /// It is not a 68000 instruction: EASy68K assembles it to the object code
    /// `$FFFFFFFF`, which its simulator reads as "halt"
    /// (`Directives/simhalt.htm`). Here it is an executable item of the Program
    /// like any other, those four bytes at its own address. The Interpreter pauses
    /// after it without modifying a register and resumes at the following
    /// instruction when it is run or stepped again.
    SIMHALT,
}
