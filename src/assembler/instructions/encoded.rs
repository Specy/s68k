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

use crate::machine::Processor;
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
        /// and its target is the instruction's first word, as EASy68K
        /// assembles it.
        target: u32,
        /// The index register and the width it is read at.
        index: IndexRegister,
    },

    /// The 68020's indexed modes beyond the 68000's, in the effective address
    /// modes of `d8(An,Xn)` and `d8(PC,Xn)`: a scaled index in the brief
    /// extension word, `(4,a0,d1.w*4)`, and everything the full extension word
    /// encodes, `(1000,a0,d1.l)`, `(d0.l*4)`, `([8,a0],d1.w,4)`,
    /// `([label,pc,d1.w])` (the M68000 Programmer's Reference Manual, 2.2.8
    /// to 2.2.10 and 2.2.13 to 2.2.15).
    ///
    /// The extension word is kept as it is encoded, with what it says read
    /// through [`ExtensionWord`], so that every word decodes to a value and
    /// encodes back to itself; the decoder makes one only for the 68020, and
    /// only for a word whose bits 10–8 are not clear, so a word the 68000 reads
    /// is an [`IndirectIndex`](Operand::IndirectIndex) or a
    /// [`PcIndex`](Operand::PcIndex) on every processor. Its fields fit in the
    /// twelve bytes the 68000's own Operands take in WebAssembly, so the
    /// Instructions the decode cache keeps are no larger for it.
    Extended {
        /// The base register the effective address field names: an address
        /// register, or the program counter. The extension word says whether
        /// it is suppressed.
        base: IndexBase,
        /// The extension word. A brief one is kept with its low byte 0: its
        /// displacement is `displacement`, as encoded at the address the word
        /// is at, so that the same Operand is the same value wherever it is.
        word: ExtensionWord,
        /// The base displacement, sign extended from its width, 0 when it is
        /// null: the brief word's byte or the full word's word or long. For the
        /// program counter as the base, not suppressed, it is the address the
        /// displacement reaches from the extension word instead, as
        /// [`PcIndex`](Operand::PcIndex) keeps it.
        displacement: i32,
        /// The outer displacement of a memory indirect mode, sign extended
        /// from its width, 0 when it is null or there is none.
        outer: i32,
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

// The 68020's Operand takes no more room than the 68000's do in WebAssembly,
// so the Instructions the decode cache keeps are no larger for it.
#[cfg(target_pointer_width = "32")]
const _: () = assert!(std::mem::size_of::<Operand>() == 12);

/// The base register of an [`Operand::Extended`]: an address register,
/// `A0` to `A7`, or the program counter, as the effective address field names
/// it, mode 6 or mode 7 with register 3.
///
/// It is one byte, so that the Operand that carries it stays as small as the
/// 68000's.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
pub struct IndexBase(u8);

impl IndexBase {
    /// The program counter, `d8(PC,Xn)`'s mode.
    pub const PC: IndexBase = IndexBase(8);

    /// The address register of number `number`, of which the low three bits
    /// are read.
    pub fn address(number: u8) -> IndexBase {
        IndexBase(number & 7)
    }

    /// Whether it is the program counter.
    pub fn is_pc(self) -> bool {
        self.0 == 8
    }

    /// The address register's number, `None` for the program counter.
    pub fn address_register(self) -> Option<u8> {
        (self.0 < 8).then_some(self.0)
    }
}

/// How wide a base or an outer displacement of the full extension word is:
/// null, which takes no word and counts as 0, a word or a long.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
pub enum DisplacementSize {
    /// No displacement: none is encoded, and it counts as 0.
    Null,
    /// One word, sign extended.
    Word,
    /// Two words.
    Long,
}

impl DisplacementSize {
    /// The bytes it takes.
    pub fn bytes(self) -> u32 {
        match self {
            DisplacementSize::Null => 0,
            DisplacementSize::Word => 2,
            DisplacementSize::Long => 4,
        }
    }

    /// The bits of the full extension word that say it: `01`, `10` or `11`,
    /// in the BD SIZE field (bits 5–4) and the low two bits of I/IS alike.
    pub fn code(self) -> u16 {
        match self {
            DisplacementSize::Null => 1,
            DisplacementSize::Word => 2,
            DisplacementSize::Long => 3,
        }
    }

    /// The size a two-bit code says, as the 68020 reads it: `10` a word, `11`
    /// a long, and `01` and the reserved `00` none.
    pub fn of_code(code: u16) -> DisplacementSize {
        match code & 3 {
            2 => DisplacementSize::Word,
            3 => DisplacementSize::Long,
            _ => DisplacementSize::Null,
        }
    }
}

/// Whether and how an [`Operand::Extended`] reads its address from memory:
/// the full extension word's IS and I/IS fields.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
pub enum Indirection {
    /// `(bd,An,Xn)`: the address is the sum.
    None,
    /// `([bd,An,Xn],od)`: the index is added before the long at the sum is
    /// read, and the outer displacement to what it holds. With the index
    /// suppressed it is the 68020's memory indirect with no index,
    /// `([bd,An],od)`.
    Preindexed,
    /// `([bd,An],Xn,od)`: the long at the base and the base displacement is
    /// read, and the index and the outer displacement are added to what it
    /// holds.
    Postindexed,
}

/// The extension word of an [`Operand::Extended`], as it is encoded, and what
/// the 68020 reads in it (the M68000 Programmer's Reference Manual, figure
/// 2-2 and table 2-2):
///
/// * the index register in bits 15–12, its width in bit 11 and its scale in
///   bits 10–9, for both formats;
/// * bit 8, clear for the brief format, whose low byte is the displacement,
///   and set for the full one;
/// * in the full one, BS (bit 7) and IS (bit 6), which suppress the base and
///   the index, BD SIZE (bits 5–4), and I/IS (bits 2–0), the indirection and
///   the outer displacement's size.
///
/// The combinations the manual reserves, BD SIZE `00` and I/IS `100` (and
/// `1xx` with the index suppressed), are read as Musashi reads them, bit by
/// bit: a BD SIZE of `00` is a null displacement, and bit 2 of I/IS makes the
/// indirection postindexed. No assembler writes them, and the disassembler
/// prints an instruction that has one as its words
/// ([`is_canonical`](ExtensionWord::is_canonical)).
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
pub struct ExtensionWord(pub u16);

impl ExtensionWord {
    /// The bit that marks the full format.
    pub const FULL: u16 = 0x0100;
    /// BS, the base suppressed.
    pub const BASE_SUPPRESSED: u16 = 0x0080;
    /// IS, the index suppressed.
    pub const INDEX_SUPPRESSED: u16 = 0x0040;

    /// Whether it is the full format.
    pub fn is_full(self) -> bool {
        self.0 & Self::FULL != 0
    }

    /// The index register and the width it is read at, whether or not it is
    /// suppressed.
    pub fn index_register(self) -> IndexRegister {
        let number = ((self.0 >> 12) & 7) as u8;
        IndexRegister {
            register: match self.0 & 0x8000 != 0 {
                true => RegisterOperand::Address(number),
                false => RegisterOperand::Data(number),
            },
            size: match self.0 & 0x0800 != 0 {
                true => Size::Long,
                false => Size::Word,
            },
        }
    }

    /// How far the index is shifted left: 0 to 3, for a scale of 1, 2, 4 or 8.
    pub fn scale_shift(self) -> u32 {
        ((self.0 >> 9) & 3) as u32
    }

    /// The scale, 1, 2, 4 or 8.
    pub fn scale(self) -> u8 {
        1 << self.scale_shift()
    }

    /// The index register, `None` when it is suppressed.
    pub fn index(self) -> Option<IndexRegister> {
        (!self.index_suppressed()).then(|| self.index_register())
    }

    /// Whether the base register is suppressed, which only the full format
    /// can say.
    pub fn base_suppressed(self) -> bool {
        self.is_full() && self.0 & Self::BASE_SUPPRESSED != 0
    }

    /// Whether the index is suppressed, which only the full format can say.
    pub fn index_suppressed(self) -> bool {
        self.is_full() && self.0 & Self::INDEX_SUPPRESSED != 0
    }

    /// The width of the base displacement in the full format; the brief
    /// format's is its low byte.
    pub fn base_displacement_size(self) -> DisplacementSize {
        DisplacementSize::of_code(self.0 >> 4)
    }

    /// Whether and how the address is read from memory.
    pub fn indirection(self) -> Indirection {
        match (self.is_full(), self.0 & 7) {
            (false, _) | (true, 0) => Indirection::None,
            (true, 1..=3) => Indirection::Preindexed,
            (true, _) => Indirection::Postindexed,
        }
    }

    /// The width of the outer displacement, null when there is no
    /// indirection.
    pub fn outer_displacement_size(self) -> DisplacementSize {
        match self.indirection() {
            Indirection::None => DisplacementSize::Null,
            _ => DisplacementSize::of_code(self.0),
        }
    }

    /// The bytes of extension words it takes, itself included.
    pub fn bytes(self) -> u32 {
        match self.is_full() {
            false => 2,
            true => {
                2 + self.base_displacement_size().bytes() + self.outer_displacement_size().bytes()
            }
        }
    }

    /// The full extension word of these parts; the index register, its width
    /// and its scale are read from `index` and `scale` even when `index` is
    /// suppressed, so that the word says what was written.
    pub fn full(
        index: IndexRegister,
        scale: u8,
        base_suppressed: bool,
        index_suppressed: bool,
        base_displacement: DisplacementSize,
        indirection: Indirection,
        outer_displacement: DisplacementSize,
    ) -> ExtensionWord {
        let mut word = Self::FULL | Self::index_bits(index, scale);
        if base_suppressed {
            word |= Self::BASE_SUPPRESSED;
        }
        if index_suppressed {
            word |= Self::INDEX_SUPPRESSED;
        }
        word |= base_displacement.code() << 4;
        word |= match indirection {
            Indirection::None => 0,
            Indirection::Preindexed => outer_displacement.code(),
            Indirection::Postindexed => 4 | outer_displacement.code(),
        };
        ExtensionWord(word)
    }

    /// The brief extension word of an index register with a scale, as an
    /// [`Operand::Extended`] keeps it: its low byte, the displacement, is 0,
    /// as the displacement is the Operand's.
    pub fn brief(index: IndexRegister, scale: u8) -> ExtensionWord {
        ExtensionWord(Self::index_bits(index, scale))
    }

    /// Bits 15–9: the index register, its width and its scale.
    fn index_bits(index: IndexRegister, scale: u8) -> u16 {
        let register = match index.register {
            RegisterOperand::Data(number) => (number & 7) as u16,
            RegisterOperand::Address(number) => 8 | (number & 7) as u16,
        };
        let long = match index.size {
            Size::Long => 0x0800,
            _ => 0,
        };
        let scale = match scale {
            2 => 1,
            4 => 2,
            8 => 3,
            _ => 0,
        };
        register << 12 | long | scale << 9
    }

    /// Whether an assembler writes this word, and the disassembler can print
    /// what reassembles to it: no reserved combination, bit 3 of the full
    /// format clear, and the index bits clear when the index is suppressed.
    pub fn is_canonical(self) -> bool {
        if !self.is_full() {
            return true;
        }
        let reserved = (self.0 >> 4) & 3 == 0
            || self.0 & 0x8 != 0
            || match self.index_suppressed() {
                false => self.0 & 7 == 4,
                true => self.0 & 4 != 0 || self.0 & 0xFE00 != 0,
            };
        !reserved
    }
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
/// writes it, and only the Assembler reads the number, to check that the
/// distance fits its field (`Analyzer::check_pc_distance`).
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

/// The offset or the width of a bit field, as its extension word holds it:
/// a number in five bits, or a data register that holds it.
#[derive(Copy, Clone, Debug, Serialize, PartialEq, Eq)]
pub enum FieldValue {
    /// An offset of 0 to 31, or a width of 1 to 31, 0 standing for 32.
    Immediate(u8),
    /// The data register that holds it: an offset read as a signed long, a
    /// width taken modulo 32, 0 standing for 32.
    Register(u8),
}

/// The `{offset:width}` of a 68020 bit-field instruction.
#[derive(Copy, Clone, Debug, Serialize, PartialEq, Eq)]
pub struct BitField {
    /// The bit the field starts at, counted from the most significant bit of
    /// the operand: of a data register, modulo 32; in memory, from the most
    /// significant bit of the byte the operand names, any distance either way.
    pub offset: FieldValue,
    /// How many bits the field holds, 1 to 32.
    pub width: FieldValue,
}

/// Which of the eight bit-field instructions an [`Instruction::BF`] is, in
/// the order of bits 10–8 of its operation word.
#[derive(Copy, Clone, Debug, Serialize, PartialEq, Eq)]
pub enum BitFieldOperation {
    /// `bftst`: the flags of the field.
    Test,
    /// `bfextu`: the field into `Dn`, zero extended.
    ExtractUnsigned,
    /// `bfchg`: the field complemented.
    Change,
    /// `bfexts`: the field into `Dn`, sign extended.
    ExtractSigned,
    /// `bfclr`: the field cleared.
    Clear,
    /// `bfffo`: the offset of the field's first set bit into `Dn`.
    FindFirstOne,
    /// `bfset`: the field set.
    Set,
    /// `bfins`: the low bits of `Dn` into the field.
    Insert,
}

impl BitFieldOperation {
    /// The eight, in the order of their code.
    pub const ALL: [BitFieldOperation; 8] = [
        BitFieldOperation::Test,
        BitFieldOperation::ExtractUnsigned,
        BitFieldOperation::Change,
        BitFieldOperation::ExtractSigned,
        BitFieldOperation::Clear,
        BitFieldOperation::FindFirstOne,
        BitFieldOperation::Set,
        BitFieldOperation::Insert,
    ];

    /// Its mnemonic, lower case.
    pub fn mnemonic(self) -> &'static str {
        match self {
            BitFieldOperation::Test => "bftst",
            BitFieldOperation::ExtractUnsigned => "bfextu",
            BitFieldOperation::Change => "bfchg",
            BitFieldOperation::ExtractSigned => "bfexts",
            BitFieldOperation::Clear => "bfclr",
            BitFieldOperation::FindFirstOne => "bfffo",
            BitFieldOperation::Set => "bfset",
            BitFieldOperation::Insert => "bfins",
        }
    }

    /// Whether it names a data register beside the field: `bfextu`,
    /// `bfexts` and `bfffo` write one, `bfins` reads one.
    pub fn has_register(self) -> bool {
        matches!(
            self,
            BitFieldOperation::ExtractUnsigned
                | BitFieldOperation::ExtractSigned
                | BitFieldOperation::FindFirstOne
                | BitFieldOperation::Insert
        )
    }

    /// Whether it writes the field back.
    pub fn writes(self) -> bool {
        matches!(
            self,
            BitFieldOperation::Change
                | BitFieldOperation::Clear
                | BitFieldOperation::Set
                | BitFieldOperation::Insert
        )
    }
}

/// The operand of the 68020's `trapcc`, which the processor reads past and
/// only a handler looks at, by the address the exception stacks: none, a
/// word or a long, as the operation word's opmode says (`%100`, `%010` and
/// `%011`).
#[derive(Copy, Clone, Debug, Serialize, PartialEq, Eq)]
pub enum TrapOperand {
    /// `trapcc`, with no operand.
    None,
    /// `trapcc.w #n`.
    Word(u16),
    /// `trapcc.l #n`.
    Long(u32),
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
/// What is kept is what the processor reads. The bits it ignores are not:
/// bits 10–8 of a brief index word on the 68000 and the 68010, which the
/// 68020 reads ([`Operand::Extended`]), and the high byte of the word that holds a byte
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
    /// `trap #0-15`: `#0` to `#14` go through vectors 32 to 46, and `#15` is
    /// EASy68K's input and output trap.
    TRAP(u8),
    /// `rte`: pop the status register and the program counter an exception
    /// pushed. Privileged.
    RTE,
    /// `stop #n`: load the status register and wait for an interrupt.
    /// Privileged. Not run yet.
    STOP(u16),
    /// `reset`: assert the reset line to the devices. Privileged. Not run yet.
    RESET,
    /// `move usp,An` and `move An,usp`: the user stack pointer, which only
    /// supervisor mode can reach. Privileged.
    MOVEUSP {
        /// `true` for `move An,usp`, `false` for `move usp,An`.
        to_usp: bool,
        /// The address register on the other side.
        register: RegisterOperand,
    },
    /// A word of the form `$Axxx`, the line 1010 emulator: it raises an
    /// exception of its own (vector 10), so that software can give it a
    /// meaning. The whole word is kept.
    LINEA(u16),
    /// A word of the form `$Fxxx` other than SIMHALT's `$FFFF $FFFF`, the line
    /// 1111 emulator (vector 11), where the 68020 finds its coprocessors. The
    /// whole word is kept.
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
    /// `chk <ea>,Dn`: the exception of vector 6 when the register is outside
    /// 0 to `<ea>`.
    CHK(Operand, RegisterOperand),
    /// `trapv`: the exception of vector 7 when the overflow flag is set.
    TRAPV,
    /// `illegal`, `$4AFC`: the illegal instruction exception, vector 4,
    /// always.
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
    /// `rtd #d`, the 68010's: pop the return address, as `rts` does, then add
    /// the displacement to the stack pointer, so that a subroutine takes its
    /// arguments off the stack as it returns. The displacement is kept sign
    /// extended, as `link`'s is.
    RTD(u32),
    /// `bkpt #n`, the 68010's breakpoint, `n` from 0 to 7: the processor runs
    /// a breakpoint acknowledge cycle, which nothing answers here, and takes
    /// the illegal instruction exception, vector 4, as the 68010 does whatever
    /// answers the cycle.
    BKPT(u8),
    /// `movec Rc,Rn` and `movec Rn,Rc`, the 68010's: a long between a general
    /// register and a control register. Privileged. A control register the
    /// processor does not have is an illegal instruction when it runs, so the
    /// code is kept as it is encoded, whatever it is.
    MOVEC {
        /// `true` for `movec Rn,Rc`, which writes the control register.
        to_control: bool,
        /// The control register, by its twelve-bit code.
        control: ControlRegister,
        /// The general register on the other side.
        register: RegisterOperand,
    },
    /// `moves.<size>`, the 68010's: a move between a general register and
    /// memory in the address space SFC or DFC names. Privileged.
    MOVES {
        /// `ToMemory` is `moves Rn,<ea>`, which writes through DFC;
        /// `FromMemory` is `moves <ea>,Rn`, which reads through SFC.
        direction: TargetDirection,
        /// `.b`, `.w` or `.l`.
        size: Size,
        /// The general register, a data or an address register.
        register: RegisterOperand,
        /// The memory operand, memory alterable.
        target: Operand,
    },
    /// `chk.l <ea>,Dn`, the 68020's: the whole register against 0 and a long
    /// bound, both signed, and the exception of vector 6 outside them.
    CHKL(Operand, RegisterOperand),
    /// `link.l An,#n`, the 68020's: `link` with a 32-bit displacement.
    LINKL(RegisterOperand, u32),
    /// `trapcc`, `trapcc.w #n` and `trapcc.l #n`, the 68020's: the exception
    /// of vector 7 when the condition is true, with the operand, if any,
    /// left for the handler. `t` and `f` are conditions like the others here.
    TRAPcc(Condition, TrapOperand),
    /// `muls.l` and `mulu.l`, the 68020's: `<ea>,Dl`, 32 bits by 32 into
    /// the 32 of `Dl`, or `<ea>,Dh:Dl`, into 64.
    MULL {
        /// The multiplier, a long.
        source: Operand,
        /// `Dl`: the multiplicand, and the low 32 bits of the product.
        low: u8,
        /// `Dh`, the high 32 bits of a 64-bit product, or `None` for a
        /// 32-bit one, whose extension word leaves the field unread.
        high: Option<u8>,
        /// Signed or unsigned.
        sign: Sign,
    },
    /// `pack -(Ax),-(Ay),#adj` and `pack Dx,Dy,#adj`, the 68020's: the low
    /// digits of two bytes, the word they make plus the adjustment, packed
    /// into one byte. The source, the destination and the adjustment.
    PACK(Operand, Operand, u16),
    /// `unpk -(Ax),-(Ay),#adj` and `unpk Dx,Dy,#adj`, the 68020's: the two
    /// digits of a byte spread into the low digits of a word, plus the
    /// adjustment. The source, the destination and the adjustment.
    UNPK(Operand, Operand, u16),
    /// `chk2.<size> <ea>,Rn`, the 68020's: the register against the bounds
    /// pair at `<ea>`, the lower first, and the exception of vector 6 outside
    /// them.
    CHK2(Operand, RegisterOperand, Size),
    /// `cmp2.<size> <ea>,Rn`, the 68020's: `chk2` without the exception.
    CMP2(Operand, RegisterOperand, Size),
    /// `cas.<size> Dc,Du,<ea>`, the 68020's: compare the operand with `Dc`,
    /// then write `Du` to it when they are equal, or the operand into `Dc`
    /// when not.
    CAS {
        /// `Dc`, the value compared.
        compare: u8,
        /// `Du`, the value written when they are equal.
        update: u8,
        /// The memory operand.
        target: Operand,
        /// `.b`, `.w` or `.l`.
        size: Size,
    },
    /// `cas2.<size> Dc1:Dc2,Du1:Du2,(Rn1):(Rn2)`, the 68020's: `cas` of two
    /// operands at once, each through the address a register holds, both
    /// written when both are equal.
    CAS2 {
        /// `Dc1` and `Dc2`.
        compare: (u8, u8),
        /// `Du1` and `Du2`.
        update: (u8, u8),
        /// `Rn1` and `Rn2`, data or address registers.
        pointers: (RegisterOperand, RegisterOperand),
        /// `.w` or `.l`.
        size: Size,
    },
    /// The 68020's eight bit-field instructions, which EASy68K's `opt bit`
    /// gives a 68000 program too: a field of a data register, or of memory
    /// from the byte the operand names, `{offset:width}`.
    BF {
        /// Which of the eight.
        operation: BitFieldOperation,
        /// The data register or the place in memory the field is in.
        target: Operand,
        /// Where the field is and how wide.
        field: BitField,
        /// The data register `bfextu`, `bfexts` and `bfffo` write and `bfins`
        /// reads; 0 for the others, whose extension word leaves it clear.
        register: u8,
    },
    /// `divs.l`, `divu.l`, `divsl.l` and `divul.l`, the 68020's: a dividend
    /// of 32 bits in `Dq`, or of 64 in `Dr:Dq` when `wide`, divided by a long,
    /// the quotient into `Dq` and the remainder into `Dr`. `divs.l <ea>,Dq`
    /// is the 32-bit form with `Dr` the same register as `Dq`, which keeps
    /// the quotient alone; `divsl.l <ea>,Dr:Dq` the same with another.
    DIVL {
        /// The divisor, a long.
        source: Operand,
        /// `Dq`: the dividend's low 32 bits, then the quotient.
        quotient: u8,
        /// `Dr`: the dividend's high 32 bits when `wide`, then the remainder.
        remainder: u8,
        /// Whether the dividend is the 64 bits of `Dr:Dq`.
        wide: bool,
        /// Signed or unsigned.
        sign: Sign,
    },
}

/// A control register of the 68010 and the 68020, as `movec` names it by the
/// twelve-bit code of its extension word.
///
/// Every code is kept, the ones no processor has included, since `movec` with
/// one is an instruction that raises the illegal instruction exception when it
/// runs, and its words decode and print like any other. The 68010 has four:
/// SFC and DFC, the function codes `moves` reads and writes through, the
/// user stack pointer, and VBR, the vector base register. The 68020 adds
/// four: CACR and CAAR, the cache's control and address registers, and its
/// two supervisor stack pointers, MSP and ISP.
#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
pub struct ControlRegister(pub u16);

impl ControlRegister {
    /// The source function code, `$000`.
    pub const SFC: ControlRegister = ControlRegister(0x000);
    /// The destination function code, `$001`.
    pub const DFC: ControlRegister = ControlRegister(0x001);
    /// The 68020's cache control register, `$002`.
    pub const CACR: ControlRegister = ControlRegister(0x002);
    /// The user stack pointer, `$800`.
    pub const USP: ControlRegister = ControlRegister(0x800);
    /// The vector base register, `$801`.
    pub const VBR: ControlRegister = ControlRegister(0x801);
    /// The 68020's cache address register, `$802`.
    pub const CAAR: ControlRegister = ControlRegister(0x802);
    /// The 68020's master stack pointer, `$803`.
    pub const MSP: ControlRegister = ControlRegister(0x803);
    /// The 68020's interrupt stack pointer, `$804`.
    pub const ISP: ControlRegister = ControlRegister(0x804);

    /// The control register of the low twelve bits of `code`.
    pub fn from_code(code: u16) -> ControlRegister {
        ControlRegister(code & 0x0FFF)
    }

    /// Its name as `movec` writes it, `sfc`, `dfc`, `usp`, `vbr`, `cacr`,
    /// `caar`, `msp` or `isp`, or `None` for a code no processor s68k has
    /// gives a register.
    pub fn name(self) -> Option<&'static str> {
        Some(match self {
            ControlRegister::SFC => "sfc",
            ControlRegister::DFC => "dfc",
            ControlRegister::USP => "usp",
            ControlRegister::VBR => "vbr",
            ControlRegister::CACR => "cacr",
            ControlRegister::CAAR => "caar",
            ControlRegister::MSP => "msp",
            ControlRegister::ISP => "isp",
            _ => return None,
        })
    }

    /// The first processor whose `movec` reaches it, or `None` for a code no
    /// processor s68k has gives a register, which is an illegal instruction
    /// on every one.
    pub fn processor(self) -> Option<Processor> {
        match self {
            ControlRegister::SFC
            | ControlRegister::DFC
            | ControlRegister::USP
            | ControlRegister::VBR => Some(Processor::M68010),
            ControlRegister::CACR
            | ControlRegister::CAAR
            | ControlRegister::MSP
            | ControlRegister::ISP => Some(Processor::M68020),
            _ => None,
        }
    }
}
