//! The differential harness of the 68010 dialect: Musashi beside the
//! Interpreter, instruction by instruction (the asm-editor's
//! `docs/design/m68k-machine.md`, decision 3).
//!
//! No single-step suite covers the 68010, so every first word the decoder
//! takes as a 68010 instruction is run from random states, by s68k and by
//! Musashi as a 68010, one instruction each, and the two compared: the data
//! and address registers, both stack pointers, the status register, the
//! program counter, VBR, SFC and DFC, every byte either wrote, which is where
//! the frames of the exceptions are, and the cycles. One case in eight starts
//! with T set, so that the trace exception's frame is compared too.
//!
//! Musashi is not the 68010 everywhere, and [`RELAXATIONS`] says where and
//! why it is not compared, as the conformance suite's skip list does. Its
//! cycles are a table per instruction, the 68010's maxima where the manual
//! gives maxima and no loop mode, so they are counted and reported, by
//! instruction, rather than asserted ([`cycles_are_reported`]).
//!
//! Opt in, as it needs Musashi (`scripts/fetch-musashi.sh`):
//!
//! `cargo test --release --features musashi differential -- --ignored --nocapture`

use std::collections::BTreeMap;

#[cfg(feature = "moira")]
use crate::conformance::moira::Moira;
#[cfg(feature = "musashi")]
use crate::conformance::musashi::{Musashi, CPU_TYPE_68000, CPU_TYPE_68010};
use crate::conformance::reference::State;
use crate::debugger::MutationOperation;
use crate::instructions::{Instruction, RegisterOperand, Size};
use crate::interpreter::{Cpu, Interpreter, RuntimeError, Termination};
use crate::machine::decode::{decode_words, is_legal};
use crate::machine::disassemble::disassemble_words;
#[cfg(feature = "moira")]
use crate::machine::timing;
use crate::machine::Processor;

/// Where an instruction runs from.
const CODE: u32 = 0x2000;
/// Where the handler of vector `v` is: `HANDLERS + 4 * v`.
const HANDLERS: u32 = 0xF000;
/// The data the address registers point into.
const DATA: u32 = 0x3000;
/// The supervisor stack's region and the user stack's.
const SUPERVISOR_STACK: u32 = 0x5000;
const USER_STACK: u32 = 0x6000;
/// Where the vector table is when VBR moves it.
const MOVED_TABLE: u32 = 0x8000;

/// A small xorshift generator, so that every run draws the same cases.
struct Random(u64);

impl Random {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn word(&mut self) -> u16 {
        self.next() as u16
    }

    fn long(&mut self) -> u32 {
        self.next() as u32
    }

    fn below(&mut self, bound: u32) -> u32 {
        (self.next() % bound as u64) as u32
    }

    /// A long that is often one of the edges.
    fn data(&mut self) -> u32 {
        match self.below(8) {
            0 => 0,
            1 => 0xFFFF_FFFF,
            2 => 0x8000_0000,
            3 => self.below(256),
            4 => 0x0000_8000 | self.below(0x8000),
            _ => self.long(),
        }
    }

    fn bytes(&mut self, length: usize) -> Vec<u8> {
        (0..length).map(|_| self.next() as u8).collect()
    }
}

/// One case: the registers, and the memory around them.
struct Case {
    state: State,
    regions: Vec<(u32, Vec<u8>)>,
}

