//! Exceptions: the vectors, the frames and the program counter a 68000
//! stacks, as pure functions of what was raised (ADR 0008).
//!
//! The Interpreter raises an Exception for every error the 68000 has one
//! for: it reads the vector, pushes the frame this module describes on the
//! supervisor stack, and jumps to the handler. The 68010 reads the vector
//! through its vector base register and pushes frames of its own, which
//! begin with a format word (ADR 0011); the 68020 has the 68010's until its
//! batch gives it its own.
//!
//! **What the frames hold.**
//!
//! * Groups 1 and 2, every exception but the two of group 0: the status
//!   register, then the program counter, six bytes. The 68010 adds the
//!   format/vector-offset word, format `$0` and four times the vector, eight
//!   bytes ([`Frame::Format0`]).
//! * Group 0, the bus error and the address error: the special status word,
//!   the access address, the instruction register, the status register and
//!   the program counter, fourteen bytes. The 68010 pushes its format `$8`
//!   frame of 29 words instead ([`Frame::Format8`]).
//!
//! **The program counter stacked.** An illegal instruction, a privilege
//! violation and the line A and line F words stack the address of the
//! instruction, so that a handler can read it and decide; `trap`, `trapv`,
//! `chk` and a division by zero stack the next instruction's, since the
//! instruction has run. An address error stacks what the 68000's prefetch had
//! reached when the access faulted, which depends on the instruction and its
//! operands: [`address_error_pc`] holds the rule, read from the
//! SingleStepTests/m68000 suite (ADR 0010), which records it for every one of
//! its 55,606 address errors. A bus error, which the Memory map raises and the
//! suite has none of, aborts the instruction at the same access as an address
//! error would, and stacks what the same rule says.

use crate::instructions::{AddressWidth, Instruction, Operand, Size};
use crate::interpreter::{Access, RuntimeError};
use crate::machine::interrupts;

/// The vector of the bus error, which the Memory map raises.
pub const BUS_ERROR: u8 = 2;
/// The vector of the address error.
pub const ADDRESS_ERROR: u8 = 3;
/// The vector of an illegal instruction: `illegal`, and a word that is no
/// instruction.
pub const ILLEGAL_INSTRUCTION: u8 = 4;
/// The vector of a division by zero.
pub const DIVISION_BY_ZERO: u8 = 5;
/// The vector of `chk` out of bounds.
pub const CHK: u8 = 6;
/// The vector of `trapv` with V set.
pub const TRAPV: u8 = 7;
/// The vector of a privilege violation.
pub const PRIVILEGE_VIOLATION: u8 = 8;
/// The vector of the trace exception.
pub const TRACE: u8 = 9;
/// The vector of a line A word, `$Axxx`.
pub const LINE_A: u8 = 10;
/// The vector of a line F word, `$Fxxx`.
pub const LINE_F: u8 = 11;
/// The vector of the format error, the 68010's: `rte` found a frame whose
/// format it does not know.
pub const FORMAT_ERROR: u8 = 14;
/// The vector of `trap #0`; `trap #n` is this plus `n`.
pub const TRAP_0: u8 = 32;
/// The vector of `trap #15`, which a `trap #15` task EASy68K does not know
/// goes through: EASy68K's task trap is a `trap #15` that is not taken.
pub const TRAP_15: u8 = 47;

/// The autovector of an IRQ at level 1, at `$064`.
pub const FIRST_AUTOVECTOR: u8 = interrupts::autovector(1);
/// The autovector of an IRQ at level 7, at `$07C`.
pub const LAST_AUTOVECTOR: u8 = interrupts::autovector(7);

/// The address of a vector in the table, from `$000` to `$3FC`.
pub const fn vector_address(vector: u8) -> u32 {
    vector as u32 * 4
}

