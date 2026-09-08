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
#[derive(Debug, Clone, Serialize, Deserialize, Copy)]
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
}

/// The index register of an indexed Operand, with the width it is read at.
#[derive(Debug, Clone, Serialize, Copy)]
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
#[derive(Debug, Clone, Serialize, Copy)]
pub enum Operand {
    /// `#5`, stored in 32 bits whatever size the instruction carries.
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

    /// `label(pc)`, as the displacement the Assembler worked out.
    ///
    /// The source writes an address and the Assembler stores the distance from
    /// it to the extension word of this instruction, which is
    /// [`EXTENSION_WORD_OFFSET`] bytes past the instruction's own address: the
    /// Interpreter adds the two back together and reaches the address that was
    /// written. The Program holds no encoded words, so this displacement is the
    /// only thing that says where the operand was measured from.
    PcDisplacement {
        /// The displacement, sign extended from the sixteen bits the 68000
        /// encodes it in.
        offset: i32,
    },
    /// `label(pc,d1.w)`.
    PcIndex {
        /// The displacement, sign extended from the eight bits the 68000
        /// encodes it in.
        offset: i32,
        /// The index register and the width it is read at.
        index: IndexRegister,
    },

    /// `$2000`, and a Label, which is its address by the time it gets here.
    Absolute(usize),
}

/// How far the extension word of an instruction sits from the instruction's
/// own address, which is what a PC-relative Operand is measured from.
///
/// On a 68000 the displacement of `d16(PC)` is added to the address of the
/// extension word that holds it, which is the word after the operation word:
/// two bytes past the instruction. s68k stores no encoded words and every
/// instruction is four bytes ([`INSTRUCTION_SIZE`](crate::assembler::layout::INSTRUCTION_SIZE)),
/// so the same two bytes are all that is needed to make the pair round-trip:
/// the Assembler works out `label - (address + 2)` and the Interpreter reads
/// back `address + 2 + displacement`. Both sides read this constant, which is
/// what keeps them the same arithmetic.
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

/// One assembled instruction, ready to run.
///
/// The Mnemonic a student wrote and the variant here are not one to one: the
/// Assembler normalises as it stores (`tests/corpus/README.md`,
/// "Normalisations stay visible"), so `add #1,d0` is [`Instruction::ADDI`] and
/// `move.l d0,a0` is [`Instruction::MOVEA`]. [`super::lowering`] is where that
/// happens and [`super::table::Family`] is what decides it.
#[derive(Clone, Debug, Serialize, Copy)]
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
    /// width written.
    EXT(RegisterOperand, Size, Size),
    /// `tst.<size> <ea>`
    TST(Operand, Size),
    /// `cmp.<size> <ea>,Dn`
    CMP(Operand, RegisterOperand, Size),
    /// `Bcc <address>`
    Bcc(u32, Condition),
    /// `Scc <ea>`
    Scc(Operand, Condition),
    /// `DBcc Dn,<address>`
    DBcc(RegisterOperand, u32, Condition),
    /// `bra <address>`
    BRA(u32), //could use offset instead of address
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
    /// `bsr <address>`
    BSR(u32),
    /// `trap #0-15`; only `#15`, the I/O trap, is simulated.
    TRAP(u8),
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
    /// and CCR" (the implementation notes, phase 3).
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
    /// `illegal`: end the run, always.
    ILLEGAL,
    /// `rts`
    RTS,
    /// `nop`
    NOP,
    /// `simhalt`, the Directive that ends the run.
    ///
    /// It is not a 68000 instruction: EASy68K assembles it to the object code
    /// `$FFFFFFFF`, which its simulator reads as "halt"
    /// (`Directives/simhalt.htm`). Here it is an executable item of the Program
    /// like any other, four bytes at its own address, and the Interpreter ends
    /// the run on it with the status the Terminate task gives, modifying no
    /// register.
    SIMHALT,
}
