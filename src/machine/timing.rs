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
//! one to count for. Only the 68000's is filled. The 68010 and the 68020 count
//! as a 68000 until their batches give them their own (the asm-editor's
//! `docs/design/m68k-machine-plan.md`, C12 and C13); the extra each
//! instruction's data costs is the 68000's too.

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
        //the 68010's and the 68020's own tables come with their batches
        Processor::M68000 | Processor::M68010 | Processor::M68020 => m68000(instruction),
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
    }
}

/// The extra cycles of `mulu` or `muls` by `source`, the word read: two for
/// each bit set of an unsigned one, and for each change between two bits of
/// a signed one, a zero below its lowest bit (the 68000 User's Manual,
/// section 8.3, with Yacht's reading of it).
pub fn multiply(source: u16, sign: Sign) -> u64 {
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
pub fn divide(dividend: u32, divisor: u16, sign: Sign) -> u64 {
    match sign {
        Sign::Unsigned => divide_unsigned(dividend, divisor),
        Sign::Signed => divide_signed(dividend as i32, divisor as i16),
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
        assert_eq!(multiply(0xBC95, Sign::Unsigned), 56 - 38);
        assert_eq!(multiply(0x6C8A, Sign::Unsigned), 52 - 38);
        assert_eq!(multiply(0x4CA4, Sign::Unsigned), 50 - 38);
        assert_eq!(multiply(0xCBE0, Sign::Signed), 48 - 38);
        assert_eq!(multiply(0x9633, Sign::Signed), 56 - 38);
        assert_eq!(multiply(0x7C7C, Sign::Signed), 46 - 38);
        //the ends: no bit and every bit, no change and a change at every bit
        assert_eq!(multiply(0, Sign::Unsigned), 0);
        assert_eq!(multiply(0xFFFF, Sign::Unsigned), 32);
        assert_eq!(multiply(0, Sign::Signed), 0);
        assert_eq!(
            multiply(0xFFFF, Sign::Signed),
            2,
            "one change, below the lowest bit"
        );
        assert_eq!(multiply(0x5555, Sign::Signed), 32);
    }

    #[test]
    fn a_division_takes_the_time_of_the_microcodes_loop() {
        //register forms of the suite's DIVU and DIVS files, whose source takes no time
        assert_eq!(
            divide(0x6BE8_1EC0, 0x451D, Sign::Unsigned),
            10,
            "an overflow"
        );
        assert_eq!(divide(0x52C8_434E, 0x64B2, Sign::Unsigned), 120);
        assert_eq!(divide(0x321F_5C5D, 0x76DD, Sign::Unsigned), 116);
        assert_eq!(divide(0xD49D_15DC, 0xDB39, Sign::Unsigned), 114);
        assert_eq!(divide(0x7C82_59CF, 0xEE80, Sign::Signed), 16, "an overflow");
        assert_eq!(
            divide(0x906F_D3AB, 0x4078, Sign::Signed),
            18,
            "a negative one"
        );
        assert_eq!(divide(0x0647_0657, 0x869F, Sign::Signed), 138);
        assert_eq!(divide(0x462A_588A, 0x5925, Sign::Signed), 130);
        //the ends of the two ranges
        assert_eq!(
            divide(0, 1, Sign::Unsigned),
            136,
            "every step without a subtraction"
        );
        assert_eq!(
            divide(0, 1, Sign::Signed),
            150,
            "a quotient of fifteen zeros"
        );
        assert_eq!(divide(0xFFFF_8000, 0x7FFF, Sign::Signed), 156, "the most");
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
            assert_eq!(
                base(&MOVEQ(1, RegisterOperand::Data(0)), processor),
                4,
                "the 68000's until the dialects have their own"
            );
        }
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
                false
            ),
            38 + 4
        );
        let trap = RuntimeError::UnhandledTrap {
            vector: exception::TRAP_0,
        };
        assert_eq!(
            exception(&trap, exception::TRAP_0, Some(&TRAP(0)), false),
            34,
            "EASy68K's 38"
        );
        assert_eq!(
            exception(
                &RuntimeError::PrivilegeViolation,
                exception::PRIVILEGE_VIOLATION,
                Some(&RESET),
                false
            ),
            34
        );
        assert_eq!(
            exception(
                &RuntimeError::IllegalInstruction,
                exception::ILLEGAL_INSTRUCTION,
                Some(&ILLEGAL),
                false
            ),
            34
        );
        let fault = |instruction: &Instruction, access| {
            let error = RuntimeError::AddressError {
                address: 1,
                size: Size::Word,
                access,
            };
            exception(&error, exception::ADDRESS_ERROR, Some(instruction), false)
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