/// A case of the instruction whose first word is `first`, one in eight of
/// them with T set when `trace` says so.
fn case(first: u16, random: &mut Random, trace: bool) -> Case {
    let mut code = vec![first];
    for _ in 0..5 {
        //mostly even, so that most displacements and addresses reach a word
        let word = random.word();
        code.push(if random.below(4) == 0 {
            word
        } else {
            word & !1
        });
    }
    let moved = random.below(4) == 0;
    let mut table = Vec::with_capacity(0x400);
    for vector in 0..256u32 {
        table.extend_from_slice(&(HANDLERS + 4 * vector).to_be_bytes());
    }
    let mut regions = vec![
        (0, table.clone()),
        (
            CODE,
            code.iter().flat_map(|word| word.to_be_bytes()).collect(),
        ),
        (DATA, random.bytes(0x1000)),
        (SUPERVISOR_STACK, random.bytes(0x1000)),
        (USER_STACK, random.bytes(0x1000)),
        (HANDLERS, [0x4E, 0x71].repeat(0x200)),
    ];
    if moved {
        regions.push((MOVED_TABLE, table));
    }
    let supervisor = random.below(4) != 0;
    let traced = random.below(8) == 0 && trace;
    let sr = (traced as u16) << 15
        | (supervisor as u16) << 13
        | (random.below(8) as u16) << 8
        | random.below(32) as u16;
    let mut a = [0; 7];
    for register in a.iter_mut() {
        *register = DATA + 0x400 + random.below(0x400) * 2;
    }
    let mut d = [0; 8];
    for register in d.iter_mut() {
        *register = random.data();
    }
    let state = State {
        d,
        a,
        usp: USER_STACK + 0x800 + random.below(0x100) * 2,
        ssp: SUPERVISOR_STACK + 0x800 + random.below(0x100) * 2,
        sr,
        pc: CODE,
        vbr: if moved { MOVED_TABLE } else { 0 },
        sfc: random.below(8) as u8,
        dfc: random.below(8) as u8,
    };
    Case { state, regions }
}

/// The Interpreter for a case, a 68010's.
fn interpreter(case: &Case) -> Interpreter {
    let state = &case.state;
    let cpu = Cpu::from_state(state.d, state.a, state.usp, state.ssp, state.sr)
        .with_control_registers(state.vbr, state.sfc, state.dfc);
    let mut words = Vec::new();
    for (start, bytes) in &case.regions {
        for (index, pair) in bytes.chunks_exact(2).enumerate() {
            let word = u16::from_be_bytes([pair[0], pair[1]]);
            if word != 0 {
                words.push((*start as usize + 2 * index, word));
            }
        }
    }
    Interpreter::from_dialect_state(Processor::M68010, cpu, state.pc as usize, &words)
}

/// The state the Interpreter left.
fn state_of(interpreter: &Interpreter) -> State {
    let cpu = interpreter.get_cpu();
    let register = |register| interpreter.get_register_value(register, Size::Long);
    let mut d = [0; 8];
    for (index, value) in d.iter_mut().enumerate() {
        *value = register(RegisterOperand::Data(index as u8));
    }
    let mut a = [0; 7];
    for (index, value) in a.iter_mut().enumerate() {
        *value = register(RegisterOperand::Address(index as u8));
    }
    State {
        d,
        a,
        usp: cpu.get_usp(),
        ssp: cpu.get_ssp(),
        sr: cpu.get_sr(),
        pc: interpreter.get_pc() as u32,
        vbr: cpu.get_vbr(),
        sfc: cpu.get_sfc(),
        dfc: cpu.get_dfc(),
    }
}

/// The bytes the Interpreter's one step wrote, and the vectors it took.
fn interpreter_writes(interpreter: &Interpreter) -> (Vec<u32>, Vec<u8>) {
    let mut written = Vec::new();
    let mut vectors = Vec::new();
    for step in interpreter.get_last_steps(1) {
        for mutation in step.get_mutations() {
            match mutation {
                MutationOperation::WriteMemory { address, size, .. } => {
                    written.extend((0..size.to_bytes() as u32).map(|byte| *address as u32 + byte))
                }
                MutationOperation::WriteMemoryBytes { address, new, .. } => {
                    written.extend((0..new.len() as u32).map(|byte| *address as u32 + byte))
                }
                MutationOperation::PushException { vector, .. } => vectors.push(*vector),
                _ => {}
            }
        }
    }
    (written, vectors)
}

/// One place Musashi is not compared, and why.
struct Relaxation {
    /// What it covers, in a word.
    name: &'static str,
    /// Why.
    reason: &'static str,
}

