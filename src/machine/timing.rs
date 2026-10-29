//! The 68000's timing: how many clock cycles each instruction, each
//! exception and each IRQ takes, for the cycle counter (the asm-editor's
//! `docs/design/m68k-machine.md`, decision 23).
//!
//! **The source.** The time of an instruction is its base time plus the time
//! of its effective addresses, from section 8 of the 68000 User's Manual,
//! with the corrections of Yacht ("Yet Another Cycle Hunting Table"), and
//! every value is checked against the lengths the SingleStepTests/m68000
//! suite records for its 317,500 tests (ADR 0010). Where EASy68K's own tables
//! differ, the 68000 wins: its TRAP takes 34 cycles here and 38 in EASy68K.
//! Every time assumes a bus that answers at once, as the suite's does.
//!
//! **Three parts.** A step's cycles are added up from:
//!
//! * the [`base`] of its instruction, which depends on nothing but the
//!   instruction and its operands' addressing modes. The decode cache works it
//!   out once, when it keeps the instruction, so that a step adds one number;
//! * the extra the instruction's data costs, which the Interpreter adds as it
//!   runs it: [`multiply`] and [`divide`] by their operands, a shift or a
//!   rotate by its count ([`SHIFT_PER_BIT`]), a branch taken or not, a `DBcc`
//!   whose counter runs out, an `Scc` that sets its register, and a bit of a
//!   data register above 15 ([`HIGH_BIT`]);
//! * an exception's time, which replaces the instruction's: [`exception()`]
//!   says what a step whose instruction raised one takes in all. A trace
//!   exception adds [`TRACE`] after the instruction it follows, and an IRQ
//!   step takes [`IRQ`].
//!
//! **`trap #15`** is EASy68K's service trap, and its tasks cost nothing, as in
//! EASy68K. A task number EASy68K does not know is taken through vector 47,
//! a `trap` like any other.
//!
//! **The dialects.** Each [`Processor`] has its own table: [`base`] takes the
//! one to count for, and [`Extras`] says what the data of an instruction adds
//! on it, so that a step asks nothing of the processor.
//!
//! * The 68010's is section 9 of the manual where it gives exact times, and
//!   Moira's 68010 where the manual gives a maximum or is wrong: Moira counts
//!   its cycles access by access and is checked against cputester on the
//!   hardware. The differential harness (`src/test/differential.rs`, the
//!   `moira` feature) checks every 68010 instruction against it, address
//!   errors and loop mode included ([`loop_mode`]). Its multiplications and
//!   divisions take Moira's models ([`multiply`], [`divide`]), and
//!   [`m68010`] holds its exceptions' times.
//! * The 68020's is an estimate, the **cache case** of the M68020 User's
//!   Manual's timing tables (section 8 of the 1995 edition, section 9 of the
//!   1984 one, whose values are the same): the instruction in the cache, and
//!   no overlap with the instructions around it. The manual gives a best, a
//!   cache and a worst case for every row and says that exact times cannot
//!   be worked out from the tables, as they depend on the cache, the
//!   prefetch and the overlap of neighbouring instructions; the cache case
//!   is the deterministic middle, and the column Musashi's 68020 keeps
//!   (the asm-editor's `docs/research/m68k-cpu-variants.md`, 3.2 to 3.4).
//!   [`m68020`] holds its exceptions' times and says where the manual gives
//!   none and what this takes instead. Operands are taken as aligned, as the
//!   tables assume, so a word or a long at an odd address takes no more.
//!   [`Processor::cycles_exact`] says which of the three counts are exact.

use super::Processor;
use crate::instructions::BitFieldOperation;
use crate::instructions::{
    AddressWidth, BranchSize, DisplacementSize, Indirection, Instruction, Operand, RegisterOperand,
    ShiftDirection, Sign, Size, TargetDirection, TrapOperand,
};
use crate::interpreter::{Access, RuntimeError};

use super::exception;

/// The extra cycles of each bit a shift or a rotate of a data register moves,
/// on a 68000 and a 68010.
pub const SHIFT_PER_BIT: u64 = 2;

/// The extra cycles of `bchg`, `bclr` and `bset` on a bit of a data register
/// above 15, which the 68000 changes in the register's high word, and the
/// 68010 too.
pub const HIGH_BIT: u64 = 2;

/// The extra cycles of a `Bcc.s` that branches, over one that does not, and
/// of a `Bcc.w` that does not branch, over one that does.
pub const BRANCH_EXTRA: u64 = 2;

/// The extra cycles of a `DBcc` whose condition is true, over one that
/// branches back.
pub const DBCC_CONDITION_TRUE: u64 = 2;

/// The extra cycles of a `DBcc` whose counter runs out, over one that branches
/// back.
pub const DBCC_EXPIRED: u64 = 4;

/// The extra cycles of an `Scc` to a data register whose condition is true:
/// it writes the whole byte of ones.
pub const SCC_TRUE: u64 = 2;

/// The cycles of an IRQ taken: the interrupt acknowledge, the frame and the
/// fetch of the handler's first words (the 68000 User's Manual, table 8-14).
/// EASy68K also adds 44 when the request is made, which the 68000 does not.
pub const IRQ: u64 = 44;

/// The cycles of a trace exception, after the instruction it follows.
pub const TRACE: u64 = 34;

/// The cycles of the exceptions of group 1, an illegal instruction, a
/// privilege violation and the line A and line F words: the instruction does
/// not run, and the step takes only these.
pub const GROUP_ONE: u32 = 34;

/// The cycles of `trap #0` to `#14`, and of a `trap #15` taken through vector
/// 47, from the instruction's first word to the handler's: EASy68K counts 38.
pub const TRAP: u32 = 34;

/// The cycles of `trapv` with V set.
pub const TRAPV: u32 = 34;

/// The cycles of a division by zero, besides its source's effective address:
/// EASy68K counts 42 in all.
pub const DIVISION_BY_ZERO: u32 = 38;

/// The cycles of `chk` out of bounds above the bound, besides its source's
/// effective address. EASy68K counts 44 in all, whichever way it is out.
pub const CHK_ABOVE: u32 = 38;

/// The cycles of `chk` out of bounds below zero, besides its source's
/// effective address.
pub const CHK_BELOW: u32 = 40;

/// The cycles of an address error or a bus error from the start of the
/// access it aborted to the handler's first instruction: what MAME's
/// microcode takes on every one of the suite's 55,606 address errors. The
/// manual's 50 leaves out the access the error aborted, and the time of the
/// instruction before it, which [`address_error_lead`] adds.
pub const GROUP_ZERO: u32 = 58;

/// The cycles `stop` takes before the processor waits.
pub const STOP_CYCLES: u16 = 4;

/// The cycles `reset` takes, asserting the reset line for 124 of them: its
/// only effect, as there are no devices behind the line (decision 13).
/// EASy68K counts the same.
pub const RESET_CYCLES: u16 = 132;

/// The cycles of an instruction that do not depend on its data, for
/// `processor`: every cycle of most instructions, and what the data-dependent
/// ones take at least. The decode cache keeps it beside the instruction.
pub fn base(instruction: &Instruction, processor: Processor) -> u16 {
    match processor {
        Processor::M68000 => m68000(instruction),
        Processor::M68010 => m68010(instruction),
        Processor::M68020 => m68020(instruction),
    }
}

/// What the data of an instruction adds to its base on one processor, where
/// the processors differ, and the times of the exceptions that are steps or
/// parts of steps of their own: the Interpreter keeps the one of its
/// Program's processor, so that a step adds a number and asks nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Extras {
    /// A `Bcc.s` that branches, over one that does not.
    pub short_branch_taken: u64,
    /// A `Bcc.w` that does not branch, over one that does.
    pub word_branch_not_taken: u64,
    /// A `DBcc` whose condition is true, over one that branches back.
    pub dbcc_condition_true: u64,
    /// A `DBcc` whose counter runs out, over one that branches back.
    pub dbcc_expired: u64,
    /// An `Scc` to a data register whose condition is true.
    pub scc_true: u64,
    /// An IRQ taken.
    pub irq: u64,
    /// A trace exception, after the instruction it follows.
    pub trace: u64,
    /// Whether the processor has the 68010's loop mode: only the 68010 does.
    pub loop_mode: bool,
    /// Each bit a shift or a rotate of a data register moves: the 68020's
    /// barrel shifter takes no more for more bits.
    pub shift_per_bit: u64,
    /// A bit of a data register above 15 that `bchg`, `bclr` or `bset`
    /// changes.
    pub high_bit: u64,
}

impl Extras {
    /// The extras of `processor`.
    pub const fn of(processor: Processor) -> Extras {
        match processor {
            Processor::M68000 => Extras {
                short_branch_taken: BRANCH_EXTRA,
                word_branch_not_taken: BRANCH_EXTRA,
                dbcc_condition_true: DBCC_CONDITION_TRUE,
                dbcc_expired: DBCC_EXPIRED,
                scc_true: SCC_TRUE,
                irq: IRQ,
                trace: TRACE,
                loop_mode: false,
                shift_per_bit: SHIFT_PER_BIT,
                high_bit: HIGH_BIT,
            },
            Processor::M68010 => Extras {
                short_branch_taken: 4,
                word_branch_not_taken: 0,
                dbcc_condition_true: 0,
                dbcc_expired: 6,
                scc_true: 0,
                irq: m68010::IRQ,
                trace: m68010::TRACE,
                loop_mode: true,
                shift_per_bit: SHIFT_PER_BIT,
                high_bit: HIGH_BIT,
            },
            //`Bcc.s` 4 and 6 taken, `Bcc.w` and `Bcc.l` 6 either way, `DBcc` 6 and 10 when its
            //counter runs out, `Scc` 4 either way, a shift by any count and any bit alike
            Processor::M68020 => Extras {
                short_branch_taken: 2,
                word_branch_not_taken: 0,
                dbcc_condition_true: 0,
                dbcc_expired: 4,
                scc_true: 0,
                irq: m68020::IRQ,
                trace: m68020::TRACE,
                loop_mode: false,
                shift_per_bit: 0,
                high_bit: 0,
            },
        }
    }
}

/// Whether `size` is a long, which the tables give a column of its own.
fn is_long(size: Size) -> bool {
    size == Size::Long
}

/// The effective address time of `operand` read at `size`, the time of the
/// extension words and of the read itself (the 68000 User's Manual, table
/// 8-1): nothing for a register, one read for `(An)`, and the longer modes
/// more. A long reads two words.
fn effective_address(operand: &Operand, size: Size) -> u16 {
    let word = match operand {
        Operand::Register(_) => return 0,
        Operand::Indirect(_) | Operand::PostIndirect(_) | Operand::Immediate(_) => 4,
        Operand::PreIndirect(_) => 6,
        Operand::IndirectDisplacement { .. }
        | Operand::PcDisplacement { .. }
        | Operand::Absolute {
            width: AddressWidth::Word,
            ..
        } => 8,
        //the 68020's own modes, which no 68000 or 68010 decodes, as `d8(An,Xn)`
        Operand::IndirectIndex { .. } | Operand::PcIndex { .. } | Operand::Extended { .. } => 10,
        Operand::Absolute {
            width: AddressWidth::Long,
            ..
        } => 12,
    };
    match is_long(size) {
        true => word + 4,
        false => word,
    }
}

/// The time of `move`'s destination: its effective address time, but for
/// `-(An)`, whose decrement the 68000 makes while it prefetches, and which
/// takes no more than `(An)`.
fn move_destination(operand: &Operand, size: Size) -> u16 {
    match operand {
        Operand::PreIndirect(register) => effective_address(&Operand::Indirect(*register), size),
        _ => effective_address(operand, size),
    }
}

/// The time of a control addressing mode, the address it names and no read,
/// for `jmp`, `jsr`, `lea` and `pea` (table 8-10), as `(An)`, `(d16,An)`,
/// `(d8,An,Xn)`, `(xxx).W` and `(xxx).L`, the two relative to the program
/// counter taking the time of the two relative to an address register.
fn control(operand: &Operand, times: [u16; 5]) -> u16 {
    match operand {
        Operand::IndirectDisplacement { .. } | Operand::PcDisplacement { .. } => times[1],
        Operand::IndirectIndex { .. } | Operand::PcIndex { .. } | Operand::Extended { .. } => {
            times[2]
        }
        Operand::Absolute {
            width: AddressWidth::Word,
            ..
        } => times[3],
        Operand::Absolute {
            width: AddressWidth::Long,
            ..
        } => times[4],
        _ => times[0],
    }
}

/// Whether `operand` is a register or an immediate, which makes an `add`,
/// `sub`, `and`, `or` or `adda` of a long take two cycles more.
fn register_or_immediate(operand: &Operand) -> bool {
    matches!(operand, Operand::Register(_) | Operand::Immediate(_))
}

