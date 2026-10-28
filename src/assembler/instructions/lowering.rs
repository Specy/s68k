//! From a parsed Operand to an encoded one, and from a [`Family`] to an
//! [`Instruction`].
//!
//! This is the last step of the front end: the analyzer has already said that
//! the Operation is well formed, so nothing here diagnoses anything. It
//! evaluates the Expressions through [`Values`], picks the 68000 encoding the
//! line assembles as, and builds the [`Instruction`] the Interpreter runs and
//! the encoder ([`crate::machine::encode`]) writes as words.
//!
//! # Which encoding
//!
//! EASy68K's, always (the asm-editor's `docs/design/m68k-machine.md`, decision
//! 5, and `docs/research/easy68k-machine-behaviour.md` §13 and §15). Where an
//! instruction has several, EASy68K tries its forms in order and the first that
//! fits wins, so that order is written here family by family:
//!
//! * `cmp #n,dn` is `cmp <ea>,dn`, and `cmp #n,<memory>` is `cmpi`;
//! * `and #n,dn` and `or #n,dn` are `and <ea>,dn` and `or <ea>,dn`, and into
//!   memory they are `andi` and `ori`; `eor #n` is `eori`;
//! * `add #n,an` and `sub #n,an` are `adda` and `suba`, and into anything else
//!   `addi` and `subi`;
//! * `move` into an address register is `movea`, `cmp` into one `cmpa`, and
//!   two postincrements `cmpm`.
//!
//! Then the optimisations, which EASy68K always makes and which `#n.l` turns
//! off ([`super::choice`]): `move.l #n,dn` of −128 to 127 is `moveq`, and an
//! `add`, `addi`, `adda`, `sub`, `subi` or `suba` of 1 to 8 is `addq` or
//! `subq`. Each needs the immediate to be a back reference, and so do a short
//! branch and a short absolute address, which is why an Operand is lowered
//! with that fact beside it ([`Lowered`]).
//!
//! An immediate is stored at the size the instruction reads it at, zero
//! extended, and `exg an,dn` as `exg dn,an`: one value per encoding, so that
//! decoding the words gives back the Instruction that was encoded
//! ([`Instruction`], "One value per encoding").
//!
//! [`lower`] has one arm per [`Family`] and no catch-all, so a Family that
//! gains a variant does not compile until it is given an encoding.

use crate::assembler::ast::{self, SizeSuffix};

use super::choice;
use super::encoded::{
    BranchSize, IndexRegister, Instruction, Operand, RegisterOperand, ShiftDirection, Size,
    TargetDirection, EXTENSION_WORD_OFFSET,
};
use super::table::{BitOperation, Family, ImmediateKind, LogicalKind, ShiftKind, ShiftWay};
use crate::machine::Processor;

/// What the lowering needs to know that the tree does not hold: the value of
/// every Expression in it, whether it is a back reference, and where the
/// instruction sits.
///
/// The analyzer implements it (`analyzer::Context`). `None` from
/// [`value_of`](Values::value_of) means the value cannot be known — an
/// undefined Symbol, a division by zero — in which case the Diagnostic has
/// already been raised and nothing is lowered.
pub trait Values {
    /// The value of one Expression, in 64 bits.
    fn value_of(&self, expression: &ast::Expr) -> Option<i64>;

    /// Whether every Symbol in the Expression is defined above the line, or on
    /// it: EASy68K's back reference, which is what a short branch, a short
    /// address and the quick forms need ([`super::choice`]). An Expression of
    /// numbers alone is one.
    fn is_back_reference(&self, expression: &ast::Expr) -> bool;

    /// The address the instruction being lowered is laid out at.
    ///
    /// A PC-relative Operand needs it, because `(pc,d1.w)`, which writes no
    /// address, names its own extension word, and so does a branch, whose
    /// size depends on how far it reaches.
    fn instruction_address(&self) -> i64;

    /// The processor the Program is written for, which decides what a branch
    /// written `.l` is ([`choice::branch_size`]). The 68000 unless said.
    fn processor(&self) -> Processor {
        Processor::M68000
    }
}

/// One Operand lowered, with the two facts about how it was written that
/// EASy68K's choices of encoding read ([`super::choice`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Lowered {
    /// The Operand itself.
    pub operand: Operand,
    /// Whether every Symbol in its Expression is defined above the line, which
    /// is always so for an Operand that has no Expression, and for one of
    /// numbers alone.
    pub back_reference: bool,
    /// Whether it is an immediate written `#n.l`, which keeps the long form of
    /// `move.l`, `add` and `sub`.
    pub long: bool,
}

impl Lowered {
    /// An Operand written with numbers alone and no `#n.l`, which is what a
    /// hand-built line is: every choice is then made by its value.
    pub fn known(operand: Operand) -> Self {
        Self {
            operand,
            back_reference: true,
            long: false,
        }
    }
}

/// What the lowering reads from the line beyond its Operands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Site {
    /// The size suffix the Operation was written with, `None` when it has
    /// none: what tells `move.l #1,d0`, which is a `moveq`, from `move #1,d0`,
    /// which is a word, and a branch written `.s` from one whose size is
    /// chosen.
    pub written: Option<SizeSuffix>,
    /// The address the instruction is laid out at, which is what a branch's
    /// displacement is measured from.
    pub address: u32,
    /// The processor the Program is written for, which decides what a branch
    /// written `.l` is.
    pub processor: Processor,
}

