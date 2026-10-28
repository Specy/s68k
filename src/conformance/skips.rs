//! What the conformance harness skips and relaxes, in one place, with a
//! reason for every entry (ADR 0010; the asm-editor's
//! `docs/design/m68k-machine-plan.md`, planning decision 6).
//!
//! Batch 1 of the machine has no exceptions, one A7 and no cycle counter, so
//! a test that needs any of them is skipped, and each such entry names the
//! milestone of the plan whose work lets it go with an [`Until`]: the
//! milestone deletes every line of [`SKIPS`] and [`MUSASHI`] that names it,
//! and C9 turns [`CHECK_LENGTH`] on. An entry marked
//! [`Until::Never`] is a choice of s68k's and stays.
//!
//! A **skip** leaves a test out. A **relaxation** runs it and compares less,
//! for the suite's own documented errata; every relaxation is one line of
//! [`RELAXATIONS`] with its source.

use super::suite::{Kind, Test};

/// The milestone whose work makes a skip unnecessary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Until {
    /// Exceptions, privilege, the two stack pointers and trace.
    C6,
    /// Interrupts, `stop` and `reset`.
    C8,
    /// Cycles.
    C9,
    /// Never: s68k means to differ here.
    Never,
}

/// Which tests a [`Skip`] leaves out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum When {
    /// Every test of the file of this name, without `.json.bin`.
    File(&'static str),
    /// `trap #15`, operation word `$4E4F`.
    TrapFifteen,
    /// A test whose bus cycles include an address error (`re` or `we`).
    AddressError,
    /// A test whose final state is exception processing through this vector:
    /// a frame written, the vector read, and the PC on what it held.
    Vector(u8),
    /// Exception processing through any other vector.
    AnyVector,
    /// A test whose S bit changes, which swaps A7 between the two stack
    /// pointers.
    ModeChange,
    /// A test that changes the stack pointer of the mode it is not in.
    OtherStackPointer,
}

/// One kind of test left out, and why.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Skip {
    /// The short name the report counts it under.
    pub name: &'static str,
    pub when: When,
    pub until: Until,
    pub reason: &'static str,
}

/// The tests left out, the first that matches a test counting it. Files come
/// first, then what a test's data shows.
#[rustfmt::skip]
pub const SKIPS: &[Skip] = &[
    Skip { name: "trap #15", when: When::TrapFifteen, until: Until::Never, reason: "trap #15 is EASy68K's service trap in either mode, not vector 47 (design decision 12)" },
    Skip { name: "TRAP", when: When::File("TRAP"), until: Until::C6, reason: "trap #0-14 go through vectors 32 to 46, which need exceptions" },
    Skip { name: "line A and F", when: When::File("ILLEGAL_LINEA"), until: Until::C6, reason: "a line A word takes vector 10" },
    Skip { name: "line A and F", when: When::File("ILLEGAL_LINEF"), until: Until::C6, reason: "a line F word takes vector 11" },
    Skip { name: "RTE", when: When::File("RTE"), until: Until::C6, reason: "rte pops an exception frame and can swap the stack pointers" },
    Skip { name: "MOVE USP", when: When::File("MOVEtoUSP"), until: Until::C6, reason: "move An,usp needs the second stack pointer" },
    Skip { name: "MOVE USP", when: When::File("MOVEfromUSP"), until: Until::C6, reason: "move usp,An needs the second stack pointer" },
    Skip { name: "STOP", when: When::File("STOP"), until: Until::C8, reason: "stop loads SR and waits for an interrupt" },
    Skip { name: "RESET", when: When::File("RESET"), until: Until::C8, reason: "reset is privileged and takes 132 cycles, and s68k does not run it yet" },
    Skip { name: "address error", when: When::AddressError, until: Until::C6, reason: "an address error takes vector 3 with a group 0 frame, and s68k ends the program; 7,412 are a jump, branch or return to an odd address, which the 68000 faults in the jump's own prefetch and s68k in the next step's fetch" },
    Skip { name: "privilege violation", when: When::Vector(8), until: Until::C6, reason: "a privileged instruction in user mode takes vector 8" },
    Skip { name: "CHK", when: When::Vector(6), until: Until::C6, reason: "chk out of bounds takes vector 6" },
    Skip { name: "TRAPV", when: When::Vector(7), until: Until::C6, reason: "trapv with V set takes vector 7" },
    Skip { name: "other exception", when: When::AnyVector, until: Until::C6, reason: "exception processing, which s68k ends the program on" },
    Skip { name: "mode change", when: When::ModeChange, until: Until::C6, reason: "a change of S swaps A7 between USP and SSP, and s68k has one A7" },
    Skip { name: "other stack pointer", when: When::OtherStackPointer, until: Until::C6, reason: "the stack pointer of the other mode changes, and s68k has one A7" },
];

/// What a [`Relaxation`] compares less.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Relax {
    /// The SR bits of `mask` are not compared for these files.
    Flags { files: &'static [&'static str], mask: u16 },
    /// The length in cycles is not compared for these files, when
    /// [`CHECK_LENGTH`] is on.
    Length { files: &'static [&'static str] },
    /// Memory the test does not list reads as zero: `Memory::zeroed`.
    ZeroMemory,
    /// No trace exception is taken: `Interpreter::from_state`.
    NoTrace,
}

/// One of the suite's documented errata, and what the harness does about it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Relaxation {
    pub relax: Relax,
    pub source: &'static str,
}

/// N and V, the SR's bits 3 and 1.
const N_AND_V: u16 = 0x000A;