/// How an Exception names itself in the call stack and the History: `Trace`,
/// `TRAP #3`, `Divide by zero`, `IRQ 2`.
pub fn vector_name(vector: u8) -> String {
    match vector {
        BUS_ERROR => "Bus error".to_string(),
        ADDRESS_ERROR => "Address error".to_string(),
        ILLEGAL_INSTRUCTION => "Illegal instruction".to_string(),
        DIVISION_BY_ZERO => "Divide by zero".to_string(),
        CHK => "CHK".to_string(),
        TRAPV => "TRAPV".to_string(),
        PRIVILEGE_VIOLATION => "Privilege violation".to_string(),
        TRACE => "Trace".to_string(),
        LINE_A => "Line A".to_string(),
        LINE_F => "Line F".to_string(),
        FORMAT_ERROR => "Format error".to_string(),
        TRAP_0..=TRAP_15 => format!("TRAP #{}", vector - TRAP_0),
        FIRST_AUTOVECTOR..=LAST_AUTOVECTOR => {
            format!("IRQ {}", vector - interrupts::AUTOVECTOR_BASE)
        }
        other => format!("Vector {other}"),
    }
}

/// Whether `task` is one of EASy68K's `trap #15` tasks, which s68k carries
/// out or refuses with its reason: every other number goes through vector 47,
/// as EASy68K's `TRAP()` answers `TRAP_TRAP` for one (`CODE9.CPP`, and the
/// asm-editor's `docs/research/easy68k-machine-behaviour.md` §3). EASy68K reads
/// the task as the signed low byte of D0, so 128 and above are none of them.
pub fn is_easy68k_task(task: u8) -> bool {
    matches!(task, 0..=25 | 30..=33 | 40..=43 | 50..=62 | 70..=77 | 80..=96 | 100..=107)
}

/// The vector `error` goes through, or `None` when the 68000 has no exception
/// for it and the program ends.
pub fn vector_of(error: &RuntimeError) -> Option<u8> {
    match error {
        RuntimeError::BusError { .. } => Some(BUS_ERROR),
        RuntimeError::AddressError { .. } => Some(ADDRESS_ERROR),
        RuntimeError::IllegalInstruction => Some(ILLEGAL_INSTRUCTION),
        RuntimeError::DivisionByZero => Some(DIVISION_BY_ZERO),
        RuntimeError::ChkOutOfBounds { .. } => Some(CHK),
        RuntimeError::OverflowException => Some(TRAPV),
        RuntimeError::PrivilegeViolation => Some(PRIVILEGE_VIOLATION),
        RuntimeError::Trace => Some(TRACE),
        RuntimeError::LineA(_) => Some(LINE_A),
        RuntimeError::LineF(_) => Some(LINE_F),
        RuntimeError::FormatError { .. } => Some(FORMAT_ERROR),
        RuntimeError::UnhandledTrap { vector } => Some(*vector),
        RuntimeError::Irq { level } => Some(interrupts::autovector(*level)),
        RuntimeError::UnsupportedTrapTask { task } if !is_easy68k_task(*task) => Some(TRAP_15),
        _ => None,
    }
}

/// Which of the 68000's groups an exception is in, which decides what its
/// frame holds and whether a trace exception follows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Group {
    /// The address and bus errors, which abort the instruction and push the
    /// long frame.
    Zero,
    /// An illegal instruction, a privilege violation, trace, an IRQ, the
    /// line A and F words and the 68010's format error: the instruction did
    /// not run, and is not traced. An IRQ is taken between two instructions,
    /// as a step of its own.
    One,
    /// `trap`, `trapv`, `chk` and a division by zero, which an instruction
    /// raises as it runs: it has run, and a trace exception follows it when T
    /// was set.
    Two,
}

/// The group of the exception through `vector`.
pub fn group(vector: u8) -> Group {
    match vector {
        BUS_ERROR | ADDRESS_ERROR => Group::Zero,
        DIVISION_BY_ZERO | CHK | TRAPV | TRAP_0..=TRAP_15 => Group::Two,
        _ => Group::One,
    }
}