/// The 68000's table: the base time of every instruction.
fn m68000(instruction: &Instruction) -> u16 {
    use Instruction::*;
    let ea = effective_address;
    //byte or word, long
    let by = |size: Size, short: u16, long: u16| if is_long(size) { long } else { short };
    match *instruction {
        MOVE(ref source, ref destination, size) => {
            4 + ea(source, size) + move_destination(destination, size)
        }
        MOVEA(ref source, _, size) => 4 + ea(source, size),
        MOVEQ(..) => 4,
        MOVEM {
            direction,
            size,
            registers_mask,
            ref target,
        } => {
            let each = by(size, 4, 8) * registers_mask.count_ones() as u16;
            let mode = match direction {
                TargetDirection::FromMemory => control(target, [12, 16, 18, 16, 20]),
                TargetDirection::ToMemory => control(target, [8, 12, 14, 12, 16]),
            };
            mode + each
        }
        //`<ea>,Dn` and `Dn,<ea>`, the second a read and a write of memory
        ADD(ref source, ref destination, size)
        | SUB(ref source, ref destination, size)
        | AND(ref source, ref destination, size)
        | OR(ref source, ref destination, size) => match destination {
            Operand::Register(_) => {
                by(size, 4, 6)
                    + ea(source, size)
                    + 2 * (is_long(size) && register_or_immediate(source)) as u16
            }
            _ => by(size, 8, 12) + ea(destination, size),
        },
        EOR(_, ref destination, size) => match destination {
            Operand::Register(_) => by(size, 4, 8),
            _ => by(size, 8, 12) + ea(destination, size),
        },
        CMP(ref source, _, size) => by(size, 4, 6) + ea(source, size),
        ADDA(ref source, _, size) | SUBA(ref source, _, size) => {
            by(size, 8, 6)
                + ea(source, size)
                + 2 * (is_long(size) && register_or_immediate(source)) as u16
        }
        CMPA(ref source, _, size) => 6 + ea(source, size),
        ADDX(ref source, _, size) | SUBX(ref source, _, size) => match source {
            Operand::Register(_) => by(size, 4, 8),
            _ => by(size, 18, 30),
        },
        ABCD(ref source, _) | SBCD(ref source, _) => match source {
            Operand::Register(_) => 6,
            _ => 18,
        },
        CMPM(_, _, size) => by(size, 12, 20),
        ADDQ(_, ref destination, size) | SUBQ(_, ref destination, size) => match destination {
            //an address register is changed in all 32 bits, at either size (Yacht: the manual's 4
            //for a word is wrong)
            Operand::Register(RegisterOperand::Address(_)) => 8,
            Operand::Register(_) => by(size, 4, 8),
            _ => by(size, 8, 12) + ea(destination, size),
        },
        //Yacht: `andi.l #,Dn` is 16, as `addi.l` is, where the manual says 14
        ADDI(_, ref destination, size)
        | SUBI(_, ref destination, size)
        | ANDI(_, ref destination, size)
        | ORI(_, ref destination, size)
        | EORI(_, ref destination, size) => match destination {
            Operand::Register(_) => by(size, 8, 16),
            _ => by(size, 12, 20) + ea(destination, size),
        },
        CMPI(_, ref destination, size) => match destination {
            Operand::Register(_) => by(size, 8, 14),
            _ => by(size, 8, 12) + ea(destination, size),
        },
        //what the data costs is the Interpreter's to add: [`multiply`] and [`divide`]
        MULx(ref source, ..) => 38 + ea(source, Size::Word),
        DIVx(ref source, ..) => ea(source, Size::Word),
        CLR(ref operand, size)
        | NEG(ref operand, size)
        | NEGX(ref operand, size)
        | NOT(ref operand, size) => match operand {
            Operand::Register(_) => by(size, 4, 6),
            _ => by(size, 8, 12) + ea(operand, size),
        },
        NBCD(ref operand) => match operand {
            Operand::Register(_) => 6,
            _ => 8 + ea(operand, Size::Byte),
        },
        TST(ref operand, size) => 4 + ea(operand, size),
        //Yacht: 10 and the effective address on memory, where the manual says 14
        TAS(ref operand) => match operand {
            Operand::Register(_) => 4,
            _ => 10 + ea(operand, Size::Byte),
        },
        //an `Scc` that sets a data register takes [`SCC_TRUE`] more
        Scc(ref operand, _) => match operand {
            Operand::Register(_) => 4,
            _ => 8 + ea(operand, Size::Byte),
        },
        SWAP(_) | EXT(..) | NOP | TRAPV | MOVEUSP { .. } => 4,
        EXG(..) => 6,
        LEA(ref source, _) => control(source, [4, 8, 12, 8, 12]),
        PEA(ref source) => control(source, [12, 16, 20, 16, 20]),
        JMP(ref target) => control(target, [8, 10, 14, 10, 12]),
        JSR(ref target) => control(target, [16, 18, 22, 18, 20]),
        //a short branch not taken, and a word one taken, the shorter of their two times
        Bcc(_, _, size) => match size {
            BranchSize::Short => 8,
            BranchSize::Word | BranchSize::Long => 10,
        },
        BRA(..) => 10,
        BSR(..) => 18,
        //one that branches back; [`DBCC_CONDITION_TRUE`] and [`DBCC_EXPIRED`] are the others
        DBcc(..) => 10,
        LINK(..) => 16,
        UNLK(_) => 12,
        RTS => 16,
        RTE | RTR => 20,
        //a shift or a rotate of a register moves [`SHIFT_PER_BIT`] more per bit: of an
        //immediate count here, of a register's when it runs; one of memory moves one bit
        ASd(ref count, ref operand, _, size)
        | LSd(ref count, ref operand, _, size)
        | ROd(ref count, ref operand, _, size)
        | ROXd(ref count, ref operand, _, size) => match operand {
            Operand::Register(_) => by(size, 6, 8) + immediate_shift(count),
            _ => 8 + ea(operand, Size::Word),
        },
        BTST(ref bit, ref operand) => match (bit, operand) {
            (Operand::Register(_), Operand::Register(_)) => 6,
            //Yacht: a bit of an immediate takes the time of one of a register by an immediate
            (_, Operand::Register(_) | Operand::Immediate(_)) => 10,
            (Operand::Register(_), _) => 4 + ea(operand, Size::Byte),
            _ => 8 + ea(operand, Size::Byte),
        },
        //a bit above 15 of a data register takes [`HIGH_BIT`] more
        BCHG(ref bit, ref operand) | BSET(ref bit, ref operand) => match (bit, operand) {
            (Operand::Register(_), Operand::Register(_)) => 6,
            (_, Operand::Register(_)) => 10,
            (Operand::Register(_), _) => 8 + ea(operand, Size::Byte),
            _ => 12 + ea(operand, Size::Byte),
        },
        BCLR(ref bit, ref operand) => match (bit, operand) {
            (Operand::Register(_), Operand::Register(_)) => 8,
            (_, Operand::Register(_)) => 12,
            (Operand::Register(_), _) => 8 + ea(operand, Size::Byte),
            _ => 12 + ea(operand, Size::Byte),
        },
        MOVEP { size, .. } => by(size, 16, 24),
        MOVEtoCCR(ref source) | MOVEtoSR(ref source) => 12 + ea(source, Size::Word),
        MOVEfromSR(ref destination) => match destination {
            Operand::Register(_) => 6,
            _ => 8 + ea(destination, Size::Word),
        },
        //the 68010's instruction, at the 68010's time until its batch gives it a table
        MOVEfromCCR(ref destination) => match destination {
            Operand::Register(_) => 4,
            _ => 8 + ea(destination, Size::Word),
        },
        ANDItoCCR(_) | ORItoCCR(_) | EORItoCCR(_) | ANDItoSR(_) | ORItoSR(_) | EORItoSR(_) => 20,
        //out of bounds, the exception's time replaces this ([`exception`])
        CHK(ref source, _) => 10 + ea(source, Size::Word),
        //`trap #15` is EASy68K's service trap, whose tasks take no time; the other traps
        //always raise their exception, whose time is the step's ([`exception`])
        TRAP(_) => 0,
        STOP(_) => STOP_CYCLES,
        RESET => RESET_CYCLES,
        //always an exception of group 1, whose time is the step's
        ILLEGAL | IllegalWord(_) | LINEA(_) | LINEF(_) => 0,
        //EASy68K's pause, which is no instruction of the 68000's
        SIMHALT => 0,
        //the 68010's, which the 68000 never decodes; their times are `m68010`'s
        RTD(_) | BKPT(_) | MOVEC { .. } | MOVES { .. } => 0,
        //the 68020's, which neither the 68000 nor the 68010 decodes; their times are `m68020`'s
        CHKL(..)
        | MULL { .. }
        | DIVL { .. }
        | PACK(..)
        | UNPK(..)
        | CHK2(..)
        | CMP2(..)
        | CAS { .. }
        | CAS2 { .. }
        | LINKL(..)
        | TRAPcc(..) => 0,
        //EASy68K's, which its `opt bit` gives the 68000: the 68020's cache case and the 68000's
        //effective address time, for a field in four bytes of memory or fewer; one in five takes
        //[`bit_field_in_five_bytes`] more
        BF {
            operation,
            ref target,
            ..
        } => {
            let (register, memory) = match operation {
                BitFieldOperation::Test => (7, 12),
                BitFieldOperation::ExtractUnsigned | BitFieldOperation::ExtractSigned => (8, 13),
                BitFieldOperation::Change | BitFieldOperation::Clear | BitFieldOperation::Set => {
                    (12, 16)
                }
                BitFieldOperation::FindFirstOne => (18, 24),
                BitFieldOperation::Insert => (10, 15),
            };
            match target {
                Operand::Register(_) => register,
                _ => memory + ea(target, Size::Byte),
            }
        }
    }
}

/// The cycles of the bits a shift or a rotate of a data register by an
/// immediate count moves, on a 68000 and a 68010: [`SHIFT_PER_BIT`] each, in
/// its base time. A register's count is the data's, which the Interpreter
/// adds as it runs.
fn immediate_shift(count: &Operand) -> u16 {
    match *count {
        Operand::Immediate(bits) => (SHIFT_PER_BIT as u32 * (bits % 64)) as u16,
        _ => 0,
    }
}

/// The extra cycles of a bit field in memory that spans five bytes, over one
/// in four or fewer: the 68020's cache case, which EASy68K counts too
/// ([`base`]).
pub fn bit_field_in_five_bytes(operation: BitFieldOperation) -> u64 {
    match operation {
        BitFieldOperation::Test => 4,
        BitFieldOperation::ExtractUnsigned | BitFieldOperation::ExtractSigned => 5,
        BitFieldOperation::Change
        | BitFieldOperation::Clear
        | BitFieldOperation::Set
        | BitFieldOperation::FindFirstOne => 8,
        BitFieldOperation::Insert => 6,
    }
}

/// The 68010's own constants (the 68000 User's Manual, section 9).
pub mod m68010 {
    /// The cycles of an IRQ taken.
    pub const IRQ: u64 = 46;
    /// The cycles of a trace exception.
    pub const TRACE: u64 = 38;
    /// The cycles of an illegal instruction, a privilege violation and the
    /// line A and F words.
    pub const GROUP_ONE: u32 = 38;
    /// The cycles of `bkpt`: an illegal instruction after a breakpoint
    /// acknowledge cycle. The manual counts that cycle as 4 and gives 45;
    /// Moira, which cputester checks, 42.
    pub const BREAKPOINT: u32 = 42;
    /// The cycles of `movec` of a control register the 68010 does not have:
    /// the manual's 46 when it is read, and 44 when it is written, as Moira
    /// has them.
    pub const ILLEGAL_CONTROL_REGISTER: u32 = 46;
    /// The same, for a `movec` that writes the control register.
    pub const ILLEGAL_CONTROL_REGISTER_WRITTEN: u32 = 44;
    /// The cycles of `trap #0` to `#14`.
    pub const TRAP: u32 = 38;
    /// The cycles of `trapv` with V set: the manual's 38, Moira's 40.
    pub const TRAPV: u32 = 40;
    /// The cycles of a division by zero, besides its source's effective
    /// address.
    pub const DIVISION_BY_ZERO: u32 = 42;
    /// The cycles of `chk` out of bounds above the bound, besides its
    /// source's effective address.
    pub const CHK_ABOVE: u32 = 42;
    /// The cycles of `chk` out of bounds below zero, besides its source's
    /// effective address: the manual's 44, a maximum.
    pub const CHK_BELOW: u32 = 44;
    /// The cycles of `rte` of a frame whose format is not the 68010's.
    pub const FORMAT_ERROR: u32 = 50;
    /// The cycles of `rte` of a format `$8` frame whose version is not the
    /// one s68k writes.
    pub const ILLEGAL_REVISION: u32 = 70;
    /// The cycles of an address error or a bus error from the start of the
    /// access it aborted: the manual's 126, and the eight of the aborted
    /// access the 68000's [`GROUP_ZERO`](super::GROUP_ZERO) counts as well.
    pub const GROUP_ZERO: u32 = 134;
    /// The cycles of `reset`.
    pub const RESET_CYCLES: u16 = 130;
}

/// The cycles of a `DBcc` that branches back in the 68010's loop mode: it
/// fetches nothing.
pub const LOOP_CONTINUE: u16 = 6;

/// The cycles of a `DBcc` in loop mode whose condition is true, which ends
/// the loop and fetches the instructions after it.
pub const LOOP_CONDITION_TRUE: u64 = 12;

/// What the 68010's loop mode does to an instruction it can loop (the 68000
/// User's Manual, appendix A): one of one word, with its memory operands
/// `(An)`, `(An)+` or `-(An)`, and a `DBcc` of displacement −4 after it.
///
/// In loop mode the processor fetches neither the instruction nor the `DBcc`
/// again: the instruction takes its time and [`change`](LoopMode::change),
/// the `DBcc` that branches back [`LOOP_CONTINUE`], and the one whose counter
/// runs out [`exit`](LoopMode::exit), which depends on the instruction it
/// ends the loop of. The values are Moira's, measured on every form, whose
/// 68010 is checked against cputester, and its sums are the manual's tables
/// 9-3 to 9-17 where those agree with it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoopMode {
    /// What the instruction takes in loop mode beside its time out of it:
    /// the fetch it does not make, and what the loop's own microcode adds.
    pub change: i16,
    /// The cycles of the `DBcc` whose counter runs out after it.
    pub exit: u16,
}

/// The loop mode of `instruction`, or `None` when the 68010 cannot loop it.
pub fn loop_mode(instruction: &Instruction) -> Option<LoopMode> {
    use Instruction::*;
    //the three modes of a loop, and an operand that is a register
    let looped = |operand: &Operand| {
        matches!(
            operand,
            Operand::Indirect(_) | Operand::PostIndirect(_) | Operand::PreIndirect(_)
        )
    };
    let register = |operand: &Operand| matches!(operand, Operand::Register(_));
    let mode = |change: i16, exit: u16| Some(LoopMode { change, exit });
    match *instruction {
        MOVE(ref source, ref destination, size)
            if (looped(source) || register(source)) && looped(destination) =>
        {
            let change = match destination {
                Operand::PreIndirect(_) => -2,
                _ => -4,
            };
            let exit = match (size, source, destination) {
                (
                    Size::Long,
                    Operand::Register(_),
                    Operand::Indirect(_) | Operand::PostIndirect(_),
                ) => 10,
                (Size::Long, ..) => 8,
                _ => 10,
            };
            mode(change, exit)
        }
        CLR(ref operand, _) if looped(operand) => mode(-4, 12),
        NEG(ref operand, _) | NEGX(ref operand, _) | NOT(ref operand, _) if looped(operand) => {
            mode(-2, 10)
        }
        NBCD(ref operand) if looped(operand) => mode(0, 10),
        TST(ref operand, size) if looped(operand) => match size {
            Size::Long => mode(0, 8),
            _ => mode(-2, 10),
        },
        ADD(ref source, Operand::Register(_), _)
        | SUB(ref source, Operand::Register(_), _)
        | AND(ref source, Operand::Register(_), _)
        | OR(ref source, Operand::Register(_), _)
            if looped(source) =>
        {
            mode(2, 10)
        }
        ADD(Operand::Register(_), ref destination, _)
        | SUB(Operand::Register(_), ref destination, _)
        | AND(Operand::Register(_), ref destination, _)
        | OR(Operand::Register(_), ref destination, _)
        | EOR(Operand::Register(_), ref destination, _)
            if looped(destination) =>
        {
            mode(-2, 10)
        }
        CMP(ref source, _, size) if looped(source) => match size {
            Size::Long => mode(0, 10),
            _ => mode(-2, 10),
        },
        ADDA(ref source, _, size) | SUBA(ref source, _, size) if looped(source) => match size {
            Size::Long => mode(2, 10),
            _ => mode(0, 10),
        },
        CMPA(ref source, ..) if looped(source) => mode(-2, 10),
        ADDX(Operand::PreIndirect(_), _, size) | SUBX(Operand::PreIndirect(_), _, size) => {
            match size {
                Size::Long => mode(-4, 10),
                _ => mode(-2, 10),
            }
        }
        ABCD(Operand::PreIndirect(_), _) | SBCD(Operand::PreIndirect(_), _) => mode(0, 10),
        CMPM(_, _, size) => match size {
            Size::Long => mode(-2, 10),
            _ => mode(-4, 10),
        },
        ASd(_, ref operand, ..)
        | LSd(_, ref operand, ..)
        | ROd(_, ref operand, ..)
        | ROXd(_, ref operand, ..)
            if looped(operand) =>
        {
            mode(0, 10)
        }
        _ => None,
    }
}