/// The encoded form of one parsed Operand, or `None` when a value in it cannot
/// be worked out.
///
/// `extension_word` is the address of the instruction's extension word that a
/// PC-relative Operand's displacement would sit in ([`extension_word_offset`]
/// past the instruction), which `(pc,d1.w)` reaches since it writes no address.
///
/// A register list becomes its mask as an [`Operand::Immediate`], which is how
/// 1.4.2 carried one and what [`Family::Movem`]'s lowering reads back. A
/// special register has no encoded form at all: `sr` and `ccr` are part of the
/// [`Instruction`] that names them and `usp` is not assembled.
///
/// An absolute address gets its width here, by EASy68K's rule
/// ([`choice::address_width`]): forced by `.w` or `.l`, else a word for a back
/// reference a word holds, and a long otherwise.
pub fn lower_operand(
    operand: &ast::Operand,
    values: &dyn Values,
    extension_word: i64,
) -> Option<Lowered> {
    let mut back_reference = true;
    let mut long = false;
    let lowered = match operand {
        ast::Operand::Immediate {
            value,
            long: written_long,
            ..
        } => {
            back_reference = values.is_back_reference(value);
            long = *written_long;
            Operand::Immediate(values.value_of(value)? as u32)
        }
        ast::Operand::DataRegisterDirect { register, .. } => {
            Operand::Register(RegisterOperand::Data(register.number))
        }
        ast::Operand::AddressRegisterDirect { register, .. } => {
            Operand::Register(RegisterOperand::Address(register.number))
        }
        ast::Operand::Indirect { register, .. } => Operand::Indirect(register.number),
        ast::Operand::Postincrement { register, .. } => Operand::PostIndirect(register.number),
        ast::Operand::Predecrement { register, .. } => Operand::PreIndirect(register.number),
        ast::Operand::Displacement {
            displacement, base, ..
        } => Operand::IndirectDisplacement {
            offset: values.value_of(displacement)? as i16 as i32,
            base: RegisterOperand::Address(base.number),
        },
        ast::Operand::Index {
            displacement,
            base,
            index,
            ..
        } => Operand::IndirectIndex {
            base: RegisterOperand::Address(base.number),
            offset: match displacement {
                Some(displacement) => values.value_of(displacement)? as i8 as i32,
                None => 0,
            },
            index: lower_index_register(index),
        },
        // An address is 32 bits wide, so a negative value is the address its
        // two's complement names and never a 64-bit number: `as u32` first,
        // which is what 1.4.2 stored as well.
        ast::Operand::Absolute { value, size, .. } => {
            back_reference = values.is_back_reference(value);
            let address = values.value_of(value)? as u32;
            Operand::Absolute {
                address: address as usize,
                width: choice::address_width(*size, back_reference, address),
            }
        }
        ast::Operand::RegisterList { items, .. } => Operand::Immediate(register_mask(items) as u32),
        // A PC-relative Operand is written as the address it reaches and
        // stored as that address. The analyzer has already refused one whose
        // distance from the extension word does not fit its field
        // ([`pc_relative_offset`]).
        ast::Operand::PcDisplacement { displacement, .. } => {
            back_reference = values.is_back_reference(displacement);
            Operand::PcDisplacement {
                target: values.value_of(displacement)? as u32,
            }
        }
        ast::Operand::PcIndex {
            displacement,
            index,
            ..
        } => Operand::PcIndex {
            target: match displacement {
                Some(displacement) => {
                    back_reference = values.is_back_reference(displacement);
                    values.value_of(displacement)? as u32
                }
                // `(pc,d1.w)` writes no address at all: the displacement is
                // zero, as it is in `(a0,d1.w)` (`docs/grammar.md` 2.5), so it
                // reaches its own extension word.
                None => extension_word as u32,
            },
            index: lower_index_register(index),
        },
        ast::Operand::SpecialRegister { .. } => return None,
    };
    Some(Lowered {
        operand: lowered,
        back_reference,
        long,
    })
}

/// How far past an instruction the extension word of its PC-relative Operand
/// sits: [`EXTENSION_WORD_OFFSET`], the word after the operation word, but for
/// `movem`, whose register mask comes first, and a bit instruction with a
/// static bit number, whose number does. Those two are 4.
///
/// The 68000 adds the displacement to the address of the word that holds it,
/// so this is what a PC-relative distance is measured from, both when the
/// analyzer checks it and when `(pc,d1.w)` is given the address it reaches.
/// No other instruction puts a word before a PC-relative Operand's: a
/// PC-relative Operand is read and never written, so it is the first Operand
/// everywhere else, or follows a register.
pub fn extension_word_offset(family: Family, operands: &[ast::Operand]) -> i64 {
    match (family, operands.first()) {
        (Family::Movem, _) | (Family::Bit(_), Some(ast::Operand::Immediate { .. })) => 4,
        _ => EXTENSION_WORD_OFFSET as i64,
    }
}

/// The distance from a PC-relative Operand's extension word, `extension_offset`
/// past the instruction being lowered ([`extension_word_offset`]), to the
/// address it names: the displacement the 68000 encodes, which the analyzer
/// holds to the width of its field.
pub fn pc_relative_offset(
    values: &dyn Values,
    address: &ast::Expr,
    extension_offset: i64,
) -> Option<i64> {
    let target = values.value_of(address)?;
    Some(target - (values.instruction_address() + extension_offset))
}

/// The index register of an indexed Operand. No size written means `.w`, the
/// 68000's own default; a byte index does not exist and the analyzer refuses
/// one before this is reached.
fn lower_index_register(index: &ast::IndexRegister) -> IndexRegister {
    IndexRegister {
        register: lower_register(index.register),
        size: match index.size {
            Some(SizeSuffix::Long) => Size::Long,
            _ => Size::Word,
        },
    }
}

/// One general register, in the encoded form.
fn lower_register(register: ast::Register) -> RegisterOperand {
    match register.kind {
        ast::RegisterKind::Data => RegisterOperand::Data(register.number),
        ast::RegisterKind::Address => RegisterOperand::Address(register.number),
    }
}

