//! From words to an [`Instruction`]: the decoder.
//!
//! [`decode`] reads the first word of an instruction, finds the row of the
//! opcode map it belongs to, and reads the extension words the row asks for:
//! an immediate, a displacement, a `movem` mask, an address. What comes back
//! is the [`Instruction`] the Assembler would have built for the same
//! encoding, and its size in bytes.
//!
//! # The opcode map
//!
//! `LINES` holds one table per line, the top four bits of the first word, as
//! the operation code map of the M68000 Programmer's Reference Manual
//! (Section 8) groups them. A row is a pattern — the bits that must match
//! under a mask — the first [`Processor`] that has it, a test of the rest of
//! the word (the effective-address modes and the sizes the instruction
//! allows), and the function that reads the operands. Rows overlap where the 68000's encodings
//! do: `addx` sits inside `add Dn,<ea>` where the effective address would be a
//! register, `cmpm` inside `eor`, `dbcc` inside `scc`, `movep` inside the
//! dynamic bit instructions. The narrower row comes first, and the first row
//! that accepts the word wins. A word no row accepts is an illegal instruction,
//! [`Instruction::IllegalWord`].
//!
//! Whether a word is an instruction depends on the first word alone, never on
//! the extension words: that is what [`is_legal`] answers, and what the test
//! against Musashi's table checks for all 65,536 of them.
//!
//! # Dialects
//!
//! Every row is gated on its first processor, so a word the 68010 or the 68020
//! adds is illegal on the processors before it: `move ccr,<ea>` (`$42C0` to
//! `$42F9`) on the 68000, `extb.l` (`$49C0` to `$49C7`) on both the 68000 and
//! the 68010. Only the 68000 is decoded in full; the other instructions and the
//! addressing modes of the later processors are not read yet, and decode as
//! illegal for every processor.
//!
//! Two readings differ by processor (`docs/research/m68k-cpu-variants.md`
//! §2.1 in the asm-editor):
//!
//! * a branch whose displacement byte is `$FF`, which is −1 on the 68000 and
//!   the 68010 and a 32-bit displacement on the 68020, in a row of its own;
//! * the brief extension word of `d8(An,Xn)` and `d8(PC,Xn)`, whose bits 10–8
//!   the 68000 and the 68010 ignore and the 68020 reads as a scale and as the
//!   mark of its full extension word. Those are the 68020's addressing modes,
//!   so every processor reads the word as the 68000 does for now.
//!
//! # SIMHALT
//!
//! `$FFFF $FFFF` is [`Instruction::SIMHALT`], EASy68K's halt, for every
//! processor. It is recognised before line F, which `$FFFF` alone would be.

use crate::instructions::{
    AddressWidth, BranchSize, Condition, IndexRegister, Instruction, Operand, RegisterOperand,
    ShiftDirection, Sign, Size, TargetDirection,
};

use super::Processor;

/// One decoded instruction, and how many bytes its words took up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Decoded {
    /// The instruction the words encode.
    pub instruction: Instruction,
    /// The bytes it takes up, its first word included: 2 to
    /// [`MAX_68000_SIZE`] on the 68000.
    pub size: u32,
}

/// The longest a 68000 instruction is, in bytes: the operation word and two
/// longs, as in `move.l #n,$xxxxxxxx.l`.
pub const MAX_68000_SIZE: u32 = 10;

/// Decodes the instruction at `address` for `processor`, reading its words
/// through `read`.
///
/// `read` is asked for the word at an address, the instruction's own first
/// and then each extension word in turn, never for more than the instruction
/// needs. The addresses go up by two from `address` and wrap at 32 bits; the
/// caller decides what lies past the end of its memory. An error from `read`
/// is answered as it is, except while looking for SIMHALT's second word: a
/// `$FFFF` whose next word cannot be read is line F.
///
/// The addresses also place the instruction: a branch target and a
/// PC-relative operand are worked out from the address of the word their
/// displacement is in, so the same words decode to different targets at
/// different addresses.
pub fn decode<E>(
    address: u32,
    processor: Processor,
    mut read: impl FnMut(u32) -> Result<u16, E>,
) -> Result<Decoded, E> {
    let mut error = None;
    let decoded = decode_with(address, processor, &mut |at| match read(at) {
        Ok(word) => Some(word),
        Err(failure) => {
            error = Some(failure);
            None
        }
    });
    match (decoded, error) {
        (Some(decoded), _) => Ok(decoded),
        (None, Some(error)) => Err(error),
        // `decode_with` answers `None` only when a read failed, and every failed
        // read leaves its error behind.
        (None, None) => unreachable!("a decode failed without a failed read"),
    }
}

/// The words of an instruction ran out before it was complete.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Truncated {
    /// The address of the first word that was asked for and not there.
    pub address: u32,
}

/// Decodes the instruction whose first word is `words[0]`, at `address`, for
/// `processor`. It is [`decode`] over a slice: the error says where the words
/// ran out.
pub fn decode_words(
    words: &[u16],
    address: u32,
    processor: Processor,
) -> Result<Decoded, Truncated> {
    decode(address, processor, |at| {
        let index = at.wrapping_sub(address) / 2;
        words
            .get(index as usize)
            .copied()
            .ok_or(Truncated { address: at })
    })
}

/// Whether `word` is the first word of an instruction of `processor`.
///
/// Line A and line F words are: each is an instruction that raises an
/// exception of its own. So is `$4AFC`, `illegal`. Every other word that
/// [`decode`] answers with [`Instruction::IllegalWord`] is not.
pub fn is_legal(word: u16, processor: Processor) -> bool {
    row_for(word, processor).is_some()
}

/// The first processor whose decoder reads `word` as an instruction, or
/// `None` when none does yet.
///
/// A word can mean different instructions to different processors — `$60FF`
/// is a short branch by −1 on the 68000 and a long branch on the 68020 — and
/// this is the oldest that has any.
pub fn first_processor(word: u16) -> Option<Processor> {
    Processor::ALL
        .into_iter()
        .find(|processor| is_legal(word, *processor))
}

/// The decoder proper, over a reader that answers `None` for a word it cannot
/// read; `None` back means a read failed.
fn decode_with(
    address: u32,
    processor: Processor,
    read: &mut dyn FnMut(u32) -> Option<u16>,
) -> Option<Decoded> {
    let mut words = Words {
        read,
        next: address,
    };
    let word = words.word()?;
    let instruction = match row_for(word, processor) {
        Some(row) => (row.build)(&mut words, word)?,
        None => Instruction::IllegalWord(word),
    };
    Some(Decoded {
        instruction,
        size: words.next.wrapping_sub(address),
    })
}

/// The row of the opcode map that reads `word` for `processor`, if any.
fn row_for(word: u16, processor: Processor) -> Option<&'static Row> {
    LINES[(word >> 12) as usize]
        .iter()
        .find(|row| word & row.mask == row.bits && processor.has(row.first) && (row.legal)(word))
}

// ---------------------------------------------------------------------------
// Reading the extension words
// ---------------------------------------------------------------------------

/// The words of one instruction, read in order.
struct Words<'r> {
    read: &'r mut dyn FnMut(u32) -> Option<u16>,
    /// The address of the next word to read, which is also what a PC-relative
    /// displacement in that word is measured from.
    next: u32,
}

impl Words<'_> {
    /// The next word.
    fn word(&mut self) -> Option<u16> {
        let word = (self.read)(self.next)?;
        self.next = self.next.wrapping_add(2);
        Some(word)
    }

    /// The next two words, as a long, the high word first.
    fn long(&mut self) -> Option<u32> {
        let high = self.word()? as u32;
        let low = self.word()? as u32;
        Some(high << 16 | low)
    }

    /// The next word, sign extended, added to the address it was read from:
    /// the target of a 16-bit displacement.
    fn displaced(&mut self) -> Option<u32> {
        let at = self.next;
        Some(at.wrapping_add(self.word()? as i16 as i32 as u32))
    }

    /// An immediate of `size`. A byte one occupies a word, of which the low
    /// byte is the value; a long one occupies two words. The value is kept at
    /// its size, zero extended.
    fn immediate(&mut self, size: Size) -> Option<u32> {
        Some(match size {
            Size::Byte => (self.word()? & 0xFF) as u32,
            Size::Word => self.word()? as u32,
            Size::Long => self.long()?,
        })
    }

    /// The Operand a six-bit effective-address field names, mode in bits 5–3
    /// and register in bits 2–0, reading its extension words. `size` is the
    /// size an immediate has. The rows have already refused the modes the
    /// instruction does not take, and mode 7 with a register above 4.
    fn ea(&mut self, field: u16, size: Size) -> Option<Operand> {
        let register = (field & 7) as u8;
        Some(match (field >> 3) & 7 {
            0 => Operand::Register(RegisterOperand::Data(register)),
            1 => Operand::Register(RegisterOperand::Address(register)),
            2 => Operand::Indirect(register),
            3 => Operand::PostIndirect(register),
            4 => Operand::PreIndirect(register),
            5 => Operand::IndirectDisplacement {
                offset: self.word()? as i16 as i32,
                base: RegisterOperand::Address(register),
            },
            6 => {
                let extension = self.word()?;
                Operand::IndirectIndex {
                    base: RegisterOperand::Address(register),
                    offset: extension as u8 as i8 as i32,
                    index: index_register(extension),
                }
            }
            _ => match register {
                0 => Operand::Absolute {
                    address: self.word()? as i16 as i32 as u32 as usize,
                    width: AddressWidth::Word,
                },
                1 => Operand::Absolute {
                    address: self.long()? as usize,
                    width: AddressWidth::Long,
                },
                2 => Operand::PcDisplacement {
                    target: self.displaced()?,
                },
                3 => {
                    let at = self.next;
                    let extension = self.word()?;
                    Operand::PcIndex {
                        target: at.wrapping_add(extension as u8 as i8 as i32 as u32),
                        index: index_register(extension),
                    }
                }
                // 4; the rows refuse 5 to 7.
                _ => Operand::Immediate(self.immediate(size)?),
            },
        })
    }
}

