//! From an [`Instruction`] to words: the encoder.
//!
//! [`encode`] writes the words of one instruction at an address: the operation
//! word, then the extension words in the order the 68000 reads them — an
//! immediate before the destination's words, a `movem` mask or a static bit
//! number before the effective address's, the source's words before the
//! destination's in a `move`. It is the inverse of the decoder
//! ([`super::decode`]): for every Instruction it encodes,
//! `decode(encode(i))` is `i` again, and for every sequence of words whose
//! ignored bits are clear, `encode(decode(words))` is the words.
//!
//! # What it refuses
//!
//! An [`Instruction`] is a value, and a value can hold what no 68000 word
//! does, so the encoder checks what it writes rather than truncating it:
//!
//! * a displacement or a value that does not fit its field — a short branch
//!   more than a byte away or to the next instruction, a PC-relative distance,
//!   an immediate wider than its size, a quick count outside 1 to 8 — is
//!   [`EncodeError::OutOfRange`];
//! * an Operand an instruction has no encoding for — `jmp #5`, `add.w a0,(a1)`,
//!   a byte from an address register — is [`EncodeError::NoEncoding`].
//!
//! The Assembler never asks for either: the analyzer refuses the lines that
//! would, and the lowering keeps every immediate at its size
//! ([`crate::assembler::instructions::lowering`]). An error here means an
//! Instruction built some other way, or a range the Assembler does not check
//! yet: the distance of a branch is only known once instructions have their
//! real sizes.
//!
//! # Processors
//!
//! The encoder writes what it is given and asks no processor: an instruction
//! is the 68000's, the 68010's or the 68020's by what it is, and whether a
//! Program may use it is the analyzer's question. `move ccr,<ea>` is written
//! as the 68010 has it, `extb.l` and a [`BranchSize::Long`] as the 68020 has
//! them, and the decoder reads those words back only for those processors.
//!
//! [`size`] measures an instruction without placing it: the size of an
//! encoding depends on its Operands' modes and never on their values or its
//! address, so it is what the Layout can ask before anything is placed.

use std::fmt;

use crate::instructions::{
    AddressWidth, BranchSize, Condition, IndexRegister, Instruction, Operand, RegisterOperand,
    ShiftDirection, Sign, Size, TargetDirection,
};

use super::decode::{
    ABSOLUTE_LONG, ABSOLUTE_WORD, ALL, ALTERABLE, AN, CONTROL, DATA, DATA_ALTERABLE,
    DATA_NOT_IMMEDIATE, DISPLACEMENT, DN, IMMEDIATE, INDEX, INDIRECT, MEMORY_ALTERABLE,
    MOVEM_FROM_MEMORY, MOVEM_TO_MEMORY, PC_DISPLACEMENT, PC_INDEX, POSTINCREMENT, PREDECREMENT,
};

/// Encodes `instruction` at `address`: its words, the operation word first.
///
/// The address places the instruction: a branch and a PC-relative Operand
/// encode the distance from the word their displacement is in to the address
/// they name, so the same Instruction is different words at different
/// addresses, and may not fit at all at one of them.
pub fn encode(instruction: &Instruction, address: u32) -> Result<Vec<u16>, EncodeError> {
    let mut encoder = Encoder {
        address,
        words: Vec::with_capacity(5),
        check: true,
    };
    encoder.instruction(instruction)?;
    Ok(encoder.words)
}

/// [`encode`], as the bytes the words are in memory, the high byte of each
/// word first.
pub fn encode_bytes(instruction: &Instruction, address: u32) -> Result<Vec<u8>, EncodeError> {
    Ok(encode(instruction, address)?
        .into_iter()
        .flat_map(u16::to_be_bytes)
        .collect())
}

/// How many bytes `instruction` encodes to, wherever it is placed.
///
/// No range is checked, because none changes the size: a short branch is two
/// bytes however far it reaches. An Operand the instruction has no encoding
/// for is still [`EncodeError::NoEncoding`].
pub fn size(instruction: &Instruction) -> Result<u32, EncodeError> {
    let mut encoder = Encoder {
        address: 0,
        words: Vec::with_capacity(5),
        check: false,
    };
    encoder.instruction(instruction)?;
    Ok(encoder.words.len() as u32 * 2)
}

/// Why an [`Instruction`] could not be encoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EncodeError {
    /// A displacement or a value does not fit the field the instruction
    /// encodes it in.
    OutOfRange {
        /// The field.
        field: Field,
        /// The value it would have held: a displacement in bytes, an address
        /// read as a signed number, an immediate as it was stored.
        value: i64,
    },
    /// The instruction has no encoding with these Operands, or with this one
    /// where it stands.
    NoEncoding {
        /// What has none, as a clause: "`jmp` takes a control operand".
        reason: &'static str,
    },
}

/// A field of an encoding that holds a number, and so has a range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    /// The displacement byte of a short branch: −128 to 127, and not 0, which
    /// marks a word branch.
    ShortBranch,
    /// The displacement word of a word branch or a `DBcc`.
    WordBranch,
    /// The displacement word of `d16(PC)`.
    PcDisplacement,
    /// The displacement byte of `d8(PC,Xn)`.
    PcIndex,
    /// The displacement word of `d16(An)`, and of `movep`.
    Displacement,
    /// The displacement byte of `d8(An,Xn)`.
    IndexDisplacement,
    /// An absolute short address, which a word holds sign extended.
    ShortAddress,
    /// An absolute long address, which is 32 bits.
    LongAddress,
    /// An immediate of this size, stored zero extended.
    Immediate(Size),
    /// The count of `addq` and `subq`, and of a shift by an immediate: 1 to 8.
    Count,
    /// The vector of `trap`: 0 to 15.
    TrapVector,
    /// A static bit number, which is the low byte of its word.
    BitNumber,
    /// The displacement of `link`, a word sign extended.
    LinkDisplacement,
    /// A register number, 0 to 7.
    Register,
}

impl Field {
    /// The lowest and the highest value the field holds.
    pub fn range(self) -> (i64, i64) {
        match self {
            Field::ShortBranch | Field::PcIndex | Field::IndexDisplacement => (-128, 127),
            Field::WordBranch
            | Field::PcDisplacement
            | Field::Displacement
            | Field::ShortAddress
            | Field::LinkDisplacement => (-32768, 32767),
            Field::LongAddress => (0, u32::MAX as i64),
            Field::Immediate(size) => (0, mask(size) as i64),
            Field::Count => (1, 8),
            Field::TrapVector => (0, 15),
            Field::BitNumber => (0, 255),
            Field::Register => (0, 7),
        }
    }

    /// Whether the field holds `value`.
    pub fn holds(self, value: i64) -> bool {
        let (min, max) = self.range();
        (min..=max).contains(&value) && !(self == Field::ShortBranch && value == 0)
    }

    /// How a message names the field.
    pub fn name(self) -> &'static str {
        match self {
            Field::ShortBranch => "the displacement of a short branch",
            Field::WordBranch => "the displacement of a word branch",
            Field::PcDisplacement => "the displacement of `d(PC)`",
            Field::PcIndex => "the displacement of `d(PC,Xn)`",
            Field::Displacement => "the displacement of `d(An)`",
            Field::IndexDisplacement => "the displacement of `d(An,Xn)`",
            Field::ShortAddress => "an absolute short address",
            Field::LongAddress => "an absolute long address",
            Field::Immediate(Size::Byte) => "a byte immediate",
            Field::Immediate(Size::Word) => "a word immediate",
            Field::Immediate(Size::Long) => "a long immediate",
            Field::Count => "a quick count",
            Field::TrapVector => "the vector of `trap`",
            Field::BitNumber => "a static bit number",
            Field::LinkDisplacement => "the displacement of `link`",
            Field::Register => "a register number",
        }
    }
}

