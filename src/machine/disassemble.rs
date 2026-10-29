//! From an [`Instruction`] to text: the disassembler.
//!
//! [`disassemble`] prints one instruction the way EASy68K's source writes it:
//! `MOVE.L D0,(A1)+`, `BRA.S $1004`, `MOVEM.L D0-D2/A0,-(A7)`. What it prints
//! is a line EASy68K reads, though not always one it encodes the same way:
//! `ADD.W #$5,D0` is an `addq` there, and an address with no `.W` may be one.
//! [`disassemble_bytes`] decodes a run of memory and prints each instruction
//! in it, and a word that is no instruction as the `DC.W` that would put it
//! there.
//!
//! # The rules
//!
//! * The mnemonic is upper case, with the size the instruction works at —
//!   `.B`, `.W` or `.L` — wherever the instruction has more than one, and the
//!   branch size `.S`, `.W` or `.L` on every branch. `MULU.W`, `DIVS.W` and
//!   `CHK.W` carry theirs, as the 68020 has a long form of each. The
//!   instructions that name `SR`, `CCR` or `USP`, the bit instructions, `Scc`,
//!   `TAS` and the decimal ones carry none.
//! * Registers are `D0` to `D7` and `A0` to `A7`; the stack pointer is `A7`.
//! * Numbers are hexadecimal with a `$`, but for two kinds. The counts kept
//!   inside the operation word — the quick count of `ADDQ` and `SUBQ`, a shift
//!   count, the `MOVEQ` value and the `TRAP` vector — and a static bit number
//!   are decimal; `MOVEQ`'s is signed, `MOVEQ #-1,D0`. Displacements are signed
//!   decimal: `-8(A6)`, `4(A0,D1.W)`, `LINK A6,#-4`.
//! * An immediate prints at the size of its instruction: `MOVE.B #$FF,D0`.
//! * An absolute short address prints the address its word sign extends to,
//!   with `.W`: the word `$8000` is `$FFFF8000.W`, which reassembles to it,
//!   and `$1000` is `$1000.W`. A long one prints the address alone,
//!   `$E00010`.
//! * A branch target, a PC-relative operand and an absolute address print as
//!   the name the [`Symbols`] give them, when they give one: `BRA.S loop`,
//!   `LEA table(PC),A0`, `JSR print.W`; as a number otherwise.
//! * A `MOVEM` list is the canonical one: lowest register first, data
//!   registers before address registers, ranges within one bank,
//!   `D0-D2/A0/A6`.
//! * A shift in memory prints no count, `ASL.W (A0)`, which is how it is
//!   written.
//! * The 68020's indexed modes print in its Motorola syntax: the brief word's
//!   scale after the index, `4(A0,D1.W*4)`, and the full word's parts in
//!   parentheses, with brackets for memory indirection, every displacement
//!   with its width and a null one left out, `(1000.W,A0,D1.L)`,
//!   `([8.W,A0],D1.W*2,4.W)`; a suppressed base is `ZA0` to `ZA7` or `ZPC`,
//!   and a suppressed index `ZD0` where there is no indirection. What prints
//!   reassembles under `opt p=68020` to the same words; an extension word no
//!   assembler writes, a reserved combination or the full word of an index
//!   with no displacement, prints as `DC.W`.
//! * The 68020's instructions print as `opt p=68020` writes them: a register
//!   pair `MULS.L D0,D1:D2`, `CAS2.L D0:D1,D2:D3,(A0):(D4)`, `DIVSL.L` for a
//!   32-bit division with a remainder and `DIVS.L D0,D1` for one without, a
//!   bit field after its Operand, `BFEXTU (A0){D1:32},D2`, a width of 32
//!   written 32, and `TRAPEQ.W #$1`.
//! * A line A word, a line F word and a word that is no instruction print as
//!   `DC.W $A000`; EASy68K's halt as `SIMHALT`.
//! * A `MOVEM` of no register runs, and no line writes it, as a register list
//!   is never empty: it prints as the `DC.W` of its words,
//!   `DC.W $4890,$0000`, which [`disassemble_words`] reads from memory.

use crate::instructions::{
    AddressWidth, BranchSize, Condition, DisplacementSize, ExtensionWord, IndexBase, IndexRegister,
    Indirection, Instruction, Operand, RegisterOperand, ShiftDirection, Sign, Size,
    TargetDirection, TrapOperand,
};
use crate::instructions::{BitFieldOperation, FieldValue};

use super::decode::{decode, Truncated};
use super::Dialect;
#[cfg(test)]
use super::Processor;

/// Names for addresses: what a disassembly prints in place of a number when it
/// has one, a Label of the Program for instance.
pub trait Symbols {
    /// The name of `address`, or `None` to print the number.
    fn name_of(&self, address: u32) -> Option<String>;
}

impl<F: Fn(u32) -> Option<String>> Symbols for F {
    fn name_of(&self, address: u32) -> Option<String> {
        self(address)
    }
}

/// One instruction, as EASy68K writes it, with the names `symbols` gives the
/// addresses in it.
///
/// An instruction no line writes prints as the `DC.W` of its words, which
/// this has to make: it encodes it at address 0, so the words are the ones
/// memory holds but for the displacement of a PC-relative operand. What reads
/// memory has the words, and prints through [`disassemble_words`].
pub fn disassemble(instruction: &Instruction, symbols: Option<&dyn Symbols>) -> String {
    Printer { symbols }.instruction(instruction, None)
}