/// The `movem` mask of a register list: one bit a register, `d0` the lowest and
/// `a7` the highest, ranges filled in.
pub fn register_mask(items: &[ast::RegisterListItem]) -> u16 {
    let mut mask = 0u16;
    for item in items {
        match item {
            ast::RegisterListItem::Single { register, .. } => mask |= 1 << register.mask_index(),
            ast::RegisterListItem::Range { from, to, .. } => {
                for index in from.mask_index()..=to.mask_index() {
                    mask |= 1 << index;
                }
            }
        }
    }
    mask
}

/// The size an instruction works at: what the source wrote, or the default of
/// the [`Form`](super::table::Form) it matched.
///
/// `.s` is a branch displacement and not an operand size, so it answers `None`
/// like an absent suffix on an unsized instruction.
pub fn lower_size(size: Option<SizeSuffix>, default: Option<Size>) -> Option<Size> {
    match size {
        Some(SizeSuffix::Byte) => Some(Size::Byte),
        Some(SizeSuffix::Word) => Some(Size::Word),
        Some(SizeSuffix::Long) => Some(Size::Long),
        Some(SizeSuffix::Short) | None => default,
    }
}

/// The [`Instruction`] one checked Operation encodes to.
///
/// This is what the analyzer calls once it has nothing left to say about the
/// line. `written` is the size suffix the Operation was written with and `size`
/// the size it works at, already defaulted by [`lower_size`]. It answers the
/// shapes that name a half of the status register first — `sr` and `ccr` are
/// Operands the encoded [`Operand`] has no form for, so they are read from the
/// tree rather than lowered — and everything else by lowering each Operand and
/// handing the result to [`lower`].
pub fn lower_operation(
    family: Family,
    written: Option<SizeSuffix>,
    size: Option<Size>,
    operands: &[ast::Operand],
    values: &dyn Values,
) -> Option<Instruction> {
    let address = values.instruction_address();
    let extension_word = address + extension_word_offset(family, operands);
    if let Some(instruction) = lower_status_register(family, operands, values, extension_word) {
        return Some(instruction);
    }
    let lowered: Vec<Lowered> = operands
        .iter()
        .map(|operand| lower_operand(operand, values, extension_word))
        .collect::<Option<Vec<_>>>()?;
    let site = Site {
        written,
        address: address as u32,
        processor: values.processor(),
    };
    lower(family, size, &lowered, &site)
}

/// The Operations that read or write `sr` or `ccr`, or `None` when this is not
/// one of them.
///
/// `move <ea>,ccr` and `move <ea>,sr` are a word of which the first takes the
/// low byte; `move sr,<ea>` and `move ccr,<ea>` write one; `andi`, `ori` and
/// `eori` take a byte into `ccr` and a word into `sr`
/// (`Reference/68ks4d.htm`, `Reference/68ks6b.htm`). Every other Mnemonic
/// answers `None` here and is lowered the ordinary way; so does `usp`, which is
/// not implemented at all.
fn lower_status_register(
    family: Family,
    operands: &[ast::Operand],
    values: &dyn Values,
    extension_word: i64,
) -> Option<Instruction> {
    let [first, second] = operands else {
        return None;
    };
    use ast::SpecialRegister::{Ccr, Sr};
    let lowered =
        |operand| lower_operand(operand, values, extension_word).map(|lowered| lowered.operand);
    match (
        family,
        half_of_the_status_register(first),
        half_of_the_status_register(second),
    ) {
        (Family::Move, None, Some(Ccr)) => {
            Some(Instruction::MOVEtoCCR(at_size(lowered(first)?, Size::Word)))
        }
        (Family::Move, None, Some(Sr)) => {
            Some(Instruction::MOVEtoSR(at_size(lowered(first)?, Size::Word)))
        }
        (Family::Move, Some(Ccr), None) => Some(Instruction::MOVEfromCCR(lowered(second)?)),
        (Family::Move, Some(Sr), None) => Some(Instruction::MOVEfromSR(lowered(second)?)),
        (Family::Immediate(kind), None, Some(destination)) => {
            let ast::Operand::Immediate { value, .. } = first else {
                return None;
            };
            let value = values.value_of(value)?;
            Some(match (kind, destination) {
                (ImmediateKind::And, Ccr) => Instruction::ANDItoCCR(value as u8),
                (ImmediateKind::Or, Ccr) => Instruction::ORItoCCR(value as u8),
                (ImmediateKind::Eor, Ccr) => Instruction::EORItoCCR(value as u8),
                (ImmediateKind::And, Sr) => Instruction::ANDItoSR(value as u16),
                (ImmediateKind::Or, Sr) => Instruction::ORItoSR(value as u16),
                (ImmediateKind::Eor, Sr) => Instruction::EORItoSR(value as u16),
                // `addi`, `subi` and `cmpi` have no status-register form and no
                // row of the table offers one, so the analyzer has already
                // refused the line.
                _ => return None,
            })
        }
        _ => None,
    }
}

/// The half of the status register an Operand names, when it names one.
///
/// `usp` is not one of them: it is not implemented at all, and the analyzer has
/// already said so, so it answers `None` and is lowered by nothing.
fn half_of_the_status_register(operand: &ast::Operand) -> Option<ast::SpecialRegister> {
    match operand {
        ast::Operand::SpecialRegister {
            register: register @ (ast::SpecialRegister::Sr | ast::SpecialRegister::Ccr),
            ..
        } => Some(*register),
        _ => None,
    }
}