impl fmt::Display for EncodeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EncodeError::OutOfRange { field, value } => {
                let (min, max) = field.range();
                let not_zero = if *field == Field::ShortBranch {
                    " and not 0"
                } else {
                    ""
                };
                write!(
                    formatter,
                    "{} is {min} to {max}{not_zero}, and {value} is outside it",
                    field.name()
                )
            }
            EncodeError::NoEncoding { reason } => write!(formatter, "no 68000 encoding: {reason}"),
        }
    }
}

impl std::error::Error for EncodeError {}

/// The bits of an immediate of `size`.
fn mask(size: Size) -> u32 {
    match size {
        Size::Byte => 0xFF,
        Size::Word => 0xFFFF,
        Size::Long => 0xFFFF_FFFF,
    }
}

/// A refusal, as an `Err`.
fn no_encoding<T>(reason: &'static str) -> Result<T, EncodeError> {
    Err(EncodeError::NoEncoding { reason })
}

/// The words of one instruction, written in order.
struct Encoder {
    /// The address of the operation word.
    address: u32,
    /// The words so far; the first is the operation word, written last.
    words: Vec<u16>,
    /// Whether ranges are checked: [`size`] does not, as no range changes a
    /// size.
    check: bool,
}

impl Encoder {
    /// The address of the next word to be written, which is what a
    /// PC-relative displacement in that word is measured from.
    fn next(&self) -> u32 {
        self.address.wrapping_add(2 * self.words.len() as u32)
    }

    fn word(&mut self, word: u16) {
        self.words.push(word);
    }

    fn long(&mut self, long: u32) {
        self.word((long >> 16) as u16);
        self.word(long as u16);
    }

    /// `value` against `field`, when ranges are checked.
    fn fits(&self, field: Field, value: i64) -> Result<(), EncodeError> {
        if self.check && !field.holds(value) {
            return Err(EncodeError::OutOfRange { field, value });
        }
        Ok(())
    }

    /// A register number, which every field holds in three bits.
    fn number(&self, number: u8) -> Result<u16, EncodeError> {
        if number > 7 {
            return Err(EncodeError::OutOfRange {
                field: Field::Register,
                value: number as i64,
            });
        }
        Ok(number as u16)
    }

    /// A data register's number.
    fn data(&self, register: RegisterOperand) -> Result<u16, EncodeError> {
        match register {
            RegisterOperand::Data(number) => self.number(number),
            RegisterOperand::Address(_) => no_encoding("the register there is a data register"),
        }
    }

    /// An address register's number.
    fn address_register(&self, register: RegisterOperand) -> Result<u16, EncodeError> {
        match register {
            RegisterOperand::Address(number) => self.number(number),
            RegisterOperand::Data(_) => no_encoding("the register there is an address register"),
        }
    }

    /// A data register given as an Operand.
    fn data_operand(&self, operand: Operand) -> Result<u16, EncodeError> {
        match operand {
            Operand::Register(register) => self.data(register),
            _ => no_encoding("the operand there is a data register"),
        }
    }

    /// An immediate of `size`: a byte in the low half of a word, a word, or a
    /// long in two words.
    fn immediate(&mut self, value: u32, size: Size) -> Result<(), EncodeError> {
        self.fits(Field::Immediate(size), value as i64)?;
        match size {
            Size::Byte => self.word(value as u16 & 0xFF),
            Size::Word => self.word(value as u16),
            Size::Long => self.long(value),
        }
        Ok(())
    }

    /// A word displacement to `target`, measured from the word it is written
    /// in.
    fn displacement_to(&mut self, target: u32, field: Field) -> Result<(), EncodeError> {
        let displacement = target.wrapping_sub(self.next()) as i32 as i64;
        self.fits(field, displacement)?;
        self.word(displacement as u16);
        Ok(())
    }

    /// The brief extension word of an indexed mode: the index register, its
    /// width and the displacement byte. Bits 10–8 are clear, as the 68000
    /// ignores them.
    fn brief(
        &mut self,
        index: IndexRegister,
        displacement: i64,
        field: Field,
    ) -> Result<(), EncodeError> {
        self.fits(field, displacement)?;
        let register = match index.register {
            RegisterOperand::Data(number) => self.number(number)?,
            RegisterOperand::Address(number) => 0x8 | self.number(number)?,
        };
        let long = match index.size {
            Size::Word => 0,
            Size::Long => 0x0800,
            Size::Byte => return no_encoding("an index register is a word or a long"),
        };
        self.word(register << 12 | long | (displacement as u8 as u16));
        Ok(())
    }

    /// The six-bit effective-address field of `operand`, mode in bits 5–3 and
    /// register in bits 2–0, with its extension words written. `size` is the
    /// size an immediate has; `allowed` the modes the position takes, as the
    /// decoder's rows name them, and anything else is refused with `reason`.
    fn ea(
        &mut self,
        operand: Operand,
        size: Size,
        allowed: u16,
        reason: &'static str,
    ) -> Result<u16, EncodeError> {
        if mode_of(&operand) & allowed == 0 {
            return no_encoding(reason);
        }
        Ok(match operand {
            Operand::Register(RegisterOperand::Data(number)) => self.number(number)?,
            Operand::Register(RegisterOperand::Address(number)) => 0o10 | self.number(number)?,
            Operand::Indirect(number) => 0o20 | self.number(number)?,
            Operand::PostIndirect(number) => 0o30 | self.number(number)?,
            Operand::PreIndirect(number) => 0o40 | self.number(number)?,
            Operand::IndirectDisplacement { offset, base } => {
                let register = self.address_register(base)?;
                self.fits(Field::Displacement, offset as i64)?;
                self.word(offset as u16);
                0o50 | register
            }
            Operand::IndirectIndex {
                base,
                offset,
                index,
            } => {
                let register = self.address_register(base)?;
                self.brief(index, offset as i64, Field::IndexDisplacement)?;
                0o60 | register
            }
            Operand::Absolute {
                address,
                width: AddressWidth::Word,
            } => {
                self.fits(Field::LongAddress, address as i64)?;
                self.fits(Field::ShortAddress, address as u32 as i32 as i64)?;
                self.word(address as u16);
                0o70
            }
            Operand::Absolute {
                address,
                width: AddressWidth::Long,
            } => {
                self.fits(Field::LongAddress, address as i64)?;
                self.long(address as u32);
                0o71
            }
            Operand::PcDisplacement { target } => {
                self.displacement_to(target, Field::PcDisplacement)?;
                0o72
            }
            Operand::PcIndex { target, index } => {
                let displacement = target.wrapping_sub(self.next()) as i32 as i64;
                self.brief(index, displacement, Field::PcIndex)?;
                0o73
            }
            Operand::Immediate(value) => {
                self.immediate(value, size)?;
                0o74
            }
        })
    }