/// One instruction decoded from `bytes`, its words, as EASy68K writes it,
/// with the names `symbols` gives the addresses in it: what
/// [`disassemble`] prints, and for an instruction no line writes, the
/// `DC.W` of those words.
pub fn disassemble_words(
    instruction: &Instruction,
    bytes: &[u8],
    symbols: Option<&dyn Symbols>,
) -> String {
    Printer { symbols }.instruction(instruction, Some(bytes))
}

/// `DC.W` and the words of `bytes`, which are an even number of them:
/// `DC.W $4890,$0000`.
fn words(bytes: &[u8]) -> String {
    let words: Vec<String> = bytes
        .chunks_exact(2)
        .map(|word| format!("${:02X}{:02X}", word[0], word[1]))
        .collect();
    format!("DC.W {}", words.join(","))
}

/// One instruction of a run of memory, as [`disassemble_bytes`] reads it, and
/// as the Interpreter's `disassemble_at` answers it (`{address, size, bytes,
/// text}` in ts-lib).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Line {
    /// The address of its first byte.
    pub address: u32,
    /// How many bytes it takes up.
    pub size: u32,
    /// Those bytes.
    pub bytes: Vec<u8>,
    /// What they say: an instruction, or a `DC.W` or `DC.B` for bytes that are
    /// not one.
    pub text: String,
}

/// Every instruction in `bytes`, the first at `address`, decoded for
/// `processor`.
///
/// Each instruction starts where the one before it ended. When the bytes end
/// in the middle of an instruction, each whole word left prints as a `DC.W`,
/// and an odd byte at the end as a `DC.B`.
pub fn disassemble_bytes(
    bytes: &[u8],
    address: u32,
    processor: impl Into<Dialect>,
    symbols: Option<&dyn Symbols>,
) -> Vec<Line> {
    let processor = processor.into();
    let printer = Printer { symbols };
    let word_at = |offset: usize| -> Option<u16> {
        Some(u16::from_be_bytes([
            *bytes.get(offset)?,
            *bytes.get(offset + 1)?,
        ]))
    };
    let mut lines = Vec::new();
    let mut offset = 0usize;
    while offset < bytes.len() {
        let at = address.wrapping_add(offset as u32);
        let decoded = decode(at, processor, |word_address| {
            word_at(offset + word_address.wrapping_sub(at) as usize).ok_or(Truncated {
                address: word_address,
            })
        });
        let (size, text) = match (decoded, word_at(offset)) {
            (Ok(decoded), _) => (
                decoded.size as usize,
                printer.instruction(
                    &decoded.instruction,
                    Some(&bytes[offset..offset + decoded.size as usize]),
                ),
            ),
            (Err(_), Some(word)) => (2, format!("DC.W ${word:04X}")),
            (Err(_), None) => (1, format!("DC.B ${:02X}", bytes[offset])),
        };
        lines.push(Line {
            address: at,
            size: size as u32,
            bytes: bytes[offset..offset + size].to_vec(),
            text,
        });
        offset += size;
    }
    lines
}

/// The printer, with the names it may use.
struct Printer<'s> {
    symbols: Option<&'s dyn Symbols>,
}