/// The [`Instruction`] a Family encodes from its Operands.
///
/// `size` is the size the instruction works at, already defaulted by
/// [`lower_size`]; it is `None` exactly for the unsized families. `operands`
/// are the Operands of the matched Form, already lowered, and `site` is what
/// the Operation wrote and where it is.
///
/// `None` means the shape is not one this Family encodes, which the analyzer
/// has already reported: lowering never raises a Diagnostic of its own.
pub fn lower(
    family: Family,
    size: Option<Size>,
    operands: &[Lowered],
    site: &Site,
) -> Option<Instruction> {
    let operand = |index: usize| operands.get(index).map(|lowered| lowered.operand);
    match family {
        Family::Move => lower_move(size?, operands, site),
        Family::Movea => Some(Instruction::MOVEA(
            at_size(operand(0)?, size?),
            register(operand(1))?,
            size?,
        )),
        Family::Movem => lower_movem(size?, operand(0)?, operand(1)?),
        Family::Moveq => Some(Instruction::MOVEQ(
            immediate(operand(0))? as u8,
            register(operand(1))?,
        )),
        Family::AddSub { subtract } => lower_add_sub(subtract, size?, operands),
        Family::AddSubExtended { subtract } => {
            let (source, destination) = (operand(0)?, operand(1)?);
            Some(match subtract {
                false => Instruction::ADDX(source, destination, size?),
                true => Instruction::SUBX(source, destination, size?),
            })
        }
        Family::AddSubDecimal { subtract } => {
            // No size reaches the encoded instruction: `abcd` and `sbcd` work
            // on one byte and the table gives them no other size, so the
            // Instruction carries none (`Reference/68ks8e.htm`).
            let (source, destination) = (operand(0)?, operand(1)?);
            Some(match subtract {
                false => Instruction::ABCD(source, destination),
                true => Instruction::SBCD(source, destination),
            })
        }
        Family::AddSubAddress { subtract } => {
            let (source, destination) = (operands.first()?, register(operand(1))?);
            Some(into_address_register(subtract, size?, source, destination))
        }
        Family::AddSubQuick { subtract } => {
            let (count, destination) = (immediate(operand(0))? as u8, operand(1)?);
            Some(match subtract {
                false => Instruction::ADDQ(count, destination, size?),
                true => Instruction::SUBQ(count, destination, size?),
            })
        }
        Family::Immediate(kind) => lower_immediate(kind, size?, operands),
        Family::Cmp => lower_cmp(size?, operand(0)?, operand(1)?),
        Family::Cmpa => Some(Instruction::CMPA(
            at_size(operand(0)?, size?),
            register(operand(1))?,
            size?,
        )),
        Family::Cmpm => Some(Instruction::CMPM(operand(0)?, operand(1)?, size?)),
        Family::Logical(kind) => lower_logical(kind, size?, operand(0)?, operand(1)?),
        Family::Divide(sign) => Some(Instruction::DIVx(
            at_size(operand(0)?, Size::Word),
            register(operand(1))?,
            sign,
        )),
        Family::Multiply(sign) => Some(Instruction::MULx(
            at_size(operand(0)?, Size::Word),
            register(operand(1))?,
            sign,
        )),
        Family::Clr => Some(Instruction::CLR(operand(0)?, size?)),
        Family::Neg => Some(Instruction::NEG(operand(0)?, size?)),
        Family::NegExtended => Some(Instruction::NEGX(operand(0)?, size?)),
        Family::NegDecimal => Some(Instruction::NBCD(operand(0)?)),
        Family::Not => Some(Instruction::NOT(operand(0)?, size?)),
        Family::Tst => Some(Instruction::TST(operand(0)?, size?)),
        Family::Ext => {
            // `ext.w` reads a byte and writes a word; `ext.l` reads a word and
            // writes a long.
            let (read, written) = match size? {
                Size::Word => (Size::Byte, Size::Word),
                Size::Long => (Size::Word, Size::Long),
                Size::Byte => return None,
            };
            Some(Instruction::EXT(register(operand(0))?, read, written))
        }
        Family::ExtByteToLong => Some(Instruction::EXT(
            register(operand(0))?,
            Size::Byte,
            Size::Long,
        )),
        Family::Swap => Some(Instruction::SWAP(register(operand(0))?)),
        Family::Exg => Some(lower_exg(register(operand(0))?, register(operand(1))?)),
        Family::Lea => Some(Instruction::LEA(operand(0)?, register(operand(1))?)),
        Family::Pea => Some(Instruction::PEA(operand(0)?)),
        Family::Link => Some(Instruction::LINK(
            register(operand(0))?,
            immediate(operand(1))?,
        )),
        Family::Unlk => Some(Instruction::UNLK(register(operand(0))?)),
        Family::Jmp => Some(Instruction::JMP(operand(0)?)),
        Family::Jsr => Some(Instruction::JSR(operand(0)?)),
        Family::Bra => {
            let (target, size) = branch(operands.first()?, site)?;
            Some(Instruction::BRA(target, size))
        }
        Family::Bsr => {
            let (target, size) = branch(operands.first()?, site)?;
            Some(Instruction::BSR(target, size))
        }
        Family::Bcc(condition) => {
            let (target, size) = branch(operands.first()?, site)?;
            Some(Instruction::Bcc(target, condition, size))
        }
        Family::Scc(condition) => Some(Instruction::Scc(operand(0)?, condition)),
        Family::DBcc(condition) => Some(Instruction::DBcc(
            register(operand(0))?,
            address(operand(1))?,
            condition,
        )),
        Family::Shift(kind, way) => lower_shift(kind, way, size?, operands),
        Family::Bit(operation) => {
            // An immediate destination is the dynamic form's, `btst d0,#5`,
            // and is read as a byte.
            let (bit, destination) = (operand(0)?, at_size(operand(1)?, Size::Byte));
            Some(match operation {
                BitOperation::Test => Instruction::BTST(bit, destination),
                BitOperation::Set => Instruction::BSET(bit, destination),
                BitOperation::Clear => Instruction::BCLR(bit, destination),
                BitOperation::Change => Instruction::BCHG(bit, destination),
            })
        }
        Family::Trap => Some(Instruction::TRAP(immediate(operand(0))? as u8)),
        Family::Rts => Some(Instruction::RTS),
        Family::Nop => Some(Instruction::NOP),
        Family::Movep => lower_movep(size?, operand(0)?, operand(1)?),
        Family::Tas => Some(Instruction::TAS(operand(0)?)),
        Family::Rtr => Some(Instruction::RTR),
        Family::Chk => Some(Instruction::CHK(
            at_size(operand(0)?, Size::Word),
            register(operand(1))?,
        )),
        Family::Trapv => Some(Instruction::TRAPV),
        Family::Illegal => Some(Instruction::ILLEGAL),
    }
}