    /// Writes the instruction, its operation word last into the first slot.
    fn instruction(&mut self, instruction: &Instruction) -> Result<(), EncodeError> {
        use Instruction as I;
        // The operation word comes first and is known last.
        self.word(0);
        let first = match *instruction {
            // Line 0: the immediates, the status register immediates, the bit
            // instructions and movep.
            I::ORI(value, destination, size) => self.immediate_into(0, value, destination, size)?,
            I::ANDI(value, destination, size) => {
                self.immediate_into(1, value, destination, size)?
            }
            I::SUBI(value, destination, size) => {
                self.immediate_into(2, value, destination, size)?
            }
            I::ADDI(value, destination, size) => {
                self.immediate_into(3, value, destination, size)?
            }
            I::EORI(value, destination, size) => {
                self.immediate_into(5, value, destination, size)?
            }
            I::CMPI(value, destination, size) => {
                self.immediate_into(6, value, destination, size)?
            }
            I::ORItoCCR(value) => self.status(0x003C, value as u16),
            I::ORItoSR(value) => self.status(0x007C, value),
            I::ANDItoCCR(value) => self.status(0x023C, value as u16),
            I::ANDItoSR(value) => self.status(0x027C, value),
            I::EORItoCCR(value) => self.status(0x0A3C, value as u16),
            I::EORItoSR(value) => self.status(0x0A7C, value),
            I::BTST(bit, target) => self.bit(0, bit, target)?,
            I::BCHG(bit, target) => self.bit(1, bit, target)?,
            I::BCLR(bit, target) => self.bit(2, bit, target)?,
            I::BSET(bit, target) => self.bit(3, bit, target)?,
            I::MOVEP {
                direction,
                size,
                register,
                target,
            } => {
                let data = self.data(register)?;
                let Operand::IndirectDisplacement { offset, base } = target else {
                    return no_encoding("`movep` reaches memory through `d16(An)` only");
                };
                let base = self.address_register(base)?;
                self.fits(Field::Displacement, offset as i64)?;
                self.word(offset as u16);
                let to_memory = match direction {
                    TargetDirection::ToMemory => 0x0080,
                    TargetDirection::FromMemory => 0,
                };
                let long = match size {
                    Size::Word => 0,
                    Size::Long => 0x0040,
                    Size::Byte => return no_encoding("`movep` moves a word or a long"),
                };
                0x0108 | data << 9 | to_memory | long | base
            }
            // Lines 1 to 3: move and movea.
            I::MOVE(source, destination, size) => {
                if size == Size::Byte && mode_of(&source) == AN {
                    return no_encoding("an address register is never read a byte at a time");
                }
                let source = self.ea(source, size, ALL, "`move` reads any operand")?;
                let destination = self.ea(
                    destination,
                    size,
                    DATA_ALTERABLE,
                    "`move` writes a data alterable operand; into an address register it is `movea`",
                )?;
                let destination = (destination & 7) << 9 | (destination >> 3) << 6;
                move_size(size) << 12 | destination | source
            }
            I::MOVEA(source, register, size) => {
                if size == Size::Byte {
                    return no_encoding("`movea` moves a word or a long");
                }
                let register = self.address_register(register)?;
                let source = self.ea(source, size, ALL, "`movea` reads any operand")?;
                move_size(size) << 12 | register << 9 | 0x0040 | source
            }
            // Line 4.
            I::NEGX(target, size) => self.one_operand(0x4000, target, size)?,
            I::CLR(target, size) => self.one_operand(0x4200, target, size)?,
            I::NEG(target, size) => self.one_operand(0x4400, target, size)?,
            I::NOT(target, size) => self.one_operand(0x4600, target, size)?,
            I::TST(target, size) => self.one_operand(0x4A00, target, size)?,
            I::MOVEfromSR(target) => {
                0x40C0
                    | self.ea(
                        target,
                        Size::Word,
                        DATA_ALTERABLE,
                        "`move sr` writes a data alterable operand",
                    )?
            }
            I::MOVEfromCCR(target) => {
                0x42C0
                    | self.ea(
                        target,
                        Size::Word,
                        DATA_ALTERABLE,
                        "`move ccr` writes a data alterable operand",
                    )?
            }
            I::MOVEtoCCR(source) => {
                0x44C0
                    | self.ea(
                        source,
                        Size::Word,
                        DATA,
                        "`move` into `ccr` reads a data operand",
                    )?
            }
            I::MOVEtoSR(source) => {
                0x46C0
                    | self.ea(
                        source,
                        Size::Word,
                        DATA,
                        "`move` into `sr` reads a data operand",
                    )?
            }
            I::NBCD(target) => {
                0x4800
                    | self.ea(
                        target,
                        Size::Byte,
                        DATA_ALTERABLE,
                        "`nbcd` writes a data alterable operand",
                    )?
            }
            I::SWAP(register) => 0x4840 | self.data(register)?,
            I::PEA(source) => {
                0x4840 | self.ea(source, Size::Long, CONTROL, "`pea` takes a control operand")?
            }
            I::EXT(register, Size::Byte, Size::Word) => 0x4880 | self.data(register)?,
            I::EXT(register, Size::Word, Size::Long) => 0x48C0 | self.data(register)?,
            I::EXT(register, Size::Byte, Size::Long) => 0x49C0 | self.data(register)?,
            I::EXT(..) => {
                return no_encoding("`ext` widens a byte to a word or a long, or a word to a long")
            }
            I::MOVEM {
                direction,
                size,
                registers_mask,
                target,
            } => {
                let long = match size {
                    Size::Word => 0,
                    Size::Long => 0x0040,
                    Size::Byte => return no_encoding("`movem` moves words or longs"),
                };
                self.word(registers_mask);
                let (from_memory, target) = match direction {
                    TargetDirection::ToMemory => (
                        0,
                        self.ea(
                            target,
                            size,
                            MOVEM_TO_MEMORY,
                            "`movem` writes registers to a control alterable operand or `-(An)`",
                        )?,
                    ),
                    TargetDirection::FromMemory => (
                        0x0400,
                        self.ea(
                            target,
                            size,
                            MOVEM_FROM_MEMORY,
                            "`movem` reads registers from a control operand or `(An)+`",
                        )?,
                    ),
                };
                0x4880 | from_memory | long | target
            }
            I::TAS(target) => {
                0x4AC0
                    | self.ea(
                        target,
                        Size::Byte,
                        DATA_ALTERABLE,
                        "`tas` writes a data alterable operand",
                    )?
            }
            I::ILLEGAL => 0x4AFC,
            I::TRAP(vector) => {
                self.fits(Field::TrapVector, vector as i64)?;
                0x4E40 | (vector as u16 & 0xF)
            }
            I::LINK(register, displacement) => {
                let register = self.address_register(register)?;
                let signed = displacement as i32 as i64;
                self.fits(Field::LinkDisplacement, signed)?;
                self.word(displacement as u16);
                0x4E50 | register
            }
            I::UNLK(register) => 0x4E58 | self.address_register(register)?,
            I::MOVEUSP { to_usp, register } => {
                let register = self.address_register(register)?;
                match to_usp {
                    true => 0x4E60 | register,
                    false => 0x4E68 | register,
                }
            }
            I::RESET => 0x4E70,
            I::NOP => 0x4E71,
            I::STOP(value) => {
                self.word(value);
                0x4E72
            }
            I::RTE => 0x4E73,
            I::RTS => 0x4E75,
            I::TRAPV => 0x4E76,
            I::RTR => 0x4E77,
            I::JSR(target) => {
                0x4E80 | self.ea(target, Size::Long, CONTROL, "`jsr` takes a control operand")?
            }
            I::JMP(target) => {
                0x4EC0 | self.ea(target, Size::Long, CONTROL, "`jmp` takes a control operand")?
            }
            I::CHK(bound, register) => {
                let register = self.data(register)?;
                0x4180
                    | register << 9
                    | self.ea(bound, Size::Word, DATA, "`chk` reads a data operand")?
            }
            I::LEA(source, register) => {
                let register = self.address_register(register)?;
                0x41C0
                    | register << 9
                    | self.ea(source, Size::Long, CONTROL, "`lea` takes a control operand")?
            }
            // Line 5.
            I::ADDQ(count, target, size) => self.quick(0x5000, count, target, size)?,
            I::SUBQ(count, target, size) => self.quick(0x5100, count, target, size)?,
            I::Scc(target, condition) => {
                0x50C0
                    | condition.code() << 8
                    | self.ea(
                        target,
                        Size::Byte,
                        DATA_ALTERABLE,
                        "`Scc` writes a data alterable operand",
                    )?
            }
            I::DBcc(register, target, condition) => {
                let register = self.data(register)?;
                self.displacement_to(target, Field::WordBranch)?;
                0x50C8 | condition.code() << 8 | register
            }
            // Line 6.
            I::BRA(target, size) => self.branch(Condition::True, target, size)?,
            I::BSR(target, size) => self.branch(Condition::False, target, size)?,
            I::Bcc(target, condition, size) => {
                if matches!(condition, Condition::True | Condition::False) {
                    return no_encoding(
                        "the conditions `t` and `f` of a branch are `bra` and `bsr`",
                    );
                }
                self.branch(condition, target, size)?
            }
            // Line 7.
            I::MOVEQ(value, register) => 0x7000 | self.data(register)? << 9 | value as u16,
            // Line 8.
            I::OR(source, destination, size) => self.logical(0x8000, source, destination, size)?,
            I::DIVx(source, register, sign) => {
                let opcode = match sign {
                    Sign::Unsigned => 0x80C0,
                    Sign::Signed => 0x81C0,
                };
                self.word_into_data(opcode, source, register, "a divide reads a data operand")?
            }
            I::SBCD(source, destination) => self.extended(0x8100, source, destination, None)?,
            // Line 9.
            I::SUB(source, destination, size) => {
                self.arithmetic(0x9000, source, destination, size)?
            }
            I::SUBA(source, register, size) => {
                self.into_address(0x90C0, source, register, size, "`suba` reads any operand")?
            }
            I::SUBX(source, destination, size) => {
                self.extended(0x9100, source, destination, Some(size))?
            }
            // Line A.
            I::LINEA(word) => {
                if word & 0xF000 != 0xA000 {
                    return no_encoding("a line A word begins with `$A`");
                }
                word
            }
            // Line B.
            I::CMP(source, register, size) => {
                if size == Size::Byte && mode_of(&source) == AN {
                    return no_encoding("an address register is never read a byte at a time");
                }
                let register = self.data(register)?;
                0xB000
                    | register << 9
                    | standard_size(size) << 6
                    | self.ea(source, size, ALL, "`cmp` reads any operand")?
            }
            I::CMPA(source, register, size) => {
                self.into_address(0xB0C0, source, register, size, "`cmpa` reads any operand")?
            }
            I::CMPM(source, destination, size) => {
                let (Operand::PostIndirect(y), Operand::PostIndirect(x)) = (source, destination)
                else {
                    return no_encoding("`cmpm` compares two `(An)+` operands");
                };
                0xB108 | self.number(x)? << 9 | standard_size(size) << 6 | self.number(y)?
            }
            I::EOR(source, destination, size) => {
                let register = self.data_operand(source)?;
                0xB100
                    | register << 9
                    | standard_size(size) << 6
                    | self.ea(
                        destination,
                        size,
                        DATA_ALTERABLE,
                        "`eor` writes a data alterable operand",
                    )?
            }
            // Line C.
            I::AND(source, destination, size) => self.logical(0xC000, source, destination, size)?,
            I::MULx(source, register, sign) => {
                let opcode = match sign {
                    Sign::Unsigned => 0xC0C0,
                    Sign::Signed => 0xC1C0,
                };
                self.word_into_data(opcode, source, register, "a multiply reads a data operand")?
            }
            I::ABCD(source, destination) => self.extended(0xC100, source, destination, None)?,
            I::EXG(first, second) => {
                match (first, second) {
                    (RegisterOperand::Data(x), RegisterOperand::Data(y)) => {
                        0xC140 | self.number(x)? << 9 | self.number(y)?
                    }
                    (RegisterOperand::Address(x), RegisterOperand::Address(y)) => {
                        0xC148 | self.number(x)? << 9 | self.number(y)?
                    }
                    (RegisterOperand::Data(x), RegisterOperand::Address(y)) => {
                        0xC188 | self.number(x)? << 9 | self.number(y)?
                    }
                    (RegisterOperand::Address(_), RegisterOperand::Data(_)) => return no_encoding(
                        "`exg` of a data and an address register is written data register first",
                    ),
                }
            }
            // Line D.
            I::ADD(source, destination, size) => {
                self.arithmetic(0xD000, source, destination, size)?
            }
            I::ADDA(source, register, size) => {
                self.into_address(0xD0C0, source, register, size, "`adda` reads any operand")?
            }
            I::ADDX(source, destination, size) => {
                self.extended(0xD100, source, destination, Some(size))?
            }
            // Line E.
            I::ASd(count, target, direction, size) => {
                self.shift(0, count, target, direction, size)?
            }
            I::LSd(count, target, direction, size) => {
                self.shift(1, count, target, direction, size)?
            }
            I::ROXd(count, target, direction, size) => {
                self.shift(2, count, target, direction, size)?
            }
            I::ROd(count, target, direction, size) => {
                self.shift(3, count, target, direction, size)?
            }
            // Line F, and SIMHALT.
            I::LINEF(word) => {
                if word & 0xF000 != 0xF000 {
                    return no_encoding("a line F word begins with `$F`");
                }
                word
            }
            I::SIMHALT => {
                self.word(0xFFFF);
                0xFFFF
            }
            I::IllegalWord(word) => word,
        };
        self.words[0] = first;
        Ok(())
    }