impl Printer<'_> {
    /// An address, by name when it has one.
    fn address(&self, address: u32) -> String {
        self.symbols
            .and_then(|symbols| symbols.name_of(address))
            .unwrap_or_else(|| format!("${address:X}"))
    }

    /// An Operand of an instruction that works at `size`, which is the size an
    /// immediate is printed at.
    fn operand(&self, operand: &Operand, size: Size) -> String {
        match operand {
            Operand::Immediate(value) => format!("#${:X}", value & mask(size)),
            Operand::Register(register) => name(register),
            Operand::Indirect(register) => format!("(A{register})"),
            Operand::PostIndirect(register) => format!("(A{register})+"),
            Operand::PreIndirect(register) => format!("-(A{register})"),
            Operand::IndirectDisplacement { offset, base } => format!("{offset}({})", name(base)),
            Operand::IndirectIndex {
                base,
                offset,
                index,
            } => format!(
                "{offset}({},{}.{})",
                name(base),
                name(&index.register),
                suffix(index.size)
            ),
            Operand::Absolute {
                address,
                width: AddressWidth::Word,
            } => {
                let address = *address as u32;
                match self.symbols.and_then(|symbols| symbols.name_of(address)) {
                    Some(label) => format!("{label}.W"),
                    None => format!("${:X}.W", address as u16 as i16 as u32),
                }
            }
            Operand::Absolute {
                address,
                width: AddressWidth::Long,
            } => self.address(*address as u32),
            Operand::PcDisplacement { target } => format!("{}(PC)", self.address(*target)),
            Operand::PcIndex { target, index } => format!(
                "{}(PC,{}.{})",
                self.address(*target),
                name(&index.register),
                suffix(index.size)
            ),
            Operand::Extended {
                base,
                word,
                displacement,
                outer,
            } => self.extended(*base, *word, *displacement, *outer),
        }
    }

    /// An [`Operand::Extended`], in the Motorola syntax of the 68020: the
    /// brief format as the 68000's index with a scale, `4(A0,D1.W*4)`, and the
    /// full one with every displacement's width written, `.W` or `.L`, a null
    /// one left out, so that it reassembles to the same words:
    /// `(1000.W,A0,D1.L)`, `([8.W,A0],D1.W*2,4.W)`, `([table.L,PC,D0.L])`. A
    /// suppressed base is `ZPC`, or `ZA0` to `ZA7`, `ZA0` left out where
    /// something else is written beside it; a suppressed index is `ZD0` where
    /// there is no indirection, which tells `(4.W,A0,ZD0)` from the 68000's
    /// `4(A0)`, and left out inside brackets.
    fn extended(
        &self,
        base: IndexBase,
        word: ExtensionWord,
        displacement: i32,
        outer: i32,
    ) -> String {
        let index = || {
            let IndexRegister { register, size } = word.index_register();
            let scale = match word.scale() {
                1 => String::new(),
                scale => format!("*{scale}"),
            };
            format!("{}.{}{scale}", name(&register), suffix(size))
        };
        let pc_relative = base.is_pc() && !word.base_suppressed();
        if !word.is_full() {
            let at = match pc_relative {
                true => self.address(displacement as u32),
                false => displacement.to_string(),
            };
            let base = match base.address_register() {
                Some(number) => format!("A{number}"),
                None => "PC".to_string(),
            };
            return format!("{at}({base},{})", index());
        }
        let sized = |value: i32, size: DisplacementSize, address: bool| {
            let value = match address {
                true => self.address(value as u32),
                false => value.to_string(),
            };
            match size {
                DisplacementSize::Null => None,
                DisplacementSize::Word => Some(format!("{value}.W")),
                DisplacementSize::Long => Some(format!("{value}.L")),
            }
        };
        let base_displacement = match (pc_relative, word.base_displacement_size()) {
            //from the program counter a null displacement reaches the extension word, which is
            //the address written when there is no other way to write it
            (true, DisplacementSize::Null) => None,
            (_, size) => sized(displacement, size, pc_relative),
        };
        let base_name = match (base.address_register(), word.base_suppressed()) {
            (Some(number), false) => Some(format!("A{number}")),
            (None, false) => Some("PC".to_string()),
            (Some(0), true) => None,
            (Some(number), true) => Some(format!("ZA{number}")),
            (None, true) => Some("ZPC".to_string()),
        };
        let index = match word.index() {
            Some(_) => Some(index()),
            None => None,
        };
        //a base suppressed in A0's place is written only when nothing else would be
        let group = |parts: Vec<Option<String>>| -> String {
            let parts: Vec<String> = parts.into_iter().flatten().collect();
            match parts.is_empty() {
                true => "ZA0".to_string(),
                false => parts.join(","),
            }
        };
        let outer = sized(outer, word.outer_displacement_size(), false);
        match word.indirection() {
            Indirection::None => {
                let index = index.or_else(|| Some("ZD0".to_string()));
                format!("({})", group(vec![base_displacement, base_name, index]))
            }
            Indirection::Preindexed => {
                let inner = group(vec![base_displacement, base_name, index]);
                let after = outer.map(|outer| format!(",{outer}")).unwrap_or_default();
                format!("([{inner}]{after})")
            }
            Indirection::Postindexed => {
                let inner = group(vec![base_displacement, base_name]);
                let after: Vec<String> = [index, outer].into_iter().flatten().collect();
                match after.is_empty() {
                    true => format!("([{inner}])"),
                    false => format!("([{inner}],{})", after.join(",")),
                }
            }
        }
    }

    /// `MNEMONIC.S source,destination`, both at the instruction's size.
    fn two(&self, mnemonic: &str, size: Size, source: &Operand, destination: &Operand) -> String {
        format!(
            "{mnemonic}.{} {},{}",
            suffix(size),
            self.operand(source, size),
            self.operand(destination, size)
        )
    }

    /// `MNEMONIC.S source,Rn`.
    fn into_register(
        &self,
        mnemonic: &str,
        size: Size,
        source: &Operand,
        register: &RegisterOperand,
    ) -> String {
        self.two(mnemonic, size, source, &Operand::Register(*register))
    }

    /// `MNEMONIC.S operand`.
    fn one(&self, mnemonic: &str, size: Size, operand: &Operand) -> String {
        format!(
            "{mnemonic}.{} {}",
            suffix(size),
            self.operand(operand, size)
        )
    }

    /// `MNEMONIC operand`, for an instruction with one size only.
    fn plain(&self, mnemonic: &str, operand: &Operand, size: Size) -> String {
        format!("{mnemonic} {}", self.operand(operand, size))
    }

    /// `MNEMONIC.S #value,destination`, for the immediate instructions.
    fn immediate(&self, mnemonic: &str, value: u32, destination: &Operand, size: Size) -> String {
        self.two(mnemonic, size, &Operand::Immediate(value), destination)
    }

    /// A shift or rotate: a count in decimal or a register, then a data
    /// register; or a place in memory and no count.
    fn shift(
        &self,
        kind: &str,
        count: &Operand,
        target: &Operand,
        direction: &ShiftDirection,
        size: Size,
    ) -> String {
        let mnemonic = format!(
            "{kind}{}.{}",
            match direction {
                ShiftDirection::Left => "L",
                ShiftDirection::Right => "R",
            },
            suffix(size)
        );
        let target_text = self.operand(target, size);
        match (count, target) {
            (Operand::Immediate(count), Operand::Register(_)) => {
                format!("{mnemonic} #{count},{target_text}")
            }
            (_, Operand::Register(_)) => {
                format!("{mnemonic} {},{target_text}", self.operand(count, size))
            }
            _ => format!("{mnemonic} {target_text}"),
        }
    }

    /// A bit instruction: a static bit number in decimal, or a register.
    fn bit(&self, mnemonic: &str, bit: &Operand, target: &Operand) -> String {
        let bit = match bit {
            Operand::Immediate(number) => format!("#{number}"),
            other => self.operand(other, Size::Byte),
        };
        format!("{mnemonic} {bit},{}", self.operand(target, Size::Byte))
    }

    /// A branch to `target`, with its size.
    fn branch(&self, mnemonic: &str, target: u32, size: BranchSize) -> String {
        let size = match size {
            BranchSize::Short => "S",
            BranchSize::Word => "W",
            BranchSize::Long => "L",
        };
        format!("{mnemonic}.{size} {}", self.address(target))
    }

    /// A `movem` or `movep`: what moves first when it goes to memory, the
    /// memory first when it comes back.
    fn transfer(
        &self,
        mnemonic: &str,
        size: Size,
        direction: &TargetDirection,
        registers: String,
        memory: &Operand,
    ) -> String {
        let memory = self.operand(memory, size);
        match direction {
            TargetDirection::ToMemory => {
                format!("{mnemonic}.{} {registers},{memory}", suffix(size))
            }
            TargetDirection::FromMemory => {
                format!("{mnemonic}.{} {memory},{registers}", suffix(size))
            }
        }
    }

    /// The instruction, whose words are `bytes` when the caller has them.
    fn instruction(&self, instruction: &Instruction, bytes: Option<&[u8]>) -> String {
        use Instruction as I;
        if !is_written(instruction) {
            return match bytes {
                Some(bytes) => words(bytes),
                None => match super::encode::encode_bytes(instruction, 0) {
                    Ok(bytes) => words(&bytes),
                    Err(_) => "DC.W".to_string(),
                },
            };
        }
        match instruction {
            I::MOVE(source, destination, size) => self.two("MOVE", *size, source, destination),
            I::MOVEA(source, register, size) => {
                self.into_register("MOVEA", *size, source, register)
            }
            I::MOVEQ(value, register) => format!("MOVEQ #{},{}", *value as i8, name(register)),
            I::MOVEM {
                direction,
                size,
                registers_mask,
                target,
            } => {
                // The mask of a predecrement is kept reversed, as it is
                // encoded; the list reads the other way round.
                let mask = match target {
                    Operand::PreIndirect(_) => registers_mask.reverse_bits(),
                    _ => *registers_mask,
                };
                self.transfer("MOVEM", *size, direction, register_list(mask), target)
            }
            I::MOVEP {
                direction,
                size,
                register,
                target,
            } => self.transfer("MOVEP", *size, direction, name(register), target),
            I::ADD(source, destination, size) => self.two("ADD", *size, source, destination),
            I::SUB(source, destination, size) => self.two("SUB", *size, source, destination),
            I::ADDA(source, register, size) => self.into_register("ADDA", *size, source, register),
            I::SUBA(source, register, size) => self.into_register("SUBA", *size, source, register),
            I::CMPA(source, register, size) => self.into_register("CMPA", *size, source, register),
            I::ADDX(source, destination, size) => self.two("ADDX", *size, source, destination),
            I::SUBX(source, destination, size) => self.two("SUBX", *size, source, destination),
            I::ADDQ(count, destination, size) => format!(
                "ADDQ.{} #{count},{}",
                suffix(*size),
                self.operand(destination, *size)
            ),
            I::SUBQ(count, destination, size) => format!(
                "SUBQ.{} #{count},{}",
                suffix(*size),
                self.operand(destination, *size)
            ),
            I::ADDI(value, destination, size) => self.immediate("ADDI", *value, destination, *size),
            I::SUBI(value, destination, size) => self.immediate("SUBI", *value, destination, *size),
            I::ANDI(value, destination, size) => self.immediate("ANDI", *value, destination, *size),
            I::ORI(value, destination, size) => self.immediate("ORI", *value, destination, *size),
            I::EORI(value, destination, size) => self.immediate("EORI", *value, destination, *size),
            I::CMPI(value, destination, size) => self.immediate("CMPI", *value, destination, *size),
            I::CMP(source, register, size) => self.into_register("CMP", *size, source, register),
            I::CMPM(source, destination, size) => self.two("CMPM", *size, source, destination),
            I::AND(source, destination, size) => self.two("AND", *size, source, destination),
            I::OR(source, destination, size) => self.two("OR", *size, source, destination),
            I::EOR(source, destination, size) => self.two("EOR", *size, source, destination),
            I::DIVx(source, register, sign) => {
                let mnemonic = match sign {
                    Sign::Signed => "DIVS",
                    Sign::Unsigned => "DIVU",
                };
                self.into_register(mnemonic, Size::Word, source, register)
            }
            I::MULx(source, register, sign) => {
                let mnemonic = match sign {
                    Sign::Signed => "MULS",
                    Sign::Unsigned => "MULU",
                };
                self.into_register(mnemonic, Size::Word, source, register)
            }
            I::CHK(bound, register) => self.into_register("CHK", Size::Word, bound, register),
            I::NEG(operand, size) => self.one("NEG", *size, operand),
            I::NEGX(operand, size) => self.one("NEGX", *size, operand),
            I::NOT(operand, size) => self.one("NOT", *size, operand),
            I::CLR(operand, size) => self.one("CLR", *size, operand),
            I::TST(operand, size) => self.one("TST", *size, operand),
            I::NBCD(operand) => self.plain("NBCD", operand, Size::Byte),
            I::TAS(operand) => self.plain("TAS", operand, Size::Byte),
            I::ABCD(source, destination) => format!(
                "ABCD {},{}",
                self.operand(source, Size::Byte),
                self.operand(destination, Size::Byte)
            ),
            I::SBCD(source, destination) => format!(
                "SBCD {},{}",
                self.operand(source, Size::Byte),
                self.operand(destination, Size::Byte)
            ),
            I::SWAP(register) => format!("SWAP {}", name(register)),
            I::EXT(register, read, written) => match (read, written) {
                (Size::Byte, Size::Long) => format!("EXTB.L {}", name(register)),
                (_, written) => format!("EXT.{} {}", suffix(*written), name(register)),
            },
            I::EXG(first, second) => format!("EXG {},{}", name(first), name(second)),
            I::LEA(source, register) => format!(
                "LEA {},{}",
                self.operand(source, Size::Long),
                name(register)
            ),
            I::PEA(source) => self.plain("PEA", source, Size::Long),
            I::JMP(target) => self.plain("JMP", target, Size::Long),
            I::JSR(target) => self.plain("JSR", target, Size::Long),
            I::LINK(register, displacement) => {
                format!("LINK {},#{}", name(register), *displacement as i32)
            }
            I::UNLK(register) => format!("UNLK {}", name(register)),
            I::BRA(target, size) => self.branch("BRA", *target, *size),
            I::BSR(target, size) => self.branch("BSR", *target, *size),
            I::Bcc(target, condition, size) => {
                self.branch(&format!("B{}", code(condition)), *target, *size)
            }
            I::DBcc(register, target, condition) => format!(
                "DB{} {},{}",
                code(condition),
                name(register),
                self.address(*target)
            ),
            I::Scc(operand, condition) => {
                self.plain(&format!("S{}", code(condition)), operand, Size::Byte)
            }
            I::ASd(count, target, direction, size) => {
                self.shift("AS", count, target, direction, *size)
            }
            I::LSd(count, target, direction, size) => {
                self.shift("LS", count, target, direction, *size)
            }
            I::ROd(count, target, direction, size) => {
                self.shift("RO", count, target, direction, *size)
            }
            I::ROXd(count, target, direction, size) => {
                self.shift("ROX", count, target, direction, *size)
            }
            I::BTST(bit, target) => self.bit("BTST", bit, target),
            I::BCHG(bit, target) => self.bit("BCHG", bit, target),
            I::BCLR(bit, target) => self.bit("BCLR", bit, target),
            I::BSET(bit, target) => self.bit("BSET", bit, target),
            I::MOVEtoCCR(source) => format!("MOVE {},CCR", self.operand(source, Size::Word)),
            I::MOVEfromCCR(destination) => {
                format!("MOVE CCR,{}", self.operand(destination, Size::Word))
            }
            I::MOVEtoSR(source) => format!("MOVE {},SR", self.operand(source, Size::Word)),
            I::MOVEfromSR(destination) => {
                format!("MOVE SR,{}", self.operand(destination, Size::Word))
            }
            I::ANDItoCCR(value) => format!("ANDI #${value:X},CCR"),
            I::ORItoCCR(value) => format!("ORI #${value:X},CCR"),
            I::EORItoCCR(value) => format!("EORI #${value:X},CCR"),
            I::ANDItoSR(value) => format!("ANDI #${value:X},SR"),
            I::ORItoSR(value) => format!("ORI #${value:X},SR"),
            I::EORItoSR(value) => format!("EORI #${value:X},SR"),
            I::MOVEUSP {
                to_usp: true,
                register,
            } => format!("MOVE {},USP", name(register)),
            I::MOVEUSP {
                to_usp: false,
                register,
            } => format!("MOVE USP,{}", name(register)),
            I::TRAP(vector) => format!("TRAP #{vector}"),
            I::STOP(value) => format!("STOP #${value:X}"),
            I::RTE => "RTE".to_string(),
            I::RTD(displacement) => format!("RTD #{}", *displacement as i32),
            I::BKPT(vector) => format!("BKPT #{vector}"),
            I::MOVEC {
                to_control,
                control,
                register,
            } => {
                //a code no processor has is not written, and printed as its words above
                let control = control.name().unwrap_or_default().to_uppercase();
                match to_control {
                    true => format!("MOVEC {},{control}", name(register)),
                    false => format!("MOVEC {control},{}", name(register)),
                }
            }
            I::MOVES {
                direction,
                size,
                register,
                target,
            } => self.transfer("MOVES", *size, direction, name(register), target),
            I::RESET => "RESET".to_string(),
            I::RTR => "RTR".to_string(),
            I::RTS => "RTS".to_string(),
            I::NOP => "NOP".to_string(),
            I::TRAPV => "TRAPV".to_string(),
            I::ILLEGAL => "ILLEGAL".to_string(),
            I::SIMHALT => "SIMHALT".to_string(),
            I::LINEA(word) | I::LINEF(word) | I::IllegalWord(word) => format!("DC.W ${word:04X}"),
            I::CHKL(bound, register) => self.into_register("CHK", Size::Long, bound, register),
            I::BF {
                operation,
                target,
                field,
                register,
            } => {
                let value = |value: &FieldValue, width: bool| match value {
                    FieldValue::Register(number) => format!("D{number}"),
                    FieldValue::Immediate(0) if width => "32".to_string(),
                    FieldValue::Immediate(number) => number.to_string(),
                };
                let field = format!(
                    "{}{{{}:{}}}",
                    self.operand(target, Size::Byte),
                    value(&field.offset, false),
                    value(&field.width, true)
                );
                let mnemonic = operation.mnemonic().to_uppercase();
                match operation {
                    BitFieldOperation::Insert => format!("{mnemonic} D{register},{field}"),
                    _ if operation.has_register() => format!("{mnemonic} {field},D{register}"),
                    _ => format!("{mnemonic} {field}"),
                }
            }
            I::PACK(source, destination, adjustment) | I::UNPK(source, destination, adjustment) => {
                let mnemonic = match instruction {
                    I::PACK(..) => "PACK",
                    _ => "UNPK",
                };
                format!(
                    "{mnemonic} {},{},{}",
                    self.operand(source, Size::Byte),
                    self.operand(destination, Size::Byte),
                    self.operand(&Operand::Immediate(*adjustment as u32), Size::Word)
                )
            }
            I::CHK2(bounds, register, size) => self.into_register("CHK2", *size, bounds, register),
            I::CMP2(bounds, register, size) => self.into_register("CMP2", *size, bounds, register),
            I::CAS {
                compare,
                update,
                target,
                size,
            } => format!(
                "CAS.{} D{compare},D{update},{}",
                suffix(*size),
                self.operand(target, *size)
            ),
            I::CAS2 {
                compare,
                update,
                pointers,
                size,
            } => format!(
                "CAS2.{} D{}:D{},D{}:D{},({}):({})",
                suffix(*size),
                compare.0,
                compare.1,
                update.0,
                update.1,
                name(&pointers.0),
                name(&pointers.1)
            ),
            I::MULL {
                source,
                low,
                high,
                sign,
            } => {
                let mnemonic = match sign {
                    Sign::Signed => "MULS.L",
                    Sign::Unsigned => "MULU.L",
                };
                let destination = match high {
                    Some(high) => format!("D{high}:D{low}"),
                    None => format!("D{low}"),
                };
                format!(
                    "{mnemonic} {},{destination}",
                    self.operand(source, Size::Long)
                )
            }
            I::DIVL {
                source,
                quotient,
                remainder,
                wide,
                sign,
            } => {
                let stem = match sign {
                    Sign::Signed => "DIVS",
                    Sign::Unsigned => "DIVU",
                };
                let (mnemonic, destination) = match (wide, remainder == quotient) {
                    (true, _) => (format!("{stem}.L"), format!("D{remainder}:D{quotient}")),
                    (false, true) => (format!("{stem}.L"), format!("D{quotient}")),
                    (false, false) => (format!("{stem}L.L"), format!("D{remainder}:D{quotient}")),
                };
                format!(
                    "{mnemonic} {},{destination}",
                    self.operand(source, Size::Long)
                )
            }
            I::LINKL(register, displacement) => {
                format!("LINK.L {},#{}", name(register), *displacement as i32)
            }
            I::TRAPcc(condition, operand) => {
                let mnemonic = format!("TRAP{}", code(condition));
                match operand {
                    TrapOperand::None => mnemonic,
                    TrapOperand::Word(value) => {
                        self.one(&mnemonic, Size::Word, &Operand::Immediate(*value as u32))
                    }
                    TrapOperand::Long(value) => {
                        self.one(&mnemonic, Size::Long, &Operand::Immediate(*value))
                    }
                }
            }
        }
    }
}

