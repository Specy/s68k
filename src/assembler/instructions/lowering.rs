//! From a parsed Operand to an encoded one, and from a [`Family`] to an
//! [`Instruction`].
//!
//! This is the last step of the front end: the analyzer has already said that
//! the Operation is well formed, so nothing here diagnoses anything. It
//! evaluates the Expressions through [`Values`], normalises the Mnemonic the
//! way `tests/corpus/README.md` ("Normalisations stay visible") describes —
//! `add #1,d0` becomes an `addi`, `move.l d0,a0` a `movea`, `cmp (a0)+,(a1)+` a
//! `cmpm` — and builds the [`Instruction`] the Interpreter runs.
//!
//! [`lower`] has one arm per [`Family`] and no catch-all, so a Family that
//! gains a variant does not compile until it is given an encoding.

use crate::assembler::ast::{self, SizeSuffix};

use super::encoded::{
    IndexRegister, Instruction, Operand, RegisterOperand, ShiftDirection, Size, TargetDirection,
    EXTENSION_WORD_OFFSET,
};
use super::table::{BitOperation, Family, ImmediateKind, LogicalKind, ShiftKind, ShiftWay};

/// What the lowering needs to know that the tree does not hold: the value of
/// every Expression in it.
///
/// The analyzer implements it (`analyzer::Context`), and so will the evaluator
/// of `src/assembler/expr.rs` when it arrives. `None` means the value cannot be
/// known — an undefined Symbol, a division by zero — in which case the
/// Diagnostic has already been raised and nothing is lowered.
pub trait Values {
    /// The value of one Expression, in 64 bits.
    fn value_of(&self, expression: &ast::Expr) -> Option<i64>;

    /// The address the instruction being lowered is laid out at.
    ///
    /// Only a PC-relative Operand needs it, and it needs it because the source
    /// writes an address there and the encoded Operand holds a displacement
    /// from this instruction ([`EXTENSION_WORD_OFFSET`]). Everything else is
    /// lowered without knowing where it sits.
    fn instruction_address(&self) -> i64;
}

/// The encoded form of one parsed Operand, or `None` when a value in it cannot
/// be worked out.
///
/// A register list becomes its mask as an [`Operand::Immediate`], which is how
/// 1.4.2 carried one and what [`Family::Movem`]'s lowering reads back. A
/// special register has no encoded form at all: `sr` and `ccr` are part of the
/// [`Instruction`] that names them and `usp` is not assembled.
pub fn lower_operand(operand: &ast::Operand, values: &dyn Values) -> Option<Operand> {
    Some(match operand {
        ast::Operand::Immediate { value, .. } => Operand::Immediate(values.value_of(value)? as u32),
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
        ast::Operand::Absolute { value, .. } => {
            Operand::Absolute(values.value_of(value)? as u32 as usize)
        }
        ast::Operand::RegisterList { items, .. } => Operand::Immediate(register_mask(items) as u32),
        // A PC-relative Operand is written as the address it reaches and stored
        // as the distance from this instruction's extension word to it, which
        // is the whole of what makes the two sides agree: the analyzer has
        // already refused a distance the field cannot hold, so the truncation
        // here only ever throws away bits a reported line would have had.
        ast::Operand::PcDisplacement { displacement, .. } => Operand::PcDisplacement {
            offset: pc_relative_offset(values, displacement)? as i16 as i32,
        },
        ast::Operand::PcIndex {
            displacement,
            index,
            ..
        } => Operand::PcIndex {
            offset: match displacement {
                Some(displacement) => pc_relative_offset(values, displacement)? as i8 as i32,
                // `(pc,d1.w)` writes no address at all, so there is none to
                // measure from: the displacement is zero, as it is in
                // `(a0,d1.w)` (`docs/grammar.md` 2.5).
                None => 0,
            },
            index: lower_index_register(index),
        },
        ast::Operand::SpecialRegister { .. } => return None,
    })
}