    /// `ori`, `andi`, `subi`, `addi`, `eori` or `cmpi`, by `kind` in bits
    /// 11–9: the immediate's words, then the destination's.
    fn immediate_into(
        &mut self,
        kind: u16,
        value: u32,
        destination: Operand,
        size: Size,
    ) -> Result<u16, EncodeError> {
        self.immediate(value, size)?;
        let destination = self.ea(
            destination,
            size,
            DATA_ALTERABLE,
            "an immediate instruction writes a data alterable operand",
        )?;
        Ok(kind << 9 | standard_size(size) << 6 | destination)
    }

    /// `andi`, `ori` or `eori` to `ccr` or `sr`: the operation word, and the
    /// immediate in the word after it, of which `ccr` takes the low byte.
    fn status(&mut self, opcode: u16, value: u16) -> u16 {
        self.word(value);
        opcode
    }

    /// A bit instruction by `kind` (`btst`, `bchg`, `bclr`, `bset`): the bit
    /// number from a data register, or static from the word after the
    /// operation word.
    fn bit(&mut self, kind: u16, bit: Operand, target: Operand) -> Result<u16, EncodeError> {
        let test = kind == 0;
        match bit {
            Operand::Register(register) => {
                let register = self.data(register)?;
                let (allowed, reason) = match test {
                    true => (DATA, "`btst` reads a data operand"),
                    false => (
                        DATA_ALTERABLE,
                        "a bit instruction writes a data alterable operand",
                    ),
                };
                let target = self.ea(target, Size::Byte, allowed, reason)?;
                Ok(0x0100 | register << 9 | kind << 6 | target)
            }
            Operand::Immediate(number) => {
                self.fits(Field::BitNumber, number as i64)?;
                self.word(number as u16 & 0xFF);
                let (allowed, reason) = match test {
                    true => (
                        DATA_NOT_IMMEDIATE,
                        "a static `btst` reads a data operand other than an immediate",
                    ),
                    false => (
                        DATA_ALTERABLE,
                        "a bit instruction writes a data alterable operand",
                    ),
                };
                let target = self.ea(target, Size::Byte, allowed, reason)?;
                Ok(0x0800 | kind << 6 | target)
            }
            _ => no_encoding("a bit number is a data register or an immediate"),
        }
    }

