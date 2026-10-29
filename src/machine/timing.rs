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
//! * The 68020 counts as a 68010 until its batch gives it the cache case (the
//!   asm-editor's `docs/design/m68k-machine-plan.md`, C13), without loop
//!   mode, which it does not have.

use super::Processor;
use crate::instructions::{
    AddressWidth, BranchSize, Instruction, Operand, RegisterOperand, Sign, Size, TargetDirection,
};
use crate::interpreter::{Access, RuntimeError};

use super::exception;

/// The extra cycles of each bit a shift or a rotate of a data register moves.
pub const SHIFT_PER_BIT: u64 = 2;

/// The extra cycles of `bchg`, `bclr` and `bset` on a bit of a data register
/// above 15, which the 68000 changes in the register's high word.
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
        //the 68020 counts as a 68010 until its batch gives it the cache case
        Processor::M68010 | Processor::M68020 => m68010(instruction),
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
            },
            Processor::M68010 | Processor::M68020 => Extras {
                short_branch_taken: 4,
                word_branch_not_taken: 0,
                dbcc_condition_true: 0,
                dbcc_expired: 6,
                scc_true: 0,
                irq: m68010::IRQ,
                trace: m68010::TRACE,
                loop_mode: matches!(processor, Processor::M68010),
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
        Operand::IndirectIndex { .. } | Operand::PcIndex { .. } => 10,
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
        Operand::IndirectIndex { .. } | Operand::PcIndex { .. } => times[2],
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
        //a shift or a rotate of a register moves [`SHIFT_PER_BIT`] more per bit, its count
        //being the data of a register; one of memory moves one bit
        ASd(_, ref operand, _, size)
        | LSd(_, ref operand, _, size)
        | ROd(_, ref operand, _, size)
        | ROXd(_, ref operand, _, size) => match operand {
            Operand::Register(_) => by(size, 6, 8),
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
            Operand::IndirectIndex { .. } => 14,
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

/// The extra cycles of `mulu` or `muls` by `source`, the word read, on
/// `processor`.
///
/// On a 68000, two for each bit set of an unsigned one, and for each change
/// between two bits of a signed one, a zero below its lowest bit (the 68000
/// User's Manual, section 8.3, with Yacht's reading of it). On a 68010 an
/// unsigned one takes none, and a signed one two when the source is negative:
/// the manual gives only the maxima, 40 and 42, and Moira's model, which
/// cputester checks, the rest.
pub fn multiply(source: u16, sign: Sign, processor: Processor) -> u64 {
    if processor.has(Processor::M68010) {
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
pub fn divide(dividend: u32, divisor: u16, sign: Sign, processor: Processor) -> u64 {
    if processor.has(Processor::M68010) {
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
/// `instruction` is `None` for one that could not be fetched.
pub fn exception(
    error: &RuntimeError,
    vector: u8,
    instruction: Option<&Instruction>,
    source_faulted: bool,
    processor: Processor,
) -> u64 {
    if processor.has(Processor::M68010) {
        return exception_68010(error, vector, instruction, source_faulted);
    }
    let source_time = |instruction: Option<&Instruction>| match instruction {
        Some(Instruction::CHK(source, _) | Instruction::DIVx(source, ..)) => {
            effective_address(source, Size::Word) as u32
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
        RuntimeError::OverflowException => TRAPV,
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
        RuntimeError::OverflowException => m68010::TRAPV,
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
            Operand::IndirectIndex { .. } | Operand::PcIndex { .. } => 6,
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
            Operand::IndirectIndex { .. } => lead + 6,
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
        for processor in Processor::ALL {
            assert_eq!(base(&MOVEQ(1, RegisterOperand::Data(0)), processor), 4);
        }
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