/// Whether a line can write `instruction`: all but a `MOVEM` of no register,
/// which the 68000 runs, transferring nothing, and EASy68K cannot assemble,
/// as its register list is never empty, and a `MOVEC` of a control register
/// no processor s68k has, which has no name to write and raises an illegal
/// instruction exception.
fn is_written(instruction: &Instruction) -> bool {
    match instruction {
        Instruction::MOVEM {
            registers_mask: 0, ..
        } => false,
        Instruction::MOVEC { control, .. } => control.name().is_some(),
        _ => operands(instruction).iter().all(is_written_operand),
    }
}

/// Whether a line can write `operand`: all but an [`Operand::Extended`] whose
/// extension word no assembler writes ([`ExtensionWord::is_canonical`]), and
/// the full format's index with no displacement, which an assembler writes in
/// the brief format, as its displacement of 0 fits a byte.
fn is_written_operand(operand: &Operand) -> bool {
    match operand {
        Operand::Extended { word, .. } => {
            word.is_canonical()
                && !(word.is_full()
                    && !word.base_suppressed()
                    && !word.index_suppressed()
                    && word.indirection() == Indirection::None
                    && word.base_displacement_size() == DisplacementSize::Null)
        }
        _ => true,
    }
}

/// The Operands of `instruction` that can be [`Operand::Extended`]: those of
/// an effective address.
fn operands(instruction: &Instruction) -> Vec<Operand> {
    use Instruction::*;
    match *instruction {
        ADDA(a, ..)
        | SUBA(a, ..)
        | CMPA(a, ..)
        | MOVEA(a, ..)
        | ADDQ(_, a, _)
        | SUBQ(_, a, _)
        | ADDI(_, a, _)
        | SUBI(_, a, _)
        | ANDI(_, a, _)
        | ORI(_, a, _)
        | EORI(_, a, _)
        | CMPI(_, a, _)
        | DIVx(a, ..)
        | MULx(a, ..)
        | CLR(a, _)
        | LEA(a, _)
        | PEA(a)
        | NEG(a, _)
        | NEGX(a, _)
        | NBCD(a)
        | TST(a, _)
        | CMP(a, ..)
        | Scc(a, _)
        | NOT(a, _)
        | JSR(a)
        | JMP(a)
        | MOVEtoCCR(a)
        | MOVEfromCCR(a)
        | MOVEtoSR(a)
        | MOVEfromSR(a)
        | TAS(a)
        | CHK(a, _)
        | CHKL(a, _) => vec![a],
        MULL { source, .. } | DIVL { source, .. } => vec![source],
        CHK2(a, ..) | CMP2(a, ..) | CAS { target: a, .. } | BF { target: a, .. } => vec![a],
        MOVEM { target, .. } | MOVES { target, .. } | MOVEP { target, .. } => vec![target],
        MOVE(a, b, _)
        | ADD(a, b, _)
        | SUB(a, b, _)
        | OR(a, b, _)
        | AND(a, b, _)
        | EOR(a, b, _)
        | ASd(a, b, ..)
        | ROd(a, b, ..)
        | LSd(a, b, ..)
        | ROXd(a, b, ..)
        | BTST(a, b)
        | BCLR(a, b)
        | BSET(a, b)
        | BCHG(a, b) => vec![a, b],
        _ => vec![],
    }
}