    /// `negx`, `clr`, `neg`, `not` and `tst`: a size and a data alterable
    /// operand.
    fn one_operand(
        &mut self,
        opcode: u16,
        target: Operand,
        size: Size,
    ) -> Result<u16, EncodeError> {
        let target = self.ea(
            target,
            size,
            DATA_ALTERABLE,
            "the instruction takes a data alterable operand",
        )?;
        Ok(opcode | standard_size(size) << 6 | target)
    }

    /// `addq` or `subq`: the count in bits 11–9, 8 written as 0, and an
    /// alterable destination, of which an address register is a word or a
    /// long.
    fn quick(
        &mut self,
        opcode: u16,
        count: u8,
        target: Operand,
        size: Size,
    ) -> Result<u16, EncodeError> {
        self.fits(Field::Count, count as i64)?;
        if size == Size::Byte && mode_of(&target) == AN {
            return no_encoding("an address register is never written a byte at a time");
        }
        let target = self.ea(
            target,
            size,
            ALTERABLE,
            "a quick form writes an alterable operand",
        )?;
        Ok(opcode | (count as u16 & 7) << 9 | standard_size(size) << 6 | target)
    }

    /// `bra`, `bsr` or a `Bcc`: the condition in bits 11–8 and the
    /// displacement, measured from the word after the operation word, in the
    /// operation word's low byte, in a word after a `$00`, or in a long after
    /// a `$FF`.
    fn branch(
        &mut self,
        condition: Condition,
        target: u32,
        size: BranchSize,
    ) -> Result<u16, EncodeError> {
        let displacement = target.wrapping_sub(self.next()) as i32 as i64;
        let byte = match size {
            BranchSize::Short => {
                self.fits(Field::ShortBranch, displacement)?;
                displacement as u8 as u16
            }
            BranchSize::Word => {
                self.fits(Field::WordBranch, displacement)?;
                self.word(displacement as u16);
                0x00
            }
            BranchSize::Long => {
                self.long(displacement as u32);
                0xFF
            }
        };
        Ok(0x6000 | condition.code() << 8 | byte)
    }

    /// `or` and `and`: `<ea>,dn` into a data register, reading any data
    /// operand, and `dn,<ea>` into memory.
    fn logical(
        &mut self,
        opcode: u16,
        source: Operand,
        destination: Operand,
        size: Size,
    ) -> Result<u16, EncodeError> {
        let size_bits = standard_size(size) << 6;
        match destination {
            Operand::Register(register) => {
                let register = self.data(register)?;
                let source = self.ea(source, size, DATA, "`and` and `or` read a data operand")?;
                Ok(opcode | register << 9 | size_bits | source)
            }
            _ => {
                let register = self.data_operand(source)?;
                let destination = self.ea(
                    destination,
                    size,
                    MEMORY_ALTERABLE,
                    "`and` and `or` write a data register or memory; an immediate is `andi` or `ori`",
                )?;
                Ok(opcode | 0x0100 | register << 9 | size_bits | destination)
            }
        }
    }

    /// `add` and `sub`: `<ea>,dn` into a data register, reading any operand,
    /// an address register as a word or a long; `dn,<ea>` into memory.
    fn arithmetic(
        &mut self,
        opcode: u16,
        source: Operand,
        destination: Operand,
        size: Size,
    ) -> Result<u16, EncodeError> {
        let size_bits = standard_size(size) << 6;
        match destination {
            Operand::Register(register) => {
                if size == Size::Byte && mode_of(&source) == AN {
                    return no_encoding("an address register is never read a byte at a time");
                }
                let register = self.data(register)?;
                let source = self.ea(source, size, ALL, "`add` and `sub` read any operand")?;
                Ok(opcode | register << 9 | size_bits | source)
            }
            _ => {
                let register = self.data_operand(source)?;
                let destination = self.ea(
                    destination,
                    size,
                    MEMORY_ALTERABLE,
                    "`add` and `sub` write a register or memory, from a data register",
                )?;
                Ok(opcode | 0x0100 | register << 9 | size_bits | destination)
            }
        }
    }

    /// `adda`, `suba` and `cmpa`: bit 8 says a long, and any source, read at
    /// that size.
    fn into_address(
        &mut self,
        opcode: u16,
        source: Operand,
        register: RegisterOperand,
        size: Size,
        reason: &'static str,
    ) -> Result<u16, EncodeError> {
        let long = match size {
            Size::Word => 0,
            Size::Long => 0x0100,
            Size::Byte => return no_encoding("an address register takes a word or a long"),
        };
        let register = self.address_register(register)?;
        let source = self.ea(source, size, ALL, reason)?;
        Ok(opcode | register << 9 | long | source)
    }

    /// `divu`, `divs`, `mulu` and `muls`: a word read from any data operand
    /// into a data register.
    fn word_into_data(
        &mut self,
        opcode: u16,
        source: Operand,
        register: RegisterOperand,
        reason: &'static str,
    ) -> Result<u16, EncodeError> {
        let register = self.data(register)?;
        Ok(opcode | register << 9 | self.ea(source, Size::Word, DATA, reason)?)
    }

    /// `abcd`, `sbcd`, `addx` and `subx`: `Dy,Dx`, or `-(Ay),-(Ax)` with bit 3
    /// set, y in bits 2–0 and x in bits 11–9. The two decimal ones have no
    /// size field.
    fn extended(
        &mut self,
        opcode: u16,
        source: Operand,
        destination: Operand,
        size: Option<Size>,
    ) -> Result<u16, EncodeError> {
        let (memory, y, x) = match (source, destination) {
            (
                Operand::Register(RegisterOperand::Data(y)),
                Operand::Register(RegisterOperand::Data(x)),
            ) => (0, y, x),
            (Operand::PreIndirect(y), Operand::PreIndirect(x)) => (0x0008, y, x),
            _ => return no_encoding("the extend group takes `Dy,Dx` or `-(Ay),-(Ax)`"),
        };
        let size_bits = size.map_or(0, |size| standard_size(size) << 6);
        Ok(opcode | self.number(x)? << 9 | size_bits | memory | self.number(y)?)
    }

    /// A shift or rotate by `kind` (`%00` arithmetic, `%01` logical, `%10`
    /// through the extend flag, `%11` rotate): of a data register by a count of
    /// 1 to 8 or by a register, or of a word in memory by one.
    fn shift(
        &mut self,
        kind: u16,
        count: Operand,
        target: Operand,
        direction: ShiftDirection,
        size: Size,
    ) -> Result<u16, EncodeError> {
        let left = match direction {
            ShiftDirection::Left => 0x0100,
            ShiftDirection::Right => 0,
        };
        match target {
            Operand::Register(register) => {
                let register = self.data(register)?;
                let count = match count {
                    Operand::Immediate(count) => {
                        self.fits(Field::Count, count as i64)?;
                        (count as u16 & 7) << 9
                    }
                    Operand::Register(counter) => 0x0020 | self.data(counter)? << 9,
                    _ => return no_encoding("a shift count is a data register or 1 to 8"),
                };
                Ok(0xE000 | count | left | standard_size(size) << 6 | kind << 3 | register)
            }
            _ => {
                if count != Operand::Immediate(1) || size != Size::Word {
                    return no_encoding("a shift in memory shifts a word by one place");
                }
                let target = self.ea(
                    target,
                    Size::Word,
                    MEMORY_ALTERABLE,
                    "a shift in memory writes a memory alterable operand",
                )?;
                Ok(0xE0C0 | kind << 9 | left | target)
            }
        }
    }
}