/// Where Musashi is not the 68010, or not s68k's, and is not compared: the
/// harness's skip list.
const RELAXATIONS: &[Relaxation] = &[
    Relaxation {
        name: "address error",
        reason: "Musashi emulates address errors in 68000 mode only (its m68kconf.h); a case in \
                 which s68k takes one, vector 3, or halts on one, pushing a trace frame at an \
                 odd stack pointer, is not compared",
    },
    Relaxation {
        name: "trace after an exception",
        reason: "Musashi forgets a pending trace when the instruction takes an exception; after \
                 `trap`, `trapv`, `chk` and a division by zero the 68000, MAME's, and the 68010 \
                 take the trace exception too, its frame over the first, as s68k does. A traced \
                 case that takes two exceptions is not compared",
    },
    Relaxation {
        name: "trap #15",
        reason:
            "EASy68K's service trap, which s68k carries out and Musashi takes through vector 47",
    },
    Relaxation {
        name: "simhalt",
        reason: "`$FFFF $FFFF` is EASy68K's halt, which pauses s68k and is line F to Musashi",
    },
    Relaxation {
        name: "rte",
        reason: "Musashi pops a format $8 frame whatever its version number, stacks the address \
                 after the `rte` for a format error and leaves the flags; s68k checks the \
                 version and, as Moira, which cputester checks, stacks the `rte` and sets N, Z \
                 and V. The random frames make most formats errors, so `rte` is compared by the \
                 hand tests instead",
    },
    Relaxation {
        name: "movec of a 68020 register",
        reason: "Musashi lets a 68010 `movec` CACR ($002) without an exception; the manual says \
                 any code but the 68010's four is an illegal instruction, as s68k does",
    },
    Relaxation {
        name: "move16",
        reason: "Musashi runs the 68040's `move16`, $F600 to $F63F, whatever the processor; on a \
                 68010 they are line F words, vector 11, as s68k takes them",
    },
    Relaxation {
        name: "the end of the bus",
        reason: "s68k ends the program, out of bounds, at a word or a long that straddles the \
                 end of the 16 MB where the 68000 and the 68010 wrap round to $000000, as \
                 Musashi does; a case that runs into it is not compared",
    },
    Relaxation {
        name: "undefined flags",
        reason: "N and V after `abcd`, `sbcd` and `nbcd`, N, Z, V and C after `chk` in bounds, \
                 N, Z and C after a division that overflows, and N, Z and V after one by zero \
                 are undefined (the Programmer's Reference Manual), and s68k sets them as \
                 MAME's 68000 does, which Musashi does not; they are left out of the status \
                 register compared and of the one a division by zero stacks, and a traced case \
                 of these instructions, whose trace frame stacks them, is not compared. Against \
                 Moira, the 68010 cputester measured, only the decimal instructions' N and V and \
                 the flags of a division by zero are left out, C among them, which s68k leaves \
                 as the 68000 found it",
    },
    Relaxation {
        name: "invalid decimal",
        reason: "`abcd`, `sbcd` and `nbcd` of a byte with a digit above 9 are undefined, and \
                 s68k answers what MAME's 68000 answers, which Musashi does not; such a case is \
                 not compared, nor one of `nbcd` of a mode the harness does not read the byte of",
    },
    Relaxation {
        name: "divs $80000000 by -1",
        reason: "Musashi gives `divs` of $80000000 by -1 a quotient of 0 with V clear; it \
                 overflows, as MAME's 68000 and s68k have it",
    },
    Relaxation {
        name: "link a7",
        reason: "`link a7,#d` pushes A7 as it was before it moves, as MAME's 68000 and Moira's \
                 68010, which cputester checks, push it; Musashi pushes it four lower. The long \
                 pushed is not compared",
    },
];

/// The flags of the status register an instruction leaves undefined and
/// s68k and the reference set differently ("undefined flags" of
/// [`RELAXATIONS`]), as a mask of what is compared, given the state s68k
/// left, whether the division it ran was by zero, and whether the reference
/// is Musashi or Moira.
fn compared_flags(instruction: &Instruction, after: &State, by_zero: bool, musashi: bool) -> u16 {
    const N: u16 = 8;
    const Z: u16 = 4;
    const V: u16 = 2;
    const C: u16 = 1;
    match instruction {
        Instruction::ABCD(..) | Instruction::SBCD(..) | Instruction::NBCD(_) => !(N | V),
        Instruction::DIVx(..) if by_zero => !(N | Z | V | C),
        Instruction::CHK(..) if musashi => !(N | Z | V | C),
        Instruction::DIVx(..) if musashi && after.sr & V != 0 => !(N | Z | C),
        _ => 0xFFFF,
    }
}