/// The index register of a brief extension word: bit 15 picks the data or the
/// address registers, bits 14–12 the register, bit 11 a word or a long.
///
/// Bits 10–8 are not read. The 68000 and the 68010 ignore them; the 68020
/// reads bits 10–9 as a scale and bit 8 as the mark of its full extension
/// word, which are the 68020's addressing modes and are not decoded yet, so
/// every processor reads the word as the 68000 does. The displacement, bits
/// 7–0, is read by the caller.
fn index_register(extension: u16) -> IndexRegister {
    let number = ((extension >> 12) & 7) as u8;
    IndexRegister {
        register: if extension & 0x8000 != 0 {
            RegisterOperand::Address(number)
        } else {
            RegisterOperand::Data(number)
        },
        size: if extension & 0x0800 != 0 {
            Size::Long
        } else {
            Size::Word
        },
    }
}

// ---------------------------------------------------------------------------
// The effective-address modes a row accepts
// ---------------------------------------------------------------------------

/// `Dn`
pub(super) const DN: u16 = 1 << 0;
/// `An`
pub(super) const AN: u16 = 1 << 1;
/// `(An)`
pub(super) const INDIRECT: u16 = 1 << 2;
/// `(An)+`
pub(super) const POSTINCREMENT: u16 = 1 << 3;
/// `-(An)`
pub(super) const PREDECREMENT: u16 = 1 << 4;
/// `d16(An)`
pub(super) const DISPLACEMENT: u16 = 1 << 5;
/// `d8(An,Xn)`
pub(super) const INDEX: u16 = 1 << 6;
/// `(xxx).W`
pub(super) const ABSOLUTE_WORD: u16 = 1 << 7;
/// `(xxx).L`
pub(super) const ABSOLUTE_LONG: u16 = 1 << 8;
/// `d16(PC)`
pub(super) const PC_DISPLACEMENT: u16 = 1 << 9;
/// `d8(PC,Xn)`
pub(super) const PC_INDEX: u16 = 1 << 10;
/// `#<data>`
pub(super) const IMMEDIATE: u16 = 1 << 11;

/// The Programmer's Reference Manual's addressing categories (Section 2,
/// "Effective Addressing Mode Categories"), as sets of the modes above: the
/// alterable ones first.
pub(super) const ALTERABLE: u16 = DN
    | AN
    | INDIRECT
    | POSTINCREMENT
    | PREDECREMENT
    | DISPLACEMENT
    | INDEX
    | ABSOLUTE_WORD
    | ABSOLUTE_LONG;
/// Every mode.
pub(super) const ALL: u16 = ALTERABLE | PC_DISPLACEMENT | PC_INDEX | IMMEDIATE;
/// Everything but an address register.
pub(super) const DATA: u16 = ALL & !AN;
/// Data and alterable.
pub(super) const DATA_ALTERABLE: u16 = ALTERABLE & !AN;
/// In memory and alterable.
pub(super) const MEMORY_ALTERABLE: u16 = ALTERABLE & !(DN | AN);
/// A place in memory that is not walked over.
pub(super) const CONTROL: u16 =
    INDIRECT | DISPLACEMENT | INDEX | ABSOLUTE_WORD | ABSOLUTE_LONG | PC_DISPLACEMENT | PC_INDEX;
/// Control and alterable.
pub(super) const CONTROL_ALTERABLE: u16 = CONTROL & !(PC_DISPLACEMENT | PC_INDEX);
/// Data, without an immediate: what `btst #n,<ea>` tests.
pub(super) const DATA_NOT_IMMEDIATE: u16 = DATA & !IMMEDIATE;
/// Where `movem` writes registers to.
pub(super) const MOVEM_TO_MEMORY: u16 = CONTROL_ALTERABLE | PREDECREMENT;
/// Where `movem` reads registers from.
pub(super) const MOVEM_FROM_MEMORY: u16 = CONTROL | POSTINCREMENT;

/// The mode the effective-address field in bits 5–0 of `word` names, as one
/// of the bits above, or 0 for mode 7 with a register above 4.
fn mode_of(word: u16) -> u16 {
    mode(word >> 3, word)
}

/// The mode a mode and a register field name, each read from its low three
/// bits.
fn mode(mode: u16, register: u16) -> u16 {
    match (mode & 7, register & 7) {
        (0, _) => DN,
        (1, _) => AN,
        (2, _) => INDIRECT,
        (3, _) => POSTINCREMENT,
        (4, _) => PREDECREMENT,
        (5, _) => DISPLACEMENT,
        (6, _) => INDEX,
        (_, 0) => ABSOLUTE_WORD,
        (_, 1) => ABSOLUTE_LONG,
        (_, 2) => PC_DISPLACEMENT,
        (_, 3) => PC_INDEX,
        (_, 4) => IMMEDIATE,
        _ => 0,
    }
}

// ---------------------------------------------------------------------------
// The rows
// ---------------------------------------------------------------------------

/// One row of the opcode map.
struct Row {
    /// The bits of the first word the pattern fixes.
    mask: u16,
    /// What they must be.
    bits: u16,
    /// The first processor that has the instruction.
    first: Processor,
    /// The rest of the test: the modes and the sizes the instruction takes,
    /// from the first word alone.
    legal: fn(u16) -> bool,
    /// The operands, read from the first word and the extension words; `None`
    /// means a word could not be read.
    build: fn(&mut Words, u16) -> Option<Instruction>,
}

/// A row of the 68000's.
const fn row(
    mask: u16,
    bits: u16,
    legal: fn(u16) -> bool,
    build: fn(&mut Words, u16) -> Option<Instruction>,
) -> Row {
    gated(Processor::M68000, mask, bits, legal, build)
}

/// A row a later processor introduced.
const fn gated(
    first: Processor,
    mask: u16,
    bits: u16,
    legal: fn(u16) -> bool,
    build: fn(&mut Words, u16) -> Option<Instruction>,
) -> Row {
    Row {
        mask,
        bits,
        first,
        legal,
        build,
    }
}

/// The pattern is the whole test.
fn any(_: u16) -> bool {
    true
}

/// The effective address in bits 5–0 is one of `MODES`.
fn ea<const MODES: u16>(word: u16) -> bool {
    mode_of(word) & MODES != 0
}

/// The size in bits 7–6 is not `%11`.
fn sized(word: u16) -> bool {
    (word >> 6) & 3 != 3
}

/// A size in bits 7–6 and an effective address in `MODES` in bits 5–0, of
/// which an address register is only a word or a long: the 68000 has no byte
/// access to one.
fn sized_ea<const MODES: u16>(word: u16) -> bool {
    sized(word) && ea::<MODES>(word) && !(word & 0x00C0 == 0 && mode_of(word) == AN)
}

/// `move`: a source of any mode, a byte from an address register excepted,
/// and a data-alterable destination in bits 11–6, register first.
fn move_legal(word: u16) -> bool {
    let destination = mode(word >> 6, word >> 9);
    let byte = word & 0x3000 == 0x1000;
    destination & DATA_ALTERABLE != 0 && ea::<ALL>(word) && !(byte && mode_of(word) == AN)
}

/// The opcode map, one table per line (bits 15–12).
static LINES: [&[Row]; 16] = [
    LINE_0, LINE_1, LINE_2, LINE_3, LINE_4, LINE_5, LINE_6, LINE_7, LINE_8, LINE_9, LINE_A, LINE_B,
    LINE_C, LINE_D, LINE_E, LINE_F,
];

/// Line 0: bit manipulation, `movep` and the immediates.
static LINE_0: &[Row] = &[
    row(0xFFFF, 0x003C, any, ori_to_ccr),
    row(0xFFFF, 0x007C, any, ori_to_sr),
    row(0xFFFF, 0x023C, any, andi_to_ccr),
    row(0xFFFF, 0x027C, any, andi_to_sr),
    row(0xFFFF, 0x0A3C, any, eori_to_ccr),
    row(0xFFFF, 0x0A7C, any, eori_to_sr),
    row(0xF138, 0x0108, any, movep),
    row(0xF1C0, 0x0100, ea::<DATA>, bit_dynamic),
    row(0xF100, 0x0100, ea::<DATA_ALTERABLE>, bit_dynamic),
    row(0xFFC0, 0x0800, ea::<DATA_NOT_IMMEDIATE>, bit_static),
    row(0xFF00, 0x0800, ea::<DATA_ALTERABLE>, bit_static),
    row(0xFF00, 0x0000, sized_ea::<DATA_ALTERABLE>, immediate),
    row(0xFF00, 0x0200, sized_ea::<DATA_ALTERABLE>, immediate),
    row(0xFF00, 0x0400, sized_ea::<DATA_ALTERABLE>, immediate),
    row(0xFF00, 0x0600, sized_ea::<DATA_ALTERABLE>, immediate),
    row(0xFF00, 0x0A00, sized_ea::<DATA_ALTERABLE>, immediate),
    row(0xFF00, 0x0C00, sized_ea::<DATA_ALTERABLE>, immediate),
];

/// Line 1: `move.b`.
static LINE_1: &[Row] = &[row(0xF000, 0x1000, move_legal, move_)];

/// Line 2: `move.l` and `movea.l`.
static LINE_2: &[Row] = &[
    row(0xF1C0, 0x2040, ea::<ALL>, movea),
    row(0xF000, 0x2000, move_legal, move_),
];

/// Line 3: `move.w` and `movea.w`.
static LINE_3: &[Row] = &[
    row(0xF1C0, 0x3040, ea::<ALL>, movea),
    row(0xF000, 0x3000, move_legal, move_),
];