/// The frame an Exception pushes on the supervisor stack, the status register
/// aside, which is always the one from before the exception.
///
/// The 68000's are [`Short`](Frame::Short) and [`Group0`](Frame::Group0);
/// the 68010's begin with a format/vector-offset word, the format in its
/// four high bits and four times the vector below, which `rte` reads to know
/// what to pop: [`Format0`](Frame::Format0) and [`Format8`](Frame::Format8).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Frame {
    /// The frame of groups 1 and 2: the status register at the stack
    /// pointer, then the program counter.
    Short {
        /// The program counter stacked.
        pc: u32,
    },
    /// The frame of group 0: the special status word at the stack pointer,
    /// the access address, the instruction register, the status register and
    /// the program counter.
    Group0 {
        /// The program counter stacked.
        pc: u32,
        /// The address the access that faulted was made to, all 32 bits.
        access_address: u32,
        /// The first word of the instruction that faulted.
        instruction_register: u16,
        /// [`status_word`]'s.
        status_word: u16,
    },
    /// The 68010's frame of every exception but the two of group 0, format
    /// `$0`: the status register, the program counter and the
    /// format/vector-offset word, eight bytes.
    Format0 {
        /// The program counter stacked.
        pc: u32,
        /// The vector, whose offset, four times it, the format word holds.
        vector: u8,
    },
    /// The 68010's frame of the bus error and the address error, format
    /// `$8`, 29 words: the status register, the program counter, the
    /// format/vector-offset word, the special status word, the fault address,
    /// the data output, data input and instruction input buffers, each after
    /// a word that is not written, and sixteen words of internal information,
    /// the first of which holds a version number.
    ///
    /// The internal words and the three buffers are written as zeros, the
    /// version number 0 among them: s68k does not continue an instruction
    /// from them, and `rte` restarts the instruction the frame stacks the
    /// address of (the asm-editor's `docs/design/m68k-machine.md`, decision
    /// 3, as Musashi does).
    Format8 {
        /// The program counter stacked: the address of the instruction that
        /// faulted.
        pc: u32,
        /// The vector, 2 or 3.
        vector: u8,
        /// The address the access that faulted was made to, all 32 bits.
        fault_address: u32,
        /// [`special_status_word`]'s.
        special_status_word: u16,
    },
}

/// The words of a [`Frame::Format8`] that are not written: the stack pointer
/// goes down by 29 words, and only 26 are written, the three words before
/// each buffer being "unused, reserved" (the 68000 User's Manual, figure 6-8).
const FORMAT_8_UNWRITTEN: [usize; 3] = [7, 9, 11];

/// The format/vector-offset word of a 68010 frame of `format` through
/// `vector`.
pub const fn format_word(format: u16, vector: u8) -> u16 {
    (format << 12) | (vector as u16 * 4)
}

impl Frame {
    /// The frame's words, from the stack pointer up, with `sr` the status
    /// register it stacks.
    pub fn words(&self, sr: u16) -> Vec<u16> {
        let high = |long: u32| (long >> 16) as u16;
        let low = |long: u32| long as u16;
        match *self {
            Frame::Short { pc } => vec![sr, high(pc), low(pc)],
            Frame::Group0 {
                pc,
                access_address,
                instruction_register,
                status_word,
            } => vec![
                status_word,
                high(access_address),
                low(access_address),
                instruction_register,
                sr,
                high(pc),
                low(pc),
            ],
            Frame::Format0 { pc, vector } => vec![sr, high(pc), low(pc), format_word(0, vector)],
            Frame::Format8 {
                pc,
                vector,
                fault_address,
                special_status_word,
            } => {
                let mut words = vec![
                    sr,
                    high(pc),
                    low(pc),
                    format_word(8, vector),
                    special_status_word,
                    high(fault_address),
                    low(fault_address),
                ];
                //the three buffers, each after a reserved word, and the internal information
                words.resize(29, 0);
                words
            }
        }
    }

    /// Whether the word at `index` of [`words`](Frame::words) is written: all
    /// but the three reserved words of a [`Frame::Format8`], whose memory is
    /// left as it was.
    pub fn is_written(&self, index: usize) -> bool {
        !matches!(self, Frame::Format8 { .. }) || !FORMAT_8_UNWRITTEN.contains(&index)
    }