/// `movep`, in whichever direction the data register stands.
///
/// The other Operand is a displacement one — the only Addressing mode `movep`
/// takes (`Reference/68ks4g.htm`) — and the analyzer has already said so, which
/// is why nothing here diagnoses a target that is not one.
fn lower_movep(size: Size, first: Operand, second: Operand) -> Option<Instruction> {
    let (register, target, direction) = match (first, second) {
        (Operand::Register(register @ RegisterOperand::Data(_)), target) => {
            (register, target, TargetDirection::ToMemory)
        }
        (target, Operand::Register(register @ RegisterOperand::Data(_))) => {
            (register, target, TargetDirection::FromMemory)
        }
        _ => return None,
    };
    Some(Instruction::MOVEP {
        direction,
        size,
        register,
        target,
    })
}

/// `move`: a `movea` into an address register, a `moveq` when EASy68K makes
/// one ([`choice::moveq_value`]), and a `move` otherwise.
fn lower_move(size: Size, operands: &[Lowered], site: &Site) -> Option<Instruction> {
    let (source, destination) = (operands.first()?, operands.get(1)?);
    Some(match destination.operand {
        Operand::Register(register @ RegisterOperand::Address(_)) => {
            Instruction::MOVEA(at_size(source.operand, size), register, size)
        }
        Operand::Register(register @ RegisterOperand::Data(_)) => match (source.operand, size) {
            (Operand::Immediate(value), Size::Long) => {
                match choice::moveq_value(site.written, source.back_reference, source.long, value) {
                    Some(value) => Instruction::MOVEQ(value, register),
                    None => Instruction::MOVE(source.operand, destination.operand, size),
                }
            }
            _ => Instruction::MOVE(at_size(source.operand, size), destination.operand, size),
        },
        _ => Instruction::MOVE(at_size(source.operand, size), destination.operand, size),
    })
}

/// The count of the `addq` or `subq` an immediate Operand makes, when EASy68K
/// makes one ([`choice::quick_count`]).
fn quick(operand: &Lowered) -> Option<u8> {
    match operand.operand {
        Operand::Immediate(value) => {
            choice::quick_count(operand.back_reference, operand.long, value)
        }
        _ => None,
    }
}

/// `add` and `sub`, in EASy68K's order of forms (`addfl` in its
/// `INSTTABL.CPP`): into an address register an `adda`/`suba`; an immediate
/// into anything else an `addi`/`subi`; then `<ea>,dn` and `dn,<ea>`. Each
/// immediate one is an `addq`/`subq` when the quick form applies.
///
/// An address register into memory has no encoding, and the analyzer has
/// refused it.
fn lower_add_sub(subtract: bool, size: Size, operands: &[Lowered]) -> Option<Instruction> {
    let (source, destination) = (operands.first()?, operands.get(1)?);
    Some(match (source.operand, destination.operand) {
        (_, Operand::Register(register @ RegisterOperand::Address(_))) => {
            into_address_register(subtract, size, source, register)
        }
        (Operand::Immediate(value), destination) => match (quick(source), subtract) {
            (Some(count), false) => Instruction::ADDQ(count, destination, size),
            (Some(count), true) => Instruction::SUBQ(count, destination, size),
            (None, false) => Instruction::ADDI(at_size_value(value, size), destination, size),
            (None, true) => Instruction::SUBI(at_size_value(value, size), destination, size),
        },
        (source, destination @ Operand::Register(RegisterOperand::Data(_)))
        | (source @ Operand::Register(RegisterOperand::Data(_)), destination) => match subtract {
            false => Instruction::ADD(source, destination, size),
            true => Instruction::SUB(source, destination, size),
        },
        _ => return None,
    })
}

/// `adda` or `suba`, which is an `addq` or a `subq` of 1 to 8: EASy68K makes
/// the quick form of every add or subtract of an immediate into an address
/// register (`arithReg` in its `BUILD.CPP`).
fn into_address_register(
    subtract: bool,
    size: Size,
    source: &Lowered,
    destination: RegisterOperand,
) -> Instruction {
    let into = Operand::Register(destination);
    match (quick(source), subtract) {
        (Some(count), false) => Instruction::ADDQ(count, into, size),
        (Some(count), true) => Instruction::SUBQ(count, into, size),
        (None, false) => Instruction::ADDA(at_size(source.operand, size), destination, size),
        (None, true) => Instruction::SUBA(at_size(source.operand, size), destination, size),
    }
}

/// The six instructions that take an immediate and write it somewhere, of
/// which `addi` and `subi` are `addq` and `subq` when the quick form applies:
/// EASy68K converts the written-out forms too.
fn lower_immediate(kind: ImmediateKind, size: Size, operands: &[Lowered]) -> Option<Instruction> {
    let source = operands.first()?;
    let (value, destination) = (
        at_size_value(immediate(Some(source.operand))?, size),
        operands.get(1)?.operand,
    );
    Some(match (kind, quick(source)) {
        (ImmediateKind::Add, Some(count)) => Instruction::ADDQ(count, destination, size),
        (ImmediateKind::Sub, Some(count)) => Instruction::SUBQ(count, destination, size),
        (ImmediateKind::Add, None) => Instruction::ADDI(value, destination, size),
        (ImmediateKind::Sub, None) => Instruction::SUBI(value, destination, size),
        (ImmediateKind::And, _) => Instruction::ANDI(value, destination, size),
        (ImmediateKind::Or, _) => Instruction::ORI(value, destination, size),
        (ImmediateKind::Eor, _) => Instruction::EORI(value, destination, size),
        (ImmediateKind::Cmp, _) => Instruction::CMPI(value, destination, size),
    })
}