/// Which relaxation keeps a case of `first` from being run, if one does.
fn relaxed(first: u16, code: &[u16]) -> Option<&'static str> {
    if first == 0x4E4F {
        return Some("trap #15");
    }
    if first == 0xFFFF {
        return Some("simhalt");
    }
    if first == 0x4E73 {
        return Some("rte");
    }
    if (first & 0xFFFE) == 0x4E7A && !matches!(code[1] & 0x0FFF, 0x000 | 0x001 | 0x800 | 0x801) {
        return Some("movec of a 68020 register");
    }
    if (0xF600..=0xF63F).contains(&first) {
        return Some("move16");
    }
    None
}

/// Which relaxation keeps a case of `instruction` from being run, by the
/// data it starts from, if one does.
fn relaxed_data(
    instruction: &Instruction,
    state: &State,
    fresh: &Interpreter,
) -> Option<&'static str> {
    use crate::instructions::Operand;
    let a = |number: u8| match number {
        7 if state.sr & 0x2000 != 0 => state.ssp,
        7 => state.usp,
        _ => state.a[number as usize],
    };
    let byte = |address: u32| {
        fresh
            .get_memory()
            .read_bytes(address as usize & 0xFF_FFFF, 1)
            .unwrap()[0] as u32
    };
    let step = |number: u8| if number == 7 { 2 } else { 1 };
    //the byte a decimal instruction reads through `operand`, `before` bytes of the
    //predecrements of the same register already made; `None` for a mode not looked at
    let data = |operand: &Operand, before: u32| match operand {
        Operand::Register(RegisterOperand::Data(number)) => Some(state.d[*number as usize]),
        Operand::Indirect(number) | Operand::PostIndirect(number) => Some(byte(a(*number))),
        Operand::PreIndirect(number) => Some(byte(a(*number).wrapping_sub(step(*number) + before))),
        _ => None,
    };
    let invalid =
        |value: Option<u32>| value.is_none_or(|byte| byte & 0x0F > 9 || byte & 0xF0 > 0x90);
    match instruction {
        Instruction::ABCD(source, destination) | Instruction::SBCD(source, destination) => {
            let before = match (source, destination) {
                (Operand::PreIndirect(x), Operand::PreIndirect(y)) if x == y => step(*x),
                _ => 0,
            };
            (invalid(data(source, 0)) || invalid(data(destination, before)))
                .then_some("invalid decimal")
        }
        Instruction::NBCD(operand) if invalid(data(operand, 0)) => Some("invalid decimal"),
        Instruction::DIVx(
            source,
            RegisterOperand::Data(number),
            crate::instructions::Sign::Signed,
        ) if state.d[*number as usize] == 0x8000_0000
            && matches!(source, Operand::Register(RegisterOperand::Data(_)))
            && data(source, 0).is_some_and(|divisor| divisor as u16 == 0xFFFF) =>
        {
            Some("divs $80000000 by -1")
        }
        _ => None,
    }
}

/// A 68010 to compare the Interpreter with: Musashi or Moira.
trait Reference {
    /// How a report names it.
    const NAME: &'static str;
    /// Whether it is Musashi, whose gaps [`RELAXATIONS`] lists; Moira has
    /// those of the end of the bus and EASy68K's own words only, and its
    /// address errors are compared on their cycles.
    const MUSASHI: bool;
    fn load(&mut self, state: &State, regions: &[(u32, Vec<u8>)]);
    fn step(&mut self) -> u32;
    fn state(&self) -> State;
    fn byte(&mut self, address: u32) -> u8;
    fn written(&self) -> Vec<u32>;
}

#[cfg(feature = "musashi")]
impl Reference for Musashi {
    const NAME: &'static str = "Musashi";
    const MUSASHI: bool = true;
    fn load(&mut self, state: &State, regions: &[(u32, Vec<u8>)]) {
        Musashi::load(self, state, regions)
    }
    fn step(&mut self) -> u32 {
        Musashi::step(self)
    }
    fn state(&self) -> State {
        Musashi::state(self)
    }
    fn byte(&mut self, address: u32) -> u8 {
        self.read(address, 1)[0]
    }
    fn written(&self) -> Vec<u32> {
        Musashi::written(self)
    }
}