/// The effective address time of `operand` when the instruction does not
/// read it, only works its address out, for `clr`, `Scc`, `tas` and
/// `move sr,<ea>` on the 68010 (table 9-1, "no fetch"), the same at every
/// size.
fn no_fetch(operand: &Operand) -> u16 {
    match operand {
        Operand::Register(_) => 0,
        Operand::Indirect(_) => 2,
        Operand::PostIndirect(_)
        | Operand::PreIndirect(_)
        | Operand::IndirectDisplacement { .. }
        | Operand::PcDisplacement { .. }
        | Operand::Absolute {
            width: AddressWidth::Word,
            ..
        } => 4,
        Operand::IndirectIndex { .. }
        | Operand::PcIndex { .. }
        | Operand::Extended { .. }
        | Operand::Absolute {
            width: AddressWidth::Long,
            ..
        } => 8,
        Operand::Immediate(_) => 0,
    }
}

/// The 68010's table: the base time of every instruction, where it differs
/// from the 68000's, which every other row keeps (the 68000 User's Manual,
/// section 9).
fn m68010(instruction: &Instruction) -> u16 {
    use Instruction::*;
    let ea = effective_address;
    let by = |size: Size, short: u16, long: u16| if is_long(size) { long } else { short };
    let memory = |operand: &Operand| !register_or_immediate(operand);
    match *instruction {
        //no two cycles more for a register or an immediate, which `adda` and `suba` keep
        ADD(ref source, Operand::Register(_), size)
        | SUB(ref source, Operand::Register(_), size)
        | AND(ref source, Operand::Register(_), size)
        | OR(ref source, Operand::Register(_), size) => by(size, 4, 6) + ea(source, size),
        //a long from memory takes two less
        CMP(ref source, _, Size::Long) if memory(source) => 4 + ea(source, Size::Long),
        EOR(_, Operand::Register(_), size) => by(size, 4, 6),
        ADDI(_, Operand::Register(_), Size::Long)
        | SUBI(_, Operand::Register(_), Size::Long)
        | ANDI(_, Operand::Register(_), Size::Long)
        | ORI(_, Operand::Register(_), Size::Long)
        | EORI(_, Operand::Register(_), Size::Long) => 14,
        CMPI(_, Operand::Register(_), Size::Long) => 12,
        ADDX(Operand::Register(_), _, Size::Long) | SUBX(Operand::Register(_), _, Size::Long) => 6,
        //what the data costs is the Interpreter's to add: [`multiply`] and [`divide`]
        MULx(ref source, ..) => 40 + ea(source, Size::Word),
        //no read before the write (table 9-10); a long of a data register is 4, as Moira has it
        CLR(Operand::Register(_), _) => 4,
        CLR(ref operand, size) => {
            by(size, 4, 6)
                + match operand {
                    Operand::Register(_) => 0,
                    Operand::Indirect(_) | Operand::PostIndirect(_) => 4,
                    Operand::PreIndirect(_) => 6,
                    Operand::IndirectIndex { .. }
                    | Operand::PcIndex { .. }
                    | Operand::Extended { .. }
                    | Operand::Absolute {
                        width: AddressWidth::Long,
                        ..
                    } => 12,
                    _ => 8,
                }
                + is_long(size) as u16 * (!register_or_immediate(operand)) as u16 * 2
        }
        Scc(ref operand, _) => match operand {
            Operand::Register(_) => 4,
            _ => 8 + no_fetch(operand),
        },
        TAS(ref operand) => match operand {
            Operand::Register(_) => 4,
            _ => 14 + no_fetch(operand),
        },
        MOVEfromSR(ref destination) | MOVEfromCCR(ref destination) => match destination {
            Operand::Register(_) => 4,
            Operand::IndirectIndex { .. } | Operand::Extended { .. } => 14,
            _ => 8 + no_fetch(destination),
        },
        MOVEUSP { .. } => 6,
        ANDItoCCR(_) | ORItoCCR(_) | EORItoCCR(_) | ANDItoSR(_) | ORItoSR(_) | EORItoSR(_) => 16,
        CHK(ref source, _) => 8 + ea(source, Size::Word),
        //a short branch not taken, which [`Extras::short_branch_taken`] makes one taken
        Bcc(_, _, BranchSize::Short) => 6,
        //a bit of an immediate takes the time of one of memory
        BTST(Operand::Register(_), Operand::Immediate(_)) => 8,
        //a bit cleared in memory by a register's number takes two more; by an immediate's, not
        BCLR(Operand::Register(_), ref operand) if memory(operand) => 10 + ea(operand, Size::Byte),
        RTE => 24,
        RESET => m68010::RESET_CYCLES,
        RTD(_) => 16,
        //always an illegal instruction, whose time is the step's
        BKPT(_) => 0,
        //the control register into the general one, then the other way
        MOVEC { to_control, .. } => match to_control {
            false => 10,
            true => 12,
        },
        MOVES {
            size, ref target, ..
        } => {
            let long = is_long(size) as u16 * 4;
            18 + long
                + match target {
                    Operand::Indirect(_) => 0,
                    Operand::IndirectIndex { .. }
                    | Operand::Extended { .. }
                    | Operand::Absolute {
                        width: AddressWidth::Long,
                        ..
                    } => 6,
                    _ => 2,
                }
        }
        _ => m68000(instruction),
    }
}

/// The 68020's own constants: the cache case of the M68020 User's Manual's
/// exception tables (8.2.17 and 8.2.18 in the 1995 edition), and the times
/// this estimate takes where the manual gives none.
pub mod m68020 {
    /// The cycles of an IRQ taken: "Interrupt (I-Stack)".
    pub const IRQ: u64 = 26;
    /// What an IRQ taken with M set adds for the throwaway frame it pushes
    /// on the interrupt stack: "Interrupt (M-Stack)" is 41.
    pub const THROWAWAY_FRAME: u64 = 15;
    /// The cycles of a trace exception.
    pub const TRACE: u64 = 25;
    /// The cycles of an illegal instruction, a privilege violation and the
    /// line A and F words.
    pub const GROUP_ONE: u32 = 20;
    /// The cycles of `trap #0` to `#14`, and of a `trap #15` taken through
    /// vector 47.
    pub const TRAP: u32 = 20;
    /// The cycles of `trapcc` and `trapv` that trap, which push the six words
    /// of format `$2`. The manual gives no time for `chk`, `chk.l` and `chk2`
    /// out of bounds, nor for a division by zero, which push the same frame:
    /// they take this, and their source's effective address, as on the 68000
    /// they take its trap's time and theirs.
    pub const TRAP_WITH_ADDRESS: u32 = 25;
    /// The cycles of `bkpt`: the manual's breakpoint acknowledge cycle, 10,
    /// and the illegal instruction exception that follows it when nothing
    /// answers, 20.
    pub const BREAKPOINT: u32 = 30;
    /// The cycles of `rte` of a frame it cannot return from, which the
    /// manual does not give: `rte`'s own 21, and an exception's 20.
    pub const FORMAT_ERROR: u32 = 41;
    /// The cycles of a bus error that pushes the short bus fault frame,
    /// `$A`: "Bus Cycle Fault (Short)".
    pub const SHORT_BUS_FAULT: u32 = 43;
    /// The cycles of a bus or an address error that pushes the long bus
    /// fault frame, `$B`: "Bus Cycle Fault (Long)". The manual gives no time
    /// for what the instruction did before the fault, which this leaves out.
    pub const LONG_BUS_FAULT: u32 = 79;
    /// The cycles of `rte` of a frame of format `$0` or `$2`.
    pub const RTE: u16 = 21;
    /// What `rte` adds for a throwaway frame, `$1`, which it pops before the
    /// frame under it: "RTE (Throwaway)" is 16, and the time of the `rte` of
    /// that frame.
    pub const RTE_THROWAWAY: u64 = 16;
    /// What `rte` adds for the short bus fault frame: 43 in all.
    pub const RTE_SHORT_FAULT: u64 = 22;
    /// What `rte` adds for the long bus fault frame: 92 in all.
    pub const RTE_LONG_FAULT: u64 = 71;
    /// What `cas` and `cas2` add when the compare succeeds and they write:
    /// 15 and 25, where 12 and 22 do not write.
    pub const SWAPPED: u64 = 3;
    /// The cycles `stop` takes before the processor waits.
    pub const STOP_CYCLES: u16 = 8;
    /// The cycles of `reset`.
    pub const RESET_CYCLES: u16 = 518;
}

/// What `rte` on a 68020 adds to its [`m68020::RTE`] for the frame it
/// returned from, of `format`, after a throwaway frame when `throwaway`.
pub fn rte_68020(format: u16, throwaway: bool) -> u64 {
    let fault = match format {
        0xA => m68020::RTE_SHORT_FAULT,
        0xB => m68020::RTE_LONG_FAULT,
        _ => 0,
    };
    fault + throwaway as u64 * m68020::RTE_THROWAWAY
}

/// The row of `operand`, read at `size`, in the 68020's effective address
/// tables: its index in [`FETCH`] and the others. The rows are the 68000's
/// modes, `Dn` or `An`, `(An)`, `(An)+`, `-(An)`, `(d16,An)` or `(d16,PC)`
/// and the absolute word and long, then an immediate of a byte or a word and
/// one of a long, then the brief extension word, `(d8,An,Xn)` and its 68020
/// form with a scale, and last the full one's twelve, by its base
/// displacement, null, a word or a long, each without memory indirection and
/// then with an outer displacement null, a word or a long: `(B)`, `([B],I)`,
/// `([B],I,d16)`, `([B],I,d32)`, `(d16,B)`, `([d16,B],I)`, and so on. As the
/// manual's notes say, the base being an address register, the program
/// counter or suppressed, the index being there or not, before or after the
/// indirection, its size and its scale change nothing. A table's row for a
/// mode none of its instructions takes, an immediate in the calculate tables
/// or `(An)+` in the jump table, is 0.
///
/// The manual also has a row for `(d16,An,Xn)`, a full extension word with a
/// word displacement, whose times are lower than those of the same word's
/// `(d16,B)` row; this takes the `(d16,B)` row, as Musashi does.
fn row(operand: &Operand, size: Size) -> usize {
    let width = |size: DisplacementSize| match size {
        DisplacementSize::Null => 0,
        DisplacementSize::Word => 1,
        DisplacementSize::Long => 2,
    };
    match *operand {
        Operand::Register(_) => 0,
        Operand::Indirect(_) => 1,
        Operand::PostIndirect(_) => 2,
        Operand::PreIndirect(_) => 3,
        Operand::IndirectDisplacement { .. } | Operand::PcDisplacement { .. } => 4,
        Operand::Absolute {
            width: AddressWidth::Word,
            ..
        } => 5,
        Operand::Absolute {
            width: AddressWidth::Long,
            ..
        } => 6,
        Operand::Immediate(_) => 7 + is_long(size) as usize,
        Operand::IndirectIndex { .. } | Operand::PcIndex { .. } => 9,
        Operand::Extended { word, .. } if !word.is_full() => 9,
        Operand::Extended { word, .. } => {
            let outer = match word.indirection() {
                Indirection::None => 0,
                _ => 1 + width(word.outer_displacement_size()),
            };
            10 + 4 * width(word.base_displacement_size()) + outer
        }
    }
}

/// Fetch effective address (the manual's 8.2.1): working out the address and
/// reading the operand, by [`row`].
const FETCH: [u16; 22] = [
    0, 4, 4, 5, 5, 4, 4, 2, 4, 7, 7, 12, 14, 14, 9, 14, 16, 16, 13, 18, 20, 20,
];

/// Fetch immediate effective address (8.2.2): an immediate word, or an
/// extension word, then the operand, by [`row`].
const FETCH_IMMEDIATE_WORD: [u16; 22] = [
    2, 4, 6, 5, 5, 5, 6, 4, 6, 9, 9, 14, 16, 16, 11, 16, 18, 18, 15, 20, 22, 22,
];

/// Fetch immediate effective address with an immediate long, by [`row`].
/// The manual's table has no row for `([d16,B],I,d32)` here, nor in the word
/// column: it takes `([d16,B],I,d16)`'s, as every other outer long does its
/// word's.
const FETCH_IMMEDIATE_LONG: [u16; 22] = [
    4, 4, 8, 7, 7, 7, 8, 6, 8, 11, 11, 16, 18, 18, 13, 18, 20, 20, 17, 22, 24, 24,
];

/// Calculate effective address (8.2.3): working out the address without
/// reading the operand, but for the pointer a memory indirect mode reads, by
/// [`row`].
const CALCULATE: [u16; 22] = [
    0, 2, 2, 2, 2, 2, 4, 0, 0, 4, 6, 11, 13, 13, 8, 13, 15, 15, 12, 17, 19, 19,
];