/// `cmp`, in EASy68K's order of forms (`cmpfl`): into a data register a `cmp`,
/// an immediate included; into an address register a `cmpa`; an immediate
/// into memory a `cmpi`; two postincrements a `cmpm`.
fn lower_cmp(size: Size, source: Operand, destination: Operand) -> Option<Instruction> {
    Some(match (source, destination) {
        (_, Operand::Register(register @ RegisterOperand::Data(_))) => {
            Instruction::CMP(at_size(source, size), register, size)
        }
        (_, Operand::Register(register @ RegisterOperand::Address(_))) => {
            Instruction::CMPA(at_size(source, size), register, size)
        }
        (Operand::Immediate(value), _) => {
            Instruction::CMPI(at_size_value(value, size), destination, size)
        }
        (Operand::PostIndirect(_), Operand::PostIndirect(_)) => {
            Instruction::CMPM(source, destination, size)
        }
        _ => return None,
    })
}

/// `and`, `or` and `eor`, in EASy68K's order of forms (`andfl`, `orfl`,
/// `eorfl`): into a data register `and <ea>,dn`, an immediate included; an
/// immediate into memory `andi`; a data register into memory `and dn,<ea>`.
/// `eor` has no `<ea>,dn` form, so an immediate is `eori` wherever it goes.
fn lower_logical(
    kind: LogicalKind,
    size: Size,
    source: Operand,
    destination: Operand,
) -> Option<Instruction> {
    let into_a_data_register = matches!(destination, Operand::Register(RegisterOperand::Data(_)));
    Some(match (kind, source) {
        (LogicalKind::And, _) if into_a_data_register => {
            Instruction::AND(at_size(source, size), destination, size)
        }
        (LogicalKind::Or, _) if into_a_data_register => {
            Instruction::OR(at_size(source, size), destination, size)
        }
        (LogicalKind::And, Operand::Immediate(value)) => {
            Instruction::ANDI(at_size_value(value, size), destination, size)
        }
        (LogicalKind::Or, Operand::Immediate(value)) => {
            Instruction::ORI(at_size_value(value, size), destination, size)
        }
        (LogicalKind::Eor, Operand::Immediate(value)) => {
            Instruction::EORI(at_size_value(value, size), destination, size)
        }
        (_, Operand::Register(RegisterOperand::Data(_))) => match kind {
            LogicalKind::And => Instruction::AND(source, destination, size),
            LogicalKind::Or => Instruction::OR(source, destination, size),
            LogicalKind::Eor => Instruction::EOR(source, destination, size),
        },
        _ => return None,
    })
}

/// `exg`, with the data register first when it swaps one with an address
/// register: the 68000 has one encoding of the pair, with the data register in
/// bits 11–9, so `exg a1,d0` and `exg d0,a1` are the same instruction
/// (`exg` in EASy68K's `BUILD.CPP`).
fn lower_exg(first: RegisterOperand, second: RegisterOperand) -> Instruction {
    match (first, second) {
        (RegisterOperand::Address(_), RegisterOperand::Data(_)) => Instruction::EXG(second, first),
        _ => Instruction::EXG(first, second),
    }
}

/// The target of a `bra`, `bsr` or `Bcc` and the size of its displacement,
/// chosen by EASy68K's rule ([`choice::branch_size`]) from the size the
/// Operation was written with and how far the target is.
fn branch(target: &Lowered, site: &Site) -> Option<(u32, BranchSize)> {
    let address = address(Some(target.operand))?;
    let displacement = choice::branch_displacement(site.address, address);
    Some((
        address,
        choice::branch_size(
            site.written,
            target.back_reference,
            displacement,
            site.processor,
        ),
    ))
}

/// `movem`, in whichever direction the register list stands.
///
/// The mask of a list that goes into a predecrement is stored reversed, because
/// that is the order the 68000 writes the registers in; the fixture printer
/// undoes it (`tests/corpus/README.md`, "`movem` register lists").
fn lower_movem(size: Size, first: Operand, second: Operand) -> Option<Instruction> {
    let (mut mask, target, direction) = match (first, second) {
        (Operand::Immediate(mask), target) => (mask as u16, target, TargetDirection::ToMemory),
        (Operand::Register(register), target) => {
            (1 << register.to_index(), target, TargetDirection::ToMemory)
        }
        (target, Operand::Immediate(mask)) => (mask as u16, target, TargetDirection::FromMemory),
        (target, Operand::Register(register)) => (
            1 << register.to_index(),
            target,
            TargetDirection::FromMemory,
        ),
        _ => return None,
    };
    if let Operand::PreIndirect(_) = target {
        mask = mask.reverse_bits();
    }
    Some(Instruction::MOVEM {
        registers_mask: mask,
        target,
        direction,
        size,
    })
}

/// A shift or rotate. The one-Operand memory form shifts by one place, which is
/// stored as the count it means (`tests/corpus/README.md`, "Normalisations").
fn lower_shift(
    kind: ShiftKind,
    way: ShiftWay,
    size: Size,
    operands: &[Lowered],
) -> Option<Instruction> {
    let (count, target) = match operands {
        [count, target] => (count.operand, target.operand),
        [target] => (Operand::Immediate(1), target.operand),
        _ => return None,
    };
    let direction = match way {
        ShiftWay::Left => ShiftDirection::Left,
        ShiftWay::Right => ShiftDirection::Right,
    };
    Some(match kind {
        ShiftKind::Arithmetic => Instruction::ASd(count, target, direction, size),
        ShiftKind::Logical => Instruction::LSd(count, target, direction, size),
        ShiftKind::Rotate => Instruction::ROd(count, target, direction, size),
        ShiftKind::RotateExtend => Instruction::ROXd(count, target, direction, size),
    })
}