    /// How many bytes the frame takes on the stack: 6 or 14 on a 68000, 8 or
    /// 58 on a 68010.
    pub fn size(&self) -> u32 {
        match self {
            Frame::Short { .. } => 6,
            Frame::Group0 { .. } => 14,
            Frame::Format0 { .. } => 8,
            Frame::Format8 { .. } => 58,
        }
    }

    /// The program counter the frame stacks, where `rte` returns to.
    pub fn pc(&self) -> u32 {
        match *self {
            Frame::Short { pc }
            | Frame::Group0 { pc, .. }
            | Frame::Format0 { pc, .. }
            | Frame::Format8 { pc, .. } => pc,
        }
    }
}

/// The special status word of a 68010 [`Frame::Format8`] (the 68000 User's
/// Manual, figure 6-9): RR, bit 15, clear, as the processor would rerun the
/// access; IF, bit 13, for an instruction fetch, and DF, bit 12, for a data
/// read; BY, bit 9, for a byte; RW, bit 8, set for a read; and in bits 2 to 0
/// the function code of the access, as [`status_word`]'s. RM and HB are
/// clear: s68k does not say which accesses are a read-modify-write.
pub fn special_status_word(access: Access, size: Size, supervisor: bool) -> u16 {
    let fetch = match access {
        Access::Program => 0x2000,
        Access::Read => 0x1000,
        Access::Write => 0,
    };
    let byte = match size {
        Size::Byte => 0x0200,
        Size::Word | Size::Long => 0,
    };
    let read = match access {
        Access::Write => 0,
        Access::Read | Access::Program => 0x0100,
    };
    fetch | byte | read | (status_word(0, access, supervisor) & 7)
}

/// The special status word of a group 0 frame: bits 15 to 5 are the
/// instruction register's, as MAME's microcode leaves them on every address
/// error of the suite; bit 4 is set for a read; bit 3, I/N, is clear, as the
/// processor was running an instruction; bits 2 to 0 are the function code of
/// the access, from the mode the processor was in: 1 user data, 2 user
/// program, 5 supervisor data, 6 supervisor program.
pub fn status_word(instruction_register: u16, access: Access, supervisor: bool) -> u16 {
    let read = match access {
        Access::Write => 0,
        Access::Read | Access::Program => 0x10,
    };
    let space = match access {
        Access::Program => 2,
        Access::Read | Access::Write => 1,
    };
    let mode = if supervisor { 4 } else { 0 };
    (instruction_register & 0xFFE0) | read | mode | space
}

/// The bytes of extension words an operand takes, at `size` for an immediate.
fn extension_bytes(operand: &Operand, size: Size) -> u32 {
    match operand {
        Operand::Immediate(_) => match size {
            Size::Long => 4,
            Size::Byte | Size::Word => 2,
        },
        Operand::Absolute {
            width: AddressWidth::Long,
            ..
        } => 4,
        Operand::Absolute {
            width: AddressWidth::Word,
            ..
        }
        | Operand::IndirectDisplacement { .. }
        | Operand::IndirectIndex { .. }
        | Operand::PcDisplacement { .. }
        | Operand::PcIndex { .. } => 2,
        Operand::Register(_)
        | Operand::Indirect(_)
        | Operand::PostIndirect(_)
        | Operand::PreIndirect(_) => 0,
    }
}

/// Whether an operand is in memory.
fn in_memory(operand: &Operand) -> bool {
    !matches!(operand, Operand::Register(_) | Operand::Immediate(_))
}

/// The program counter stacked when the first access to `operand` faults, the
/// operand's extension words starting at `position`: the prefetch has gone
/// past the extension words of an absolute address, and past one more word
/// for a predecrement of a byte or a word, which the 68000 prefetches while it
/// decrements; a displacement or an index is still at its own word.
fn first_access_pc(operand: &Operand, size: Size, position: u32) -> u32 {
    match operand {
        Operand::PreIndirect(_) if size != Size::Long => position + 2,
        Operand::Absolute {
            width: AddressWidth::Word,
            ..
        } => position + 2,
        Operand::Absolute {
            width: AddressWidth::Long,
            ..
        } => position + 4,
        _ => position,
    }
}