/// Calculate immediate effective address (8.2.4): an extension word, then
/// the address, by [`row`]. The manual has no row for `-(An)`, which takes
/// `(An)+`'s, as it does in the calculate table.
const CALCULATE_IMMEDIATE: [u16; 22] = [
    2, 2, 4, 4, 4, 4, 4, 0, 0, 6, 8, 13, 15, 15, 10, 15, 17, 17, 14, 19, 21, 21,
];

/// Jump effective address (8.2.5): working out the address `jmp` and `jsr`
/// go to, by [`row`]; `(d16,PC)` is `(d16,An)`.
const JUMP: [u16; 22] = [
    0, 2, 0, 0, 4, 2, 2, 0, 0, 6, 6, 11, 13, 13, 8, 13, 15, 15, 12, 17, 19, 19,
];

/// The time of `move`'s destination from a register, by [`row`]: the
/// manual's MOVE table (8.2.6) is two cycles, the source's fetch and this.
const MOVE_FROM_REGISTER: [u16; 22] = [
    0, 2, 2, 3, 3, 2, 4, 0, 0, 5, 6, 10, 12, 13, 8, 12, 14, 15, 12, 16, 18, 19,
];

/// The time of `move`'s destination from an immediate: a register's to the
/// 68000's modes, and two cycles less to the 68020's.
const MOVE_FROM_IMMEDIATE: [u16; 22] = [
    0, 2, 2, 3, 3, 2, 4, 0, 0, 3, 4, 8, 10, 11, 6, 10, 12, 13, 10, 14, 16, 17,
];

/// The time of `move`'s destination from memory, whose write overlaps the
/// read before it.
const MOVE_FROM_MEMORY: [u16; 22] = [
    0, 1, 1, 1, 1, 1, 3, 0, 0, 3, 4, 8, 10, 11, 6, 10, 12, 13, 10, 14, 16, 17,
];

/// The time of fetching `operand` at `size` on a 68020 ([`FETCH`]).
fn fetch(operand: &Operand, size: Size) -> u16 {
    FETCH[row(operand, size)]
}

/// The time of fetching an immediate of `size` and then `operand`, which is
/// read at that size ([`FETCH_IMMEDIATE_WORD`]): an extension word is an
/// immediate word.
fn fetch_immediate(size: Size, operand: &Operand) -> u16 {
    let row = row(operand, size);
    match is_long(size) {
        true => FETCH_IMMEDIATE_LONG[row],
        false => FETCH_IMMEDIATE_WORD[row],
    }
}

/// The time of an extension word, then of fetching `operand`, a long when it
/// is an immediate: `muls.l`, `divs.l` and their kin.
fn fetch_after_a_word(operand: &Operand) -> u16 {
    FETCH_IMMEDIATE_WORD[row(operand, Size::Long)]
}

/// The time of working out `operand`'s address ([`CALCULATE`]).
fn calculate(operand: &Operand) -> u16 {
    CALCULATE[row(operand, Size::Word)]
}

/// The time of an extension word, then of working out `operand`'s address
/// ([`CALCULATE_IMMEDIATE`]).
fn calculate_immediate(operand: &Operand) -> u16 {
    CALCULATE_IMMEDIATE[row(operand, Size::Word)]
}

/// The time of `move` on a 68020, from the manual's table (8.2.6), which is
/// two cycles, the source's fetch, and a time of the destination that
/// depends on whether the source is a register, an immediate or memory.
fn move_68020(source: &Operand, destination: &Operand, size: Size) -> u16 {
    let to = row(destination, size);
    let destination = match source {
        Operand::Register(_) => MOVE_FROM_REGISTER[to],
        Operand::Immediate(_) => MOVE_FROM_IMMEDIATE[to],
        _ => MOVE_FROM_MEMORY[to],
    };
    2 + fetch(source, size) + destination
}

/// The 68020's table: the cache case of every instruction, from the manual's
/// tables 8.2.6 to 8.2.17, with the effective address tables its notes say
/// to add. An instruction of an extension word, `movem`, `chk2`, `muls.l`, a
/// bit field and their kin, adds the immediate tables' time of a word for it.
///
/// Where the data counts, this is what it takes at least: `Bcc.s` not taken,
/// `DBcc` that branches back, `cas` and `cas2` that do not write, and `rte`
/// of a frame of four or six words; [`Extras`] and [`rte_68020`] add the
/// rest. The manual's times do not depend on the data otherwise: a shift by
/// any count, a multiply or a division by any operand, `movem` by its count
/// of registers.
///
/// The manual gives no time for `nbcd` of memory, which takes Musashi's,
/// `nbcd`'s 6 and the fetch.
fn m68020(instruction: &Instruction) -> u16 {
    use Instruction::*;
    let register = |operand: &Operand| matches!(operand, Operand::Register(_));
    match *instruction {
        MOVE(ref source, ref destination, size) => move_68020(source, destination, size),
        MOVEA(ref source, _, size) => 2 + fetch(source, size),
        MOVEQ(..) => 2,
        MOVEM {
            direction,
            registers_mask,
            ref target,
            ..
        } => {
            let count = registers_mask.count_ones() as u16;
            let registers = match direction {
                TargetDirection::FromMemory => 8 + 4 * count,
                TargetDirection::ToMemory => 4 + 3 * count,
            };
            registers + calculate_immediate(target)
        }
        ADD(ref source, ref destination, size)
        | SUB(ref source, ref destination, size)
        | AND(ref source, ref destination, size)
        | OR(ref source, ref destination, size) => match destination {
            Operand::Register(_) => 2 + fetch(source, size),
            _ => 4 + fetch(destination, size),
        },
        EOR(_, ref destination, size) => match destination {
            Operand::Register(_) => 2,
            _ => 4 + fetch(destination, size),
        },
        CMP(ref source, _, size) | ADDA(ref source, _, size) | SUBA(ref source, _, size) => {
            2 + fetch(source, size)
        }
        CMPA(ref source, _, size) => 4 + fetch(source, size),
        ADDX(ref source, ..) | SUBX(ref source, ..) => match register(source) {
            true => 2,
            false => 12,
        },
        ABCD(ref source, _) | SBCD(ref source, _) => match register(source) {
            true => 4,
            false => 16,
        },
        CMPM(..) => 9,
        ADDQ(_, ref destination, size) | SUBQ(_, ref destination, size) => match destination {
            Operand::Register(_) => 2,
            _ => 4 + fetch(destination, size),
        },
        ADDI(_, ref destination, size)
        | SUBI(_, ref destination, size)
        | ANDI(_, ref destination, size)
        | ORI(_, ref destination, size)
        | EORI(_, ref destination, size) => match destination {
            Operand::Register(_) => 2 + fetch_immediate(size, destination),
            _ => 4 + fetch_immediate(size, destination),
        },
        CMPI(_, ref destination, size) => 2 + fetch_immediate(size, destination),
        MULx(ref source, ..) => 27 + fetch(source, Size::Word),
        DIVx(ref source, _, sign) => {
            let divide = match sign {
                Sign::Unsigned => 44,
                Sign::Signed => 56,
            };
            divide + fetch(source, Size::Word)
        }
        MULL { ref source, .. } => 43 + fetch_after_a_word(source),
        DIVL {
            ref source, sign, ..
        } => {
            let divide = match sign {
                Sign::Unsigned => 78,
                Sign::Signed => 90,
            };
            divide + fetch_after_a_word(source)
        }
        CLR(ref operand, _) => match operand {
            Operand::Register(_) => 2,
            _ => 4 + calculate(operand),
        },
        NEG(ref operand, size) | NEGX(ref operand, size) | NOT(ref operand, size) => {
            match operand {
                Operand::Register(_) => 2,
                _ => 4 + fetch(operand, size),
            }
        }
        NBCD(ref operand) => 6 + fetch(operand, Size::Byte),
        TST(ref operand, size) => 2 + fetch(operand, size),
        TAS(ref operand) => match operand {
            Operand::Register(_) => 4,
            _ => 12 + calculate(operand),
        },
        Scc(ref operand, _) => match operand {
            Operand::Register(_) => 4,
            _ => 6 + calculate(operand),
        },
        SWAP(_) | EXT(..) | TRAPV => 4,
        EXG(..) | NOP | MOVEUSP { .. } => 2,
        LEA(ref source, _) => 2 + calculate(source),
        PEA(ref source) => 5 + calculate(source),
        JMP(ref target) => 4 + JUMP[row(target, Size::Long)],
        JSR(ref target) => 5 + JUMP[row(target, Size::Long)],
        //a short branch not taken, which [`Extras::short_branch_taken`] makes one taken
        Bcc(_, _, size) => match size {
            BranchSize::Short => 4,
            BranchSize::Word | BranchSize::Long => 6,
        },
        BRA(..) => 6,
        BSR(..) => 7,
        DBcc(..) => 6,
        LINK(..) => 5,
        LINKL(..) => 6,
        UNLK(_) => 6,
        RTS | RTD(_) => 10,
        RTR => 14,
        //of a frame of four or six words; [`rte_68020`] adds the others'
        RTE => m68020::RTE,
        //a register by an immediate count, or by a register's; memory by one bit
        LSd(ref count, ref operand, ..) => match (count, operand) {
            (Operand::Register(_), Operand::Register(_)) => 6,
            (_, Operand::Register(_)) => 4,
            _ => 5 + fetch(operand, Size::Word),
        },
        ASd(_, ref operand, direction, _) => match (operand, direction) {
            (Operand::Register(_), ShiftDirection::Left) => 8,
            (Operand::Register(_), ShiftDirection::Right) => 6,
            (_, ShiftDirection::Left) => 6 + fetch(operand, Size::Word),
            (_, ShiftDirection::Right) => 5 + fetch(operand, Size::Word),
        },
        ROd(_, ref operand, ..) => match operand {
            Operand::Register(_) => 8,
            _ => 7 + fetch(operand, Size::Word),
        },
        ROXd(_, ref operand, ..) => match operand {
            Operand::Register(_) => 12,
            _ => 5 + fetch(operand, Size::Word),
        },
        //the bit's number in a register, or an immediate word before the operand
        BTST(ref bit, ref operand)
        | BCHG(ref bit, ref operand)
        | BCLR(ref bit, ref operand)
        | BSET(ref bit, ref operand) => match (bit, operand) {
            (_, Operand::Register(_)) => 4,
            (Operand::Register(_), _) => 4 + fetch(operand, Size::Byte),
            _ => 4 + fetch_immediate(Size::Word, operand),
        },
        MOVEP {
            direction, size, ..
        } => match (direction, is_long(size)) {
            (TargetDirection::ToMemory, false) => 11,
            (TargetDirection::ToMemory, true) => 17,
            (TargetDirection::FromMemory, false) => 12,
            (TargetDirection::FromMemory, true) => 18,
        },
        MOVEtoCCR(ref source) => 4 + fetch(source, Size::Word),
        MOVEtoSR(ref source) => 8 + fetch(source, Size::Word),
        MOVEfromSR(ref destination) | MOVEfromCCR(ref destination) => match destination {
            Operand::Register(_) => 4,
            _ => 5 + calculate(destination),
        },
        ANDItoCCR(_) | ORItoCCR(_) | EORItoCCR(_) | ANDItoSR(_) | ORItoSR(_) | EORItoSR(_) => 12,
        //in bounds; out of them, the exception's time replaces this ([`exception`])
        CHK(ref source, _) => 8 + fetch(source, Size::Word),
        CHKL(ref source, _) => 8 + fetch(source, Size::Long),
        CHK2(ref bounds, ..) | CMP2(ref bounds, ..) => 18 + fetch_immediate(Size::Word, bounds),
        //`trap #15` is EASy68K's service trap, whose tasks take no time; the other traps
        //always raise their exception, whose time is the step's ([`exception`])
        TRAP(_) => 0,
        STOP(_) => m68020::STOP_CYCLES,
        RESET => m68020::RESET_CYCLES,
        //always an exception, whose time is the step's
        ILLEGAL | IllegalWord(_) | LINEA(_) | LINEF(_) | BKPT(_) => 0,
        //EASy68K's pause, which is no instruction of the 68020's
        SIMHALT => 0,
        //the control register into the general one, then the other way
        MOVEC { to_control, .. } => match to_control {
            false => 6,
            true => 12,
        },
        MOVES {
            direction,
            ref target,
            ..
        } => {
            let moves = match direction {
                TargetDirection::FromMemory => 7,
                TargetDirection::ToMemory => 5,
            };
            moves + calculate_immediate(target)
        }
        PACK(ref source, ..) => match register(source) {
            true => 6,
            false => 13,
        },
        UNPK(ref source, ..) => match register(source) {
            true => 8,
            false => 13,
        },
        //one whose compare fails; [`m68020::SWAPPED`] more for one that writes
        CAS { ref target, .. } => 12 + calculate_immediate(target),
        CAS2 { .. } => 22,
        //a field in a register, or in four bytes of memory or fewer; one in five takes
        //[`bit_field_in_five_bytes`] more
        BF {
            operation,
            ref target,
            ..
        } => {
            let (register, memory) = match operation {
                BitFieldOperation::Test => (6, 11),
                BitFieldOperation::ExtractUnsigned | BitFieldOperation::ExtractSigned => (8, 13),
                BitFieldOperation::Change | BitFieldOperation::Clear | BitFieldOperation::Set => {
                    (12, 16)
                }
                BitFieldOperation::FindFirstOne => (18, 24),
                BitFieldOperation::Insert => (10, 14),
            };
            match target {
                Operand::Register(_) => register,
                _ => memory + calculate_immediate(target),
            }
        }
        //one not taken; one taken is the exception's time ([`exception`])
        TRAPcc(_, operand) => match operand {
            TrapOperand::None => 4,
            TrapOperand::Word(_) => 6,
            TrapOperand::Long(_) => 8,
        },
    }
}

/// The extra cycles of `mulu` or `muls` by `source`, the word read, on
/// `processor`.
///
/// On a 68000, two for each bit set of an unsigned one, and for each change
/// between two bits of a signed one, a zero below its lowest bit (the 68000
/// User's Manual, section 8.3, with Yacht's reading of it). On a 68010 an
/// unsigned one takes none, and a signed one two when the source is negative:
/// the manual gives only the maxima, 40 and 42, and Moira's model, which
/// cputester checks, the rest. On a 68020 none: its cache case is 27 for any
/// source.
pub fn multiply(source: u16, sign: Sign, processor: Processor) -> u64 {
    if processor.has(Processor::M68010) {
        if processor.has(Processor::M68020) {
            return 0;
        }
        return match sign {
            Sign::Unsigned => 0,
            Sign::Signed => 2 * (source >> 15) as u64,
        };
    }
    let changes = match sign {
        Sign::Unsigned => source.count_ones(),
        Sign::Signed => {
            let bits = (source as u32) << 1;
            ((bits ^ (bits >> 1)) & 0xFFFF).count_ones()
        }
    };
    2 * changes as u64
}