/// Line 4: everything else with one operand or none.
static LINE_4: &[Row] = &[
    row(0xFFFF, 0x4AFC, any, |_, _| Some(Instruction::ILLEGAL)),
    row(0xFFFF, 0x4E70, any, |_, _| Some(Instruction::RESET)),
    row(0xFFFF, 0x4E71, any, |_, _| Some(Instruction::NOP)),
    row(0xFFFF, 0x4E72, any, |words, _| {
        Some(Instruction::STOP(words.word()?))
    }),
    row(0xFFFF, 0x4E73, any, |_, _| Some(Instruction::RTE)),
    row(0xFFFF, 0x4E75, any, |_, _| Some(Instruction::RTS)),
    row(0xFFFF, 0x4E76, any, |_, _| Some(Instruction::TRAPV)),
    row(0xFFFF, 0x4E77, any, |_, _| Some(Instruction::RTR)),
    row(0xFFF0, 0x4E40, any, |_, word| {
        Some(Instruction::TRAP((word & 0xF) as u8))
    }),
    row(0xFFF8, 0x4E50, any, |words, word| {
        Some(Instruction::LINK(
            address_register(word),
            words.word()? as i16 as i32 as u32,
        ))
    }),
    row(0xFFF8, 0x4E58, any, |_, word| {
        Some(Instruction::UNLK(address_register(word)))
    }),
    row(0xFFF0, 0x4E60, any, |_, word| {
        Some(Instruction::MOVEUSP {
            to_usp: word & 0x0008 == 0,
            register: address_register(word),
        })
    }),
    row(0xFFC0, 0x4E80, ea::<CONTROL>, |words, word| {
        Some(Instruction::JSR(words.ea(word, Size::Long)?))
    }),
    row(0xFFC0, 0x4EC0, ea::<CONTROL>, |words, word| {
        Some(Instruction::JMP(words.ea(word, Size::Long)?))
    }),
    row(0xFFC0, 0x40C0, ea::<DATA_ALTERABLE>, |words, word| {
        Some(Instruction::MOVEfromSR(words.ea(word, Size::Word)?))
    }),
    row(0xFF00, 0x4000, sized_ea::<DATA_ALTERABLE>, |words, word| {
        let size = standard_size(word);
        Some(Instruction::NEGX(words.ea(word, size)?, size))
    }),
    gated(
        Processor::M68010,
        0xFFC0,
        0x42C0,
        ea::<DATA_ALTERABLE>,
        |words, word| Some(Instruction::MOVEfromCCR(words.ea(word, Size::Word)?)),
    ),
    row(0xFF00, 0x4200, sized_ea::<DATA_ALTERABLE>, |words, word| {
        let size = standard_size(word);
        Some(Instruction::CLR(words.ea(word, size)?, size))
    }),
    row(0xFFC0, 0x44C0, ea::<DATA>, |words, word| {
        Some(Instruction::MOVEtoCCR(words.ea(word, Size::Word)?))
    }),
    row(0xFF00, 0x4400, sized_ea::<DATA_ALTERABLE>, |words, word| {
        let size = standard_size(word);
        Some(Instruction::NEG(words.ea(word, size)?, size))
    }),
    row(0xFFC0, 0x46C0, ea::<DATA>, |words, word| {
        Some(Instruction::MOVEtoSR(words.ea(word, Size::Word)?))
    }),
    row(0xFF00, 0x4600, sized_ea::<DATA_ALTERABLE>, |words, word| {
        let size = standard_size(word);
        Some(Instruction::NOT(words.ea(word, size)?, size))
    }),
    row(0xFFF8, 0x4840, any, |_, word| {
        Some(Instruction::SWAP(data_register(word)))
    }),
    row(0xFFF8, 0x4880, any, |_, word| {
        Some(Instruction::EXT(
            data_register(word),
            Size::Byte,
            Size::Word,
        ))
    }),
    row(0xFFF8, 0x48C0, any, |_, word| {
        Some(Instruction::EXT(
            data_register(word),
            Size::Word,
            Size::Long,
        ))
    }),
    gated(Processor::M68020, 0xFFF8, 0x49C0, any, |_, word| {
        Some(Instruction::EXT(
            data_register(word),
            Size::Byte,
            Size::Long,
        ))
    }),
    row(0xFFC0, 0x4800, ea::<DATA_ALTERABLE>, |words, word| {
        Some(Instruction::NBCD(words.ea(word, Size::Byte)?))
    }),
    row(0xFFC0, 0x4840, ea::<CONTROL>, |words, word| {
        Some(Instruction::PEA(words.ea(word, Size::Long)?))
    }),
    row(0xFF80, 0x4880, ea::<MOVEM_TO_MEMORY>, movem),
    row(0xFF80, 0x4C80, ea::<MOVEM_FROM_MEMORY>, movem),
    row(0xFFC0, 0x4AC0, ea::<DATA_ALTERABLE>, |words, word| {
        Some(Instruction::TAS(words.ea(word, Size::Byte)?))
    }),
    row(0xFF00, 0x4A00, sized_ea::<DATA_ALTERABLE>, |words, word| {
        let size = standard_size(word);
        Some(Instruction::TST(words.ea(word, size)?, size))
    }),
    row(0xF1C0, 0x4180, ea::<DATA>, |words, word| {
        Some(Instruction::CHK(
            words.ea(word, Size::Word)?,
            data_register(word >> 9),
        ))
    }),
    row(0xF1C0, 0x41C0, ea::<CONTROL>, |words, word| {
        Some(Instruction::LEA(
            words.ea(word, Size::Long)?,
            address_register(word >> 9),
        ))
    }),
];

/// Line 5: `addq`, `subq`, `Scc` and `DBcc`.
static LINE_5: &[Row] = &[
    row(0xF0F8, 0x50C8, any, |words, word| {
        Some(Instruction::DBcc(
            data_register(word),
            words.displaced()?,
            Condition::from_code(word >> 8),
        ))
    }),
    row(0xF0C0, 0x50C0, ea::<DATA_ALTERABLE>, |words, word| {
        Some(Instruction::Scc(
            words.ea(word, Size::Byte)?,
            Condition::from_code(word >> 8),
        ))
    }),
    row(0xF100, 0x5000, sized_ea::<ALTERABLE>, quick),
    row(0xF100, 0x5100, sized_ea::<ALTERABLE>, quick),
];

/// Line 6: `bra`, `bsr` and `Bcc`.
static LINE_6: &[Row] = &[
    gated(Processor::M68020, 0xF0FF, 0x60FF, any, |words, word| {
        let at = words.next;
        let target = at.wrapping_add(words.long()?);
        Some(branch(word, target, BranchSize::Long))
    }),
    row(0xF000, 0x6000, any, |words, word| {
        let at = words.next;
        Some(match word as u8 {
            0 => branch(word, words.displaced()?, BranchSize::Word),
            // `$FF` included: on the 68000 it is −1.
            displacement => branch(
                word,
                at.wrapping_add(displacement as i8 as i32 as u32),
                BranchSize::Short,
            ),
        })
    }),
];

/// Line 7: `moveq`, whose bit 8 is zero.
static LINE_7: &[Row] = &[row(0xF100, 0x7000, any, |_, word| {
    Some(Instruction::MOVEQ(word as u8, data_register(word >> 9)))
})];

/// Line 8: `or`, `divu`, `divs` and `sbcd`.
static LINE_8: &[Row] = &[
    row(0xF1C0, 0x80C0, ea::<DATA>, |words, word| {
        divide_or_multiply(words, word, Instruction::DIVx, Sign::Unsigned)
    }),
    row(0xF1C0, 0x81C0, ea::<DATA>, |words, word| {
        divide_or_multiply(words, word, Instruction::DIVx, Sign::Signed)
    }),
    row(0xF1F0, 0x8100, any, |_, word| {
        let (source, destination) = extended_operands(word);
        Some(Instruction::SBCD(source, destination))
    }),
    row(0xF100, 0x8000, sized_ea::<DATA>, |words, word| {
        to_register(words, word, Instruction::OR)
    }),
    row(
        0xF100,
        0x8100,
        sized_ea::<MEMORY_ALTERABLE>,
        |words, word| to_memory(words, word, Instruction::OR),
    ),
];

/// Line 9: `sub`, `suba` and `subx`.
static LINE_9: &[Row] = &[
    row(0xF0C0, 0x90C0, ea::<ALL>, |words, word| {
        to_address_register(words, word, Instruction::SUBA)
    }),
    row(0xF130, 0x9100, sized, |_, word| {
        let (source, destination) = extended_operands(word);
        Some(Instruction::SUBX(source, destination, standard_size(word)))
    }),
    row(0xF100, 0x9000, sized_ea::<ALL>, |words, word| {
        to_register(words, word, Instruction::SUB)
    }),
    row(
        0xF100,
        0x9100,
        sized_ea::<MEMORY_ALTERABLE>,
        |words, word| to_memory(words, word, Instruction::SUB),
    ),
];

/// Line A: the line 1010 emulator, every word of it.
static LINE_A: &[Row] = &[row(0xF000, 0xA000, any, |_, word| {
    Some(Instruction::LINEA(word))
})];

/// Line B: `cmp`, `cmpa`, `cmpm` and `eor`.
static LINE_B: &[Row] = &[
    row(0xF0C0, 0xB0C0, ea::<ALL>, |words, word| {
        to_address_register(words, word, Instruction::CMPA)
    }),
    row(0xF138, 0xB108, sized, |_, word| {
        Some(Instruction::CMPM(
            Operand::PostIndirect((word & 7) as u8),
            Operand::PostIndirect(((word >> 9) & 7) as u8),
            standard_size(word),
        ))
    }),
    row(0xF100, 0xB000, sized_ea::<ALL>, |words, word| {
        let size = standard_size(word);
        Some(Instruction::CMP(
            words.ea(word, size)?,
            data_register(word >> 9),
            size,
        ))
    }),
    row(0xF100, 0xB100, sized_ea::<DATA_ALTERABLE>, |words, word| {
        to_memory(words, word, Instruction::EOR)
    }),
];

