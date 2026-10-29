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
use super::encoded::{BitField, BitFieldOperation, FieldValue};
use super::encoded::{
    BranchSize, ControlRegister, DisplacementSize, ExtensionWord, IndexBase, IndexRegister,
    Indirection, Instruction, Operand, RegisterOperand, ShiftDirection, Size, TargetDirection,
    TrapOperand, EXTENSION_WORD_OFFSET,
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
    /// address, names the instruction's first word, and so does a branch,
    /// whose size depends on how far it reaches.
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
    /// The `{offset:width}` written after it, a bit field's.
    pub field: Option<BitField>,
}

impl Lowered {
    /// An Operand written with numbers alone and no `#n.l`, which is what a
    /// hand-built line is: every choice is then made by its value.
    pub fn known(operand: Operand) -> Self {
        Self {
            operand,
            back_reference: true,
            long: false,
            field: None,
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
/// `(pc,d1.w)`, which writes no address, reaches the instruction's first
/// word, as EASy68K assembles it: its `opParse` gives `(PC,Xi)` the current
/// address as its displacement (`OPPARSE.CPP`, "CK 3-8-2018"), so the
/// displacement it encodes is −2, or −4 for `movem` and `btst #n`, whose mask
/// or bit number comes first ([`extension_word_offset`]).
///
/// A register list becomes its mask as an [`Operand::Immediate`], which is how
/// 1.4.2 carried one and what [`Family::Movem`]'s lowering reads back. A
/// special register has no encoded form at all: `sr` and `ccr` are part of the
/// [`Instruction`] that names them and `usp` is not assembled.
///
/// An absolute address gets its width here, by EASy68K's rule
/// ([`choice::address_width`]): forced by `.w` or `.l`, else a word for a back
/// reference a word holds, and a long otherwise.
pub fn lower_operand(operand: &ast::Operand, values: &dyn Values) -> Option<Lowered> {
    lower_operand_at(operand, values, EXTENSION_WORD_OFFSET as i64)
}

/// [`lower_operand`], for an Operand whose extension word is
/// `extension_offset` bytes past the instruction
/// ([`extension_word_offset`]), which is what a PC-relative distance is
/// measured from.
///
/// On the 68020 a displacement the 68000's field does not hold is the
/// 68020's base displacement when it is a back reference, a word or a long
/// as it needs ([`choice::displacement_size`]): `d(An)` and `d(PC)` past a
/// word, and `d(An,Xn)` and `d(PC,Xn)` past a byte, are the full extension
/// word; and an index with a scale is the brief one with it. A forward
/// reference keeps the 68000's field, which the analyzer holds it to, so a
/// line the 68000 encodes keeps its encoding under `opt p=68020`.
fn lower_operand_at(
    operand: &ast::Operand,
    values: &dyn Values,
    extension_offset: i64,
) -> Option<Lowered> {
    let extended = values.processor().has(Processor::M68020);
    //the address of the extension word, which a PC-relative distance is measured from
    let extension_word = (values.instruction_address() + extension_offset) as u32;
    let distance = |target: u32| target.wrapping_sub(extension_word) as i32 as i64;
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
        } => {
            let value = values.value_of(displacement)?;
            match extended && values.is_back_reference(displacement) && !choice::fits_a_word(value)
            {
                true => Operand::Extended {
                    base: IndexBase::address(base.number),
                    word: no_index(DisplacementSize::Long),
                    displacement: value as i32,
                    outer: 0,
                },
                false => Operand::IndirectDisplacement {
                    offset: value as i16 as i32,
                    base: RegisterOperand::Address(base.number),
                },
            }
        }
        ast::Operand::Index {
            displacement,
            base,
            index,
            ..
        } => {
            let (value, known) = match displacement {
                Some(displacement) => (
                    values.value_of(displacement)?,
                    values.is_back_reference(displacement),
                ),
                None => (0, true),
            };
            let base = IndexBase::address(base.number);
            match indexed(extended, known, value, index) {
                Some(word) => Operand::Extended {
                    base,
                    word,
                    displacement: match word.is_full() {
                        true => value as i32,
                        false => value as i8 as i32,
                    },
                    outer: 0,
                },
                None => Operand::IndirectIndex {
                    base: RegisterOperand::Address(base.address_register().unwrap_or(0)),
                    offset: value as i8 as i32,
                    index: lower_index_register(index),
                },
            }
        }
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
        // A pair of registers becomes their two indexes as an immediate, the
        // first one's in bits 7–4, as a register list becomes its mask; the
        // Family that takes one reads it back ([`pair`]).
        ast::Operand::RegisterPair { first, second, .. }
        | ast::Operand::IndirectPair { first, second, .. } => Operand::Immediate(
            (lower_register(*first).to_index() << 4 | lower_register(*second).to_index()) as u32,
        ),
        // A PC-relative Operand is written as the address it reaches and
        // stored as that address. The analyzer has already refused one whose
        // distance from the extension word does not fit its field
        // ([`pc_relative_offset`]).
        ast::Operand::PcDisplacement { displacement, .. } => {
            back_reference = values.is_back_reference(displacement);
            let target = values.value_of(displacement)? as u32;
            match extended && back_reference && !choice::fits_a_word(distance(target)) {
                true => Operand::Extended {
                    base: IndexBase::PC,
                    word: no_index(DisplacementSize::Long),
                    displacement: target as i32,
                    outer: 0,
                },
                false => Operand::PcDisplacement { target },
            }
        }
        ast::Operand::PcIndex {
            displacement,
            index,
            ..
        } => {
            let target = match displacement {
                Some(displacement) => {
                    back_reference = values.is_back_reference(displacement);
                    values.value_of(displacement)? as u32
                }
                // `(pc,d1.w)` writes no address at all, and reaches the
                // instruction's first word, as in EASy68K (`docs/grammar.md`
                // 2.5). EASy68K also throws away the displacement of
                // `(d,pc,d1.w)` there, which is a bug and not followed.
                None => values.instruction_address() as u32,
            };
            match indexed(extended, back_reference, distance(target), index) {
                Some(word) => Operand::Extended {
                    base: IndexBase::PC,
                    word,
                    displacement: target as i32,
                    outer: 0,
                },
                None => Operand::PcIndex {
                    target,
                    index: lower_index_register(index),
                },
            }
        }
        ast::Operand::FullIndex {
            base_displacement,
            base,
            index,
            indirection,
            outer_displacement,
            ..
        } => lower_full_index(
            base_displacement.as_ref(),
            *base,
            index.as_ref(),
            *indirection,
            outer_displacement.as_ref(),
            values,
            extension_word,
        )?,
        ast::Operand::SpecialRegister { .. } => return None,
        // The field's halves are numbers the analyzer has held to 0–31 for
        // the offset and 1–32 for the width, of which 32 is encoded 0.
        ast::Operand::BitField {
            target,
            offset,
            width,
            ..
        } => {
            let value = |value: &ast::FieldValue| -> Option<FieldValue> {
                Some(match value {
                    ast::FieldValue::Value { value } => {
                        FieldValue::Immediate((values.value_of(value)? & 31) as u8)
                    }
                    ast::FieldValue::Register { register } => FieldValue::Register(register.number),
                })
            };
            let field = BitField {
                offset: value(offset)?,
                width: value(width)?,
            };
            let mut lowered = lower_operand_at(target, values, extension_offset)?;
            lowered.field = Some(field);
            return Some(lowered);
        }
    };
    Some(Lowered {
        operand: lowered,
        back_reference,
        long,
        field: None,
    })
}

/// The extension word of `d(An,Xn)` or `d(PC,Xn)` on the 68020 when the
/// 68000's would not do, or `None`: the full word, with a base displacement
/// of a word or a long, for a back reference `distance` (the displacement,
/// or from the program counter the distance from the extension word) that a
/// byte does not hold; and the brief word with its scale for an index written
/// with one.
fn indexed(
    extended: bool,
    known: bool,
    distance: i64,
    index: &ast::IndexRegister,
) -> Option<ExtensionWord> {
    if !extended {
        return None;
    }
    let register = lower_index_register(index);
    let scale = index.scale.unwrap_or(1);
    if known && !choice::fits_a_byte(distance) {
        let size = match choice::fits_a_word(distance) {
            true => DisplacementSize::Word,
            false => DisplacementSize::Long,
        };
        return Some(ExtensionWord::full(
            register,
            scale,
            false,
            false,
            size,
            Indirection::None,
            DisplacementSize::Null,
        ));
    }
    (scale != 1).then(|| ExtensionWord::brief(register, scale))
}

/// The full extension word of `d(An)` or `d(PC)` past a word: the index
/// suppressed, its bits clear, and a base displacement of `size`.
fn no_index(size: DisplacementSize) -> ExtensionWord {
    ExtensionWord::full(
        SUPPRESSED_INDEX,
        1,
        false,
        true,
        size,
        Indirection::None,
        DisplacementSize::Null,
    )
}

/// The index bits of a suppressed index: clear, `d0.w`, as assemblers write
/// them.
const SUPPRESSED_INDEX: IndexRegister = IndexRegister {
    register: RegisterOperand::Data(0),
    size: Size::Word,
};

/// A 68020 indexed Operand written with what only the 68020 has, a base or
/// an index suppressed, a displacement's width, brackets, at the
/// `extension_word`'s address.
///
/// * The base is the register named, `za0` to `za7` and `zpc` suppressed, and
///   none written is `za0`'s place, as GNU `as` writes it.
/// * A suppressed index, or none, has its bits clear; with brackets either
///   way it is the 68020's memory indirect with no index.
/// * A displacement's width is the one written, or EASy68K's back-reference
///   choice ([`choice::displacement_size`]); none written is null, and from
///   the program counter reaches the extension word.
/// * `(d.w,An)` and `(d.w,PC)` are the 68000's `d16(An)` and `d16(PC)`, as
///   vasm and GNU `as` assemble them.
fn lower_full_index(
    base_displacement: Option<&ast::SizedDisplacement>,
    base: ast::BaseRegister,
    index: Option<&ast::IndexRegister>,
    indirection: ast::Indirection,
    outer_displacement: Option<&ast::SizedDisplacement>,
    values: &dyn Values,
    extension_word: u32,
) -> Option<Operand> {
    let (base, base_suppressed) = match base {
        ast::BaseRegister::Address { register } => (IndexBase::address(register.number), false),
        ast::BaseRegister::ProgramCounter => (IndexBase::PC, false),
        ast::BaseRegister::SuppressedAddress { number } => (IndexBase::address(number), true),
        ast::BaseRegister::SuppressedProgramCounter => (IndexBase::PC, true),
        ast::BaseRegister::Omitted => (IndexBase::address(0), true),
    };
    let pc_relative = base.is_pc() && !base_suppressed;
    let (base_size, displacement) = match base_displacement {
        None => (
            DisplacementSize::Null,
            match pc_relative {
                true => extension_word as i32,
                false => 0,
            },
        ),
        Some(written) => {
            let value = values.value_of(&written.value)?;
            let measured = match pc_relative {
                true => (value as u32).wrapping_sub(extension_word) as i32 as i64,
                false => value,
            };
            let known = values.is_back_reference(&written.value);
            (
                choice::displacement_size(written.size, known, measured),
                value as u32 as i32,
            )
        }
    };
    let (register, scale, index_suppressed) = match index {
        Some(index) if !index.suppressed => {
            (lower_index_register(index), index.scale.unwrap_or(1), false)
        }
        _ => (SUPPRESSED_INDEX, 1, true),
    };
    let indirection = match indirection {
        ast::Indirection::None => Indirection::None,
        ast::Indirection::Postindexed if !index_suppressed => Indirection::Postindexed,
        ast::Indirection::Preindexed | ast::Indirection::Postindexed => Indirection::Preindexed,
    };
    let (outer_size, outer) = match (indirection, outer_displacement) {
        (Indirection::None, _) | (_, None) => (DisplacementSize::Null, 0),
        (_, Some(written)) => {
            let value = values.value_of(&written.value)?;
            let known = values.is_back_reference(&written.value);
            (
                choice::displacement_size(written.size, known, value),
                value as u32 as i32,
            )
        }
    };
    let the_68000s = !base_suppressed
        && index.is_none()
        && indirection == Indirection::None
        && base_displacement.and_then(|written| written.size) == Some(SizeSuffix::Word);
    if the_68000s {
        return Some(match base.address_register() {
            Some(number) => Operand::IndirectDisplacement {
                offset: displacement as i16 as i32,
                base: RegisterOperand::Address(number),
            },
            None => Operand::PcDisplacement {
                target: displacement as u32,
            },
        });
    }
    Some(Operand::Extended {
        base,
        word: ExtensionWord::full(
            register,
            scale,
            base_suppressed,
            index_suppressed,
            base_size,
            indirection,
            outer_size,
        ),
        displacement,
        outer,
    })
}

/// How far past an instruction the extension word of its PC-relative Operand
/// sits: [`EXTENSION_WORD_OFFSET`], the word after the operation word, but for
/// `movem`, whose register mask comes first, and a bit instruction with a
/// static bit number, whose number does. Those two are 4. The 68020's `cmpi`
/// compares an immediate of `size` with a PC-relative Operand, whose
/// extension word follows the immediate's words: 4, or 6 for a long. Its
/// long multiplies and divides, `chk2`, `cmp2` and `cas` have an extension
/// word of their own first: 4.
///
/// The 68000 adds the displacement to the address of the word that holds it,
/// so this is what a PC-relative distance is measured from when the analyzer
/// checks it.
/// No other instruction puts a word before a PC-relative Operand's: a
/// PC-relative Operand is read and never written, so it is the first Operand
/// everywhere else, or follows a register.
pub fn extension_word_offset(family: Family, size: Option<Size>, operands: &[ast::Operand]) -> i64 {
    match (family, operands.first()) {
        (Family::Movem, _) | (Family::Bit(_), Some(ast::Operand::Immediate { .. })) => 4,
        (Family::Immediate(_) | Family::Cmp, Some(ast::Operand::Immediate { .. })) => match size {
            Some(Size::Long) => 6,
            _ => 4,
        },
        // the 68020's instructions whose own extension word comes first
        (Family::Multiply(_) | Family::Divide(_), _) if size == Some(Size::Long) => 4,
        (
            Family::DivideLong(_) | Family::BoundsCheck { .. } | Family::Cas | Family::BitField(_),
            _,
        ) => 4,
        _ => EXTENSION_WORD_OFFSET as i64,
    }
}

/// The distance from a PC-relative Operand's extension word, `extension_offset`
/// past the instruction being lowered ([`extension_word_offset`]), to the
/// address it names: the displacement the 68000 encodes, which the analyzer
/// holds to the width of its field.
///
/// It is read in 32 bits, as the 68000 adds it and as EASy68K subtracts it,
/// so an address below 0 is the one its two's complement names:
/// `$FFFF9004(pc)` at `$1000` reaches back 32,766 bytes, round the bottom of
/// the address space.
pub fn pc_relative_offset(
    values: &dyn Values,
    address: &ast::Expr,
    extension_offset: i64,
) -> Option<i64> {
    let target = values.value_of(address)? as u32;
    let extension_word = (values.instruction_address() + extension_offset) as u32;
    Some(target.wrapping_sub(extension_word) as i32 as i64)
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

/// The two registers of a lowered pair, `d1:d2` or `(a0):(d1)`, as
/// [`lower_operand`] keeps them, or `None` for an Operand that is no pair.
fn pair(operand: Option<Operand>) -> Option<(RegisterOperand, RegisterOperand)> {
    let register = |index: u32| match index & 0xF {
        number @ 0..=7 => RegisterOperand::Data(number as u8),
        number => RegisterOperand::Address(number as u8 - 8),
    };
    match operand? {
        Operand::Immediate(code) => Some((register(code >> 4), register(code))),
        _ => None,
    }
}

/// The number of a data register, which every register of a pair a long
/// multiply or divide takes is.
fn data_number(register: RegisterOperand) -> Option<u8> {
    match register {
        RegisterOperand::Data(number) => Some(number),
        RegisterOperand::Address(_) => None,
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
    if let Some(instruction) = lower_status_register(family, operands, values) {
        return Some(instruction);
    }
    let extension_offset = extension_word_offset(family, size, operands);
    let lowered: Vec<Lowered> = operands
        .iter()
        .map(|operand| lower_operand_at(operand, values, extension_offset))
        .collect::<Option<Vec<_>>>()?;
    let site = Site {
        written,
        address: address as u32,
        processor: values.processor(),
    };
    lower(family, size, &lowered, &site)
}

/// The Operations that read or write `sr`, `ccr` or `usp`, or `None` when
/// this is not one of them.
///
/// `move <ea>,ccr` and `move <ea>,sr` are a word of which the first takes the
/// low byte; `move sr,<ea>` and `move ccr,<ea>` write one; `andi`, `ori` and
/// `eori` take a byte into `ccr` and a word into `sr`
/// (`Reference/68ks4d.htm`, `Reference/68ks6b.htm`); `move usp,An` and
/// `move An,usp` move a long between an address register and the user stack
/// pointer (`Reference/68ks4f.htm`). Every other Mnemonic answers `None` here
/// and is lowered the ordinary way.
fn lower_status_register(
    family: Family,
    operands: &[ast::Operand],
    values: &dyn Values,
) -> Option<Instruction> {
    let [first, second] = operands else {
        return None;
    };
    if family == Family::Movec {
        return lower_movec(first, second);
    }
    if family == Family::Move {
        match (first, second) {
            (
                ast::Operand::SpecialRegister {
                    register: ast::SpecialRegister::Usp,
                    ..
                },
                ast::Operand::AddressRegisterDirect { register, .. },
            ) => {
                return Some(Instruction::MOVEUSP {
                    to_usp: false,
                    register: RegisterOperand::Address(register.number),
                })
            }
            (
                ast::Operand::AddressRegisterDirect { register, .. },
                ast::Operand::SpecialRegister {
                    register: ast::SpecialRegister::Usp,
                    ..
                },
            ) => {
                return Some(Instruction::MOVEUSP {
                    to_usp: true,
                    register: RegisterOperand::Address(register.number),
                })
            }
            _ => {}
        }
    }
    use ast::SpecialRegister::{Ccr, Sr};
    let lowered =
        |operand| lower_operand(operand, values).map(|lowered| lowered.operand);
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

/// `movec`, in whichever direction the control register stands: the
/// control register a special register names, `usp` included, and the
/// general register on the other side. Anything else the analyzer has
/// refused.
fn lower_movec(first: &ast::Operand, second: &ast::Operand) -> Option<Instruction> {
    let control = |operand: &ast::Operand| match operand {
        ast::Operand::SpecialRegister { register, .. } => Some(match register {
            ast::SpecialRegister::Sfc => ControlRegister::SFC,
            ast::SpecialRegister::Dfc => ControlRegister::DFC,
            ast::SpecialRegister::Usp => ControlRegister::USP,
            ast::SpecialRegister::Vbr => ControlRegister::VBR,
            ast::SpecialRegister::Cacr => ControlRegister::CACR,
            ast::SpecialRegister::Caar => ControlRegister::CAAR,
            ast::SpecialRegister::Msp => ControlRegister::MSP,
            ast::SpecialRegister::Isp => ControlRegister::ISP,
            ast::SpecialRegister::Sr | ast::SpecialRegister::Ccr => return None,
        }),
        _ => None,
    };
    let general = |operand: &ast::Operand| match operand {
        ast::Operand::DataRegisterDirect { register, .. } => {
            Some(RegisterOperand::Data(register.number))
        }
        ast::Operand::AddressRegisterDirect { register, .. } => {
            Some(RegisterOperand::Address(register.number))
        }
        _ => None,
    };
    let (to_control, control, register) = match (control(first), control(second)) {
        (Some(control), None) => (false, control, general(second)?),
        (None, Some(control)) => (true, control, general(first)?),
        _ => return None,
    };
    Some(Instruction::MOVEC {
        to_control,
        control,
        register,
    })
}

/// The half of the status register an Operand names, when it names one.
///
/// `usp` is not one of them: it answers `None`, and `move usp` has been lowered
/// before this is asked.
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
        // `divs.l <ea>,Dq` divides 32 bits by 32, and `divs.l <ea>,Dr:Dq` 64
        // by 32, the 68020's
        Family::Divide(sign) => match size? {
            Size::Long => {
                let source = at_size(operand(0)?, Size::Long);
                Some(match operand(1)? {
                    Operand::Register(RegisterOperand::Data(quotient)) => Instruction::DIVL {
                        source,
                        quotient,
                        remainder: quotient,
                        wide: false,
                        sign,
                    },
                    _ => {
                        let (remainder, quotient) = pair(operand(1))?;
                        Instruction::DIVL {
                            source,
                            quotient: data_number(quotient)?,
                            remainder: data_number(remainder)?,
                            wide: true,
                            sign,
                        }
                    }
                })
            }
            _ => Some(Instruction::DIVx(
                at_size(operand(0)?, Size::Word),
                register(operand(1))?,
                sign,
            )),
        },
        // `divsl.l <ea>,Dr:Dq`, 32 bits by 32 with the remainder
        Family::DivideLong(sign) => {
            let (remainder, quotient) = pair(operand(1))?;
            Some(Instruction::DIVL {
                source: at_size(operand(0)?, Size::Long),
                quotient: data_number(quotient)?,
                remainder: data_number(remainder)?,
                wide: false,
                sign,
            })
        }
        // `muls.l <ea>,Dl` keeps the low 32 bits of the product, and
        // `muls.l <ea>,Dh:Dl` all 64, the 68020's
        Family::Multiply(sign) => match size? {
            Size::Long => {
                let source = at_size(operand(0)?, Size::Long);
                Some(match operand(1)? {
                    Operand::Register(RegisterOperand::Data(low)) => Instruction::MULL {
                        source,
                        low,
                        high: None,
                        sign,
                    },
                    _ => {
                        let (high, low) = pair(operand(1))?;
                        Instruction::MULL {
                            source,
                            low: data_number(low)?,
                            high: Some(data_number(high)?),
                            sign,
                        }
                    }
                })
            }
            _ => Some(Instruction::MULx(
                at_size(operand(0)?, Size::Word),
                register(operand(1))?,
                sign,
            )),
        },
        Family::Clr => Some(Instruction::CLR(operand(0)?, size?)),
        Family::Neg => Some(Instruction::NEG(operand(0)?, size?)),
        Family::NegExtended => Some(Instruction::NEGX(operand(0)?, size?)),
        Family::NegDecimal => Some(Instruction::NBCD(operand(0)?)),
        Family::Not => Some(Instruction::NOT(operand(0)?, size?)),
        // the 68020 tests an immediate too, kept at the size it is read at
        Family::Tst => Some(Instruction::TST(at_size(operand(0)?, size?), size?)),
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
        Family::Link => Some(match size {
            Some(Size::Long) => Instruction::LINKL(register(operand(0))?, immediate(operand(1))?),
            _ => Instruction::LINK(register(operand(0))?, immediate(operand(1))?),
        }),
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
            // and is read as a byte. A static bit number is its low byte, as
            // EASy68K encodes it (`staticBit` in its `BUILD.CPP`).
            let bit = match operand(0)? {
                Operand::Immediate(number) => Operand::Immediate(number & 0xFF),
                other => other,
            };
            let destination = at_size(operand(1)?, Size::Byte);
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
        Family::Rte => Some(Instruction::RTE),
        // the immediate is the word the status register is loaded with, which
        // the analyzer has checked fits one
        Family::Stop => Some(Instruction::STOP(immediate(operand(0))? as u16)),
        Family::Reset => Some(Instruction::RESET),
        Family::Chk => Some(match size? {
            Size::Long => {
                Instruction::CHKL(at_size(operand(0)?, Size::Long), register(operand(1))?)
            }
            _ => Instruction::CHK(at_size(operand(0)?, Size::Word), register(operand(1))?),
        }),
        Family::Trapv => Some(Instruction::TRAPV),
        Family::Illegal => Some(Instruction::ILLEGAL),
        // the displacement is kept sign extended, as `link`'s is, and the
        // analyzer has checked that it fits a word
        Family::Rtd => Some(Instruction::RTD(
            immediate(operand(0))? as i32 as i16 as i32 as u32,
        )),
        Family::Bkpt => Some(Instruction::BKPT(immediate(operand(0))? as u8)),
        // `lower_status_register` lowers it, as its Operands are registers
        // that are no Addressing mode
        Family::Movec => None,
        Family::Pack { unpack } => {
            let (source, destination) = (operand(0)?, operand(1)?);
            let adjustment = at_size_value(immediate(operand(2))?, Size::Word) as u16;
            Some(match unpack {
                false => Instruction::PACK(source, destination, adjustment),
                true => Instruction::UNPK(source, destination, adjustment),
            })
        }
        Family::BoundsCheck { trap } => {
            let (bounds, register) = (operand(0)?, register(operand(1))?);
            Some(match trap {
                true => Instruction::CHK2(bounds, register, size?),
                false => Instruction::CMP2(bounds, register, size?),
            })
        }
        Family::Cas => Some(Instruction::CAS {
            compare: data_number(register(operand(0))?)?,
            update: data_number(register(operand(1))?)?,
            target: operand(2)?,
            size: size?,
        }),
        Family::Cas2 => {
            let (compare_1, compare_2) = pair(operand(0))?;
            let (update_1, update_2) = pair(operand(1))?;
            Some(Instruction::CAS2 {
                compare: (data_number(compare_1)?, data_number(compare_2)?),
                update: (data_number(update_1)?, data_number(update_2)?),
                pointers: pair(operand(2))?,
                size: size?,
            })
        }
        // the field is after the Operand it is in, the second of `bfins`
        Family::BitField(operation) => {
            let (target, register) = match operation {
                BitFieldOperation::Insert => (operands.get(1)?, register(operand(0))),
                _ => (operands.first()?, register(operand(1))),
            };
            Some(Instruction::BF {
                operation,
                target: target.operand,
                field: target.field?,
                register: match register {
                    Some(register) => data_number(register)?,
                    None => 0,
                },
            })
        }
        Family::TrapCc(condition) => Some(Instruction::TRAPcc(
            condition,
            match (operand(0), size) {
                (None, _) => TrapOperand::None,
                (Some(Operand::Immediate(value)), Some(Size::Long)) => TrapOperand::Long(value),
                (Some(Operand::Immediate(value)), _) => TrapOperand::Word(value as u16),
                (Some(_), _) => return None,
            },
        )),
        Family::Moves => {
            let (first, second) = (operand(0)?, operand(1)?);
            let size = size?;
            Some(match (first, second) {
                (Operand::Register(register), target) => Instruction::MOVES {
                    direction: TargetDirection::ToMemory,
                    size,
                    register,
                    target,
                },
                (target, Operand::Register(register)) => Instruction::MOVES {
                    direction: TargetDirection::FromMemory,
                    size,
                    register,
                    target,
                },
                _ => return None,
            })
        }
    }
}

/// `movep`, in whichever direction the data register stands.
///
/// The other Operand is a displacement one — the only Addressing mode `movep`
/// has (`Reference/68ks4g.htm`) — or `(An)`, which is encoded as `0(An)`, as
/// EASy68K encodes it (`movep` in its `BUILD.CPP`). The analyzer has already
/// refused anything else, which is why nothing here diagnoses it.
fn lower_movep(size: Size, first: Operand, second: Operand) -> Option<Instruction> {
    let displaced = |operand| match operand {
        Operand::Indirect(base) => Operand::IndirectDisplacement {
            offset: 0,
            base: RegisterOperand::Address(base),
        },
        other => other,
    };
    let (first, second) = (displaced(first), displaced(second));
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
        match lower_operand(&displacement, &Constant(-8)).map(|l| l.operand) {
            Some(Operand::IndirectDisplacement { offset, .. }) => assert_eq!(offset, -8),
            other => panic!("expected a displacement, got {other:?}"),
        }
        let index = ast::Operand::Index {
            displacement: Some(expression),
            base: address_register(0),
            index: ast::IndexRegister {
                register: data(1),
                size: None,
                scale: None,
                suppressed: false,
                span: span(),
            },
            span: span(),
        };
        match lower_operand(&index, &Constant(-1)).map(|l| l.operand) {
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
        match lower_operand(&displacement, &Constant(0x1010)).map(|l| l.operand) {
            Some(Operand::PcDisplacement { target }) => assert_eq!(target, 0x1010),
            other => panic!("expected a PC-relative operand, got {other:?}"),
        }
        match lower_operand(&displacement, &Constant(0x0ff2)).map(|l| l.operand) {
            Some(Operand::PcDisplacement { target }) => assert_eq!(target, 0x0ff2),
            other => panic!("expected a PC-relative operand, got {other:?}"),
        }
        let index = ast::Operand::PcIndex {
            displacement: Some(expression),
            index: ast::IndexRegister {
                register: data(1),
                size: None,
                scale: None,
                suppressed: false,
                span: span(),
            },
            span: span(),
        };
        match lower_operand(&index, &Constant(0x1004)).map(|l| l.operand) {
            Some(Operand::PcIndex { target, index }) => {
                assert_eq!(target, 0x1004);
                assert_eq!(index.size, Size::Word, "no size written means `.w`");
            }
            other => panic!("expected a PC-relative indexed operand, got {other:?}"),
        }
        // `(pc,d1.w)` writes no address: as in EASy68K, it reaches the
        // instruction's first word, `$1000`, and the index is added to that.
        let bare = ast::Operand::PcIndex {
            displacement: None,
            index: ast::IndexRegister {
                register: data(1),
                size: Some(SizeSuffix::Long),
                scale: None,
                suppressed: false,
                span: span(),
            },
            span: span(),
        };
        match lower_operand(&bare, &Constant(0)).map(|l| l.operand) {
            Some(Operand::PcIndex { target, index }) => {
                assert_eq!(target, 0x1000);
                assert_eq!(index.size, Size::Long);
            }
            other => panic!("expected a PC-relative indexed operand, got {other:?}"),
        }
        // The distance the analyzer checks is measured from the extension word.
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
        // In 32 bits, an address below the bottom of memory is reached by
        // wrapping round it, and so is one past the top.
        assert_eq!(
            pc_relative_offset(&Constant(0xffff_9004), &address, 2),
            Some(-32766)
        );
        assert_eq!(pc_relative_offset(&Constant(-0x6ffc), &address, 2), Some(-32766));
    }
}