/// The cycles of `divu` or `divs` of `dividend` by `divisor`, which is not
/// zero, besides the source's effective address.
///
/// The 68000 divides with a loop of microcode whose time depends on the
/// bits of the quotient, which Jorge Cwik measured and Yacht's flowcharts
/// draw. This follows the flowcharts' method, and is not his code, which is
/// GPL:
///
/// * `divu` first compares the high word of the dividend with the divisor;
///   when it is not below, the quotient overflows and the instruction takes
///   10. Otherwise it shifts the dividend left fifteen times, comparing it
///   with the divisor shifted to the high word each time: a step that shifts
///   a one out of the top subtracts at once, one that can subtract takes two
///   more, and one that cannot four more, on 76.
/// * `divs` works on the absolute values: an overflow of their quotient takes
///   16, or 18 for a negative dividend. Otherwise it takes 122, two more for a
///   negative dividend; by a positive divisor, two less for a positive
///   dividend and two more for a negative one; and two more for each zero
///   among the fifteen high bits of the absolute quotient's word: 120 to 156.
///
/// The 68010 divides faster: Moira's model, which cputester checks, of the
/// 68010's microcode, the manual giving only the maxima, 108 and 122 with the
/// source.
///
/// * `divu` overflows in 8, and otherwise takes 78 and two more for each of
///   its fifteen steps that shifts no one out of the top: 108 at most.
/// * `divs` overflows in 16, and otherwise takes 118, or 120 for a negative
///   dividend.
///
/// The 68020's cache case is 44 for `divu` and 56 for `divs`, its base, for
/// any operands: this adds nothing to it.
pub fn divide(dividend: u32, divisor: u16, sign: Sign, processor: Processor) -> u64 {
    if processor.has(Processor::M68010) {
        if processor.has(Processor::M68020) {
            return 0;
        }
        return match sign {
            Sign::Unsigned => divide_unsigned_68010(dividend, divisor),
            Sign::Signed => divide_signed_68010(dividend as i32, divisor as i16),
        };
    }
    match sign {
        Sign::Unsigned => divide_unsigned(dividend, divisor),
        Sign::Signed => divide_signed(dividend as i32, divisor as i16),
    }
}

fn divide_unsigned_68010(dividend: u32, divisor: u16) -> u64 {
    let divisor = divisor as u32;
    if dividend >> 16 >= divisor {
        return 8;
    }
    let shifted = divisor << 16;
    let mut remainder = dividend;
    let mut cycles = 78;
    for _ in 0..15 {
        let carried = remainder & 0x8000_0000 != 0;
        remainder <<= 1;
        if carried {
            remainder = remainder.wrapping_sub(shifted);
        } else {
            if remainder >= shifted {
                remainder -= shifted;
            }
            cycles += 2;
        }
    }
    cycles
}

fn divide_signed_68010(dividend: i32, divisor: i16) -> u64 {
    if dividend.unsigned_abs() >> 16 >= divisor.unsigned_abs() as u32 {
        return 16;
    }
    match dividend < 0 {
        true => 120,
        false => 118,
    }
}

fn divide_unsigned(dividend: u32, divisor: u16) -> u64 {
    let divisor = divisor as u32;
    if dividend >> 16 >= divisor {
        return 10;
    }
    let shifted = divisor << 16;
    let mut remainder = dividend;
    let mut cycles = 76;
    for _ in 0..15 {
        let carried = remainder & 0x8000_0000 != 0;
        remainder <<= 1;
        if carried {
            remainder = remainder.wrapping_sub(shifted);
        } else if remainder >= shifted {
            remainder -= shifted;
            cycles += 2;
        } else {
            cycles += 4;
        }
    }
    cycles
}

fn divide_signed(dividend: i32, divisor: i16) -> u64 {
    let negative = dividend < 0;
    let (dividend, divisor_bits) = (dividend.unsigned_abs(), divisor.unsigned_abs() as u32);
    if dividend >> 16 >= divisor_bits {
        return if negative { 18 } else { 16 };
    }
    let quotient = dividend / divisor_bits;
    let mut cycles = 122 + 2 * negative as u64;
    if divisor >= 0 {
        cycles = match negative {
            true => cycles + 2,
            false => cycles - 2,
        };
    }
    //bits 15 down to 1 of the quotient, which is below $10000 when it does not overflow
    let zeros = 15 - (quotient & 0xFFFE).count_ones() as u64;
    cycles + 2 * zeros
}

/// The cycles a step takes whose instruction raised `error`, the exception
/// through `vector`: they replace the instruction's own.
///
/// * The address error and the bus error abort the instruction at the access
///   that faulted: the time of the instruction until that access,
///   [`address_error_lead`], and [`GROUP_ZERO`]. `source_faulted` says which
///   of two operands in memory an `addx`, `subx` or `cmpm` faulted on.
/// * The exceptions of group 1 take [`GROUP_ONE`], the instruction never
///   having run.
/// * A `trap` takes [`TRAP`], `trapv` [`TRAPV`], a division by zero
///   [`DIVISION_BY_ZERO`] and `chk` [`CHK_ABOVE`] or [`CHK_BELOW`], both with
///   their source's effective address.
///
/// The 68010's and the 68020's take their own: [`m68010`]'s and [`m68020`]'s.
///
/// `instruction` is `None` for one that could not be fetched.
pub fn exception(
    error: &RuntimeError,
    vector: u8,
    instruction: Option<&Instruction>,
    source_faulted: bool,
    processor: Processor,
) -> u64 {
    if processor.has(Processor::M68020) {
        return exception_68020(error, vector, instruction);
    }
    if processor.has(Processor::M68010) {
        return exception_68010(error, vector, instruction, source_faulted);
    }
    let source_time = |instruction: Option<&Instruction>| match instruction {
        Some(Instruction::CHK(source, _) | Instruction::DIVx(source, ..)) => {
            effective_address(source, Size::Word) as u32
        }
        Some(Instruction::CHKL(source, _) | Instruction::DIVL { source, .. }) => {
            effective_address(source, Size::Long) as u32
        }
        _ => 0,
    };
    let cycles = match *error {
        RuntimeError::AddressError { access, .. } | RuntimeError::BusError { access, .. } => {
            let lead = instruction.map_or(0, |instruction| {
                address_error_lead(instruction, access, source_faulted)
            });
            lead + GROUP_ZERO
        }
        RuntimeError::DivisionByZero => DIVISION_BY_ZERO + source_time(instruction),
        RuntimeError::ChkOutOfBounds { value, bound } => {
            chk_out_of_bounds(value, bound) + source_time(instruction)
        }
        //the 68020's, which no 68000 runs
        RuntimeError::Chk2OutOfBounds { .. } => CHK_ABOVE,
        RuntimeError::OverflowException | RuntimeError::ConditionalTrap => TRAPV,
        _ => match exception::group(vector) {
            exception::Group::Two => TRAP,
            exception::Group::Zero | exception::Group::One => GROUP_ONE,
        },
    };
    cycles as u64
}

/// [`exception()`] on a 68010 (table 9-19): what the 68000's take, but for
/// the longer frames, and `bkpt`, `movec` of a control register the 68010
/// does not have and `rte` of a frame it cannot return from.
fn exception_68010(
    error: &RuntimeError,
    vector: u8,
    instruction: Option<&Instruction>,
    source_faulted: bool,
) -> u64 {
    let source_time = |instruction: Option<&Instruction>| match instruction {
        Some(Instruction::CHK(source, _) | Instruction::DIVx(source, ..)) => {
            effective_address(source, Size::Word) as u32
        }
        Some(Instruction::CHKL(source, _) | Instruction::DIVL { source, .. }) => {
            effective_address(source, Size::Long) as u32
        }
        _ => 0,
    };
    let cycles = match *error {
        RuntimeError::AddressError { access, .. } | RuntimeError::BusError { access, .. } => {
            let lead = instruction.map_or(0, |instruction| {
                address_error_lead_68010(instruction, access, source_faulted)
            });
            lead + m68010::GROUP_ZERO
        }
        RuntimeError::DivisionByZero => m68010::DIVISION_BY_ZERO + source_time(instruction),
        //the 68010 compares the register with the bound first, and with zero after, as
        //numbers: no overflow of the 68000's subtraction moves a case from one to the other
        RuntimeError::ChkOutOfBounds { value, bound } => {
            let time = match value > bound {
                true => m68010::CHK_ABOVE,
                false => m68010::CHK_BELOW,
            };
            time + source_time(instruction)
        }
        RuntimeError::OverflowException | RuntimeError::ConditionalTrap => m68010::TRAPV,
        //the 68020's, which no 68010 runs
        RuntimeError::Chk2OutOfBounds { .. } => m68010::CHK_ABOVE,
        RuntimeError::FormatError { format: 8 } => m68010::ILLEGAL_REVISION,
        RuntimeError::FormatError { .. } => m68010::FORMAT_ERROR,
        RuntimeError::IllegalInstruction => match instruction {
            Some(Instruction::BKPT(_)) => m68010::BREAKPOINT,
            Some(Instruction::MOVEC {
                to_control: false, ..
            }) => m68010::ILLEGAL_CONTROL_REGISTER,
            Some(Instruction::MOVEC { .. }) => m68010::ILLEGAL_CONTROL_REGISTER_WRITTEN,
            _ => m68010::GROUP_ONE,
        },
        _ => match exception::group(vector) {
            exception::Group::Two => m68010::TRAP,
            exception::Group::Zero | exception::Group::One => m68010::GROUP_ONE,
        },
    };
    cycles as u64
}

/// [`exception()`] on a 68020: the cache case of the manual's exception
/// tables, whose time replaces the instruction's ([`m68020`]).
///
/// * A bus or an address error takes the time of the frame it pushes, the
///   short one for a bus error on the fetch of an instruction none of which
///   ran, and the long one otherwise.
/// * `trapcc` and `trapv` take [`m68020::TRAP_WITH_ADDRESS`], and `chk`,
///   `chk2` and a division by zero the same with their source's effective
///   address, `chk2`'s and a long division's after the extension word.
/// * The others of groups 1 and 2 take [`m68020::GROUP_ONE`] and
///   [`m68020::TRAP`], but `bkpt`, [`m68020::BREAKPOINT`], and the format
///   error, [`m68020::FORMAT_ERROR`].
fn exception_68020(error: &RuntimeError, vector: u8, instruction: Option<&Instruction>) -> u64 {
    let source_time = || match instruction {
        Some(Instruction::CHK(source, _) | Instruction::DIVx(source, ..)) => {
            fetch(source, Size::Word) as u32
        }
        Some(Instruction::CHKL(source, _)) => fetch(source, Size::Long) as u32,
        Some(Instruction::DIVL { source, .. }) => fetch_after_a_word(source) as u32,
        Some(Instruction::CHK2(source, ..)) => fetch_immediate(Size::Word, source) as u32,
        _ => 0,
    };
    let cycles = match *error {
        RuntimeError::BusError { .. } if instruction.is_none() => m68020::SHORT_BUS_FAULT,
        RuntimeError::AddressError { .. } | RuntimeError::BusError { .. } => m68020::LONG_BUS_FAULT,
        RuntimeError::DivisionByZero
        | RuntimeError::ChkOutOfBounds { .. }
        | RuntimeError::Chk2OutOfBounds { .. } => m68020::TRAP_WITH_ADDRESS + source_time(),
        RuntimeError::OverflowException | RuntimeError::ConditionalTrap => {
            m68020::TRAP_WITH_ADDRESS
        }
        RuntimeError::FormatError { .. } => m68020::FORMAT_ERROR,
        RuntimeError::IllegalInstruction => match instruction {
            Some(Instruction::BKPT(_)) => m68020::BREAKPOINT,
            _ => m68020::GROUP_ONE,
        },
        _ => match exception::group(vector) {
            exception::Group::Two => m68020::TRAP,
            exception::Group::Zero | exception::Group::One => m68020::GROUP_ONE,
        },
    };
    cycles as u64
}

/// The time of `chk` out of bounds, `value` being the register's low word and
/// `bound` the source's, both sign extended.
///
/// The 68000 compares the register with the bound first, by subtracting the
/// bound in sixteen bits: when the register is above it, or the subtraction
/// overflows, it traps at [`CHK_ABOVE`]; otherwise the register is below
/// zero, and it traps at [`CHK_BELOW`]. That is MAME's microcode on every
/// out of bounds `chk` of the suite, and m68k-rs's measurement.
fn chk_out_of_bounds(value: i32, bound: i32) -> u32 {
    let difference = value - bound;
    let overflows = difference < i16::MIN as i32 || difference > i16::MAX as i32;
    match value > bound || overflows {
        true => CHK_ABOVE,
        false => CHK_BELOW,
    }
}

/// The time of the part of the instruction that ran before an address
/// error aborted it at an access of `access`: the cycles before the access
/// began.
///
/// It is the rule MAME's microcode follows, measured on the suite's 55,606
/// address errors, and a bus error is counted as an address error at the same
/// access, the suite having none:
///
/// * a jump to an odd address faults in the jump's own prefetch, after the
///   time it takes to work out its target and, for `bsr`, `rts`, `rtr` and
///   `rte`, to push or pop;
/// * most instructions fault at their first access to memory, after the time
///   of the extension words before it and of the address it works out:
///   nothing for `(An)` and `(An)+`, two cycles for the decrement of `-(An)`,
///   a word's read for a displacement and for an absolute short address, two
///   for an absolute long address, and a word and two cycles for an index;
/// * `movem` reads its mask first;
/// * `move` faults on its write after the whole time of its source;
/// * `addx`, `subx` and `cmpm` fault on their second operand after the read
///   of their first, `source_faulted` being false.
pub fn address_error_lead(instruction: &Instruction, access: Access, source_faulted: bool) -> u32 {
    use Instruction::*;
    match *instruction {
        JMP(ref target) | JSR(ref target) => match target {
            Operand::IndirectIndex { .. } | Operand::PcIndex { .. } | Operand::Extended { .. } => 6,
            Operand::Absolute {
                width: AddressWidth::Long,
                ..
            } => 4,
            Operand::Indirect(_) => 0,
            _ => 2,
        },
        BRA(..) | Bcc(..) | DBcc(..) => 2,
        BSR(..) if access == Access::Program => 10,
        RTS => 8,
        RTR | RTE => 12,
        //the mask first, and no decrement before the first write
        MOVEM { ref target, .. } => match target {
            Operand::PreIndirect(_) => 4,
            _ => 4 + lead_of(target),
        },
        ADDX(_, _, size) | SUBX(_, _, size) => match source_faulted {
            true => 2,
            false => 2 + effective_address(&Operand::Indirect(0), size) as u32,
        },
        CMPM(_, _, size) => match source_faulted {
            true => 0,
            false => effective_address(&Operand::Indirect(0), size) as u32,
        },
        MOVE(ref source, ref destination, size) => match access {
            Access::Write => {
                effective_address(source, size) as u32 + move_write_lead(source, destination)
            }
            Access::Read | Access::Program => lead_of(source),
        },
        _ => match immediate_and_memory_operand(instruction) {
            Some((before, operand)) => before + lead_of(&operand),
            None => 0,
        },
    }
}