#[cfg(feature = "moira")]
impl Reference for Moira {
    const NAME: &'static str = "Moira";
    const MUSASHI: bool = false;
    fn load(&mut self, state: &State, regions: &[(u32, Vec<u8>)]) {
        Moira::load(self, state, regions)
    }
    fn step(&mut self) -> u32 {
        Moira::step(self)
    }
    fn state(&self) -> State {
        Moira::state(self)
    }
    fn byte(&mut self, address: u32) -> u8 {
        self.read(address, 1)[0]
    }
    fn written(&self) -> Vec<u32> {
        Moira::written(self)
    }
}

/// Where Moira is not compared, by the data a case starts from.
const MOIRA_RELAXATIONS: &[Relaxation] = &[Relaxation {
    name: "divs of $80000000",
    reason: "Moira works out whether `divs` overflows from the absolute value of its dividend \
             in 32 bits, which $80000000 has none of, and counts the cycles of a division that \
             does not overflow; it overflows, and s68k counts the cycles of one that does",
}];

/// Which of [`MOIRA_RELAXATIONS`] keeps a case from being compared with
/// Moira, if one does.
fn relaxed_for_moira(code: &[u16], state: &State) -> Option<&'static str> {
    let divs_of_a_register = code[0] & 0xF1C0 == 0x81C0;
    let dividend = state.d[((code[0] >> 9) & 7) as usize];
    (divs_of_a_register && dividend == 0x8000_0000).then_some("divs of $80000000")
}

/// What one instruction's cases found.
#[derive(Default)]
struct Tally {
    run: usize,
    differences: Vec<String>,
    /// The cycles of each case where the two counted differently, s68k's
    /// first.
    cycles: BTreeMap<(u32, u32), (usize, Vec<String>)>,
    cycles_agree: usize,
}

/// The name a report gives an instruction: its mnemonic and size.
fn mnemonic(instruction: &Instruction, bytes: &[u8]) -> String {
    let text = disassemble_words(instruction, bytes, None);
    text.split(' ').next().unwrap_or_default().to_string()
}