/// Line C: `and`, `mulu`, `muls`, `abcd` and `exg`.
static LINE_C: &[Row] = &[
    row(0xF1C0, 0xC0C0, ea::<DATA>, |words, word| {
        divide_or_multiply(words, word, Instruction::MULx, Sign::Unsigned)
    }),
    row(0xF1C0, 0xC1C0, ea::<DATA>, |words, word| {
        divide_or_multiply(words, word, Instruction::MULx, Sign::Signed)
    }),
    row(0xF1F0, 0xC100, any, |_, word| {
        let (source, destination) = extended_operands(word);
        Some(Instruction::ABCD(source, destination))
    }),
    // `exg Dx,Dy`, `exg Ax,Ay` and `exg Dx,Ay`; the data register of the third
    // is always the one in bits 11–9.
    row(0xF1F8, 0xC140, any, |_, word| {
        Some(Instruction::EXG(
            data_register(word >> 9),
            data_register(word),
        ))
    }),
    row(0xF1F8, 0xC148, any, |_, word| {
        Some(Instruction::EXG(
            address_register(word >> 9),
            address_register(word),
        ))
    }),
    row(0xF1F8, 0xC188, any, |_, word| {
        Some(Instruction::EXG(
            data_register(word >> 9),
            address_register(word),
        ))
    }),
    row(0xF100, 0xC000, sized_ea::<DATA>, |words, word| {
        to_register(words, word, Instruction::AND)
    }),
    row(
        0xF100,
        0xC100,
        sized_ea::<MEMORY_ALTERABLE>,
        |words, word| to_memory(words, word, Instruction::AND),
    ),
];

/// Line D: `add`, `adda` and `addx`.
static LINE_D: &[Row] = &[
    row(0xF0C0, 0xD0C0, ea::<ALL>, |words, word| {
        to_address_register(words, word, Instruction::ADDA)
    }),
    row(0xF130, 0xD100, sized, |_, word| {
        let (source, destination) = extended_operands(word);
        Some(Instruction::ADDX(source, destination, standard_size(word)))
    }),
    row(0xF100, 0xD000, sized_ea::<ALL>, |words, word| {
        to_register(words, word, Instruction::ADD)
    }),
    row(
        0xF100,
        0xD100,
        sized_ea::<MEMORY_ALTERABLE>,
        |words, word| to_memory(words, word, Instruction::ADD),
    ),
];

/// Line E: the shifts and rotates, in memory by one place and in a data
/// register by a count or by a register.
static LINE_E: &[Row] = &[
    row(0xF8C0, 0xE0C0, ea::<MEMORY_ALTERABLE>, |words, word| {
        let target = words.ea(word, Size::Word)?;
        Some(shift(
            word >> 9,
            word,
            Operand::Immediate(1),
            target,
            Size::Word,
        ))
    }),
    row(0xF000, 0xE000, sized, |_, word| {
        let field = (word >> 9) & 7;
        let count = if word & 0x0020 != 0 {
            Operand::Register(RegisterOperand::Data(field as u8))
        } else {
            Operand::Immediate(if field == 0 { 8 } else { field as u32 })
        };
        let target = Operand::Register(data_register(word));
        Some(shift(word >> 3, word, count, target, standard_size(word)))
    }),
];

/// Line F: the line 1111 emulator, and SIMHALT.
static LINE_F: &[Row] = &[row(0xF000, 0xF000, any, |words, word| {
    if word == 0xFFFF && (words.read)(words.next) == Some(0xFFFF) {
        words.next = words.next.wrapping_add(2);
        return Some(Instruction::SIMHALT);
    }
    Some(Instruction::LINEF(word))
})];

// ---------------------------------------------------------------------------
// What the rows share
// ---------------------------------------------------------------------------

/// The size in bits 7–6: `%00` a byte, `%01` a word, `%10` a long.
fn standard_size(word: u16) -> Size {
    match (word >> 6) & 3 {
        0 => Size::Byte,
        1 => Size::Word,
        // `%10`, and `%11`, which the rows refuse.
        _ => Size::Long,
    }
}

/// The data register in the low three bits of `field`.
fn data_register(field: u16) -> RegisterOperand {
    RegisterOperand::Data((field & 7) as u8)
}

/// The address register in the low three bits of `field`.
fn address_register(field: u16) -> RegisterOperand {
    RegisterOperand::Address((field & 7) as u8)
}

/// `ori`, `andi`, `subi`, `addi`, `eori` or `cmpi`, by bits 11–9: the
/// immediate's words come first, then the destination's.
fn immediate(words: &mut Words, word: u16) -> Option<Instruction> {
    let size = standard_size(word);
    let value = words.immediate(size)?;
    let destination = words.ea(word, size)?;
    Some(match (word >> 9) & 7 {
        0 => Instruction::ORI(value, destination, size),
        1 => Instruction::ANDI(value, destination, size),
        2 => Instruction::SUBI(value, destination, size),
        3 => Instruction::ADDI(value, destination, size),
        5 => Instruction::EORI(value, destination, size),
        // 6; the rows give this function no other.
        _ => Instruction::CMPI(value, destination, size),
    })
}

/// `ori #n,ccr`: the low byte of the word that follows.
fn ori_to_ccr(words: &mut Words, _: u16) -> Option<Instruction> {
    Some(Instruction::ORItoCCR(words.word()? as u8))
}

/// `ori #n,sr`.
fn ori_to_sr(words: &mut Words, _: u16) -> Option<Instruction> {
    Some(Instruction::ORItoSR(words.word()?))
}

/// `andi #n,ccr`.
fn andi_to_ccr(words: &mut Words, _: u16) -> Option<Instruction> {
    Some(Instruction::ANDItoCCR(words.word()? as u8))
}

/// `andi #n,sr`.
fn andi_to_sr(words: &mut Words, _: u16) -> Option<Instruction> {
    Some(Instruction::ANDItoSR(words.word()?))
}

/// `eori #n,ccr`.
fn eori_to_ccr(words: &mut Words, _: u16) -> Option<Instruction> {
    Some(Instruction::EORItoCCR(words.word()? as u8))
}

/// `eori #n,sr`.
fn eori_to_sr(words: &mut Words, _: u16) -> Option<Instruction> {
    Some(Instruction::EORItoSR(words.word()?))
}

/// `movep`: bit 7 says the bytes go to memory, bit 6 that they are a long.
fn movep(words: &mut Words, word: u16) -> Option<Instruction> {
    Some(Instruction::MOVEP {
        direction: if word & 0x0080 != 0 {
            TargetDirection::ToMemory
        } else {
            TargetDirection::FromMemory
        },
        size: if word & 0x0040 != 0 {
            Size::Long
        } else {
            Size::Word
        },
        register: data_register(word >> 9),
        target: Operand::IndirectDisplacement {
            offset: words.word()? as i16 as i32,
            base: address_register(word),
        },
    })
}

/// A bit instruction by bits 7–6: `btst`, `bchg`, `bclr`, `bset`.
fn bit_instruction(word: u16, bit: Operand, target: Operand) -> Instruction {
    match (word >> 6) & 3 {
        0 => Instruction::BTST(bit, target),
        1 => Instruction::BCHG(bit, target),
        2 => Instruction::BCLR(bit, target),
        _ => Instruction::BSET(bit, target),
    }
}

/// A bit instruction whose bit number is a data register, in bits 11–9.
fn bit_dynamic(words: &mut Words, word: u16) -> Option<Instruction> {
    let target = words.ea(word, Size::Byte)?;
    Some(bit_instruction(
        word,
        Operand::Register(data_register(word >> 9)),
        target,
    ))
}

/// A bit instruction whose bit number is the low byte of the word that
/// follows, before the target's own words.
fn bit_static(words: &mut Words, word: u16) -> Option<Instruction> {
    let bit = (words.word()? & 0xFF) as u32;
    let target = words.ea(word, Size::Byte)?;
    Some(bit_instruction(word, Operand::Immediate(bit), target))
}

/// The size of `move` and `movea`, in bits 13–12: `%01` a byte, `%11` a word,
/// `%10` a long.
fn move_size(word: u16) -> Size {
    match (word >> 12) & 3 {
        1 => Size::Byte,
        3 => Size::Word,
        _ => Size::Long,
    }
}

/// `move`: the source's words, then the destination's, whose field is
/// register first in bits 11–6.
fn move_(words: &mut Words, word: u16) -> Option<Instruction> {
    let size = move_size(word);
    let source = words.ea(word, size)?;
    let destination = words.ea(((word >> 3) & 0x38) | ((word >> 9) & 7), size)?;
    Some(Instruction::MOVE(source, destination, size))
}

/// `movea`.
fn movea(words: &mut Words, word: u16) -> Option<Instruction> {
    let size = move_size(word);
    Some(Instruction::MOVEA(
        words.ea(word, size)?,
        address_register(word >> 9),
        size,
    ))
}

/// `movem`: bit 10 says the registers come from memory, bit 6 that they are
/// longs. The mask is the word after the operation word, before the
/// effective address's own, and is kept as it is encoded, reversed for a
/// predecrement.
fn movem(words: &mut Words, word: u16) -> Option<Instruction> {
    let size = if word & 0x0040 != 0 {
        Size::Long
    } else {
        Size::Word
    };
    let registers_mask = words.word()?;
    let target = words.ea(word, size)?;
    Some(Instruction::MOVEM {
        direction: if word & 0x0400 != 0 {
            TargetDirection::FromMemory
        } else {
            TargetDirection::ToMemory
        },
        size,
        registers_mask,
        target,
    })
}