/// The relaxations, one erratum each.
#[rustfmt::skip]
pub const RELAXATIONS: &[Relaxation] = &[
    Relaxation { relax: Relax::NoTrace, source: "about half the initial states set T, and the final state is the one before the trace exception (suite issue #2); snow masks trace the same way" },
    Relaxation { relax: Relax::ZeroMemory, source: "MOVE.b and MOVEP list bytes they never wrote, as zeros, in the final RAM (suite issue #1); memory starts as zeros, as in the generator" },
    Relaxation { relax: Relax::Flags { files: &["ABCD", "SBCD", "NBCD"], mask: N_AND_V }, source: "N and V are undefined for the BCD instructions (Programmer's Reference Manual), and s68k leaves them as they were; m68k-rs masks them too" },
    Relaxation { relax: Relax::Length { files: &["TAS"] }, source: "the suite's README: TAS does not have the 5-cycle read-modify-write timing" },
    Relaxation { relax: Relax::Length { files: &["TRAPV"] }, source: "the suite's README: its TRAPV tests are not verified; measured on all 2,500, vector 7 is taken exactly when V is set, in either mode, so only the length is distrusted" },
];

/// Files with failures the conformance work has found and not fixed yet: a
/// failing test of one is counted as known, and does not fail the run. Each
/// fix deletes its file's line, after which every test of the file has to
/// pass; a failure that needs a later milestone goes in [`SKIPS`] instead.
#[rustfmt::skip]
pub const KNOWN_FAILURES: &[(&str, &str)] = &[
];

/// Whether a test's length in cycles is compared. s68k counts no cycles
/// before C9, which turns this on.
pub const CHECK_LENGTH: bool = false;

/// Musashi's self-checking programs left out, by the name of their `.s`.
#[rustfmt::skip]
pub const MUSASHI: &[(&str, Until, &str)] = &[
    ("chk", Until::C6, "takes vector 6 into a handler of its own and returns with rte"),
    ("trapv", Until::C6, "takes vector 7 into a handler of its own and returns with rte"),
    ("move_usp", Until::C6, "move usp needs the second stack pointer"),
    ("abcd", Until::Never, "its checksums are of Musashi's own BCD correction, which differs from the 68000's and the suite's once a digit is above 9: $4A + $4A + X is $FB to Musashi and $9B to MAME"),
    ("sbcd", Until::Never, "its checksums are of Musashi's own BCD correction, as abcd's are"),
    ("divs", Until::Never, "its flag checksum expects N and Z unchanged after an overflow, as Musashi leaves them; MAME's microcode sets N and clears Z on every overflow of the suite"),
    ("divu", Until::Never, "its flag checksum expects N and Z unchanged after an overflow, as divs's does"),
    ("move", Until::Never, "cmpi.b #$B9,(d16,PC) at $10160 is a 68020 form, an illegal instruction on a 68000; Musashi's driver runs every program as a 68040"),
];

/// The first skip that leaves `test` of `file` out, if one does.
pub fn skip(file: &str, test: &Test) -> Option<&'static Skip> {
    let vector = exception_vector(test);
    SKIPS.iter().find(|skip| match skip.when {
        When::File(name) => name == file,
        When::TrapFifteen => file == "TRAP" && test.initial.prefetch[0] == 0x4E4F,
        When::AddressError => test.transactions.iter().any(|transaction| {
            matches!(transaction.kind, Kind::ReadAddressError | Kind::WriteAddressError)
        }),
        When::Vector(number) => vector == Some(number),
        When::AnyVector => vector.is_some(),
        When::ModeChange => (test.initial.sr ^ test.final_state.sr) & 0x2000 != 0,
        When::OtherStackPointer => match test.initial.sr & 0x2000 != 0 {
            true => test.initial.usp != test.final_state.usp,
            false => test.initial.ssp != test.final_state.ssp,
        },
    })
}

/// The SR bits not compared for `file`.
pub fn relaxed_flags(file: &str) -> u16 {
    RELAXATIONS.iter().fold(0, |mask, relaxation| match relaxation.relax {
        Relax::Flags { files, mask: bits } if files.contains(&file) => mask | bits,
        _ => mask,
    })
}

/// Whether the length of `file`'s tests is not compared.
pub fn relaxed_length(file: &str) -> bool {
    RELAXATIONS
        .iter()
        .any(|relaxation| matches!(relaxation.relax, Relax::Length { files } if files.contains(&file)))
}

/// The vector a test's final state took, if it shows exception processing:
/// a write of the frame, then the two words of a vector read in supervisor
/// data space, and the program counter at the address they hold. An
/// instruction that reads the vector table itself writes no frame first, or
/// does not land where the words point.
pub fn exception_vector(test: &Test) -> Option<u8> {
    let transactions = &test.transactions;
    let wrote = |index: usize| {
        transactions[..index]
            .iter()
            .any(|transaction| transaction.kind == Kind::Write)
    };
    let next_pc = test.final_state.pc.wrapping_sub(4);
    transactions.windows(2).enumerate().find_map(|(index, pair)| {
        let (high, low) = (pair[0], pair[1]);
        let address = high.address & 0x00FF_FFFF;
        let vector_read = high.kind == Kind::Read
            && low.kind == Kind::Read
            && high.function_code == 5
            && low.function_code == 5
            && address < 0x400
            && address % 4 == 0
            && low.address & 0x00FF_FFFF == address + 2;
        let target = (high.data << 16) | (low.data & 0xFFFF);
        (vector_read && target == next_pc && wrote(index)).then_some((address / 4) as u8)
    })
}