/// Every case of every 68010 instruction, `per_word` cases of each first
/// word, against `reference`; answers the tallies by mnemonic and the skipped
/// cases by relaxation.
fn compare<R: Reference>(
    reference: &mut R,
    per_word: usize,
) -> (BTreeMap<String, Tally>, BTreeMap<&'static str, usize>) {
    let mut tallies: BTreeMap<String, Tally> = BTreeMap::new();
    let mut skipped: BTreeMap<&'static str, usize> = BTreeMap::new();
    for first in 0..=0xFFFFu16 {
        if !is_legal(first, Processor::M68010) {
            continue;
        }
        let mut random = Random(0x9E37_79B9_7F4A_7C15 ^ (first as u64) << 17 | 1);
        for _ in 0..per_word {
            //Moira takes a pending trace exception at its next step, which is no step here
            let case = case(first, &mut random, R::MUSASHI);
            let code: Vec<u16> = case.regions[1]
                .1
                .chunks_exact(2)
                .map(|pair| u16::from_be_bytes([pair[0], pair[1]]))
                .collect();
            if let Some(relaxation) = relaxed(first, &code) {
                if R::MUSASHI || ["trap #15", "simhalt"].contains(&relaxation) {
                    *skipped.entry(relaxation).or_default() += 1;
                    continue;
                }
            }
            if !R::MUSASHI {
                if let Some(relaxation) = relaxed_for_moira(&code, &case.state) {
                    *skipped.entry(relaxation).or_default() += 1;
                    continue;
                }
            }
            let decoded = decode_words(&code, CODE, Processor::M68010).unwrap();
            let bytes: Vec<u8> = code[..decoded.size as usize / 2]
                .iter()
                .flat_map(|word| word.to_be_bytes())
                .collect();
            let name = mnemonic(&decoded.instruction, &bytes);
            let mut ours = interpreter(&case);
            if R::MUSASHI {
                if let Some(relaxation) = relaxed_data(&decoded.instruction, &case.state, &ours) {
                    *skipped.entry(relaxation).or_default() += 1;
                    continue;
                }
            }
            if let Err(RuntimeError::OutOfBounds(_)) = ours.step() {
                *skipped.entry("the end of the bus").or_default() += 1;
                continue;
            }
            let (our_writes, vectors) = interpreter_writes(&ours);
            let halted = matches!(ours.get_termination(), Some(Termination::Halted { .. }));
            //Moira's address errors are compared on their cycles: its frames hold its own
            //internal state, and stack the program counter its prefetch has reached
            let address_error = vectors.contains(&3) || halted;
            if address_error && R::MUSASHI {
                *skipped.entry("address error").or_default() += 1;
                continue;
            }
            let traced = case.state.sr & 0x8000 != 0;
            if R::MUSASHI && traced && vectors.len() > 1 {
                *skipped.entry("trace after an exception").or_default() += 1;
                continue;
            }
            let by_zero = vectors.contains(&5);
            let flags = compared_flags(&decoded.instruction, &state_of(&ours), by_zero, R::MUSASHI);
            if R::MUSASHI && traced && flags != 0xFFFF {
                *skipped.entry("undefined flags").or_default() += 1;
                continue;
            }
            reference.load(&case.state, &case.regions);
            let their_cycles = reference.step();
            let tally = tallies.entry(name.clone()).or_default();
            tally.run += 1;
            let (mut mine, mut theirs) = (state_of(&ours), reference.state());
            mine.sr &= flags;
            theirs.sr &= flags;
            let mut written = our_writes;
            written.extend(reference.written());
            written.sort_unstable();
            written.dedup();
            //the long `link a7` pushes, under A7 as it was
            if let (true, Instruction::LINK(RegisterOperand::Address(7), _)) =
                (R::MUSASHI, decoded.instruction)
            {
                let a7 = match case.state.sr & 0x2000 != 0 {
                    true => case.state.ssp,
                    false => case.state.usp,
                };
                written.retain(|&at| !(a7.wrapping_sub(4)..a7).contains(&at));
            }
            //and the low byte of the status register a division by zero stacks
            if flags != 0xFFFF && by_zero {
                written.retain(|&at| at != mine.ssp + 1);
            }
            let memory = ours.get_memory();
            let differing: Vec<String> = written
                .iter()
                .filter_map(|&at| {
                    let our = memory.read_bytes(at as usize, 1).unwrap()[0];
                    let their = reference.byte(at);
                    (our != their).then(|| format!("${at:06X}: {our:02X} vs {their:02X}"))
                })
                .collect();
            if !address_error && (mine != theirs || !differing.is_empty()) {
                if tally.differences.len() < 4 {
                    tally.differences.push(format!(
                        "{} ({:04X?}):\n    s68k    {mine:X?}\n    {:<7} {theirs:X?}\n    \
                         memory {differing:?}",
                        disassemble_words(&decoded.instruction, &bytes, None),
                        &code[..decoded.size as usize / 2],
                        R::NAME,
                    ));
                } else {
                    tally.differences.push(String::new());
                }
            }
            let our_cycles = ours.get_cycles() as u32;
            if our_cycles == their_cycles {
                tally.cycles_agree += 1;
            } else {
                let (count, examples) = tally.cycles.entry((our_cycles, their_cycles)).or_default();
                *count += 1;
                if examples.len() < 3 {
                    examples.push(disassemble_words(&decoded.instruction, &bytes, None));
                }
            }
        }
    }
    (tallies, skipped)
}