/// `addq` or `subq` by bit 8; a count of 0 in bits 11–9 means 8.
fn quick(words: &mut Words, word: u16) -> Option<Instruction> {
    let size = standard_size(word);
    let count = match ((word >> 9) & 7) as u8 {
        0 => 8,
        count => count,
    };
    let target = words.ea(word, size)?;
    Some(match word & 0x0100 {
        0 => Instruction::ADDQ(count, target, size),
        _ => Instruction::SUBQ(count, target, size),
    })
}

/// `bra`, `bsr` or a `Bcc`, by the condition in bits 11–8, where `t` is `bra`
/// and `f` is `bsr`.
fn branch(word: u16, target: u32, size: BranchSize) -> Instruction {
    match (word >> 8) & 0xF {
        0 => Instruction::BRA(target, size),
        1 => Instruction::BSR(target, size),
        code => Instruction::Bcc(target, Condition::from_code(code), size),
    }
}

/// `divu`, `divs`, `mulu` or `muls`: a word source and a data register.
fn divide_or_multiply(
    words: &mut Words,
    word: u16,
    variant: fn(Operand, RegisterOperand, Sign) -> Instruction,
    sign: Sign,
) -> Option<Instruction> {
    Some(variant(
        words.ea(word, Size::Word)?,
        data_register(word >> 9),
        sign,
    ))
}

/// The `<ea>,Dn` half of `or`, `and`, `sub` and `add`.
fn to_register(
    words: &mut Words,
    word: u16,
    variant: fn(Operand, Operand, Size) -> Instruction,
) -> Option<Instruction> {
    let size = standard_size(word);
    let source = words.ea(word, size)?;
    Some(variant(
        source,
        Operand::Register(data_register(word >> 9)),
        size,
    ))
}

/// The `Dn,<ea>` half of `or`, `and`, `sub` and `add`, and `eor`.
fn to_memory(
    words: &mut Words,
    word: u16,
    variant: fn(Operand, Operand, Size) -> Instruction,
) -> Option<Instruction> {
    let size = standard_size(word);
    let destination = words.ea(word, size)?;
    Some(variant(
        Operand::Register(data_register(word >> 9)),
        destination,
        size,
    ))
}

/// `suba`, `cmpa` or `adda`: bit 8 says a long, and the source is read at
/// that size.
fn to_address_register(
    words: &mut Words,
    word: u16,
    variant: fn(Operand, RegisterOperand, Size) -> Instruction,
) -> Option<Instruction> {
    let size = if word & 0x0100 != 0 {
        Size::Long
    } else {
        Size::Word
    };
    Some(variant(
        words.ea(word, size)?,
        address_register(word >> 9),
        size,
    ))
}

/// The two operands of `abcd`, `sbcd`, `addx` and `subx`: `Dy,Dx`, or
/// `-(Ay),-(Ax)` when bit 3 is set, with y in bits 2–0 and x in bits 11–9.
fn extended_operands(word: u16) -> (Operand, Operand) {
    let (x, y) = (((word >> 9) & 7) as u8, (word & 7) as u8);
    if word & 0x0008 != 0 {
        (Operand::PreIndirect(y), Operand::PreIndirect(x))
    } else {
        (
            Operand::Register(RegisterOperand::Data(y)),
            Operand::Register(RegisterOperand::Data(x)),
        )
    }
}