/// The size in bits 7–6: `%00` a byte, `%01` a word, `%10` a long.
fn standard_size(size: Size) -> u16 {
    match size {
        Size::Byte => 0,
        Size::Word => 1,
        Size::Long => 2,
    }
}

/// The size of `move` and `movea`, in bits 13–12: `%01` a byte, `%11` a word,
/// `%10` a long.
fn move_size(size: Size) -> u16 {
    match size {
        Size::Byte => 1,
        Size::Word => 3,
        Size::Long => 2,
    }
}

/// The mode an Operand is in, as one of the decoder's mode bits.
fn mode_of(operand: &Operand) -> u16 {
    match operand {
        Operand::Register(RegisterOperand::Data(_)) => DN,
        Operand::Register(RegisterOperand::Address(_)) => AN,
        Operand::Indirect(_) => INDIRECT,
        Operand::PostIndirect(_) => POSTINCREMENT,
        Operand::PreIndirect(_) => PREDECREMENT,
        Operand::IndirectDisplacement { .. } => DISPLACEMENT,
        Operand::IndirectIndex { .. } => INDEX,
        Operand::Absolute {
            width: AddressWidth::Word,
            ..
        } => ABSOLUTE_WORD,
        Operand::Absolute {
            width: AddressWidth::Long,
            ..
        } => ABSOLUTE_LONG,
        Operand::PcDisplacement { .. } => PC_DISPLACEMENT,
        Operand::PcIndex { .. } => PC_INDEX,
        Operand::Immediate(_) => IMMEDIATE,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine::decode::{decode_words, tests::CASES};
    use crate::machine::Processor;

    fn d(number: u8) -> Operand {
        Operand::Register(RegisterOperand::Data(number))
    }

    fn a(number: u8) -> Operand {
        Operand::Register(RegisterOperand::Address(number))
    }

    fn at(instruction: Instruction) -> Vec<u16> {
        encode(&instruction, 0x1000)
            .unwrap_or_else(|error| panic!("{instruction:?} does not encode: {error}"))
    }

    /// Every hand-assembled case of the decoder's, which are the Reference
    /// Manual's encodings and, where it has one, GNU `as`'s: the words
    /// decode to an instruction, and the instruction encodes to the words.
    #[test]
    fn every_hand_assembled_case_encodes_back_to_its_words() {
        for case in CASES {
            let decoded = decode_words(case.words, 0x1000, Processor::M68000).unwrap();
            let words = at(decoded.instruction);
            // An illegal word, line A or line F is the one word, whatever
            // follows it.
            assert_eq!(
                words,
                case.words[..decoded.size as usize / 2],
                "{} ({:?})",
                case.text,
                decoded.instruction
            );
            assert_eq!(size(&decoded.instruction), Ok(decoded.size));
        }
    }

    /// Every first word a processor decodes, with extension words that set no
    /// bit the 68000 ignores: the encoder writes the same words back, which
    /// decode to the same instruction, and measures them as the decoder did.
    #[test]
    fn every_word_the_decoder_reads_encodes_back() {
        let extension = [0x0010, 0x0024, 0x0030, 0x0040];
        for processor in Processor::ALL {
            for first in 0..=0xFFFFu16 {
                let words = [
                    first,
                    extension[0],
                    extension[1],
                    extension[2],
                    extension[3],
                ];
                let decoded = decode_words(&words, 0x1000, processor).unwrap();
                let encoded = encode(&decoded.instruction, 0x1000).unwrap_or_else(|error| {
                    panic!(
                        "{first:04X} is {:?}, which does not encode: {error}",
                        decoded.instruction
                    )
                });
                assert_eq!(
                    encoded,
                    words[..decoded.size as usize / 2],
                    "{first:04X} on the {} is {:?}",
                    processor.name(),
                    decoded.instruction
                );
                assert_eq!(size(&decoded.instruction), Ok(decoded.size));
            }
        }
    }

    /// The forms the Assembler does not accept yet, built by hand.
    #[test]
    fn the_system_instructions_encode() {
        assert_eq!(at(Instruction::RTE), [0x4E73]);
        assert_eq!(at(Instruction::STOP(0x2700)), [0x4E72, 0x2700]);
        assert_eq!(at(Instruction::RESET), [0x4E70]);
        assert_eq!(
            at(Instruction::MOVEUSP {
                to_usp: true,
                register: RegisterOperand::Address(1)
            }),
            [0x4E61]
        );
        assert_eq!(
            at(Instruction::MOVEUSP {
                to_usp: false,
                register: RegisterOperand::Address(2)
            }),
            [0x4E6A]
        );
        assert_eq!(at(Instruction::TRAP(0)), [0x4E40]);
        assert_eq!(at(Instruction::TRAP(14)), [0x4E4E]);
        assert_eq!(at(Instruction::TRAP(15)), [0x4E4F]);
        assert_eq!(at(Instruction::LINEA(0xA123)), [0xA123]);
        assert_eq!(at(Instruction::LINEF(0xF000)), [0xF000]);
        assert_eq!(at(Instruction::IllegalWord(0x4AFB)), [0x4AFB]);
        assert_eq!(at(Instruction::ILLEGAL), [0x4AFC]);
        assert_eq!(at(Instruction::SIMHALT), [0xFFFF, 0xFFFF]);
        // The 68010's and the 68020's, which only their decoders read back.
        assert_eq!(at(Instruction::MOVEfromCCR(d(0))), [0x42C0]);
        assert_eq!(
            at(Instruction::EXT(
                RegisterOperand::Data(3),
                Size::Byte,
                Size::Long
            )),
            [0x49C3]
        );
        assert_eq!(
            at(Instruction::BRA(0x0001_1002, BranchSize::Long)),
            [0x60FF, 0x0001, 0x0000]
        );
    }

    /// One instruction of each family, from a hand-built value: the
    /// encodings EASy68K chooses (the asm-editor's
    /// `docs/research/easy68k-machine-behaviour.md` §15) and the extension
    /// words in the order the 68000 reads them.
    #[test]
    fn each_family_encodes_as_the_reference_manual_writes_it() {
        let long = Size::Long;
        let word = Size::Word;
        let byte = Size::Byte;
        // The three encodings of `add.w #5,d0`.
        assert_eq!(at(Instruction::ADDQ(5, d(0), word)), [0x5A40]);
        assert_eq!(at(Instruction::ADDI(100, d(0), word)), [0x0640, 0x0064]);
        assert_eq!(
            at(Instruction::ADD(Operand::Immediate(5), d(0), word)),
            [0xD07C, 0x0005]
        );
        assert_eq!(at(Instruction::ADD(d(1), d(0), word)), [0xD041]);
        assert_eq!(
            at(Instruction::ADD(d(0), Operand::Indirect(0), word)),
            [0xD150]
        );
        assert_eq!(
            at(Instruction::ADDA(
                Operand::Immediate(100),
                RegisterOperand::Address(0),
                long
            )),
            [0xD1FC, 0x0000, 0x0064]
        );
        assert_eq!(at(Instruction::ADDQ(3, a(0), word)), [0x5648]);
        assert_eq!(at(Instruction::SUBQ(8, a(7), long)), [0x518F]);
        // `cmp`, `cmpi`, `cmpa`, `cmpm`.
        assert_eq!(
            at(Instruction::CMP(
                Operand::Immediate(5),
                RegisterOperand::Data(0),
                word
            )),
            [0xB07C, 0x0005]
        );
        assert_eq!(
            at(Instruction::CMPI(5, Operand::Indirect(0), word)),
            [0x0C50, 0x0005]
        );
        assert_eq!(
            at(Instruction::CMPA(d(0), RegisterOperand::Address(0), long)),
            [0xB1C0]
        );
        assert_eq!(
            at(Instruction::CMPM(
                Operand::PostIndirect(0),
                Operand::PostIndirect(1),
                byte
            )),
            [0xB308]
        );
        // `and`, `andi`, `eori` and the status register.
        assert_eq!(
            at(Instruction::AND(Operand::Immediate(0xF), d(0), word)),
            [0xC07C, 0x000F]
        );
        assert_eq!(
            at(Instruction::ANDI(0xF, Operand::Indirect(0), word)),
            [0x0250, 0x000F]
        );
        assert_eq!(at(Instruction::EORI(1, d(0), word)), [0x0A40, 0x0001]);
        assert_eq!(at(Instruction::ANDItoCCR(0x1F)), [0x023C, 0x001F]);
        assert_eq!(at(Instruction::ORItoSR(0x0700)), [0x007C, 0x0700]);
        // `move` and its kin.
        assert_eq!(
            at(Instruction::MOVEQ(5, RegisterOperand::Data(5))),
            [0x7A05]
        );
        assert_eq!(
            at(Instruction::MOVEQ(0xFF, RegisterOperand::Data(3))),
            [0x76FF]
        );
        assert_eq!(
            at(Instruction::MOVE(Operand::Immediate(5), d(0), word)),
            [0x303C, 0x0005]
        );
        assert_eq!(
            at(Instruction::MOVEA(
                Operand::Immediate(5),
                RegisterOperand::Address(0),
                long
            )),
            [0x207C, 0x0000, 0x0005]
        );
        assert_eq!(at(Instruction::MOVE(a(0), d(0), long)), [0x2008]);
        assert_eq!(
            at(Instruction::MOVE(Operand::Immediate(0xFF), d(2), byte)),
            [0x143C, 0x00FF]
        );
        // The source's words come before the destination's.
        assert_eq!(
            at(Instruction::MOVE(
                Operand::Immediate(0x12345678),
                Operand::Absolute {
                    address: 0x2000,
                    width: AddressWidth::Word
                },
                long
            )),
            [0x21FC, 0x1234, 0x5678, 0x2000]
        );
        assert_eq!(
            at(Instruction::MOVEtoSR(Operand::Immediate(0x2700))),
            [0x46FC, 0x2700]
        );
        assert_eq!(at(Instruction::MOVEfromSR(d(0))), [0x40C0]);
        // `exg`, data register first.
        assert_eq!(
            at(Instruction::EXG(
                RegisterOperand::Data(0),
                RegisterOperand::Address(1)
            )),
            [0xC189]
        );
        // `link`, sign extended.
        assert_eq!(
            at(Instruction::LINK(
                RegisterOperand::Address(6),
                (-8i32) as u32
            )),
            [0x4E56, 0xFFF8]
        );
        // Branches: short, word, and a `DBcc`, all from the word after the
        // operation word.
        assert_eq!(at(Instruction::BRA(0x1000, BranchSize::Short)), [0x60FE]);
        assert_eq!(
            at(Instruction::BSR(0x1100, BranchSize::Word)),
            [0x6100, 0x00FE]
        );
        assert_eq!(
            at(Instruction::Bcc(
                0x0FFA,
                Condition::Equal,
                BranchSize::Short
            )),
            [0x67F8]
        );
        assert_eq!(
            at(Instruction::DBcc(
                RegisterOperand::Data(0),
                0x1000,
                Condition::False
            )),
            [0x51C8, 0xFFFE]
        );
        // Shifts in a register by a count and by a register, and in memory.
        assert_eq!(
            at(Instruction::ASd(
                Operand::Immediate(8),
                d(1),
                ShiftDirection::Right,
                long
            )),
            [0xE081]
        );
        assert_eq!(
            at(Instruction::ROXd(d(6), d(7), ShiftDirection::Right, word)),
            [0xEC77]
        );
        assert_eq!(
            at(Instruction::LSd(
                Operand::Immediate(1),
                Operand::IndirectDisplacement {
                    offset: 4,
                    base: RegisterOperand::Address(1)
                },
                ShiftDirection::Right,
                word
            )),
            [0xE2E9, 0x0004]
        );
        // Bit instructions, dynamic and static; the static bit number comes
        // before the destination's words.
        assert_eq!(
            at(Instruction::BTST(d(5), Operand::Immediate(0x12))),
            [0x0B3C, 0x0012]
        );
        assert_eq!(
            at(Instruction::BCLR(
                Operand::Immediate(0),
                Operand::Absolute {
                    address: 0x4000,
                    width: AddressWidth::Long
                }
            )),
            [0x08B9, 0x0000, 0x0000, 0x4000]
        );
        // The extend group and the decimal pair.
        assert_eq!(
            at(Instruction::ADDX(
                Operand::PreIndirect(1),
                Operand::PreIndirect(2),
                long
            )),
            [0xD589]
        );
        assert_eq!(
            at(Instruction::SBCD(
                Operand::PreIndirect(1),
                Operand::PreIndirect(0)
            )),
            [0x8109]
        );
        // `movem`, whose mask is the first extension word.
        assert_eq!(
            at(Instruction::MOVEM {
                direction: TargetDirection::ToMemory,
                size: long,
                registers_mask: 0xE080,
                target: Operand::PreIndirect(7)
            }),
            [0x48E7, 0xE080]
        );
        // Indexed operands: the brief word, with bits 10–8 clear.
        assert_eq!(
            at(Instruction::MOVE(
                Operand::IndirectIndex {
                    base: RegisterOperand::Address(2),
                    offset: -1,
                    index: IndexRegister {
                        register: RegisterOperand::Address(3),
                        size: long
                    }
                },
                d(4),
                byte
            )),
            [0x1832, 0xB8FF]
        );
    }

    /// A PC-relative displacement is measured from the word it is in: the
    /// word after the operation word, but the one after that for `movem`
    /// and `btst #n`, whose mask and bit number come first.
    #[test]
    fn a_pc_relative_displacement_is_measured_from_its_own_word() {
        let target = Operand::PcDisplacement { target: 0x1014 };
        assert_eq!(
            at(Instruction::LEA(target, RegisterOperand::Address(0))),
            [0x41FA, 0x0012]
        );
        assert_eq!(
            at(Instruction::MOVEM {
                direction: TargetDirection::FromMemory,
                size: Size::Word,
                registers_mask: 0x0101,
                target
            }),
            [0x4CBA, 0x0101, 0x0010]
        );
        assert_eq!(
            at(Instruction::BTST(Operand::Immediate(1), target)),
            [0x083A, 0x0001, 0x0010]
        );
        assert_eq!(at(Instruction::BTST(d(1), target)), [0x033A, 0x0012]);
        // `(pc,d1.w)` reaches its own extension word with a displacement of 0.
        let index = IndexRegister {
            register: RegisterOperand::Data(1),
            size: Size::Word,
        };
        assert_eq!(
            at(Instruction::JMP(Operand::PcIndex {
                target: 0x1002,
                index
            })),
            [0x4EFB, 0x1000]
        );
        // The same Instruction is other words somewhere else.
        assert_eq!(
            encode(
                &Instruction::LEA(target, RegisterOperand::Address(0)),
                0x1010
            ),
            Ok(vec![0x41FA, 0x0002])
        );
    }

    #[test]
    fn a_value_that_does_not_fit_its_field_is_out_of_range() {
        let out = |instruction: Instruction, field: Field, value: i64| {
            assert_eq!(
                encode(&instruction, 0x1000),
                Err(EncodeError::OutOfRange { field, value }),
                "{instruction:?}"
            );
        };
        out(
            Instruction::BRA(0x1082, BranchSize::Short),
            Field::ShortBranch,
            128,
        );
        out(
            Instruction::BRA(0x1002, BranchSize::Short),
            Field::ShortBranch,
            0,
        );
        out(
            Instruction::BRA(0x9002, BranchSize::Word),
            Field::WordBranch,
            0x8000,
        );
        out(
            Instruction::DBcc(RegisterOperand::Data(0), 0x9002, Condition::False),
            Field::WordBranch,
            0x8000,
        );
        out(
            Instruction::LEA(
                Operand::PcDisplacement { target: 0x9002 },
                RegisterOperand::Address(0),
            ),
            Field::PcDisplacement,
            0x8000,
        );
        out(
            Instruction::JMP(Operand::PcIndex {
                target: 0x1082,
                index: IndexRegister {
                    register: RegisterOperand::Data(0),
                    size: Size::Word,
                },
            }),
            Field::PcIndex,
            128,
        );
        out(
            Instruction::MOVE(Operand::Immediate(0x100), d(0), Size::Byte),
            Field::Immediate(Size::Byte),
            0x100,
        );
        out(
            Instruction::ADDI(0xFFFF_FFFF, d(0), Size::Word),
            Field::Immediate(Size::Word),
            0xFFFF_FFFF,
        );
        out(Instruction::ADDQ(9, d(0), Size::Long), Field::Count, 9);
        out(Instruction::SUBQ(0, d(0), Size::Long), Field::Count, 0);
        out(
            Instruction::ASd(
                Operand::Immediate(9),
                d(0),
                ShiftDirection::Left,
                Size::Long,
            ),
            Field::Count,
            9,
        );
        out(Instruction::TRAP(16), Field::TrapVector, 16);
        out(
            Instruction::LINK(RegisterOperand::Address(6), 0xFFFC),
            Field::LinkDisplacement,
            0xFFFC,
        );
        out(
            Instruction::JMP(Operand::Absolute {
                address: 0x8000,
                width: AddressWidth::Word,
            }),
            Field::ShortAddress,
            0x8000,
        );
        out(
            Instruction::JMP(Operand::IndirectDisplacement {
                offset: 0x8000,
                base: RegisterOperand::Address(0),
            }),
            Field::Displacement,
            0x8000,
        );
        out(
            Instruction::BTST(Operand::Immediate(256), d(0)),
            Field::BitNumber,
            256,
        );
        out(
            Instruction::SWAP(RegisterOperand::Data(8)),
            Field::Register,
            8,
        );
        // No range is a size: a branch that does not fit is still measured.
        assert_eq!(size(&Instruction::BRA(0x1082, BranchSize::Short)), Ok(2));
        assert_eq!(
            EncodeError::OutOfRange {
                field: Field::ShortBranch,
                value: 0
            }
            .to_string(),
            "the displacement of a short branch is -128 to 127 and not 0, and 0 is outside it"
        );
    }

    #[test]
    fn an_operand_the_instruction_has_no_encoding_for_is_refused() {
        let refused = |instruction: Instruction| {
            assert!(
                matches!(
                    encode(&instruction, 0x1000),
                    Err(EncodeError::NoEncoding { .. })
                ),
                "{instruction:?} encodes"
            );
            assert!(size(&instruction).is_err(), "{instruction:?} has a size");
        };
        refused(Instruction::JMP(Operand::Immediate(5)));
        refused(Instruction::JSR(Operand::PostIndirect(0)));
        refused(Instruction::LEA(d(0), RegisterOperand::Address(0)));
        refused(Instruction::LEA(
            Operand::Indirect(0),
            RegisterOperand::Data(0),
        ));
        refused(Instruction::ADD(a(0), Operand::Indirect(1), Size::Word));
        refused(Instruction::ADD(
            Operand::Immediate(1),
            Operand::Indirect(1),
            Size::Word,
        ));
        refused(Instruction::ADD(a(0), d(0), Size::Byte));
        refused(Instruction::AND(a(0), d(0), Size::Word));
        refused(Instruction::MOVE(a(0), d(0), Size::Byte));
        refused(Instruction::MOVE(d(0), a(0), Size::Word));
        refused(Instruction::MOVE(d(0), Operand::Immediate(0), Size::Word));
        refused(Instruction::MOVEA(
            d(0),
            RegisterOperand::Address(0),
            Size::Byte,
        ));
        refused(Instruction::ADDQ(1, a(0), Size::Byte));
        refused(Instruction::ADDI(1, a(0), Size::Word));
        refused(Instruction::CLR(a(0), Size::Word));
        refused(Instruction::CMPI(
            1,
            Operand::PcDisplacement { target: 0 },
            Size::Word,
        ));
        refused(Instruction::CMPM(
            Operand::Indirect(0),
            Operand::PostIndirect(1),
            Size::Byte,
        ));
        refused(Instruction::EXG(
            RegisterOperand::Address(0),
            RegisterOperand::Data(0),
        ));
        refused(Instruction::Bcc(0x1000, Condition::True, BranchSize::Word));
        refused(Instruction::BTST(
            Operand::Immediate(1),
            Operand::Immediate(5),
        ));
        refused(Instruction::BSET(
            d(0),
            Operand::PcDisplacement { target: 0 },
        ));
        refused(Instruction::ASd(
            Operand::Immediate(2),
            Operand::Indirect(0),
            ShiftDirection::Left,
            Size::Word,
        ));
        refused(Instruction::ASd(
            Operand::Immediate(1),
            Operand::Indirect(0),
            ShiftDirection::Left,
            Size::Long,
        ));
        refused(Instruction::MOVEM {
            direction: TargetDirection::ToMemory,
            size: Size::Long,
            registers_mask: 1,
            target: Operand::PostIndirect(7),
        });
        refused(Instruction::MOVEP {
            direction: TargetDirection::ToMemory,
            size: Size::Word,
            register: RegisterOperand::Data(0),
            target: Operand::Indirect(0),
        });
        refused(Instruction::EXT(
            RegisterOperand::Data(0),
            Size::Long,
            Size::Word,
        ));
        refused(Instruction::LINEA(0xF000));
        refused(Instruction::ADDX(d(0), Operand::PreIndirect(0), Size::Long));
    }

    #[test]
    fn the_bytes_are_the_words_high_byte_first() {
        assert_eq!(
            encode_bytes(
                &Instruction::MOVE(Operand::Immediate(0x1234), d(0), Size::Word),
                0x1000
            ),
            Ok(vec![0x30, 0x3C, 0x12, 0x34])
        );
        assert_eq!(
            size(&Instruction::MOVE(
                Operand::Immediate(0x12345678),
                Operand::Absolute {
                    address: 0x12345678,
                    width: AddressWidth::Long
                },
                Size::Long
            )),
            Ok(10)
        );
    }
}