/// The decoder and Musashi agree on which first words are 68010
/// instructions, but for the groups where Musashi is not the 68010, as for
/// the 68000 (`machine::decode`'s `the_legal_first_words_are_musashis_68000_words`):
/// `illegal`, the `$6xFF` branches, which are −1 before the 68020, and the
/// line F words Musashi gives the 68020 and later.
#[cfg(feature = "musashi")]
#[test]
#[ignore = "needs Musashi: scripts/fetch-musashi.sh, then --features musashi"]
fn the_legal_first_words_are_musashis_68010_words() {
    let musashi = Musashi::lock();
    let mut exceptions: BTreeMap<&str, usize> = BTreeMap::new();
    for word in 0..=0xFFFFu16 {
        let ours = is_legal(word, Processor::M68010);
        let theirs = musashi.is_valid(word, CPU_TYPE_68010);
        if ours == theirs {
            continue;
        }
        let group = match word {
            0x4AFC => "illegal",
            _ if word & 0xF0FF == 0x60FF => "a $FF branch",
            0xF000..=0xFFFF => "line F",
            _ => panic!(
                "{word:04X}: s68k {ours}, Musashi {theirs} on the 68010 (and {} on the 68000)",
                musashi.is_valid(word, CPU_TYPE_68000)
            ),
        };
        assert!(
            ours,
            "{word:04X} is an instruction to Musashi and not to s68k"
        );
        *exceptions.entry(group).or_default() += 1;
    }
    assert_eq!(
        exceptions,
        BTreeMap::from([("a $FF branch", 16), ("illegal", 1), ("line F", 2305)])
    );
}

/// Every 68010 instruction against Musashi: no state differs outside the
/// relaxations.
#[cfg(feature = "musashi")]
#[test]
#[ignore = "needs Musashi: scripts/fetch-musashi.sh, then --features musashi"]
fn every_68010_instruction_does_what_musashi_does() {
    let per_word = std::env::var("S68K_DIFFERENTIAL_CASES")
        .ok()
        .and_then(|cases| cases.parse().ok())
        .unwrap_or(2);
    let (tallies, skipped) = compare(&mut Musashi::lock(), per_word);
    assert_no_differences(&tallies, &skipped, true);
}

/// The tallies' state differences, which must be none, and what was skipped.
fn assert_no_differences(
    tallies: &BTreeMap<String, Tally>,
    skipped: &BTreeMap<&'static str, usize>,
    musashi: bool,
) {
    let run: usize = tallies.values().map(|tally| tally.run).sum();
    let mut report = String::new();
    let mut differing = 0;
    for (name, tally) in tallies {
        if tally.differences.is_empty() {
            continue;
        }
        differing += tally.differences.len();
        report += &format!(
            "{name}: {} of {} cases differ\n{}\n",
            tally.differences.len(),
            tally.run,
            tally
                .differences
                .iter()
                .filter(|text| !text.is_empty())
                .cloned()
                .collect::<Vec<_>>()
                .join("\n")
        );
    }
    println!("{run} cases run, {skipped:?} skipped");
    let relaxations = match musashi {
        true => RELAXATIONS,
        false => MOIRA_RELAXATIONS,
    };
    for relaxation in relaxations {
        println!("relaxed, {}: {}", relaxation.name, relaxation.reason);
    }
    assert!(
        differing == 0,
        "{differing} of {run} cases differ:\n{report}"
    );
}

/// The cycles of every 68010 instruction against Musashi's table, by
/// instruction: how many cases agree, and where they do not, what each
/// counted.
#[cfg(feature = "musashi")]
#[test]
#[ignore = "needs Musashi: scripts/fetch-musashi.sh, then --features musashi"]
fn cycles_are_reported() {
    let (tallies, _) = compare(&mut Musashi::lock(), 1);
    report_cycles::<Musashi>(&tallies);
}

/// Every 68010 instruction against Moira: no state differs, and every case
/// takes the cycles Moira counts.
#[cfg(feature = "moira")]
#[test]
#[ignore = "needs Moira: scripts/fetch-moira.sh, then --features moira"]
fn every_68010_instruction_takes_moiras_cycles() {
    let per_word = std::env::var("S68K_DIFFERENTIAL_CASES")
        .ok()
        .and_then(|cases| cases.parse().ok())
        .unwrap_or(2);
    let (tallies, skipped) = compare(&mut Moira::lock(), per_word);
    let differing = report_cycles::<Moira>(&tallies);
    assert_no_differences(&tallies, &skipped, false);
    assert_eq!(differing, 0, "cases whose cycles differ from Moira's");
}