/// The bits an immediate of `size` keeps.
fn mask(size: Size) -> u32 {
    match size {
        Size::Byte => 0xFF,
        Size::Word => 0xFFFF,
        Size::Long => 0xFFFF_FFFF,
    }
}

/// `B`, `W` or `L`.
fn suffix(size: Size) -> &'static str {
    match size {
        Size::Byte => "B",
        Size::Word => "W",
        Size::Long => "L",
    }
}

/// `D3`, `A7`.
fn name(register: &RegisterOperand) -> String {
    match register {
        RegisterOperand::Data(number) => format!("D{number}"),
        RegisterOperand::Address(number) => format!("A{number}"),
    }
}

/// The condition, upper case: `EQ`, `CC`, … The aliases `HS` and `LO` print
/// as `CC` and `CS`, and `DBRA` as `DBF`.
fn code(condition: &Condition) -> String {
    condition.suffix().to_uppercase()
}

/// A `movem` mask, `D0` its lowest bit, as a register list.
fn register_list(mask: u16) -> String {
    let register = |index: usize| {
        if index < 8 {
            format!("D{index}")
        } else {
            format!("A{}", index - 8)
        }
    };
    let mut groups = Vec::new();
    let mut index = 0;
    while index < 16 {
        if mask & (1 << index) == 0 {
            index += 1;
            continue;
        }
        let bank_end = if index < 8 { 8 } else { 16 };
        let mut end = index;
        while end + 1 < bank_end && mask & (1 << (end + 1)) != 0 {
            end += 1;
        }
        groups.push(if end == index {
            register(index)
        } else {
            format!("{}-{}", register(index), register(end))
        });
        index = end + 1;
    }
    groups.join("/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assembler::assemble_source;
    use crate::instructions::{IndexRegister, RegisterOperand};

    fn labels(address: u32) -> Option<String> {
        match address {
            0x1000 => Some("start".to_string()),
            0x1010 => Some("loop".to_string()),
            0x2000 => Some("table".to_string()),
            0xFFFF_8000 => Some("high".to_string()),
            _ => None,
        }
    }

    fn with_labels(words: &[u16], address: u32) -> String {
        let decoded = super::super::decode::decode_words(words, address, Processor::M68000)
            .expect("complete words");
        disassemble(&decoded.instruction, Some(&labels))
    }

    #[test]
    fn targets_and_addresses_print_as_the_names_the_symbols_give() {
        assert_eq!(with_labels(&[0x600E], 0x1000), "BRA.S loop");
        assert_eq!(with_labels(&[0x51C8, 0x000C], 0x1002), "DBF D0,loop");
        assert_eq!(with_labels(&[0x41FA, 0x0FFE], 0x1000), "LEA table(PC),A0");
        assert_eq!(
            with_labels(&[0x203B, 0x000E], 0x1000),
            "MOVE.L loop(PC,D0.W),D0"
        );
        assert_eq!(with_labels(&[0x4EB8, 0x2000], 0x1000), "JSR table.W");
        assert_eq!(with_labels(&[0x4EB8, 0x8000], 0x1000), "JSR high.W");
        assert_eq!(
            with_labels(&[0x2039, 0x0000, 0x2000], 0x1000),
            "MOVE.L table,D0"
        );
        // An address with no name is a number, and an immediate never a name.
        assert_eq!(with_labels(&[0x6000, 0x0100], 0x1000), "BRA.W $1102");
        assert_eq!(
            with_labels(&[0x203C, 0x0000, 0x2000], 0x1000),
            "MOVE.L #$2000,D0"
        );
    }

    #[test]
    fn a_run_of_bytes_is_read_one_instruction_after_another() {
        let bytes = [
            0x70, 0x05, // moveq #5,d0
            0x20, 0x3C, 0x00, 0x00, 0x00, 0x2A, // move.l #42,d0
            0x4A, 0xFB, // no instruction
            0xFF, 0xFF, 0xFF, 0xFF, // simhalt
            0x60, 0xF0, // bra.s to the start
            0x4E, 0x72, // stop, without its immediate
        ];
        let lines = disassemble_bytes(&bytes, 0x1000, Processor::M68000, Some(&labels));
        let read: Vec<(u32, u32, &str)> = lines
            .iter()
            .map(|line| (line.address, line.size, line.text.as_str()))
            .collect();
        assert_eq!(
            read,
            [
                (0x1000, 2, "MOVEQ #5,D0"),
                (0x1002, 6, "MOVE.L #$2A,D0"),
                (0x1008, 2, "DC.W $4AFB"),
                (0x100A, 4, "SIMHALT"),
                (0x100E, 2, "BRA.S start"),
                // The words ran out inside `stop`: the word that is there is
                // kept as data.
                (0x1010, 2, "DC.W $4E72"),
            ]
        );
        assert_eq!(lines[1].bytes, [0x20, 0x3C, 0x00, 0x00, 0x00, 0x2A]);
        // An odd byte at the end is a byte.
        let odd = disassemble_bytes(&[0x4E, 0x71, 0x4E], 0x1000, Processor::M68000, None);
        assert_eq!(odd[1].text, "DC.B $4E");
        assert_eq!((odd[1].address, odd[1].size), (0x1002, 1));
        // The processor decides what a word is.
        let ccr = [0x42, 0xC0];
        assert_eq!(
            disassemble_bytes(&ccr, 0, Processor::M68000, None)[0].text,
            "DC.W $42C0"
        );
        assert_eq!(
            disassemble_bytes(&ccr, 0, Processor::M68010, None)[0].text,
            "MOVE CCR,D0"
        );
        // A `movem` of no register is no line anybody can write, and prints as
        // its words (review finding 9).
        let empty = disassemble_bytes(
            &[0x48, 0x90, 0x00, 0x00, 0x4C, 0xFA, 0x00, 0x00, 0x00, 0x10],
            0x1000,
            Processor::M68000,
            None,
        );
        assert_eq!(
            empty
                .iter()
                .map(|line| (line.address, line.size, line.text.as_str()))
                .collect::<Vec<_>>(),
            [
                (0x1000, 4, "DC.W $4890,$0000"),
                (0x1004, 6, "DC.W $4CFA,$0000,$0010"),
            ]
        );
        let decoded = super::super::decode::decode_words(&[0x4890, 0x0000], 0x1000, Processor::M68000).unwrap();
        assert_eq!(disassemble(&decoded.instruction, None), "DC.W $4890,$0000");
    }

    /// The Assembler's instructions print the way the decoder's do, its
    /// normalisations visible: what it stored, not what was written.
    #[test]
    fn what_the_assembler_stored_prints_as_easy68k_writes_it() {
        let source = "\
    org $1000
start:
    add #1,d0
    move.l #-1,d1
    move.b #-1,d2
    moveq #-1,d3
    movem.l d0-d2/a0/a6,-(sp)
    movem.w (sp)+,d3
    lsl (a0)
    lsl.l #8,d0
    lea data(pc),a0
    bra start
    cmp.w #5,d0
    and.l #$ff,d1
    link a6,#-8
    rts
data: dc.l 0
";
        let assembly = assemble_source(source);
        let program = assembly.program.expect("the source assembles");
        let printed: Vec<String> = program
            .instructions()
            .iter()
            .map(|instruction| disassemble(&instruction.instruction, None))
            .collect();
        assert_eq!(
            printed,
            [
                // The quick forms EASy68K always chooses.
                "ADDQ.W #1,D0",
                "MOVEQ #-1,D1",
                // The Assembler keeps an immediate at its size.
                "MOVE.B #$FF,D2",
                "MOVEQ #-1,D3",
                "MOVEM.L D0-D2/A0/A6,-(A7)",
                "MOVEM.W (A7)+,D3",
                "LSL.W (A0)",
                "LSL.L #8,D0",
                // Every instruction at its real size: `data` is `$102c`.
                "LEA $102C(PC),A0",
                // A branch back to a Label above it, within a byte.
                "BRA.S $1000",
                // `cmp #n,dn` is a `cmp` and `and #n,dn` an `and`, as EASy68K
                // chooses them.
                "CMP.W #$5,D0",
                "AND.L #$FF,D1",
                "LINK A6,#-8",
                "RTS",
            ]
        );
        // The Program's code is those instructions' bytes, one after the
        // other, and reading them back prints the same lines.
        let image = &program.image()[0];
        let read: Vec<String> = disassemble_bytes(
            &image.bytes[..0x2C],
            image.address as u32,
            Processor::M68000,
            None,
        )
        .into_iter()
        .map(|line| line.text)
        .collect();
        assert_eq!(read, printed);
    }

    #[test]
    fn the_operands_print_by_the_rules() {
        let print = |instruction: Instruction| disassemble(&instruction, None);
        let d0 = Operand::Register(RegisterOperand::Data(0));
        assert_eq!(
            print(Instruction::MOVE(
                Operand::IndirectDisplacement {
                    offset: -8,
                    base: RegisterOperand::Address(6)
                },
                Operand::IndirectIndex {
                    base: RegisterOperand::Address(0),
                    offset: 0,
                    index: IndexRegister {
                        register: RegisterOperand::Address(2),
                        size: Size::Long
                    }
                },
                Size::Word
            )),
            "MOVE.W -8(A6),0(A0,A2.L)"
        );
        assert_eq!(
            print(Instruction::MOVEM {
                direction: TargetDirection::ToMemory,
                size: Size::Long,
                registers_mask: 0xFFFF,
                target: Operand::PreIndirect(7)
            }),
            "MOVEM.L D0-D7/A0-A7,-(A7)"
        );
        assert_eq!(
            print(Instruction::MOVEM {
                direction: TargetDirection::FromMemory,
                size: Size::Word,
                registers_mask: 0b1000_0001_1000_0001,
                target: Operand::Indirect(0)
            }),
            "MOVEM.W (A0),D0/D7/A0/A7"
        );
        assert_eq!(
            print(Instruction::ROXd(d0, d0, ShiftDirection::Right, Size::Byte)),
            "ROXR.B D0,D0"
        );
        assert_eq!(
            print(Instruction::EXT(
                RegisterOperand::Data(1),
                Size::Byte,
                Size::Long
            )),
            "EXTB.L D1"
        );
        assert_eq!(
            print(Instruction::Bcc(
                0x2000,
                Condition::LowOrSame,
                BranchSize::Long
            )),
            "BLS.L $2000"
        );
        assert_eq!(print(Instruction::Scc(d0, Condition::CarryClear)), "SCC D0");
        assert_eq!(print(Instruction::LINEF(0xF123)), "DC.W $F123");
        assert_eq!(print(Instruction::IllegalWord(0x0009)), "DC.W $0009");
    }
}