/// The program counter a 68000 stacks for an address error that `instruction`
/// at `at` raised, `next` being the address after it, the access being
/// `access` to `address`.
///
/// It is the rule MAME's microcode follows, measured on the suite's address
/// errors:
///
/// * a jump to an odd address faults in the jump's own prefetch: `jmp`,
///   `bra` and `Bcc` stack the address after their operation word, `bsr` the
///   odd target (it has pushed its return address already), and `jsr`,
///   `DBcc`, `rts`, `rtr` and `rte` the next instruction's;
/// * `movem` has prefetched one word past itself, and `addx`, `subx`, `cmpm`
///   and `unlk` one past their operation word;
/// * a `move` that faults on its write has prefetched one word past its
///   destination's extension words, and, for an absolute long destination,
///   only to its last word when the source is in memory;
/// * every other access that faults is the instruction's first in memory:
///   the rule of the memory operand's mode, past the immediate of an
///   `addi`-like instruction.
pub fn address_error_pc(
    instruction: &Instruction,
    at: u32,
    next: u32,
    access: Access,
    address: u32,
) -> u32 {
    let start = at.wrapping_add(2);
    match instruction {
        Instruction::JMP(_) | Instruction::BRA(..) | Instruction::Bcc(..) => start,
        Instruction::BSR(..) if access == Access::Program => address,
        Instruction::JSR(_)
        | Instruction::DBcc(..)
        | Instruction::RTS
        | Instruction::RTR
        | Instruction::RTE => next,
        Instruction::MOVEM { .. } => next.wrapping_add(2),
        Instruction::ADDX(..)
        | Instruction::SUBX(..)
        | Instruction::CMPM(..)
        | Instruction::ABCD(..)
        | Instruction::SBCD(..)
        | Instruction::UNLK(_) => at.wrapping_add(4),
        Instruction::MOVE(source, destination, size) => {
            if access == Access::Write {
                let past_source = start.wrapping_add(extension_bytes(source, *size));
                match destination {
                    Operand::Absolute {
                        width: AddressWidth::Long,
                        ..
                    } if !in_memory(source) => past_source.wrapping_add(4),
                    _ => past_source.wrapping_add(2),
                }
            } else {
                first_access_pc(source, *size, start)
            }
        }
        _ => match memory_operand(instruction) {
            Some((operand, size, position)) => {
                first_access_pc(&operand, size, start.wrapping_add(position))
            }
            None => next,
        },
    }
}

/// Where the word the instruction register of a group 0 frame holds is, for
/// an address error `instruction` at `at` raised, the frame stacking `pc`:
/// the instruction's first word, but for a `move.w` written through `-(An)`,
/// which has prefetched the word before `pc` into it when it faults, as MAME's
/// microcode leaves it on every one of the suite's 120.
pub fn instruction_register_address(
    instruction: &Instruction,
    at: u32,
    pc: u32,
    access: Access,
) -> u32 {
    match instruction {
        Instruction::MOVE(_, Operand::PreIndirect(_), Size::Word) if access == Access::Write => {
            pc.wrapping_sub(2)
        }
        _ => at,
    }
}