/// A shift or rotate: the kind in the low two bits of `kind` (`%00`
/// arithmetic, `%01` logical, `%10` through the extend flag, `%11` rotate) and
/// the direction in bit 8 of `word`.
fn shift(kind: u16, word: u16, count: Operand, target: Operand, size: Size) -> Instruction {
    let direction = if word & 0x0100 != 0 {
        ShiftDirection::Left
    } else {
        ShiftDirection::Right
    };
    match kind & 3 {
        0 => Instruction::ASd(count, target, direction, size),
        1 => Instruction::LSd(count, target, direction, size),
        2 => Instruction::ROXd(count, target, direction, size),
        _ => Instruction::ROd(count, target, direction, size),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::machine::disassemble::disassemble;

    /// One hand-assembled instruction: the GNU `as` line that writes the same
    /// words, when it has one, the words, worked out from the M68000
    /// Programmer's Reference Manual's encoding of the instruction, and what
    /// they disassemble to at `$1000`.
    pub(crate) struct Case {
        pub(crate) gnu: Option<&'static str>,
        pub(crate) words: &'static [u16],
        pub(crate) text: &'static str,
    }

    const fn case(gnu: &'static str, words: &'static [u16], text: &'static str) -> Case {
        Case {
            gnu: Some(gnu),
            words,
            text,
        }
    }

    /// A case GNU `as` cannot write: it chooses another encoding of the same
    /// line (`cmp #5,d0` is `cmpi` there), or the words are no instruction it
    /// assembles.
    const fn words_only(words: &'static [u16], text: &'static str) -> Case {
        Case {
            gnu: None,
            words,
            text,
        }
    }

    /// Every family of the 68000's opcode map, line by line, with each
    /// addressing mode at least once and every rule the decoder has to know:
    /// a byte immediate in a word, a `movem` mask reversed for `-(An)`, a
    /// quick count of 0 meaning 8, the three shift forms, the extension words
    /// in order (an immediate's before the destination's, a mask or a bit
    /// number before the effective address's), and a PC-relative
    /// displacement measured from its own extension word.
    pub(crate) const CASES: &[Case] = &[
        // Line 0: the immediates, the status register immediates, the bit
        // instructions and movep.
        case("ori.b #0x12,d0", &[0x0000, 0x0012], "ORI.B #$12,D0"),
        case("ori.w #0x1234,(a1)", &[0x0051, 0x1234], "ORI.W #$1234,(A1)"),
        case(
            "ori.l #0x12345678,-(a2)",
            &[0x00A2, 0x1234, 0x5678],
            "ORI.L #$12345678,-(A2)",
        ),
        case("andi.b #0xf0,(a3)+", &[0x021B, 0x00F0], "ANDI.B #$F0,(A3)+"),
        case(
            "subi.w #1,(4,a4)",
            &[0x046C, 0x0001, 0x0004],
            "SUBI.W #$1,4(A4)",
        ),
        case(
            "addi.l #0x10000,d7",
            &[0x0687, 0x0001, 0x0000],
            "ADDI.L #$10000,D7",
        ),
        case(
            "eori.w #0xffff,(0x2000).w",
            &[0x0A78, 0xFFFF, 0x2000],
            "EORI.W #$FFFF,$2000.W",
        ),
        case(
            "cmpi.l #5,(0x12345678).l",
            &[0x0CB9, 0x0000, 0x0005, 0x1234, 0x5678],
            "CMPI.L #$5,$12345678",
        ),
        case("ori #0x1f,ccr", &[0x003C, 0x001F], "ORI #$1F,CCR"),
        case("ori #0x700,sr", &[0x007C, 0x0700], "ORI #$700,SR"),
        case("andi #0xfe,ccr", &[0x023C, 0x00FE], "ANDI #$FE,CCR"),
        case("andi #0xf8ff,sr", &[0x027C, 0xF8FF], "ANDI #$F8FF,SR"),
        case("eori #4,ccr", &[0x0A3C, 0x0004], "EORI #$4,CCR"),
        case("eori #0x2000,sr", &[0x0A7C, 0x2000], "EORI #$2000,SR"),
        case("btst d1,d0", &[0x0300], "BTST D1,D0"),
        case("bchg d2,(a0)", &[0x0550], "BCHG D2,(A0)"),
        case("bclr d3,-(a1)", &[0x07A1], "BCLR D3,-(A1)"),
        case("bset d4,(8,a2)", &[0x09EA, 0x0008], "BSET D4,8(A2)"),
        case("btst d5,#0x12", &[0x0B3C, 0x0012], "BTST D5,#$12"),
        case("btst #3,d0", &[0x0800, 0x0003], "BTST #3,D0"),
        case("bset #7,(a0)", &[0x08D0, 0x0007], "BSET #7,(A0)"),
        // The bit number is the first extension word, so the displacement is
        // the second, at `$1004`.
        case(
            "btst #1,(6,pc)",
            &[0x083A, 0x0001, 0x0006],
            "BTST #1,$100A(PC)",
        ),
        case("bchg #31,d7", &[0x0847, 0x001F], "BCHG #31,D7"),
        case(
            "bclr #0,(0x4000).l",
            &[0x08B9, 0x0000, 0x0000, 0x4000],
            "BCLR #0,$4000",
        ),
        case("movep.w (4,a1),d0", &[0x0109, 0x0004], "MOVEP.W 4(A1),D0"),
        case("movep.l (-2,a0),d3", &[0x0748, 0xFFFE], "MOVEP.L -2(A0),D3"),
        case("movep.w d2,(0,a3)", &[0x058B, 0x0000], "MOVEP.W D2,0(A3)"),
        case("movep.l d7,(10,a7)", &[0x0FCF, 0x000A], "MOVEP.L D7,10(A7)"),
        // Lines 1 to 3: move and movea, whose sizes are 01 byte, 11 word and
        // 10 long, and whose destination field is register first.
        case("move.b d1,d0", &[0x1001], "MOVE.B D1,D0"),
        case("move.l d0,(a1)+", &[0x22C0], "MOVE.L D0,(A1)+"),
        case("move.w (a0),-(a7)", &[0x3F10], "MOVE.W (A0),-(A7)"),
        case(
            "move.l #0x12345678,(0x12345678).l",
            &[0x23FC, 0x1234, 0x5678, 0x1234, 0x5678],
            "MOVE.L #$12345678,$12345678",
        ),
        case(
            "move.w (8,a0,d1.w),(-0x8000).w",
            &[0x31F0, 0x1008, 0x8000],
            "MOVE.W 8(A0,D1.W),$8000.W",
        ),
        case(
            "move.b (-1,a2,a3.l),d4",
            &[0x1832, 0xB8FF],
            "MOVE.B -1(A2,A3.L),D4",
        ),
        case(
            "move.l (0x10,pc),d0",
            &[0x203A, 0x0010],
            "MOVE.L $1012(PC),D0",
        ),
        case(
            "move.w (-2,pc,d0.w),d1",
            &[0x323B, 0x00FE],
            "MOVE.W $1000(PC,D0.W),D1",
        ),
        case("move.l a0,d0", &[0x2008], "MOVE.L A0,D0"),
        case("move.b #0x7f,d0", &[0x103C, 0x007F], "MOVE.B #$7F,D0"),
        case("movea.l a0,a1", &[0x2248], "MOVEA.L A0,A1"),
        case("movea.w #0x1234,a2", &[0x347C, 0x1234], "MOVEA.W #$1234,A2"),
        // Line 4.
        case("negx.b d0", &[0x4000], "NEGX.B D0"),
        case("negx.l (a0)", &[0x4090], "NEGX.L (A0)"),
        case("move sr,d0", &[0x40C0], "MOVE SR,D0"),
        case("clr.w (a5)+", &[0x425D], "CLR.W (A5)+"),
        case("move d0,ccr", &[0x44C0], "MOVE D0,CCR"),
        case("move #0x1f,ccr", &[0x44FC, 0x001F], "MOVE #$1F,CCR"),
        case("neg.l d3", &[0x4483], "NEG.L D3"),
        case("move (a0),sr", &[0x46D0], "MOVE (A0),SR"),
        case("move #0x2700,sr", &[0x46FC, 0x2700], "MOVE #$2700,SR"),
        case("not.b -(a0)", &[0x4620], "NOT.B -(A0)"),
        case("nbcd d1", &[0x4801], "NBCD D1"),
        case("swap d2", &[0x4842], "SWAP D2"),
        case("pea (4,a6)", &[0x486E, 0x0004], "PEA 4(A6)"),
        case("ext.w d3", &[0x4883], "EXT.W D3"),
        case("ext.l d4", &[0x48C4], "EXT.L D4"),
        // The mask of a predecrement is written reversed: D0 is bit 15.
        case(
            "movem.l d0-d2/a0,-(a7)",
            &[0x48E7, 0xE080],
            "MOVEM.L D0-D2/A0,-(A7)",
        ),
        case(
            "movem.l (a7)+,d0-d2/a0",
            &[0x4CDF, 0x0107],
            "MOVEM.L (A7)+,D0-D2/A0",
        ),
        case(
            "movem.w d3/a5-a6,(a0)",
            &[0x4890, 0x6008],
            "MOVEM.W D3/A5-A6,(A0)",
        ),
        // The mask comes first, so the displacement is measured from `$1004`.
        case(
            "movem.w (0x10,pc),d0/a0",
            &[0x4CBA, 0x0101, 0x0010],
            "MOVEM.W $1014(PC),D0/A0",
        ),
        case("tst.b d0", &[0x4A00], "TST.B D0"),
        case("tst.l (a0)+", &[0x4A98], "TST.L (A0)+"),
        case("tas (a1)", &[0x4AD1], "TAS (A1)"),
        case("illegal", &[0x4AFC], "ILLEGAL"),
        case("trap #0", &[0x4E40], "TRAP #0"),
        case("trap #15", &[0x4E4F], "TRAP #15"),
        case("link a6,#-4", &[0x4E56, 0xFFFC], "LINK A6,#-4"),
        case("unlk a6", &[0x4E5E], "UNLK A6"),
        case("move a1,usp", &[0x4E61], "MOVE A1,USP"),
        case("move usp,a2", &[0x4E6A], "MOVE USP,A2"),
        case("reset", &[0x4E70], "RESET"),
        case("nop", &[0x4E71], "NOP"),
        case("stop #0x2700", &[0x4E72, 0x2700], "STOP #$2700"),
        case("rte", &[0x4E73], "RTE"),
        case("rts", &[0x4E75], "RTS"),
        case("trapv", &[0x4E76], "TRAPV"),
        case("rtr", &[0x4E77], "RTR"),
        case("jsr (0x2000).w", &[0x4EB8, 0x2000], "JSR $2000.W"),
        case("jmp (a0)", &[0x4ED0], "JMP (A0)"),
        case(
            "jmp (0x10,pc,d1.l)",
            &[0x4EFB, 0x1810],
            "JMP $1012(PC,D1.L)",
        ),
        case("chk.w (a0),d1", &[0x4390], "CHK.W (A0),D1"),
        case("chk.w #100,d0", &[0x41BC, 0x0064], "CHK.W #$64,D0"),
        case(
            "lea (0x1000).l,a0",
            &[0x41F9, 0x0000, 0x1000],
            "LEA $1000,A0",
        ),
        case("lea (2,pc),a1", &[0x43FA, 0x0002], "LEA $1004(PC),A1"),
        // Line 5: a quick count of 0 is 8; Scc; DBcc, whose displacement is
        // measured from its own word.
        case("addq.l #1,d0", &[0x5280], "ADDQ.L #1,D0"),
        case("addq.w #8,a0", &[0x5048], "ADDQ.W #8,A0"),
        case("subq.b #3,(a1)", &[0x5711], "SUBQ.B #3,(A1)"),
        case("st d0", &[0x50C0], "ST D0"),
        case("seq (a0)", &[0x57D0], "SEQ (A0)"),
        case("shi -(a1)", &[0x52E1], "SHI -(A1)"),
        case("dbra d0,.", &[0x51C8, 0xFFFE], "DBF D0,$1000"),
        case("dbne d7,.+10", &[0x56CF, 0x0008], "DBNE D7,$100A"),
        // Line 6: a short displacement in the operation word, or a word one
        // after a `$00`, both from the address after the operation word.
        case("bra.s .+4", &[0x6002], "BRA.S $1004"),
        case("bsr.w .+0x100", &[0x6100, 0x00FE], "BSR.W $1100"),
        case("beq.s .-6", &[0x67F8], "BEQ.S $FFA"),
        case("bne.w .", &[0x6600, 0xFFFE], "BNE.W $1000"),
        case("bcc.s .+0x81", &[0x647F], "BCC.S $1081"),
        // Line 7.
        case("moveq #-1,d3", &[0x76FF], "MOVEQ #-1,D3"),
        case("moveq #127,d0", &[0x707F], "MOVEQ #127,D0"),
        // Line 8.
        case("or.b d1,d0", &[0x8001], "OR.B D1,D0"),
        case("or.l d3,(a0)", &[0x8790], "OR.L D3,(A0)"),
        words_only(&[0x847C, 0x00FF], "OR.W #$FF,D2"),
        case("divu.w d1,d0", &[0x80C1], "DIVU.W D1,D0"),
        case("divs.w #3,d2", &[0x85FC, 0x0003], "DIVS.W #$3,D2"),
        case("sbcd d1,d0", &[0x8101], "SBCD D1,D0"),
        case("sbcd -(a1),-(a0)", &[0x8109], "SBCD -(A1),-(A0)"),
        // Line 9.
        case("sub.w (a0),d1", &[0x9250], "SUB.W (A0),D1"),
        case("sub.l a0,d1", &[0x9288], "SUB.L A0,D1"),
        case("sub.b d2,(a3)", &[0x9513], "SUB.B D2,(A3)"),
        case("suba.w d0,a1", &[0x92C0], "SUBA.W D0,A1"),
        case(
            "suba.l #0x10,a7",
            &[0x9FFC, 0x0000, 0x0010],
            "SUBA.L #$10,A7",
        ),
        case("subx.l d1,d2", &[0x9581], "SUBX.L D1,D2"),
        case("subx.b -(a0),-(a1)", &[0x9308], "SUBX.B -(A0),-(A1)"),
        // Line A.
        words_only(&[0xA000], "DC.W $A000"),
        words_only(&[0xAFFF], "DC.W $AFFF"),
        // Line B.
        case("cmp.w d0,d1", &[0xB240], "CMP.W D0,D1"),
        case("cmp.l a0,d1", &[0xB288], "CMP.L A0,D1"),
        words_only(&[0xB07C, 0x0005], "CMP.W #$5,D0"),
        case("cmpa.l d0,a0", &[0xB1C0], "CMPA.L D0,A0"),
        case("cmpa.w (a1),a2", &[0xB4D1], "CMPA.W (A1),A2"),
        case("cmpm.b (a0)+,(a1)+", &[0xB308], "CMPM.B (A0)+,(A1)+"),
        case("eor.w d0,d1", &[0xB141], "EOR.W D0,D1"),
        case("eor.l d2,-(a0)", &[0xB5A0], "EOR.L D2,-(A0)"),
        // Line C.
        case("and.l (a0)+,d0", &[0xC098], "AND.L (A0)+,D0"),
        case("and.b d1,(a2)", &[0xC312], "AND.B D1,(A2)"),
        words_only(&[0xC07C, 0x000F], "AND.W #$F,D0"),
        case("mulu.w d1,d0", &[0xC0C1], "MULU.W D1,D0"),
        case("muls.w #-1,d2", &[0xC5FC, 0xFFFF], "MULS.W #$FFFF,D2"),
        case("abcd d1,d0", &[0xC101], "ABCD D1,D0"),
        case("abcd -(a1),-(a0)", &[0xC109], "ABCD -(A1),-(A0)"),
        case("exg d0,d1", &[0xC141], "EXG D0,D1"),
        case("exg a0,a1", &[0xC149], "EXG A0,A1"),
        case("exg d0,a1", &[0xC189], "EXG D0,A1"),
        // Line D.
        case("add.b d1,d0", &[0xD001], "ADD.B D1,D0"),
        case("add.w d0,(a0)", &[0xD150], "ADD.W D0,(A0)"),
        words_only(&[0xD07C, 0x0005], "ADD.W #$5,D0"),
        case(
            "adda.l #0x100,a0",
            &[0xD1FC, 0x0000, 0x0100],
            "ADDA.L #$100,A0",
        ),
        case("adda.w a1,a0", &[0xD0C9], "ADDA.W A1,A0"),
        case("addx.w d1,d2", &[0xD541], "ADDX.W D1,D2"),
        case("addx.l -(a1),-(a2)", &[0xD589], "ADDX.L -(A1),-(A2)"),
        // Line E: a count of 0 is 8, a count from a register, and memory.
        case("asl.w #1,d0", &[0xE340], "ASL.W #1,D0"),
        case("asr.l #8,d1", &[0xE081], "ASR.L #8,D1"),
        case("lsl.b d2,d3", &[0xE52B], "LSL.B D2,D3"),
        case("lsr.w #4,d4", &[0xE84C], "LSR.W #4,D4"),
        case("roxl.l #2,d5", &[0xE595], "ROXL.L #2,D5"),
        case("roxr.w d6,d7", &[0xEC77], "ROXR.W D6,D7"),
        case("rol.b #3,d0", &[0xE718], "ROL.B #3,D0"),
        case("ror.l d1,d2", &[0xE2BA], "ROR.L D1,D2"),
        case("asl.w (a0)", &[0xE1D0], "ASL.W (A0)"),
        case("lsr.w (4,a1)", &[0xE2E9, 0x0004], "LSR.W 4(A1)"),
        case("roxl.w (a2)+", &[0xE5DA], "ROXL.W (A2)+"),
        case("ror.w -(a3)", &[0xE6E3], "ROR.W -(A3)"),
        // Line F, and SIMHALT before it.
        words_only(&[0xFFFF, 0xFFFF], "SIMHALT"),
        words_only(&[0xF000], "DC.W $F000"),
        // Words that begin no 68000 instruction.
        words_only(&[0x4AFB], "DC.W $4AFB"),
        words_only(&[0x7100], "DC.W $7100"),
        words_only(&[0x00FC], "DC.W $00FC"),
        words_only(&[0x1048], "DC.W $1048"),
        words_only(&[0x4E74], "DC.W $4E74"),
        words_only(&[0x42C0], "DC.W $42C0"),
        words_only(&[0x49C0], "DC.W $49C0"),
    ];

    #[test]
    fn every_family_decodes_to_the_reference_manuals_encoding() {
        for case in CASES {
            let decoded = decode_words(case.words, 0x1000, Processor::M68000)
                .unwrap_or_else(|error| panic!("{:04X?} ran out: {error:?}", case.words));
            assert_eq!(
                disassemble(&decoded.instruction, None),
                case.text,
                "{:04X?}",
                case.words
            );
            let expected = match decoded.instruction {
                // An illegal word, line A or line F is the one word, whatever
                // follows it.
                Instruction::IllegalWord(_) | Instruction::LINEA(_) | Instruction::LINEF(_) => 2,
                _ => case.words.len() as u32 * 2,
            };
            assert_eq!(decoded.size, expected, "{:04X?}", case.words);
            assert!(decoded.size <= MAX_68000_SIZE);
        }
    }

    #[test]
    fn the_alternative_encodings_of_one_operation_decode_apart() {
        let decoded = |words: &[u16]| {
            decode_words(words, 0x1000, Processor::M68000)
                .unwrap()
                .instruction
        };
        let d0 = || Operand::Register(RegisterOperand::Data(0));
        // `add.w #5,d0` in its three encodings.
        assert_eq!(decoded(&[0x5A40]), Instruction::ADDQ(5, d0(), Size::Word));
        assert_eq!(
            decoded(&[0x0640, 0x0005]),
            Instruction::ADDI(5, d0(), Size::Word)
        );
        assert_eq!(
            decoded(&[0xD07C, 0x0005]),
            Instruction::ADD(Operand::Immediate(5), d0(), Size::Word)
        );
        // `move.l #5,d0` in its two.
        assert_eq!(
            decoded(&[0x7005]),
            Instruction::MOVEQ(5, RegisterOperand::Data(0))
        );
        assert_eq!(
            decoded(&[0x203C, 0x0000, 0x0005]),
            Instruction::MOVE(Operand::Immediate(5), d0(), Size::Long)
        );
        // `cmp.w #5,d0`, `and.w #5,d0`.
        assert_eq!(
            decoded(&[0xB07C, 0x0005]),
            Instruction::CMP(Operand::Immediate(5), RegisterOperand::Data(0), Size::Word)
        );
        assert_eq!(
            decoded(&[0x0C40, 0x0005]),
            Instruction::CMPI(5, d0(), Size::Word)
        );
        assert_eq!(
            decoded(&[0xC07C, 0x0005]),
            Instruction::AND(Operand::Immediate(5), d0(), Size::Word)
        );
        assert_eq!(
            decoded(&[0x0240, 0x0005]),
            Instruction::ANDI(5, d0(), Size::Word)
        );
        // A shift by one in memory and in a register.
        assert_eq!(
            decoded(&[0xE1D0]),
            Instruction::ASd(
                Operand::Immediate(1),
                Operand::Indirect(0),
                ShiftDirection::Left,
                Size::Word
            )
        );
        assert_eq!(
            decoded(&[0xE340]),
            Instruction::ASd(
                Operand::Immediate(1),
                d0(),
                ShiftDirection::Left,
                Size::Word
            )
        );
        // A short and a word branch to the same place, and an absolute short
        // and long address of the same place.
        assert_eq!(
            decoded(&[0x6002]),
            Instruction::BRA(0x1004, BranchSize::Short)
        );
        assert_eq!(
            decoded(&[0x6000, 0x0002]),
            Instruction::BRA(0x1004, BranchSize::Word)
        );
        assert_eq!(
            decoded(&[0x4EF8, 0x1000]),
            Instruction::JMP(Operand::Absolute {
                address: 0x1000,
                width: AddressWidth::Word
            })
        );
        assert_eq!(
            decoded(&[0x4EF9, 0x0000, 0x1000]),
            Instruction::JMP(Operand::Absolute {
                address: 0x1000,
                width: AddressWidth::Long
            })
        );
        // An absolute short address is sign extended.
        assert_eq!(
            decoded(&[0x4EF8, 0x8000]),
            Instruction::JMP(Operand::Absolute {
                address: 0xFFFF_8000,
                width: AddressWidth::Word
            })
        );
    }

    /// Every bit of a 68000 operation word means something, so no two first
    /// words decode to the same instruction when the extension words are the
    /// same: each encoding has a value of its own.
    #[test]
    fn no_two_first_words_decode_to_the_same_instruction() {
        let mut seen = std::collections::HashMap::new();
        for word in 0..=0xFFFFu16 {
            let words = [word, 0x2010, 0x3020, 0x4030, 0x5040];
            let decoded = decode_words(&words, 0x1000, Processor::M68000).unwrap();
            let key = format!("{:?}", decoded.instruction);
            if let Some(other) = seen.insert(key.clone(), word) {
                panic!("{other:04X} and {word:04X} both decode to {key}");
            }
        }
    }

    /// The 68000 ignores bits 10–8 of a brief index word and the high byte of
    /// a byte immediate; a word that sets them is the instruction it would be
    /// with them clear.
    #[test]
    fn the_bits_the_68000_ignores_decode_as_if_clear() {
        let decoded = |words: &[u16]| {
            decode_words(words, 0x1000, Processor::M68000)
                .unwrap()
                .instruction
        };
        // `move.b 8(a0,d1.w),d0` with bits 10–8 set: the MAME-derived suite
        // sets them in most of its indexed tests.
        assert_eq!(decoded(&[0x1030, 0x1708]), decoded(&[0x1030, 0x1008]));
        assert_eq!(decoded(&[0x41FB, 0x0102]), decoded(&[0x41FB, 0x0002]));
        // A byte immediate, as GNU `as` writes `move.b #-1,d0`.
        assert_eq!(decoded(&[0x103C, 0xFFFF]), decoded(&[0x103C, 0x00FF]));
        assert_eq!(decoded(&[0x0000, 0xFF12]), decoded(&[0x0000, 0x0012]));
        // `andi #n,ccr` and a static bit number take the low byte too.
        assert_eq!(decoded(&[0x023C, 0xFF1F]), Instruction::ANDItoCCR(0x1F));
        assert_eq!(decoded(&[0x0800, 0x0103]), decoded(&[0x0800, 0x0003]));
    }

    /// A branch target and a PC-relative operand are worked out from where
    /// their words are.
    #[test]
    fn the_same_words_reach_different_targets_at_different_addresses() {
        let at = |address: u32, words: &[u16]| {
            decode_words(words, address, Processor::M68000)
                .unwrap()
                .instruction
        };
        assert_eq!(
            at(0x2000, &[0x67F8]),
            Instruction::Bcc(0x1FFA, Condition::Equal, BranchSize::Short)
        );
        assert_eq!(
            at(0x2000, &[0x203A, 0x0010]),
            Instruction::MOVE(
                Operand::PcDisplacement { target: 0x2012 },
                Operand::Register(RegisterOperand::Data(0)),
                Size::Long
            )
        );
        // A displacement that wraps below address 0 wraps at 32 bits.
        assert_eq!(
            at(0, &[0x60FE]),
            Instruction::BRA(0, BranchSize::Short),
            "`bra.s *` is a branch to itself"
        );
        assert_eq!(
            at(0, &[0x6000, 0xFFF0]),
            Instruction::BRA(0xFFFF_FFF2, BranchSize::Word)
        );
    }

    #[test]
    fn simhalt_is_two_ffff_words_and_one_is_line_f() {
        let simhalt = decode_words(&[0xFFFF, 0xFFFF], 0x1000, Processor::M68000).unwrap();
        assert_eq!(simhalt.instruction, Instruction::SIMHALT);
        assert_eq!(simhalt.size, 4);
        for processor in Processor::ALL {
            let next_is_not = decode_words(&[0xFFFF, 0x4E71], 0x1000, processor).unwrap();
            assert_eq!(next_is_not.instruction, Instruction::LINEF(0xFFFF));
            assert_eq!(next_is_not.size, 2);
            assert_eq!(
                decode_words(&[0xFFFF, 0xFFFF], 0x1000, processor)
                    .unwrap()
                    .instruction,
                Instruction::SIMHALT,
                "SIMHALT is EASy68K's, for every processor"
            );
        }
        // Nothing after the `$FFFF` is no second word: line F, not an error.
        let alone = decode_words(&[0xFFFF], 0x1000, Processor::M68000).unwrap();
        assert_eq!(alone.instruction, Instruction::LINEF(0xFFFF));
        assert_eq!(
            decode_words(&[0xA123], 0x1000, Processor::M68000)
                .unwrap()
                .instruction,
            Instruction::LINEA(0xA123)
        );
    }

    #[test]
    fn running_out_of_words_says_where() {
        assert_eq!(
            decode_words(&[0x203C, 0x1234], 0x1000, Processor::M68000),
            Err(Truncated { address: 0x1004 })
        );
        assert_eq!(
            decode_words(&[], 0x1000, Processor::M68000),
            Err(Truncated { address: 0x1000 })
        );
        // `decode` hands back the reader's own error.
        let failed: Result<Decoded, &str> =
            decode(0x1000, Processor::M68000, |address| match address {
                0x1000 => Ok(0x4E72),
                _ => Err("bus error"),
            });
        assert_eq!(failed, Err("bus error"));
    }

    /// The two first members of the dialects, and the branch whose `$FF`
    /// byte the 68020 reads as a long displacement.
    #[test]
    fn a_word_a_later_processor_adds_is_illegal_before_it() {
        let decoded =
            |processor, words: &[u16]| decode_words(words, 0x1000, processor).unwrap().instruction;
        // `move ccr,d0`, the 68010's.
        assert_eq!(
            decoded(Processor::M68000, &[0x42C0]),
            Instruction::IllegalWord(0x42C0)
        );
        for processor in [Processor::M68010, Processor::M68020] {
            assert_eq!(
                decoded(processor, &[0x42C0]),
                Instruction::MOVEfromCCR(Operand::Register(RegisterOperand::Data(0)))
            );
            assert_eq!(
                decoded(processor, &[0x42F9, 0x0000, 0x2000]),
                Instruction::MOVEfromCCR(Operand::Absolute {
                    address: 0x2000,
                    width: AddressWidth::Long
                })
            );
            // Not alterable, so not `move ccr` either: `clr` has no size `%11`.
            assert_eq!(
                decoded(processor, &[0x42FA]),
                Instruction::IllegalWord(0x42FA)
            );
        }
        // `extb.l d0` to `d7`, the 68020's; on the 68000 and the 68010 it would
        // be `lea d0,a4`, which has no meaning.
        for register in 0..8u16 {
            let word = 0x49C0 | register;
            assert_eq!(
                decoded(Processor::M68000, &[word]),
                Instruction::IllegalWord(word)
            );
            assert_eq!(
                decoded(Processor::M68010, &[word]),
                Instruction::IllegalWord(word)
            );
            assert_eq!(
                decoded(Processor::M68020, &[word]),
                Instruction::EXT(
                    RegisterOperand::Data(register as u8),
                    Size::Byte,
                    Size::Long
                )
            );
        }
        // A `$FF` displacement byte is −1 before the 68020, and a long
        // displacement on it.
        for processor in [Processor::M68000, Processor::M68010] {
            assert_eq!(
                decoded(processor, &[0x60FF]),
                Instruction::BRA(0x1001, BranchSize::Short)
            );
        }
        let long = decode_words(&[0x67FF, 0x0001, 0x0000], 0x1000, Processor::M68020).unwrap();
        assert_eq!(
            long.instruction,
            Instruction::Bcc(0x0001_1002, Condition::Equal, BranchSize::Long)
        );
        assert_eq!(long.size, 6);

        assert_eq!(first_processor(0x4E71), Some(Processor::M68000));
        assert_eq!(first_processor(0x42C0), Some(Processor::M68010));
        assert_eq!(first_processor(0x49C7), Some(Processor::M68020));
        assert_eq!(first_processor(0x60FF), Some(Processor::M68000));
        // `rtd` is the 68010's, but not decoded yet.
        assert_eq!(first_processor(0x4E74), None);
        assert_eq!(first_processor(0x4AFB), None);
    }

    /// Musashi's `m68k_is_valid_instruction` for the 68000, one bit a word,
    /// as `tests/musashi/NOTICE` says it was made.
    fn musashi_68000() -> Vec<bool> {
        let text = include_str!("../../tests/musashi/m68000-valid.hex");
        let bits: Vec<bool> = text
            .lines()
            .flat_map(|line| line.chars())
            .flat_map(|digit| {
                let value = digit.to_digit(16).expect("a hex digit");
                (0..4).rev().map(move |bit| (value >> bit) & 1 == 1)
            })
            .collect();
        assert_eq!(bits.len(), 0x10000, "one bit for each of the 65,536 words");
        bits
    }

    /// The decoder and Musashi agree on which of the 65,536 first words are
    /// 68000 instructions, but for three groups of words where Musashi is
    /// not the 68000 and every one of which the decoder reads as one:
    ///
    /// * `$4AFC`, `illegal`, which Musashi's disassembler tables as no
    ///   instruction;
    /// * the 16 words `$6xFF`, which Musashi tables as the 68020's long
    ///   branches and drops for the 68000; a 68000 reads `$FF` as a
    ///   displacement of −1, as the MAME-derived single-step suite records;
    /// * 2,305 line F words that Musashi tables as the 68020's coprocessor
    ///   and the 68040's instructions and drops for the 68000; a 68000 traps
    ///   every line F word through vector 11, `$FFFF` included, which is
    ///   SIMHALT's first word.
    #[test]
    fn the_legal_first_words_are_musashis_68000_words() {
        let musashi = musashi_68000();
        let mut exceptions = Vec::new();
        for word in 0..=0xFFFFu16 {
            let legal = is_legal(word, Processor::M68000);
            if legal != musashi[word as usize] {
                assert!(
                    legal,
                    "{word:04X} is an instruction to Musashi and not to the decoder"
                );
                exceptions.push(word);
            }
        }
        let illegal = exceptions.iter().filter(|&&word| word == 0x4AFC).count();
        let branches = exceptions
            .iter()
            .filter(|&&word| word & 0xF0FF == 0x60FF)
            .count();
        let line_f = exceptions.iter().filter(|&&word| word >= 0xF000).count();
        assert_eq!((illegal, branches, line_f), (1, 16, 2305));
        assert_eq!(exceptions.len(), illegal + branches + line_f);
        // What the decoder reads the line A and line F words as.
        for word in 0xA000..=0xAFFFu16 {
            assert_eq!(
                decode_words(&[word], 0, Processor::M68000)
                    .unwrap()
                    .instruction,
                Instruction::LINEA(word)
            );
        }
        for word in 0xF000..=0xFFFFu16 {
            assert_eq!(
                decode_words(&[word, 0], 0, Processor::M68000)
                    .unwrap()
                    .instruction,
                Instruction::LINEF(word)
            );
        }
        let count = (0..=0xFFFFu16)
            .filter(|word| is_legal(*word, Processor::M68000))
            .count();
        // 45,815 instructions, with `illegal` but without line A and line F,
        // which is also the number of words the MAME-derived suite's opcode map
        // names (`docs/research/m68k-verification-references.md` §1.1).
        assert_eq!(count - 0x2000 - 1, 45_815);
    }

    /// The opt-in check of the hand-assembled words against GNU `as`.
    ///
    /// `S68K_BINUTILS_PREFIX` is the path of the m68k binutils with their
    /// common prefix, for example `/usr/bin/m68k-linux-gnu-` once
    /// `binutils-m68k-linux-gnu` is installed, or the same path inside a
    /// directory `dpkg -x` unpacked it into (with `LD_LIBRARY_PATH` pointing at
    /// its `usr/lib/x86_64-linux-gnu`). Each line is assembled on its own with
    /// `-m68000`, and the `.text` it makes must be the case's words.
    ///
    /// `cargo test --release gnu_as -- --ignored`
    #[test]
    #[ignore = "needs GNU as for m68k, named by S68K_BINUTILS_PREFIX"]
    fn the_hand_assembled_words_are_gnu_as_words() {
        use std::process::Command;
        let prefix = std::env::var("S68K_BINUTILS_PREFIX")
            .expect("S68K_BINUTILS_PREFIX names the m68k binutils, e.g. /usr/bin/m68k-linux-gnu-");
        let directory = std::env::temp_dir().join(format!("s68k-gnu-as-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let (source, object, binary) = (
            directory.join("line.s"),
            directory.join("line.o"),
            directory.join("line.bin"),
        );
        let mut checked = 0;
        for case in CASES {
            let Some(line) = case.gnu else { continue };
            std::fs::write(&source, format!("{line}\n")).unwrap();
            let assembled = Command::new(format!("{prefix}as"))
                .args(["-m68000", "--register-prefix-optional", "-o"])
                .arg(&object)
                .arg(&source)
                .output()
                .expect("GNU as runs");
            assert!(
                assembled.status.success(),
                "`{line}`: {}",
                String::from_utf8_lossy(&assembled.stderr)
            );
            let copied = Command::new(format!("{prefix}objcopy"))
                .args(["-O", "binary", "-j", ".text"])
                .arg(&object)
                .arg(&binary)
                .status()
                .expect("GNU objcopy runs");
            assert!(copied.success());
            let bytes = std::fs::read(&binary).unwrap();
            let words: Vec<u16> = bytes
                .chunks(2)
                .map(|pair| u16::from_be_bytes([pair[0], pair[1]]))
                .collect();
            assert_eq!(words, case.words, "`{line}`");
            checked += 1;
        }
        std::fs::remove_dir_all(&directory).ok();
        assert!(checked > 100, "only {checked} lines were checked");
    }
}