/// An Operand as the instruction reads it at `size`: an immediate keeps the
/// bits of that size and no others, zero extended, which is what the 68000
/// reads from its extension words (`$FF` for `move.b #-1,d0`). Every other
/// Operand is what it is.
fn at_size(operand: Operand, size: Size) -> Operand {
    match operand {
        Operand::Immediate(value) => Operand::Immediate(at_size_value(value, size)),
        other => other,
    }
}

/// The bits of `value` an immediate of `size` holds.
fn at_size_value(value: u32, size: Size) -> u32 {
    match size {
        Size::Byte => value & 0xFF,
        Size::Word => value & 0xFFFF,
        Size::Long => value,
    }
}

/// The register an Operand is, when it is one.
fn register(operand: Option<Operand>) -> Option<RegisterOperand> {
    match operand? {
        Operand::Register(register) => Some(register),
        _ => None,
    }
}

/// The value an immediate Operand carries, when it is one.
fn immediate(operand: Option<Operand>) -> Option<u32> {
    match operand? {
        Operand::Immediate(value) => Some(value),
        _ => None,
    }
}

/// The address an absolute Operand names, when it is one. Branch and jump
/// targets are resolved to addresses before they get here.
fn address(operand: Option<Operand>) -> Option<u32> {
    match operand? {
        Operand::Absolute { address, .. } => Some(address as u32),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assembler::ast::{Register, RegisterKind, RegisterListItem};
    use crate::assembler::source::Span;

    fn span() -> Span {
        Span::new(0, 2)
    }

    /// `lower` over Operands written with numbers alone, unsized, at `$1000`.
    fn lowered(family: Family, size: Option<Size>, operands: &[Operand]) -> Option<Instruction> {
        let operands: Vec<Lowered> = operands.iter().copied().map(Lowered::known).collect();
        let site = Site {
            written: None,
            address: 0x1000,
            processor: Processor::M68000,
        };
        lower(family, size, &operands, &site)
    }

    fn data(number: u8) -> Register {
        Register {
            kind: RegisterKind::Data,
            number,
            span: span(),
        }
    }

    fn address_register(number: u8) -> Register {
        Register {
            kind: RegisterKind::Address,
            number,
            span: span(),
        }
    }

    #[test]
    fn a_register_list_becomes_a_mask() {
        let items = vec![
            RegisterListItem::Range {
                from: data(0),
                to: data(2),
                span: span(),
            },
            RegisterListItem::Single {
                register: address_register(6),
                span: span(),
            },
        ];
        assert_eq!(register_mask(&items), 0b0100_0000_0000_0111);
    }

    #[test]
    fn a_range_may_cross_from_d7_into_a0() {
        let items = vec![RegisterListItem::Range {
            from: data(7),
            to: address_register(0),
            span: span(),
        }];
        assert_eq!(register_mask(&items), 0b0000_0001_1000_0000);
    }

    #[test]
    fn move_into_an_address_register_is_a_movea() {
        let operands = [
            Operand::Register(RegisterOperand::Data(0)),
            Operand::Register(RegisterOperand::Address(1)),
        ];
        assert!(matches!(
            lowered(Family::Move, Some(Size::Long), &operands),
            Some(Instruction::MOVEA(..))
        ));
    }

    #[test]
    fn add_and_cmp_choose_the_forms_easy68k_chooses() {
        let immediate_into_address = [
            Operand::Immediate(1),
            Operand::Register(RegisterOperand::Address(0)),
        ];
        assert!(
            matches!(
                lowered(
                    Family::AddSub { subtract: false },
                    Some(Size::Word),
                    &immediate_into_address
                ),
                Some(Instruction::ADDQ(1, _, Size::Word))
            ),
            "`add #1,a0` is an addq"
        );
        let large_into_address = [
            Operand::Immediate(100),
            Operand::Register(RegisterOperand::Address(0)),
        ];
        assert!(
            matches!(
                lowered(
                    Family::AddSub { subtract: false },
                    Some(Size::Word),
                    &large_into_address
                ),
                Some(Instruction::ADDA(Operand::Immediate(100), _, Size::Word))
            ),
            "`add #100,a0` is an adda"
        );
        assert!(
            matches!(
                lowered(Family::Cmp, Some(Size::Word), &immediate_into_address),
                Some(Instruction::CMPA(..))
            ),
            "`cmp #1,a0` is a cmpa"
        );
        let postincrements = [Operand::PostIndirect(0), Operand::PostIndirect(1)];
        assert!(matches!(
            lowered(Family::Cmp, Some(Size::Word), &postincrements),
            Some(Instruction::CMPM(..))
        ));
        let register_into_address = [
            Operand::Register(RegisterOperand::Data(0)),
            Operand::Register(RegisterOperand::Address(0)),
        ];
        assert!(matches!(
            lowered(
                Family::AddSub { subtract: true },
                Some(Size::Long),
                &register_into_address
            ),
            Some(Instruction::SUBA(..))
        ));
    }

    #[test]
    fn a_one_operand_shift_shifts_by_one() {
        let operands = [Operand::Indirect(0)];
        match lowered(
            Family::Shift(ShiftKind::Arithmetic, ShiftWay::Right),
            Some(Size::Word),
            &operands,
        ) {
            Some(Instruction::ASd(Operand::Immediate(1), _, ShiftDirection::Right, Size::Word)) => {
            }
            other => panic!("expected `asr.w #1,(a0)`, got {other:?}"),
        }
    }

    #[test]
    fn ext_reads_the_half_of_what_it_writes() {
        let operands = [Operand::Register(RegisterOperand::Data(0))];
        match lowered(Family::Ext, Some(Size::Long), &operands) {
            Some(Instruction::EXT(_, Size::Word, Size::Long)) => {}
            other => panic!("expected `ext.l` to read a word, got {other:?}"),
        }
        match lowered(Family::ExtByteToLong, Some(Size::Long), &operands) {
            Some(Instruction::EXT(_, Size::Byte, Size::Long)) => {}
            other => panic!("expected `extb.l` to read a byte, got {other:?}"),
        }
    }

    #[test]
    fn a_movem_into_a_predecrement_stores_its_mask_reversed() {
        let operands = [Operand::Immediate(0b111), Operand::PreIndirect(7)];
        match lowered(Family::Movem, Some(Size::Long), &operands) {
            Some(Instruction::MOVEM {
                registers_mask,
                direction: TargetDirection::ToMemory,
                ..
            }) => assert_eq!(registers_mask, 0b111u16.reverse_bits()),
            other => panic!("expected a movem to memory, got {other:?}"),
        }
        let back = [Operand::PostIndirect(7), Operand::Immediate(0b111)];
        match lowered(Family::Movem, Some(Size::Long), &back) {
            Some(Instruction::MOVEM {
                registers_mask,
                direction: TargetDirection::FromMemory,
                ..
            }) => assert_eq!(registers_mask, 0b111),
            other => panic!("expected a movem from memory, got {other:?}"),
        }
    }

    /// Every Expression answers the same value, and the instruction sits at
    /// `address`, which is all a PC-relative Operand needs.
    struct Constant(i64);

    impl Values for Constant {
        fn value_of(&self, _: &ast::Expr) -> Option<i64> {
            Some(self.0)
        }
        fn is_back_reference(&self, _: &ast::Expr) -> bool {
            true
        }
        fn instruction_address(&self) -> i64 {
            0x1000
        }
    }

    #[test]
    fn a_displacement_is_sign_extended_from_the_width_it_is_written_at() {
        let expression = ast::Expr::Number {
            value: 0,
            base: crate::assembler::token::NumberBase::Decimal,
            span: span(),
        };
        let displacement = ast::Operand::Displacement {
            displacement: expression.clone(),
            base: address_register(6),
            span: span(),
        };
        match lower_operand(&displacement, &Constant(-8), 0x1002).map(|l| l.operand) {
            Some(Operand::IndirectDisplacement { offset, .. }) => assert_eq!(offset, -8),
            other => panic!("expected a displacement, got {other:?}"),
        }
        let index = ast::Operand::Index {
            displacement: Some(expression),
            base: address_register(0),
            index: ast::IndexRegister {
                register: data(1),
                size: None,
                span: span(),
            },
            span: span(),
        };
        match lower_operand(&index, &Constant(-1), 0x1002).map(|l| l.operand) {
            Some(Operand::IndirectIndex { offset, index, .. }) => {
                assert_eq!(offset, -1);
                assert_eq!(index.size, Size::Word, "no size written means `.w`");
            }
            other => panic!("expected an indexed operand, got {other:?}"),
        }
    }

    #[test]
    fn a_pc_relative_operand_is_stored_as_the_address_it_names() {
        let expression = ast::Expr::Number {
            value: 0,
            base: crate::assembler::token::NumberBase::Decimal,
            span: span(),
        };
        // The source writes the address, and the address is what is kept,
        // forwards and backwards alike: where the extension word sits is the
        // encoder's business.
        let displacement = ast::Operand::PcDisplacement {
            displacement: expression.clone(),
            span: span(),
        };
        match lower_operand(&displacement, &Constant(0x1010), 0x1002).map(|l| l.operand) {
            Some(Operand::PcDisplacement { target }) => assert_eq!(target, 0x1010),
            other => panic!("expected a PC-relative operand, got {other:?}"),
        }
        match lower_operand(&displacement, &Constant(0x0ff2), 0x1002).map(|l| l.operand) {
            Some(Operand::PcDisplacement { target }) => assert_eq!(target, 0x0ff2),
            other => panic!("expected a PC-relative operand, got {other:?}"),
        }
        let index = ast::Operand::PcIndex {
            displacement: Some(expression),
            index: ast::IndexRegister {
                register: data(1),
                size: None,
                span: span(),
            },
            span: span(),
        };
        match lower_operand(&index, &Constant(0x1004), 0x1002).map(|l| l.operand) {
            Some(Operand::PcIndex { target, index }) => {
                assert_eq!(target, 0x1004);
                assert_eq!(index.size, Size::Word, "no size written means `.w`");
            }
            other => panic!("expected a PC-relative indexed operand, got {other:?}"),
        }
        // `(pc,d1.w)` writes no address: its displacement is zero, so it
        // reaches its own extension word, two bytes past the instruction at
        // `$1000`, and the index is the whole of it.
        let bare = ast::Operand::PcIndex {
            displacement: None,
            index: ast::IndexRegister {
                register: data(1),
                size: Some(SizeSuffix::Long),
                span: span(),
            },
            span: span(),
        };
        match lower_operand(&bare, &Constant(0), 0x1002).map(|l| l.operand) {
            Some(Operand::PcIndex { target, index }) => {
                assert_eq!(target, 0x1002);
                assert_eq!(index.size, Size::Long);
            }
            other => panic!("expected a PC-relative indexed operand, got {other:?}"),
        }
        // The distance the analyzer checks is still measured from that word.
        let address = ast::Expr::Number {
            value: 0,
            base: crate::assembler::token::NumberBase::Decimal,
            span: span(),
        };
        assert_eq!(pc_relative_offset(&Constant(0x1010), &address, 2), Some(14));
        assert_eq!(
            pc_relative_offset(&Constant(0x0ff2), &address, 2),
            Some(-16)
        );
        // `movem` and `btst #n` put a word before it, so theirs is two further.
        assert_eq!(pc_relative_offset(&Constant(0x1010), &address, 4), Some(12));
    }
}
