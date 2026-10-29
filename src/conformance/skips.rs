//! What the conformance harness skips and relaxes, in one place, with a
//! reason for every entry (ADR 0010; the asm-editor's
//! `docs/design/m68k-machine-plan.md`, planning decision 6).
//!
//! An entry that waits for a milestone of the plan names it with an
//! [`Until`]: the milestone deletes every line of [`SKIPS`] and [`MUSASHI`]
//! that names it. An entry marked [`Until::Never`] is a choice of s68k's and
//! stays. Milestone C6, exceptions, deleted its lines: every exception the
//! suite takes, its address errors among them, is checked. Milestone C8,
//! interrupts, deleted those of `stop` and `reset`, whose files are checked
//! whole. Milestone C9, cycles, turned [`CHECK_LENGTH`] on: every test's
//! length in cycles is compared, those of `tas` and `trapv` included, whose
//! relaxations it deleted, as the 68000's times agree with the suite's on all
//! 2,500 of each though the suite does not vouch for them.
//!
//! A **skip** leaves a test out. A **relaxation** runs it and compares less,
//! for the suite's own documented errata; every relaxation is one line of
//! [`RELAXATIONS`] with its source.

use super::suite::{Kind, Test};

/// The milestone whose work makes a skip unnecessary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Until {
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
];

/// What a [`Relaxation`] compares less.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Relax {
    /// The SR bits of `mask` are not compared for these files.
    Flags { files: &'static [&'static str], mask: u16 },
    /// The length in cycles is not compared for these files. No file needs
    /// it since C9.
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
    Relaxation { relax: Relax::NoTrace, source: "about half the initial states set T, and the final state and the length are the ones before the trace exception (suite issue #2); snow masks trace the same way" },
    Relaxation { relax: Relax::ZeroMemory, source: "MOVE.b and MOVEP list bytes they never wrote, as zeros, in the final RAM (suite issue #1); memory starts as zeros, as in the generator" },
    Relaxation { relax: Relax::Flags { files: &["ABCD", "SBCD", "NBCD"], mask: N_AND_V }, source: "N and V are undefined for the BCD instructions (Programmer's Reference Manual), and s68k leaves them as they were; m68k-rs masks them too" },
];

/// Files with failures the conformance work has found and not fixed yet: a
/// failing test of one is counted as known, and does not fail the run. Each
/// fix deletes its file's line, after which every test of the file has to
/// pass; a failure that needs a later milestone goes in [`SKIPS`] instead.
#[rustfmt::skip]
pub const KNOWN_FAILURES: &[(&str, &str)] = &[
];

/// Whether a test's length in cycles is compared with the cycles the step
/// took ([`Interpreter::get_cycles`]), which milestone C9 turned on.
///
/// [`Interpreter::get_cycles`]: crate::interpreter::Interpreter::get_cycles
pub const CHECK_LENGTH: bool = true;

/// Musashi's self-checking programs left out, by the name of their `.s`.
#[rustfmt::skip]
pub const MUSASHI: &[(&str, Until, &str)] = &[
    ("abcd", Until::Never, "its checksums are of Musashi's own BCD correction, which differs from the 68000's and the suite's once a digit is above 9: $4A + $4A + X is $FB to Musashi and $9B to MAME"),
    ("sbcd", Until::Never, "its checksums are of Musashi's own BCD correction, as abcd's are"),
    ("divs", Until::Never, "its flag checksum expects N and Z unchanged after an overflow, as Musashi leaves them; MAME's microcode sets N and clears Z on every overflow of the suite"),
    ("divu", Until::Never, "its flag checksum expects N and Z unchanged after an overflow, as divs's does"),
    ("move", Until::Never, "cmpi.b #$B9,(d16,PC) at $10160 is a 68020 form, an illegal instruction on a 68000; Musashi's driver runs every program as a 68040"),
];

/// The programs of Musashi's `test/mc68040` that s68k's 68020 does not run,
/// and why: none since the 68020's batch. `interrupt`, whose interrupts come
/// from a register of Musashi's test driver, is not vendored.
pub const MUSASHI_68040: &[(&str, &str)] = &[];

/// The checks of Musashi's `test/mc68040` programs that s68k's 68020 is not
/// held to, by program and by the addresses of the branches to `TEST_FAIL`
/// that make them, which the run replaces with `nop`s, and why. A relaxation
/// of the program, not a skip: the rest of it runs and is checked.
pub const MUSASHI_68040_RELAXATIONS: &[(&str, &[u32], &str)] = &[
    (
        "divs_long",
        &[0x100E8, 0x100EC, 0x100F0, 0x101DC],
        "it checks the flags the 68040 leaves where Motorola leaves them undefined: N, Z and V \
         unchanged after a division by zero, and N and Z after a 64-bit division that \
         overflows, in the sum of the flags of its loop of `divs.l`. The 68020 clears N and \
         sets Z after a division by zero, and sets N and Z from the dividend after an \
         overflow, as WinUAE has it from cputester's runs on the hardware; the quotients and \
         remainders, and every flag Motorola defines, are checked",
    ),
    (
        "chk2",
        &[0x100F0, 0x100FA],
        "`chk2 (%a7),%a0`, written with no size, assembles as a word, whose bounds, $1000 and 0, \
         are the wrong way round; the 68020 compares the whole of A0 with them sign extended, \
         as the manual says, and finds $70000001 in the range that wraps round, with no \
         trap, where Musashi compares its low word and traps, which the program expects",
    ),
    (
        "cmp2",
        &[0x100AA, 0x100BA, 0x100C8, 0x100E4],
        "the same word `cmp2 (%a7),%a0` as `chk2`'s; and the byte bounds $F0 and $70, which \
         it reads relative to the program counter, where Musashi reads a byte bound unsigned \
         and nowhere else: signed, as the 68020 reads them, $60, $F0 and $70 are in bounds, \
         and the program's own comments doubt it expects the right carry there",
    ),
    (
        "divu_long",
        &[0x100E8, 0x100EC, 0x100F0, 0x101DC],
        "as `divs_long`: the 68020 sets N and Z from the dividend's low long and sets V after a \
         division by zero, and sets N and Z from it after an overflow",
    ),
];

/// The first skip that leaves `test` of `file` out, if one does.
pub fn skip(file: &str, test: &Test) -> Option<&'static Skip> {
    SKIPS.iter().find(|skip| match skip.when {
        When::File(name) => name == file,
        When::TrapFifteen => file == "TRAP" && test.initial.prefetch[0] == 0x4E4F,
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
/// does not land where the words point. No skip reads it since every
/// exception is checked; it is how the suite's exception tests are counted.
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