/// The operand of `instruction` that is in memory, the size it is read at,
/// and how many bytes of extension words come before its own: the immediate of
/// `addi`, `subi`, `andi`, `ori`, `eori` and `cmpi`.
fn memory_operand(instruction: &Instruction) -> Option<(Operand, Size, u32)> {
    use Instruction::*;
    let (operand, size, before) = match *instruction {
        ADDI(_, operand, size)
        | SUBI(_, operand, size)
        | ANDI(_, operand, size)
        | ORI(_, operand, size)
        | EORI(_, operand, size)
        | CMPI(_, operand, size) => (operand, size, extension_bytes(&Operand::Immediate(0), size)),
        ADD(a, b, size) | SUB(a, b, size) | AND(a, b, size) | OR(a, b, size) | EOR(a, b, size) => {
            (if in_memory(&a) { a } else { b }, size, 0)
        }
        ADDA(operand, _, size)
        | SUBA(operand, _, size)
        | CMPA(operand, _, size)
        | MOVEA(operand, _, size)
        | CMP(operand, _, size)
        | ADDQ(_, operand, size)
        | SUBQ(_, operand, size)
        | CLR(operand, size)
        | NEG(operand, size)
        | NEGX(operand, size)
        | NOT(operand, size)
        | TST(operand, size)
        | ASd(_, operand, _, size)
        | LSd(_, operand, _, size)
        | ROd(_, operand, _, size)
        | ROXd(_, operand, _, size) => (operand, size, 0),
        DIVx(operand, ..)
        | MULx(operand, ..)
        | CHK(operand, _)
        | MOVEtoSR(operand)
        | MOVEtoCCR(operand)
        | MOVEfromSR(operand)
        | MOVEfromCCR(operand) => (operand, Size::Word, 0),
        _ => return None,
    };
    Some((operand, size, before))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::instructions::RegisterOperand;

    #[test]
    fn every_trap_fifteen_task_easy68k_knows_is_one() {
        for task in [
            0, 9, 25, 30, 33, 40, 43, 50, 59, 60, 62, 70, 77, 80, 96, 100, 107,
        ] {
            assert!(is_easy68k_task(task), "task {task}");
        }
        for task in [
            26, 29, 34, 39, 44, 49, 63, 69, 78, 79, 97, 99, 108, 127, 128, 255,
        ] {
            assert!(!is_easy68k_task(task), "task {task}");
        }
    }

    #[test]
    fn a_group_zero_frame_is_fourteen_bytes_in_the_68000s_order() {
        let frame = Frame::Group0 {
            pc: 0x0000_1004,
            access_address: 0x8000_2001,
            instruction_register: 0x3010,
            status_word: status_word(0x3010, Access::Read, true),
        };
        assert_eq!(frame.size(), 14);
        assert_eq!(
            frame.words(0x2704),
            [0x3015, 0x8000, 0x2001, 0x3010, 0x2704, 0x0000, 0x1004]
        );
        assert_eq!(
            Frame::Short { pc: 0x1234_5678 }.words(0x2000),
            [0x2000, 0x1234, 0x5678]
        );
    }

    #[test]
    fn the_status_word_has_the_function_code_of_the_mode() {
        assert_eq!(status_word(0, Access::Read, false) & 0x1F, 0x11);
        assert_eq!(status_word(0, Access::Write, true) & 0x1F, 0x05);
        assert_eq!(status_word(0, Access::Program, false) & 0x1F, 0x12);
        assert_eq!(status_word(0, Access::Program, true) & 0x1F, 0x16);
    }

    #[test]
    fn a_data_fault_stacks_where_the_prefetch_had_got_to() {
        let d0 = Operand::Register(RegisterOperand::Data(0));
        let word = |instruction, length: u32| {
            address_error_pc(&instruction, 0x1000, 0x1000 + length, Access::Read, 0x2001)
        };
        assert_eq!(
            word(Instruction::TST(Operand::Indirect(0), Size::Word), 2),
            0x1002
        );
        assert_eq!(
            word(Instruction::TST(Operand::PreIndirect(0), Size::Word), 2),
            0x1004
        );
        assert_eq!(
            word(Instruction::TST(Operand::PreIndirect(0), Size::Long), 2),
            0x1002
        );
        let displaced = Operand::IndirectDisplacement {
            offset: 1,
            base: RegisterOperand::Address(0),
        };
        assert_eq!(word(Instruction::ADD(displaced, d0, Size::Word), 4), 0x1002);
        let long = Operand::Absolute {
            address: 0x2001,
            width: AddressWidth::Long,
        };
        assert_eq!(word(Instruction::ADDI(5, long, Size::Long), 10), 0x100A);
        let jump = Instruction::JMP(long);
        assert_eq!(word(jump, 6), 0x1002);
    }
}