/// The distance from the extension word of the instruction being lowered to
/// the address a PC-relative Operand names.
///
/// This is the one half of the round trip the Assembler owns; the other is the
/// Interpreter adding [`EXTENSION_WORD_OFFSET`] and this number back together
/// (`src/interpreter.rs`).
pub fn pc_relative_offset(values: &dyn Values, address: &ast::Expr) -> Option<i64> {
    let target = values.value_of(address)?;
    Some(target - (values.instruction_address() + EXTENSION_WORD_OFFSET as i64))
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
/// line. It answers the shapes that name a half of the status register first —
/// `sr` and `ccr` are Operands the encoded [`Operand`] has no form for, so they
/// are read from the tree rather than lowered — and everything else by
/// evaluating each Operand and handing the result to [`lower`].
pub fn lower_operation(
    family: Family,
    size: Option<Size>,
    operands: &[ast::Operand],
    values: &dyn Values,
) -> Option<Instruction> {
    if let Some(instruction) = lower_status_register(family, operands, values) {
        return Some(instruction);
    }
    let lowered: Vec<Operand> = operands
        .iter()
        .map(|operand| lower_operand(operand, values))
        .collect::<Option<Vec<_>>>()?;
    lower(family, size, &lowered)
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
) -> Option<Instruction> {
    let [first, second] = operands else {
        return None;
    };
    use ast::SpecialRegister::{Ccr, Sr};
    match (
        family,
        half_of_the_status_register(first),
        half_of_the_status_register(second),
    ) {
        (Family::Move, None, Some(Ccr)) => {
            Some(Instruction::MOVEtoCCR(lower_operand(first, values)?))
        }
        (Family::Move, None, Some(Sr)) => {
            Some(Instruction::MOVEtoSR(lower_operand(first, values)?))
        }
        (Family::Move, Some(Ccr), None) => {
            Some(Instruction::MOVEfromCCR(lower_operand(second, values)?))
        }
        (Family::Move, Some(Sr), None) => {
            Some(Instruction::MOVEfromSR(lower_operand(second, values)?))
        }
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
/// are the Operands of the matched Form, already lowered.
///
/// `None` means the shape is not one this Family encodes, which the analyzer
/// has already reported: lowering never raises a Diagnostic of its own.
pub fn lower(family: Family, size: Option<Size>, operands: &[Operand]) -> Option<Instruction> {
    match family {
        Family::Move => lower_move(size?, operands),
        Family::Movea => Some(Instruction::MOVEA(
            *operands.first()?,
            register(operands.get(1))?,
            size?,
        )),
        Family::Movem => lower_movem(size?, operands),
        Family::Moveq => Some(Instruction::MOVEQ(
            immediate(operands.first())? as u8,
            register(operands.get(1))?,
        )),
        Family::AddSub { subtract } => lower_add_sub(subtract, size?, operands),
        Family::AddSubExtended { subtract } => {
            let (source, destination) = (*operands.first()?, *operands.get(1)?);
            Some(match subtract {
                false => Instruction::ADDX(source, destination, size?),
                true => Instruction::SUBX(source, destination, size?),
            })
        }
        Family::AddSubDecimal { subtract } => {
            // No size reaches the encoded instruction: `abcd` and `sbcd` work
            // on one byte and the table gives them no other size, so the
            // Instruction carries none (`Reference/68ks8e.htm`).
            let (source, destination) = (*operands.first()?, *operands.get(1)?);
            Some(match subtract {
                false => Instruction::ABCD(source, destination),
                true => Instruction::SBCD(source, destination),
            })
        }
        Family::AddSubAddress { subtract } => {
            let (source, destination) = (*operands.first()?, register(operands.get(1))?);
            Some(match subtract {
                false => Instruction::ADDA(source, destination, size?),
                true => Instruction::SUBA(source, destination, size?),
            })
        }
        Family::AddSubQuick { subtract } => {
            let (count, destination) = (immediate(operands.first())? as u8, *operands.get(1)?);
            Some(match subtract {
                false => Instruction::ADDQ(count, destination, size?),
                true => Instruction::SUBQ(count, destination, size?),
            })
        }
        Family::Immediate(kind) => lower_immediate(kind, size?, operands),
        Family::Cmp => lower_cmp(size?, operands),
        Family::Cmpa => Some(Instruction::CMPA(
            *operands.first()?,
            register(operands.get(1))?,
            size?,
        )),
        Family::Cmpm => Some(Instruction::CMPM(
            *operands.first()?,
            *operands.get(1)?,
            size?,
        )),
        Family::Logical(kind) => {
            let (source, destination) = (*operands.first()?, *operands.get(1)?);
            Some(match kind {
                LogicalKind::And => Instruction::AND(source, destination, size?),
                LogicalKind::Or => Instruction::OR(source, destination, size?),
                LogicalKind::Eor => Instruction::EOR(source, destination, size?),
            })
        }
        Family::Divide(sign) => Some(Instruction::DIVx(
            *operands.first()?,
            register(operands.get(1))?,
            sign,
        )),
        Family::Multiply(sign) => Some(Instruction::MULx(
            *operands.first()?,
            register(operands.get(1))?,
            sign,
        )),
        Family::Clr => Some(Instruction::CLR(*operands.first()?, size?)),
        Family::Neg => Some(Instruction::NEG(*operands.first()?, size?)),
        Family::NegExtended => Some(Instruction::NEGX(*operands.first()?, size?)),
        Family::NegDecimal => Some(Instruction::NBCD(*operands.first()?)),
        Family::Not => Some(Instruction::NOT(*operands.first()?, size?)),
        Family::Tst => Some(Instruction::TST(*operands.first()?, size?)),
        Family::Ext => {
            // `ext.w` reads a byte and writes a word; `ext.l` reads a word and
            // writes a long.
            let (read, written) = match size? {
                Size::Word => (Size::Byte, Size::Word),
                Size::Long => (Size::Word, Size::Long),
                Size::Byte => return None,
            };
            Some(Instruction::EXT(register(operands.first())?, read, written))
        }
        Family::ExtByteToLong => Some(Instruction::EXT(
            register(operands.first())?,
            Size::Byte,
            Size::Long,
        )),
        Family::Swap => Some(Instruction::SWAP(register(operands.first())?)),
        Family::Exg => Some(Instruction::EXG(
            register(operands.first())?,
            register(operands.get(1))?,
        )),
        Family::Lea => Some(Instruction::LEA(
            *operands.first()?,
            register(operands.get(1))?,
        )),
        Family::Pea => Some(Instruction::PEA(*operands.first()?)),
        Family::Link => Some(Instruction::LINK(
            register(operands.first())?,
            immediate(operands.get(1))?,
        )),
        Family::Unlk => Some(Instruction::UNLK(register(operands.first())?)),
        Family::Jmp => Some(Instruction::JMP(*operands.first()?)),
        Family::Jsr => Some(Instruction::JSR(*operands.first()?)),
        Family::Bra => Some(Instruction::BRA(address(operands.first())?)),
        Family::Bsr => Some(Instruction::BSR(address(operands.first())?)),
        Family::Bcc(condition) => Some(Instruction::Bcc(address(operands.first())?, condition)),
        Family::Scc(condition) => Some(Instruction::Scc(*operands.first()?, condition)),
        Family::DBcc(condition) => Some(Instruction::DBcc(
            register(operands.first())?,
            address(operands.get(1))?,
            condition,
        )),
        Family::Shift(kind, way) => lower_shift(kind, way, size?, operands),
        Family::Bit(operation) => {
            let (bit, destination) = (*operands.first()?, *operands.get(1)?);
            Some(match operation {
                BitOperation::Test => Instruction::BTST(bit, destination),
                BitOperation::Set => Instruction::BSET(bit, destination),
                BitOperation::Clear => Instruction::BCLR(bit, destination),
                BitOperation::Change => Instruction::BCHG(bit, destination),
            })
        }
        Family::Trap => Some(Instruction::TRAP(immediate(operands.first())? as u8)),
        Family::Rts => Some(Instruction::RTS),
        Family::Nop => Some(Instruction::NOP),
        Family::Movep => lower_movep(size?, operands),
        Family::Tas => Some(Instruction::TAS(*operands.first()?)),
        Family::Rtr => Some(Instruction::RTR),
        Family::Chk => Some(Instruction::CHK(
            *operands.first()?,
            register(operands.get(1))?,
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
fn lower_movep(size: Size, operands: &[Operand]) -> Option<Instruction> {
    let (first, second) = (*operands.first()?, *operands.get(1)?);
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

/// `move`, which is a `movea` when the destination is an address register.
fn lower_move(size: Size, operands: &[Operand]) -> Option<Instruction> {
    let (source, destination) = (*operands.first()?, *operands.get(1)?);
    Some(match destination {
        Operand::Register(register @ RegisterOperand::Address(_)) => {
            Instruction::MOVEA(source, register, size)
        }
        _ => Instruction::MOVE(source, destination, size),
    })
}

/// `add` and `sub`, which are `addi`/`subi` on an immediate source and
/// `adda`/`suba` into an address register — in that order, which is why
/// `add #1,a0` is an `addi` (`tests/corpus/README.md`, "will change").
fn lower_add_sub(subtract: bool, size: Size, operands: &[Operand]) -> Option<Instruction> {
    let (source, destination) = (*operands.first()?, *operands.get(1)?);
    Some(match (source, destination) {
        (Operand::Immediate(value), _) => match subtract {
            false => Instruction::ADDI(value, destination, size),
            true => Instruction::SUBI(value, destination, size),
        },
        (_, Operand::Register(register @ RegisterOperand::Address(_))) => match subtract {
            false => Instruction::ADDA(source, register, size),
            true => Instruction::SUBA(source, register, size),
        },
        _ => match subtract {
            false => Instruction::ADD(source, destination, size),
            true => Instruction::SUB(source, destination, size),
        },
    })
}

/// The six instructions that take an immediate and write it somewhere.
fn lower_immediate(kind: ImmediateKind, size: Size, operands: &[Operand]) -> Option<Instruction> {
    let (value, destination) = (immediate(operands.first())?, *operands.get(1)?);
    Some(match kind {
        ImmediateKind::Add => Instruction::ADDI(value, destination, size),
        ImmediateKind::Sub => Instruction::SUBI(value, destination, size),
        ImmediateKind::And => Instruction::ANDI(value, destination, size),
        ImmediateKind::Or => Instruction::ORI(value, destination, size),
        ImmediateKind::Eor => Instruction::EORI(value, destination, size),
        ImmediateKind::Cmp => Instruction::CMPI(value, destination, size),
    })
}

/// `cmp`, which is a `cmpa` into an address register, a `cmpi` from an
/// immediate and a `cmpm` between two postincrements. The `cmpa` rule is first,
/// so `cmp #1,a0` is a `cmpa` where `add #1,a0` is an `addi`
/// (`tests/corpus/README.md`, "will change").
fn lower_cmp(size: Size, operands: &[Operand]) -> Option<Instruction> {
    let (source, destination) = (*operands.first()?, *operands.get(1)?);
    Some(match (source, destination) {
        (_, Operand::Register(register @ RegisterOperand::Address(_))) => {
            Instruction::CMPA(source, register, size)
        }
        (Operand::Immediate(value), _) => Instruction::CMPI(value, destination, size),
        (Operand::PostIndirect(_), Operand::PostIndirect(_)) => {
            Instruction::CMPM(source, destination, size)
        }
        _ => Instruction::CMP(source, register(operands.get(1))?, size),
    })
}

/// `movem`, in whichever direction the register list stands.
///
/// The mask of a list that goes into a predecrement is stored reversed, because
/// that is the order the 68000 writes the registers in; the fixture printer
/// undoes it (`tests/corpus/README.md`, "`movem` register lists").
fn lower_movem(size: Size, operands: &[Operand]) -> Option<Instruction> {
    let (first, second) = (*operands.first()?, *operands.get(1)?);
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
    operands: &[Operand],
) -> Option<Instruction> {
    let (count, target) = match operands {
        [count, target] => (*count, *target),
        [target] => (Operand::Immediate(1), *target),
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

/// The register an Operand is, when it is one.
fn register(operand: Option<&Operand>) -> Option<RegisterOperand> {
    match operand? {
        Operand::Register(register) => Some(*register),
        _ => None,
    }
}

/// The value an immediate Operand carries, when it is one.
fn immediate(operand: Option<&Operand>) -> Option<u32> {
    match operand? {
        Operand::Immediate(value) => Some(*value),
        _ => None,
    }
}

/// The address an absolute Operand names, when it is one. Branch and jump
/// targets are resolved to addresses before they get here.
fn address(operand: Option<&Operand>) -> Option<u32> {
    match operand? {
        Operand::Absolute(address) => Some(*address as u32),
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
            lower(Family::Move, Some(Size::Long), &operands),
            Some(Instruction::MOVEA(..))
        ));
    }

    #[test]
    fn add_normalises_the_way_the_printer_records() {
        let immediate_into_address = [
            Operand::Immediate(1),
            Operand::Register(RegisterOperand::Address(0)),
        ];
        assert!(
            matches!(
                lower(
                    Family::AddSub { subtract: false },
                    Some(Size::Word),
                    &immediate_into_address
                ),
                Some(Instruction::ADDI(..))
            ),
            "`add #1,a0` is an addi"
        );
        assert!(
            matches!(
                lower(Family::Cmp, Some(Size::Word), &immediate_into_address),
                Some(Instruction::CMPA(..))
            ),
            "`cmp #1,a0` is a cmpa"
        );
        let postincrements = [Operand::PostIndirect(0), Operand::PostIndirect(1)];
        assert!(matches!(
            lower(Family::Cmp, Some(Size::Word), &postincrements),
            Some(Instruction::CMPM(..))
        ));
        let register_into_address = [
            Operand::Register(RegisterOperand::Data(0)),
            Operand::Register(RegisterOperand::Address(0)),
        ];
        assert!(matches!(
            lower(
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
        match lower(
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
        match lower(Family::Ext, Some(Size::Long), &operands) {
            Some(Instruction::EXT(_, Size::Word, Size::Long)) => {}
            other => panic!("expected `ext.l` to read a word, got {other:?}"),
        }
        match lower(Family::ExtByteToLong, Some(Size::Long), &operands) {
            Some(Instruction::EXT(_, Size::Byte, Size::Long)) => {}
            other => panic!("expected `extb.l` to read a byte, got {other:?}"),
        }
    }

    #[test]
    fn a_movem_into_a_predecrement_stores_its_mask_reversed() {
        let operands = [Operand::Immediate(0b111), Operand::PreIndirect(7)];
        match lower(Family::Movem, Some(Size::Long), &operands) {
            Some(Instruction::MOVEM {
                registers_mask,
                direction: TargetDirection::ToMemory,
                ..
            }) => assert_eq!(registers_mask, 0b111u16.reverse_bits()),
            other => panic!("expected a movem to memory, got {other:?}"),
        }
        let back = [Operand::PostIndirect(7), Operand::Immediate(0b111)];
        match lower(Family::Movem, Some(Size::Long), &back) {
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
        match lower_operand(&displacement, &Constant(-8)) {
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
        match lower_operand(&index, &Constant(-1)) {
            Some(Operand::IndirectIndex { offset, index, .. }) => {
                assert_eq!(offset, -1);
                assert_eq!(index.size, Size::Word, "no size written means `.w`");
            }
            other => panic!("expected an indexed operand, got {other:?}"),
        }
    }

    #[test]
    fn a_pc_relative_operand_is_stored_as_the_distance_to_what_it_names() {
        let expression = ast::Expr::Number {
            value: 0,
            base: crate::assembler::token::NumberBase::Decimal,
            span: span(),
        };
        // The instruction sits at `$1000`, so its extension word is at `$1002`
        // and an operand naming `$1010` is stored as 14.
        let displacement = ast::Operand::PcDisplacement {
            displacement: expression.clone(),
            span: span(),
        };
        match lower_operand(&displacement, &Constant(0x1010)) {
            Some(Operand::PcDisplacement { offset }) => assert_eq!(offset, 14),
            other => panic!("expected a PC-relative operand, got {other:?}"),
        }
        // An address behind the instruction is a negative displacement.
        match lower_operand(&displacement, &Constant(0x0ff2)) {
            Some(Operand::PcDisplacement { offset }) => assert_eq!(offset, -16),
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
        match lower_operand(&index, &Constant(0x1004)) {
            Some(Operand::PcIndex { offset, index }) => {
                assert_eq!(offset, 2);
                assert_eq!(index.size, Size::Word, "no size written means `.w`");
            }
            other => panic!("expected a PC-relative indexed operand, got {other:?}"),
        }
        // `(pc,d1.w)` writes no address, so there is nothing to measure from:
        // the displacement is zero and the index is the whole of it.
        let bare = ast::Operand::PcIndex {
            displacement: None,
            index: ast::IndexRegister {
                register: data(1),
                size: Some(SizeSuffix::Long),
                span: span(),
            },
            span: span(),
        };
        match lower_operand(&bare, &Constant(0)) {
            Some(Operand::PcIndex { offset, index }) => {
                assert_eq!(offset, 0);
                assert_eq!(index.size, Size::Long);
            }
            other => panic!("expected a PC-relative indexed operand, got {other:?}"),
        }
    }
}