/// [`address_error_lead`] on a 68010, whose microcode reaches some of its
/// accesses at another time than the 68000's: Moira's, which cputester
/// checks, measured on every address error of the differential harness.
///
/// * a jump to an odd address faults in the jump's own prefetch, as on the
///   68000, but a branch, `bsr` and `DBcc` have worked out nothing before it,
///   `rts` and `rtd` have popped for six cycles, `rtr` and `rte` for ten,
///   and `jmp` and `jsr` through an extension word for two cycles less;
/// * `chk` and a division read an operand of an extension word six cycles
///   later, `clr`, which writes without reading, four later, or six through
///   an index, and `move sr` and `move ccr` two later;
/// * `move` to an absolute long address after a source in memory, and `movem`
///   through an extension word, two earlier;
/// * `moves` faults after working out its address and the bus cycle of the
///   function codes: twelve cycles through a displacement or a short
///   address, sixteen through an index or a long address.
pub fn address_error_lead_68010(
    instruction: &Instruction,
    access: Access,
    source_faulted: bool,
) -> u32 {
    use Instruction::*;
    let lead = address_error_lead(instruction, access, source_faulted);
    let extension = |operand: &Operand| {
        !matches!(
            operand,
            Operand::Register(_)
                | Operand::Immediate(_)
                | Operand::Indirect(_)
                | Operand::PostIndirect(_)
                | Operand::PreIndirect(_)
        )
    };
    let indexed_or_long = |operand: &Operand| {
        matches!(
            operand,
            Operand::IndirectIndex { .. }
                | Operand::PcIndex { .. }
                | Operand::Extended { .. }
                | Operand::Absolute {
                    width: AddressWidth::Long,
                    ..
                }
        )
    };
    match *instruction {
        BRA(..) | Bcc(..) | BSR(..) | DBcc(..) if access == Access::Program => 0,
        RTS | RTD(_) => 6,
        RTR | RTE => 10,
        JMP(ref target) | JSR(ref target) if extension(target) => lead - 2,
        CHK(ref source, _) | DIVx(ref source, ..) if extension(source) => lead + 6,
        CLR(ref operand, _) if extension(operand) => match operand {
            Operand::IndirectIndex { .. } | Operand::Extended { .. } => lead + 6,
            _ => lead + 4,
        },
        MOVEfromSR(ref operand) | MOVEfromCCR(ref operand) if extension(operand) => lead + 2,
        MOVE(
            ref source,
            Operand::Absolute {
                width: AddressWidth::Long,
                ..
            },
            _,
        ) if access == Access::Write && !register_or_immediate(source) => lead - 2,
        MOVEM { ref target, .. } if extension(target) => lead - 2,
        MOVES { ref target, .. } if extension(target) => match indexed_or_long(target) {
            true => 16,
            false => 12,
        },
        _ => lead,
    }
}

/// The time before the first access to `operand`, in memory: the time of
/// its addressing mode without the access, which is four cycles.
fn lead_of(operand: &Operand) -> u32 {
    (effective_address(operand, Size::Word) as u32).saturating_sub(4)
}

/// The time between the read of `move`'s source and the write of its
/// destination: the address it works out, and the prefetch that comes before
/// the write of `-(An)`; an absolute long address after a source in memory
/// has had one of its words prefetched already.
fn move_write_lead(source: &Operand, destination: &Operand) -> u32 {
    match destination {
        Operand::PreIndirect(_) => 4,
        Operand::Absolute {
            width: AddressWidth::Long,
            ..
        } if !register_or_immediate(source) => 4,
        _ => lead_of(destination),
    }
}