/// Prints the tallies' cycles, by instruction, and answers how many cases
/// counted differently.
fn report_cycles<R: Reference>(tallies: &BTreeMap<String, Tally>) -> usize {
    let mut agree = 0;
    let mut total = 0;
    for (name, tally) in tallies {
        agree += tally.cycles_agree;
        total += tally.run;
        if !tally.cycles.is_empty() {
            let pairs: Vec<String> = tally
                .cycles
                .iter()
                .take(6)
                .map(|((ours, theirs), (count, examples))| {
                    format!("{ours} vs {theirs} ×{count} ({})", examples.join("; "))
                })
                .collect();
            println!(
                "{name}: {} of {} agree; s68k vs {} {}",
                tally.cycles_agree,
                tally.run,
                R::NAME,
                pairs.join(", ")
            );
        }
    }
    println!("cycles: {agree} of {total} cases agree with {}", R::NAME);
    total - agree
}

/// A loop of one instruction and a `dbf d7` back to it, run four times
/// around, for the 68010's loop mode: the case of the first word `first`.
#[cfg(feature = "moira")]
fn loop_case(first: u16) -> Case {
    let mut random = Random(0x5151_7A7A ^ first as u64);
    let mut case = case(first, &mut random, false);
    //the instruction, then `dbf d7` back to it, then `nop`s
    let mut code = vec![first, 0x51CF, 0xFFFC];
    code.extend([0x4E71; 3]);
    case.regions[1].1 = code.iter().flat_map(|word| word.to_be_bytes()).collect();
    case.state.d[7] = 3;
    case.state.sr &= 0x2700;
    case
}

/// The 68010's loop mode against Moira's: every one-word 68010 instruction
/// in a loop of a `dbf d7` back to it, four times round, both stepped nine
/// times, the cycles of each step compared. Every instruction the 68010
/// loops enters loop mode at the first `dbf`, and the loop's next turns take
/// their loop times; the others take their own. A case in which either takes
/// an exception, which ends the loop, is left out.
#[cfg(feature = "moira")]
#[test]
#[ignore = "needs Moira: scripts/fetch-moira.sh, then --features moira"]
fn loop_mode_takes_moiras_cycles() {
    let mut moira = Moira::lock();
    let (mut compared, mut looped, mut skipped) = (0, 0, 0);
    let mut differences = Vec::new();
    for first in 0..=0xFFFFu16 {
        if !is_legal(first, Processor::M68010) || first == 0x4E4F || first == 0xFFFF {
            continue;
        }
        let Ok(decoded) = decode_words(&[first, 0, 0, 0], CODE, Processor::M68010) else {
            continue;
        };
        if decoded.size != 2 {
            continue;
        }
        let case = loop_case(first);
        if relaxed_for_moira(&[first], &case.state).is_some() {
            skipped += 1;
            continue;
        }
        moira.load(&case.state, &case.regions);
        let theirs: Vec<u32> = (0..9).map(|_| moira.step()).collect();
        let mut ours = interpreter(&case);
        let mut mine = Vec::new();
        let mut exception = false;
        for _ in 0..9 {
            let before = ours.get_cycles();
            let stepped = ours.step();
            mine.push((ours.get_cycles() - before) as u32);
            let (_, vectors) = interpreter_writes(&ours);
            exception |= stepped.is_err() || !vectors.is_empty();
        }
        if exception || theirs.contains(&u32::MAX) || moira.state().pc != ours.get_pc() as u32 {
            skipped += 1;
            continue;
        }
        compared += 1;
        looped += (theirs[3] == timing::LOOP_CONTINUE as u32) as usize;
        if mine != theirs && differences.len() < 40 {
            let bytes = first.to_be_bytes();
            differences.push(format!(
                "{first:04X} {}: s68k {mine:?}, Moira {theirs:?}",
                disassemble_words(&decoded.instruction, &bytes, None)
            ));
        }
    }
    println!("{compared} loops compared, {looped} in loop mode, {skipped} left out");
    assert!(
        differences.is_empty(),
        "{} loops differ:\n{}",
        differences.len(),
        differences.join("\n")
    );
}