/// The operand of `instruction` in memory that its first access reaches, and
/// the time of the immediate that comes before it: `addi` and its kind read
/// one word or two before their destination.
fn immediate_and_memory_operand(instruction: &Instruction) -> Option<(u32, Operand)> {
    use Instruction::*;
    let in_memory = |operand: &Operand| !register_or_immediate(operand);
    let (before, operand) = match *instruction {
        ADDI(_, operand, size)
        | SUBI(_, operand, size)
        | ANDI(_, operand, size)
        | ORI(_, operand, size)
        | EORI(_, operand, size)
        | CMPI(_, operand, size) => (
            effective_address(&Operand::Immediate(0), size) as u32,
            operand,
        ),
        ADD(a, b, _) | SUB(a, b, _) | AND(a, b, _) | OR(a, b, _) | EOR(a, b, _) => {
            (0, if in_memory(&a) { a } else { b })
        }
        ADDA(operand, ..)
        | SUBA(operand, ..)
        | CMPA(operand, ..)
        | MOVEA(operand, ..)
        | CMP(operand, ..)
        | ADDQ(_, operand, _)
        | SUBQ(_, operand, _)
        | CLR(operand, _)
        | NEG(operand, _)
        | NEGX(operand, _)
        | NOT(operand, _)
        | TST(operand, _)
        | ASd(_, operand, ..)
        | LSd(_, operand, ..)
        | ROd(_, operand, ..)
        | ROXd(_, operand, ..)
        | DIVx(operand, ..)
        | MULx(operand, ..)
        | CHK(operand, _)
        | MOVEtoSR(operand)
        | MOVEtoCCR(operand)
        | MOVEfromSR(operand)
        | MOVEfromCCR(operand) => (0, operand),
        _ => return None,
    };
    in_memory(&operand).then_some((before, operand))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::instructions::{Condition, RegisterOperand};

    fn d(register: u8) -> Operand {
        Operand::Register(RegisterOperand::Data(register))
    }

    #[test]
    fn a_multiply_takes_two_cycles_a_bit_set_or_a_change_of_bits() {
        //the extra over 38 of register forms of the suite's MULU and MULS files
        assert_eq!(multiply(0xBC95, Sign::Unsigned, Processor::M68000), 56 - 38);
        assert_eq!(multiply(0x6C8A, Sign::Unsigned, Processor::M68000), 52 - 38);
        assert_eq!(multiply(0x4CA4, Sign::Unsigned, Processor::M68000), 50 - 38);
        assert_eq!(multiply(0xCBE0, Sign::Signed, Processor::M68000), 48 - 38);
        assert_eq!(multiply(0x9633, Sign::Signed, Processor::M68000), 56 - 38);
        assert_eq!(multiply(0x7C7C, Sign::Signed, Processor::M68000), 46 - 38);
        //the ends: no bit and every bit, no change and a change at every bit
        assert_eq!(multiply(0, Sign::Unsigned, Processor::M68000), 0);
        assert_eq!(multiply(0xFFFF, Sign::Unsigned, Processor::M68000), 32);
        assert_eq!(multiply(0, Sign::Signed, Processor::M68000), 0);
        assert_eq!(
            multiply(0xFFFF, Sign::Signed, Processor::M68000),
            2,
            "one change, below the lowest bit"
        );
        assert_eq!(multiply(0x5555, Sign::Signed, Processor::M68000), 32);
    }

    #[test]
    fn a_division_takes_the_time_of_the_microcodes_loop() {
        //register forms of the suite's DIVU and DIVS files, whose source takes no time
        assert_eq!(
            divide(0x6BE8_1EC0, 0x451D, Sign::Unsigned, Processor::M68000),
            10,
            "an overflow"
        );
        assert_eq!(
            divide(0x52C8_434E, 0x64B2, Sign::Unsigned, Processor::M68000),
            120
        );
        assert_eq!(
            divide(0x321F_5C5D, 0x76DD, Sign::Unsigned, Processor::M68000),
            116
        );
        assert_eq!(
            divide(0xD49D_15DC, 0xDB39, Sign::Unsigned, Processor::M68000),
            114
        );
        assert_eq!(
            divide(0x7C82_59CF, 0xEE80, Sign::Signed, Processor::M68000),
            16,
            "an overflow"
        );
        assert_eq!(
            divide(0x906F_D3AB, 0x4078, Sign::Signed, Processor::M68000),
            18,
            "a negative one"
        );
        assert_eq!(
            divide(0x0647_0657, 0x869F, Sign::Signed, Processor::M68000),
            138
        );
        assert_eq!(
            divide(0x462A_588A, 0x5925, Sign::Signed, Processor::M68000),
            130
        );
        //the ends of the two ranges
        assert_eq!(
            divide(0, 1, Sign::Unsigned, Processor::M68000),
            136,
            "every step without a subtraction"
        );
        assert_eq!(
            divide(0, 1, Sign::Signed, Processor::M68000),
            150,
            "a quotient of fifteen zeros"
        );
        assert_eq!(
            divide(0xFFFF_8000, 0x7FFF, Sign::Signed, Processor::M68000),
            156,
            "the most"
        );
    }

    #[test]
    fn the_base_time_is_the_manuals_with_yachts_corrections() {
        use Instruction::*;
        let a0 = Operand::Indirect(0);
        let long = Operand::Absolute {
            address: 0x2000,
            width: AddressWidth::Long,
        };
        let time = |instruction: Instruction| base(&instruction, Processor::M68000);
        assert_eq!(time(MOVE(a0, d(1), Size::Word)), 8);
        assert_eq!(time(MOVE(d(0), Operand::PreIndirect(1), Size::Long)), 12);
        assert_eq!(time(MOVE(long, long, Size::Long)), 36);
        assert_eq!(time(ADD(d(0), d(1), Size::Long)), 8);
        assert_eq!(time(ADD(Operand::Immediate(1), d(1), Size::Long)), 16);
        assert_eq!(time(ADD(a0, d(1), Size::Long)), 14);
        assert_eq!(time(ADD(d(1), a0, Size::Long)), 20);
        let a1 = Operand::Register(RegisterOperand::Address(1));
        assert_eq!(
            time(ADDQ(1, a1, Size::Word)),
            8,
            "Yacht: not the manual's 4"
        );
        assert_eq!(
            time(ANDI(1, d(0), Size::Long)),
            16,
            "Yacht: not the manual's 14"
        );
        assert_eq!(time(CMPI(1, d(0), Size::Long)), 14);
        let movem = |direction, size, target| MOVEM {
            direction,
            size,
            registers_mask: 0x7FFF,
            target,
        };
        assert_eq!(
            time(movem(
                TargetDirection::ToMemory,
                Size::Long,
                Operand::PreIndirect(7)
            )),
            8 + 15 * 8
        );
        assert_eq!(
            time(movem(
                TargetDirection::FromMemory,
                Size::Word,
                Operand::PostIndirect(7)
            )),
            12 + 15 * 4
        );
        assert_eq!(
            time(Bcc(0x1000, Condition::Equal, BranchSize::Short)),
            8,
            "not taken"
        );
        assert_eq!(
            time(Bcc(0x1000, Condition::Equal, BranchSize::Word)),
            10,
            "taken"
        );
        assert_eq!(
            time(DBcc(RegisterOperand::Data(0), 0x1000, Condition::False)),
            10
        );
        assert_eq!(time(TRAP(15)), 0, "EASy68K's tasks take no time");
        assert_eq!(time(STOP(0x2000)), 4);
        assert_eq!(time(RESET), 132);
        assert_eq!(
            time(LSd(
                Operand::Immediate(1),
                a0,
                crate::instructions::ShiftDirection::Left,
                Size::Word
            )),
            12
        );
        let left = crate::instructions::ShiftDirection::Left;
        assert_eq!(
            time(LSd(Operand::Immediate(8), d(0), left, Size::Long)),
            8 + 16,
            "an immediate count's bits are in the base"
        );
        assert_eq!(
            time(LSd(d(1), d(0), left, Size::Long)),
            8,
            "a register's are the data's"
        );
        for processor in [Processor::M68000, Processor::M68010] {
            assert_eq!(base(&MOVEQ(1, RegisterOperand::Data(0)), processor), 4);
        }
    }

    /// The 68020's addressing modes as the manual's tables have them, in the
    /// order of their rows: `Dn`, `#<data>.B,W`, `#<data>.L`, `(An)`, `(An)+`,
    /// `-(An)`, `(d16,An)`, `(xxx).W`, `(xxx).L`, `(d8,An,Xn)`, `(B)`,
    /// `(d16,B)`, `(d32,B)`, then the nine memory indirect ones from `([B],I)`
    /// to `([d32,B],I,d32)`, the full ones with the base, the index and the
    /// displacements in every arrangement the notes say does not matter.
    fn manual_modes() -> Vec<(&'static str, Operand, Size)> {
        use crate::instructions::{ExtensionWord, IndexBase, IndexRegister};
        let index = IndexRegister {
            register: RegisterOperand::Data(1),
            size: Size::Word,
        };
        let full = |name, base: u8, bd, indirection, od| {
            let base_suppressed = base == 9;
            let base = match base {
                8 => IndexBase::PC,
                9 => IndexBase::address(0),
                number => IndexBase::address(number),
            };
            let word = ExtensionWord::full(index, 4, base_suppressed, false, bd, indirection, od);
            let operand = Operand::Extended {
                base,
                word,
                displacement: 0x10,
                outer: 0x20,
            };
            (name, operand, Size::Word)
        };
        use DisplacementSize::{Long, Null, Word};
        use Indirection::{Postindexed, Preindexed};
        vec![
            ("Dn", d(1), Size::Word),
            ("#.W", Operand::Immediate(5), Size::Word),
            ("#.L", Operand::Immediate(5), Size::Long),
            ("(An)", Operand::Indirect(0), Size::Word),
            ("(An)+", Operand::PostIndirect(0), Size::Word),
            ("-(An)", Operand::PreIndirect(0), Size::Word),
            (
                "(d16,An)",
                Operand::IndirectDisplacement {
                    offset: 4,
                    base: RegisterOperand::Address(0),
                },
                Size::Word,
            ),
            (
                "(xxx).W",
                Operand::Absolute {
                    address: 0x2000,
                    width: AddressWidth::Word,
                },
                Size::Word,
            ),
            (
                "(xxx).L",
                Operand::Absolute {
                    address: 0x2000,
                    width: AddressWidth::Long,
                },
                Size::Word,
            ),
            (
                "(d8,An,Xn)",
                Operand::IndirectIndex {
                    base: RegisterOperand::Address(0),
                    offset: 4,
                    index,
                },
                Size::Word,
            ),
            full("(B)", 0, Null, Indirection::None, Null),
            full("(d16,B)", 8, Word, Indirection::None, Null),
            full("(d32,B)", 9, Long, Indirection::None, Null),
            full("([B],I)", 2, Null, Preindexed, Null),
            full("([B],I,d16)", 9, Null, Postindexed, Word),
            full("([B],I,d32)", 3, Null, Preindexed, Long),
            full("([d16,B],I)", 4, Word, Postindexed, Null),
            full("([d16,B],I,d16)", 8, Word, Preindexed, Word),
            full("([d16,B],I,d32)", 5, Word, Postindexed, Long),
            full("([d32,B],I)", 6, Long, Preindexed, Null),
            full("([d32,B],I,d16)", 7, Long, Postindexed, Word),
            full("([d32,B],I,d32)", 8, Long, Preindexed, Long),
        ]
    }

    /// The cache case of the manual's MOVE table (8.2.6), every source by
    /// every destination, the `An` column being `movea`'s. The manual's rows
    /// and column of `(d16,An,Xn)` are left out: that is the full extension
    /// word of `(d16,B)`, whose times this takes ([`row`]).
    const MOVE_CACHE_CASE: &str = "
        2 2 4 4 5 5 4 6 7 8 10 14 12 14 15 14 16 17 18 20 21
        4 4 6 6 7 7 6 8 7 8 10 14 12 14 15 14 16 17 18 20 21
        6 6 8 8 9 9 8 10 9 10 12 16 14 16 17 16 18 19 20 22 23
        6 6 7 7 7 7 7 9 9 10 12 16 14 16 17 16 18 19 20 22 23
        6 6 7 7 7 7 7 9 9 10 12 16 14 16 17 16 18 19 20 22 23
        7 7 8 8 8 8 8 10 10 11 13 17 15 17 18 17 19 20 21 23 24
        7 7 8 8 8 8 8 10 10 11 13 17 15 17 18 17 19 20 21 23 24
        6 6 7 7 7 7 7 9 9 10 12 16 14 16 17 16 18 19 20 22 23
        6 6 7 7 7 7 7 9 9 10 12 16 14 16 17 16 18 19 20 22 23
        9 9 10 10 10 10 10 12 12 13 15 19 17 19 20 19 21 22 23 25 26
        9 9 10 10 10 10 10 12 12 13 15 19 17 19 20 19 21 22 23 25 26
        11 11 12 12 12 12 12 14 14 15 17 21 19 21 22 21 23 24 25 27 28
        15 15 16 16 16 16 16 18 18 19 21 25 23 25 26 25 27 28 29 31 32
        14 14 15 15 15 15 15 17 17 18 20 24 22 24 25 24 26 27 28 30 31
        16 16 17 17 17 17 17 19 19 20 22 26 24 26 27 26 28 29 30 32 33
        16 16 17 17 17 17 17 19 19 20 22 26 24 26 27 26 28 29 30 32 33
        16 16 17 17 17 17 17 19 19 20 22 26 24 26 27 26 28 29 30 32 33
        18 18 19 19 19 19 19 21 21 22 24 28 26 28 29 28 30 31 32 34 35
        18 18 19 19 19 19 19 21 21 22 24 28 26 28 29 28 30 31 32 34 35
        20 20 21 21 21 21 21 23 23 24 26 30 28 30 31 30 32 33 34 36 37
        22 22 23 23 23 23 23 25 25 26 28 32 30 32 33 32 34 35 36 38 39
        22 22 23 23 23 23 23 25 25 26 28 32 30 32 33 32 34 35 36 38 39";

    #[test]
    fn the_68020s_move_is_the_manuals_cache_case() {
        let modes = manual_modes();
        let table: Vec<Vec<u16>> = MOVE_CACHE_CASE
            .trim()
            .lines()
            .map(|line| {
                line.split_whitespace()
                    .map(|n| n.parse().unwrap())
                    .collect()
            })
            .collect();
        assert_eq!(table.len(), modes.len());
        //the destinations: `An`, then every mode but the immediates
        let destinations: Vec<_> = modes
            .iter()
            .filter(|(name, ..)| !name.starts_with('#'))
            .collect();
        let mut checked = 0;
        for ((name, source, size), row) in modes.iter().zip(&table) {
            assert_eq!(row.len(), destinations.len() + 1, "{name}");
            let movea = Instruction::MOVEA(*source, RegisterOperand::Address(1), *size);
            assert_eq!(base(&movea, Processor::M68020), row[0], "{name} to An");
            for ((to, destination, _), &expected) in destinations.iter().zip(&row[1..]) {
                let instruction = Instruction::MOVE(*source, *destination, *size);
                assert_eq!(
                    base(&instruction, Processor::M68020),
                    expected,
                    "{name} to {to}"
                );
                checked += 1;
            }
        }
        assert_eq!(checked, 22 * 20);
    }

    /// The manual's effective address tables through the instructions whose
    /// notes add them: `tst` the fetch, `lea` the calculation, `jmp` the jump,
    /// `cmpi` the fetch after an immediate word or long, `movem` the
    /// calculation after its mask.
    #[test]
    fn the_68020s_effective_addresses_are_the_manuals_tables() {
        use Instruction::*;
        let time = |instruction: Instruction| base(&instruction, Processor::M68020);
        let fetch = [
            0, 2, 4, 4, 4, 5, 5, 4, 4, 7, 7, 9, 13, 12, 14, 14, 14, 16, 16, 18, 20, 20,
        ];
        let calculate = [6, 8, 12, 11, 13, 13, 13, 15, 15, 17, 19, 19];
        let jump = [6, 8, 12, 11, 13, 13, 13, 15, 15, 17, 19, 19];
        let fetch_word = [
            2, 4, 6, 4, 6, 5, 5, 5, 6, 9, 9, 11, 15, 14, 16, 16, 16, 18, 18, 20, 22, 22,
        ];
        let fetch_long = [
            4, 6, 8, 4, 8, 7, 7, 7, 8, 11, 11, 13, 17, 16, 18, 18, 18, 20, 20, 22, 24, 24,
        ];
        let calculate_word = [8, 10, 14, 13, 15, 15, 15, 17, 17, 19, 21, 21];
        let modes = manual_modes();
        for (index, (name, operand, size)) in modes.iter().enumerate() {
            assert_eq!(time(TST(*operand, *size)), 2 + fetch[index], "tst {name}");
            if let Operand::Immediate(_) = operand {
                continue;
            }
            let word = time(CMPI(1, *operand, Size::Word));
            assert_eq!(word, 2 + fetch_word[index], "cmpi.w {name}");
            let long = time(CMPI(1, *operand, Size::Long));
            assert_eq!(long, 2 + fetch_long[index], "cmpi.l {name}");
        }
        for (index, (name, operand, _)) in modes[10..].iter().enumerate() {
            assert_eq!(
                time(LEA(*operand, RegisterOperand::Address(1))),
                2 + calculate[index],
                "lea {name}"
            );
            assert_eq!(time(JMP(*operand)), 4 + jump[index], "jmp {name}");
            let movem = MOVEM {
                direction: TargetDirection::FromMemory,
                size: Size::Long,
                registers_mask: 0b11,
                target: *operand,
            };
            assert_eq!(
                time(movem),
                8 + 2 * 4 + calculate_word[index],
                "movem {name}"
            );
        }
        //the 68000's modes: `lea`, `jmp` and `pea`, and `movem` both ways
        let a0 = Operand::Indirect(0);
        let d16 = modes[6].1;
        let short = modes[7].1;
        let long = modes[8].1;
        let d8 = modes[9].1;
        let a1 = RegisterOperand::Address(1);
        assert_eq!(
            [a0, d16, short, long, d8].map(|mode| time(LEA(mode, a1))),
            [4, 4, 4, 6, 6]
        );
        assert_eq!(
            [a0, d16, short, long, d8].map(|mode| time(JMP(mode))),
            [6, 8, 6, 6, 10]
        );
        assert_eq!(
            [a0, d16, short, long, d8].map(|mode| time(JSR(mode))),
            [7, 9, 7, 7, 11]
        );
        assert_eq!(time(PEA(a0)), 7);
        let push = MOVEM {
            direction: TargetDirection::ToMemory,
            size: Size::Long,
            registers_mask: 0x7FFF,
            target: Operand::PreIndirect(7),
        };
        assert_eq!(time(push), 4 + 3 * 15 + 4, "-(An) as (An)+");
    }

    /// Representative rows of the manual's other tables (8.2.7 to 8.2.17),
    /// with the effective address times their notes add.
    #[test]
    fn the_68020s_instructions_are_the_manuals_cache_case() {
        use crate::instructions::{BitField, BitFieldOperation, Condition, FieldValue};
        use Instruction::*;
        let time = |instruction: Instruction| base(&instruction, Processor::M68020);
        let a0 = Operand::Indirect(0);
        let d16 = Operand::IndirectDisplacement {
            offset: 4,
            base: RegisterOperand::Address(0),
        };
        //arithmetic: the fetch of the source, or of the destination, and of an immediate
        assert_eq!(time(ADD(a0, d(0), Size::Long)), 2 + 4);
        assert_eq!(time(ADD(d(0), a0, Size::Word)), 4 + 4);
        assert_eq!(time(ADD(Operand::Immediate(1), d(0), Size::Long)), 2 + 4);
        assert_eq!(time(ADDI(1, d(0), Size::Word)), 2 + 2);
        assert_eq!(time(ADDI(1, d(0), Size::Long)), 2 + 4);
        assert_eq!(time(ANDI(1, Operand::PostIndirect(0), Size::Long)), 4 + 8);
        assert_eq!(time(ADDQ(1, a0, Size::Long)), 4 + 4);
        assert_eq!(
            time(CMPA(d16, RegisterOperand::Address(0), Size::Word)),
            4 + 5
        );
        assert_eq!(time(EOR(d(0), d(1), Size::Long)), 2);
        //multiplies and divides, whatever their data
        let sign = Sign::Unsigned;
        assert_eq!(time(MULx(d(1), RegisterOperand::Data(0), Sign::Signed)), 27);
        assert_eq!(time(DIVx(a0, RegisterOperand::Data(0), sign)), 44 + 4);
        assert_eq!(time(DIVx(d(1), RegisterOperand::Data(0), Sign::Signed)), 56);
        let mull = |source| MULL {
            source,
            low: 0,
            high: None,
            sign,
        };
        assert_eq!(time(mull(d(1))), 43 + 2, "and the extension word");
        assert_eq!(time(mull(Operand::Immediate(3))), 43 + 6);
        let divl = |source, sign| DIVL {
            source,
            quotient: 0,
            remainder: 1,
            wide: true,
            sign,
        };
        assert_eq!(time(divl(d(2), Sign::Unsigned)), 78 + 2);
        assert_eq!(time(divl(a0, Sign::Signed)), 90 + 4);
        assert_eq!(multiply(0xFFFF, Sign::Unsigned, Processor::M68020), 0);
        assert_eq!(divide(0, 1, Sign::Unsigned, Processor::M68020), 0);
        assert_eq!(divide(0x7000_0000, 1, Sign::Signed, Processor::M68020), 0);
        //single operands
        assert_eq!(time(CLR(d(0), Size::Long)), 2);
        assert_eq!(time(CLR(a0, Size::Long)), 4 + 2, "the calculation, no read");
        assert_eq!(time(NEG(a0, Size::Long)), 4 + 4);
        assert_eq!(time(TST(d16, Size::Long)), 2 + 5);
        assert_eq!(time(TAS(a0)), 12 + 2);
        assert_eq!(time(Scc(d(0), Condition::True)), 4);
        assert_eq!(time(Scc(a0, Condition::True)), 6 + 2);
        assert_eq!(
            time(EXT(RegisterOperand::Data(0), Size::Byte, Size::Long)),
            4
        );
        assert_eq!(time(NBCD(d(0))), 6);
        assert_eq!(time(NBCD(a0)), 6 + 4, "Musashi's: the manual has no row");
        //shifts: by any count, static or dynamic for the logical ones
        let left = ShiftDirection::Left;
        let right = ShiftDirection::Right;
        assert_eq!(time(LSd(Operand::Immediate(8), d(0), left, Size::Long)), 4);
        assert_eq!(time(LSd(d(1), d(0), right, Size::Long)), 6);
        assert_eq!(time(ASd(Operand::Immediate(1), d(0), left, Size::Word)), 8);
        assert_eq!(time(ASd(d(1), d(0), right, Size::Word)), 6);
        assert_eq!(time(ROd(d(1), d(0), left, Size::Byte)), 8);
        assert_eq!(time(ROXd(d(1), d(0), right, Size::Byte)), 12);
        assert_eq!(
            time(ASd(Operand::Immediate(1), a0, left, Size::Word)),
            6 + 4
        );
        assert_eq!(
            time(ROd(Operand::Immediate(1), a0, right, Size::Word)),
            7 + 4
        );
        assert_eq!(
            time(LSd(Operand::Immediate(1), a0, right, Size::Word)),
            5 + 4
        );
        //bits, by a register or an immediate word
        assert_eq!(time(BTST(Operand::Immediate(3), d(0))), 4);
        assert_eq!(time(BCHG(d(1), a0)), 4 + 4);
        assert_eq!(time(BSET(Operand::Immediate(3), d16)), 4 + 5);
        //bit fields in a register, or in memory after the extension word
        let field = BitField {
            offset: FieldValue::Immediate(0),
            width: FieldValue::Immediate(8),
        };
        let bf = |operation, target| BF {
            operation,
            target,
            field,
            register: 1,
        };
        assert_eq!(time(bf(BitFieldOperation::Test, d(0))), 6);
        assert_eq!(time(bf(BitFieldOperation::Change, d(0))), 12);
        assert_eq!(time(bf(BitFieldOperation::ExtractUnsigned, a0)), 13 + 2);
        assert_eq!(time(bf(BitFieldOperation::FindFirstOne, d16)), 24 + 4);
        assert_eq!(time(bf(BitFieldOperation::Insert, a0)), 14 + 2);
        assert_eq!(bit_field_in_five_bytes(BitFieldOperation::Insert), 20 - 14);
        //branches: `Bcc.s` not taken, the others either way; `DBcc` that branches back
        let address = 0x1000;
        assert_eq!(time(Bcc(address, Condition::Equal, BranchSize::Short)), 4);
        assert_eq!(time(Bcc(address, Condition::Equal, BranchSize::Word)), 6);
        assert_eq!(time(Bcc(address, Condition::Equal, BranchSize::Long)), 6);
        assert_eq!(time(BRA(address, BranchSize::Short)), 6);
        assert_eq!(time(BSR(address, BranchSize::Long)), 7);
        assert_eq!(
            time(DBcc(RegisterOperand::Data(0), address, Condition::False)),
            6
        );
        let extras = Extras::of(Processor::M68020);
        assert_eq!(extras.short_branch_taken, 6 - 4);
        assert_eq!(extras.word_branch_not_taken, 0);
        assert_eq!(extras.dbcc_expired, 10 - 6);
        assert_eq!(extras.dbcc_condition_true, 0);
        assert_eq!(
            (extras.scc_true, extras.shift_per_bit, extras.high_bit),
            (0, 0, 0)
        );
        assert_eq!(
            (extras.irq, extras.trace, extras.loop_mode),
            (26, 25, false)
        );
        //control
        assert_eq!(time(RTS), 10);
        assert_eq!(time(RTR), 14);
        assert_eq!(time(RTE), 21);
        assert_eq!(rte_68020(0x0, false), 0);
        assert_eq!(rte_68020(0xA, false), 43 - 21);
        assert_eq!(rte_68020(0xB, true), 92 - 21 + 16);
        assert_eq!(time(LINK(RegisterOperand::Address(6), 0)), 5);
        assert_eq!(time(LINKL(RegisterOperand::Address(6), 0)), 6);
        assert_eq!(time(UNLK(RegisterOperand::Address(6))), 6);
        assert_eq!(time(ORItoCCR(1)), 12);
        assert_eq!(time(CHK(a0, RegisterOperand::Data(0))), 8 + 4);
        assert_eq!(time(CHK2(a0, RegisterOperand::Data(0), Size::Long)), 18 + 4);
        let cas = CAS {
            compare: 0,
            update: 1,
            target: a0,
            size: Size::Long,
        };
        assert_eq!(time(cas), 12 + 2, "one that does not write");
        assert_eq!(m68020::SWAPPED, 15 - 12);
        //special moves
        assert_eq!(time(MOVEfromSR(d(0))), 4);
        assert_eq!(time(MOVEfromSR(a0)), 5 + 2);
        assert_eq!(time(MOVEtoSR(d(0))), 8);
        assert_eq!(time(MOVEtoCCR(a0)), 4 + 4);
        let movec = |to_control| MOVEC {
            to_control,
            control: crate::instructions::ControlRegister::VBR,
            register: RegisterOperand::Data(0),
        };
        assert_eq!((time(movec(false)), time(movec(true))), (6, 12));
        let movep = |direction, size| MOVEP {
            direction,
            size,
            register: RegisterOperand::Data(0),
            target: d16,
        };
        assert_eq!(time(movep(TargetDirection::ToMemory, Size::Long)), 17);
        assert_eq!(time(movep(TargetDirection::FromMemory, Size::Word)), 12);
        let moves = MOVES {
            direction: TargetDirection::FromMemory,
            size: Size::Long,
            register: RegisterOperand::Data(0),
            target: a0,
        };
        assert_eq!(time(moves), 7 + 2);
        //decimal and the others of the table of complete times
        assert_eq!(time(ABCD(d(0), d(1))), 4);
        let pre = |register| Operand::PreIndirect(register);
        assert_eq!(time(SBCD(pre(0), pre(1))), 16);
        assert_eq!(time(ADDX(pre(0), pre(1), Size::Long)), 12);
        assert_eq!(
            time(CMPM(
                Operand::PostIndirect(0),
                Operand::PostIndirect(1),
                Size::Byte
            )),
            9
        );
        assert_eq!(time(PACK(d(0), d(1), 0)), 6);
        assert_eq!(time(UNPK(pre(0), pre(1), 0)), 13);
        //exception related, not taken
        assert_eq!(time(TRAPcc(Condition::False, TrapOperand::None)), 4);
        assert_eq!(time(TRAPcc(Condition::False, TrapOperand::Long(0))), 8);
        assert_eq!(time(TRAPV), 4);
        assert_eq!(time(STOP(0x2000)), 8);
        assert_eq!(time(RESET), 518);
        assert_eq!(time(TRAP(15)), 0, "EASy68K's tasks take no time");
        assert_eq!(time(MOVEQ(1, RegisterOperand::Data(0))), 2);
        assert_eq!(time(NOP), 2);
        assert_eq!(time(SWAP(RegisterOperand::Data(0))), 4);
    }

    /// The 68020's exceptions: the manual's exception table, and the choices
    /// [`m68020`] makes where it gives none.
    #[test]
    fn the_68020s_exceptions_are_the_manuals_cache_case() {
        use Instruction::*;
        let a0 = Operand::Indirect(0);
        let taken = |error: RuntimeError, vector, instruction: Option<&Instruction>| {
            exception(&error, vector, instruction, false, Processor::M68020)
        };
        let trap = RuntimeError::UnhandledTrap {
            vector: exception::TRAP_0,
        };
        assert_eq!(taken(trap, exception::TRAP_0, Some(&TRAP(0))), 20);
        assert_eq!(
            taken(
                RuntimeError::IllegalInstruction,
                exception::ILLEGAL_INSTRUCTION,
                Some(&ILLEGAL)
            ),
            20
        );
        assert_eq!(
            taken(
                RuntimeError::PrivilegeViolation,
                exception::PRIVILEGE_VIOLATION,
                Some(&RESET)
            ),
            20
        );
        assert_eq!(
            taken(
                RuntimeError::IllegalInstruction,
                exception::ILLEGAL_INSTRUCTION,
                Some(&BKPT(0))
            ),
            10 + 20
        );
        assert_eq!(
            taken(
                RuntimeError::ConditionalTrap,
                7,
                Some(&TRAPcc(
                    crate::instructions::Condition::True,
                    TrapOperand::Word(0)
                ))
            ),
            25
        );
        assert_eq!(taken(RuntimeError::OverflowException, 7, Some(&TRAPV)), 25);
        let chk = CHK(a0, RegisterOperand::Data(0));
        let out = RuntimeError::ChkOutOfBounds { value: 5, bound: 4 };
        assert_eq!(taken(out, exception::CHK, Some(&chk)), 25 + 4);
        let divu = DIVx(a0, RegisterOperand::Data(0), Sign::Unsigned);
        assert_eq!(
            taken(
                RuntimeError::DivisionByZero,
                exception::DIVISION_BY_ZERO,
                Some(&divu)
            ),
            25 + 4
        );
        assert_eq!(
            taken(RuntimeError::FormatError { format: 8 }, 14, Some(&RTE)),
            21 + 20
        );
        let address_error = RuntimeError::AddressError {
            address: 1,
            size: Size::Word,
            access: Access::Program,
        };
        assert_eq!(
            taken(address_error, exception::ADDRESS_ERROR, Some(&JMP(a0))),
            79
        );
        let bus_error = RuntimeError::BusError {
            address: 0x2000,
            size: Size::Word,
            access: Access::Program,
            range: crate::assembler::program::MemoryRangeKind::Invalid,
        };
        assert_eq!(taken(bus_error.clone(), 2, None), 43, "the short frame");
        assert_eq!(taken(bus_error, 2, Some(&TST(a0, Size::Word))), 79);
    }

    /// The 68010's own times, Moira's where the manual gives maxima.
    #[test]
    fn the_68010_has_its_own_table() {
        use Instruction::*;
        let time = |instruction: Instruction| base(&instruction, Processor::M68010);
        let a0 = Operand::Indirect(0);
        assert_eq!(time(MOVEfromSR(d(0))), 4, "6 on a 68000");
        assert_eq!(time(MOVEfromSR(a0)), 10, "no read before the write");
        assert_eq!(time(CLR(d(0), Size::Long)), 4);
        assert_eq!(time(CLR(a0, Size::Word)), 8, "12 on a 68000");
        assert_eq!(time(ADD(d(0), d(1), Size::Long)), 6, "8 on a 68000");
        assert_eq!(time(ADDA(d(0), RegisterOperand::Address(1), Size::Long)), 8);
        assert_eq!(time(CMP(a0, RegisterOperand::Data(0), Size::Long)), 12);
        assert_eq!(time(ANDItoSR(0)), 16, "20 on a 68000");
        assert_eq!(
            time(MOVEUSP {
                to_usp: true,
                register: RegisterOperand::Address(0)
            }),
            6
        );
        assert_eq!(time(RTE), 24);
        assert_eq!(time(RESET), 130);
        assert_eq!(
            time(Bcc(0x1000, Condition::Equal, BranchSize::Short)),
            6,
            "not taken"
        );
        assert_eq!(
            time(MULx(d(0), RegisterOperand::Data(1), Sign::Unsigned)),
            40
        );
        //the data: a flat `mulu`, a `muls` of a negative source two more, `divu` by its steps
        assert_eq!(multiply(0xFFFF, Sign::Unsigned, Processor::M68010), 0);
        assert_eq!(multiply(0x8000, Sign::Signed, Processor::M68010), 2);
        assert_eq!(divide(0, 1, Sign::Unsigned, Processor::M68010), 108);
        assert_eq!(
            divide(0x0001_0000, 1, Sign::Unsigned, Processor::M68010),
            8,
            "overflow"
        );
        assert_eq!(
            divide(-7i32 as u32, 2, Sign::Signed, Processor::M68010),
            120
        );
        assert_eq!(divide(7, 2, Sign::Signed, Processor::M68010), 118);
        let extras = Extras::of(Processor::M68010);
        assert_eq!((extras.irq, extras.trace, extras.loop_mode), (46, 38, true));
        assert!(
            !Extras::of(Processor::M68020).loop_mode,
            "the 68020 has no loop mode"
        );
    }

    /// Loop mode, Moira's: the instruction's change and the `DBcc` that ends
    /// it by its count.
    #[test]
    fn loop_mode_is_for_one_word_instructions_of_the_three_modes() {
        use Instruction::*;
        let post = |register| Operand::PostIndirect(register);
        assert_eq!(
            loop_mode(&MOVE(post(0), post(1), Size::Word)),
            Some(LoopMode {
                change: -4,
                exit: 10
            })
        );
        assert_eq!(
            loop_mode(&MOVE(post(0), Operand::PreIndirect(1), Size::Long)),
            Some(LoopMode {
                change: -2,
                exit: 8
            })
        );
        assert_eq!(
            loop_mode(&CLR(post(0), Size::Byte)),
            Some(LoopMode {
                change: -4,
                exit: 12
            })
        );
        assert_eq!(
            loop_mode(&ADD(post(0), d(0), Size::Long)),
            Some(LoopMode {
                change: 2,
                exit: 10
            })
        );
        assert_eq!(
            loop_mode(&MOVE(post(0), d(1), Size::Word)),
            None,
            "a register"
        );
        assert_eq!(
            loop_mode(&TST(
                Operand::IndirectDisplacement {
                    offset: 2,
                    base: RegisterOperand::Address(0)
                },
                Size::Word
            )),
            None,
            "two words"
        );
    }

    #[test]
    fn an_exception_replaces_the_instructions_time() {
        use Instruction::*;
        let a0 = Operand::Indirect(0);
        let chk = CHK(a0, RegisterOperand::Data(0));
        let out = |value, bound| {
            exception(
                &RuntimeError::ChkOutOfBounds { value, bound },
                exception::CHK,
                Some(&chk),
                false,
                Processor::M68000,
            )
        };
        assert_eq!(out(5, 4), 38 + 4, "above the bound");
        assert_eq!(out(-1, 4), 40 + 4, "below zero");
        assert_eq!(
            out(-30_000, 10_000),
            38 + 4,
            "below zero, the subtraction overflowing"
        );
        let divu = DIVx(a0, RegisterOperand::Data(0), Sign::Unsigned);
        assert_eq!(
            exception(
                &RuntimeError::DivisionByZero,
                exception::DIVISION_BY_ZERO,
                Some(&divu),
                false,
                Processor::M68000
            ),
            38 + 4
        );
        let trap = RuntimeError::UnhandledTrap {
            vector: exception::TRAP_0,
        };
        assert_eq!(
            exception(
                &trap,
                exception::TRAP_0,
                Some(&TRAP(0)),
                false,
                Processor::M68000
            ),
            34,
            "EASy68K's 38"
        );
        assert_eq!(
            exception(
                &RuntimeError::PrivilegeViolation,
                exception::PRIVILEGE_VIOLATION,
                Some(&RESET),
                false,
                Processor::M68000
            ),
            34
        );
        assert_eq!(
            exception(
                &RuntimeError::IllegalInstruction,
                exception::ILLEGAL_INSTRUCTION,
                Some(&ILLEGAL),
                false,
                Processor::M68000
            ),
            34
        );
        let fault = |instruction: &Instruction, access| {
            let error = RuntimeError::AddressError {
                address: 1,
                size: Size::Word,
                access,
            };
            exception(
                &error,
                exception::ADDRESS_ERROR,
                Some(instruction),
                false,
                Processor::M68000,
            )
        };
        assert_eq!(fault(&TST(a0, Size::Word), Access::Read), 58);
        assert_eq!(
            fault(&TST(Operand::PreIndirect(0), Size::Word), Access::Read),
            60
        );
        assert_eq!(
            fault(&MOVE(a0, a0, Size::Word), Access::Write),
            4 + 58,
            "after its source"
        );
        assert_eq!(
            fault(&BSR(1, BranchSize::Word), Access::Program),
            10 + 58,
            "after its push"
        );
        assert_eq!(
            fault(
                &ADDX(Operand::PreIndirect(0), Operand::PreIndirect(1), Size::Long),
                Access::Read
            ),
            10 + 58,
            "on its destination"
        );
    }
}
