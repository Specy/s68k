//! The differential harness of the 68010 and 68020 dialects: Musashi beside
//! the Interpreter, instruction by instruction (the asm-editor's
//! `docs/design/m68k-machine.md`, decision 3). The 68020's runs are at the
//! end: its addressing modes, its own instructions, the 68010's instructions
//! on it, its interrupts and its `rte`, from states of its supervisor model
//! ([`case_68020`]), with Musashi's gaps there in [`RELAXATIONS_68020`] and
//! [`RELAXATIONS_SUPERVISOR`], and its cycles, which are an estimate on both
//! sides and are reported ([`the_68020s_cycles_are_reported`]).
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
use crate::conformance::musashi::{Musashi, CPU_TYPE_68000, CPU_TYPE_68010, CPU_TYPE_68EC020};
use crate::conformance::reference::State;
use crate::debugger::MutationOperation;
#[cfg(feature = "musashi")]
use crate::instructions::Operand;
use crate::instructions::{Instruction, RegisterOperand, Size};
use crate::interpreter::{Cpu, Interpreter, StackPointers, Termination};
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
/// The 68020's master stack's region.
const MASTER_STACK: u32 = 0x7000;
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
        msp: 0,
        cacr: 0,
        caar: 0,
    };
    Case { state, regions }
}

/// A case of the 68020's: [`case`]'s, with room after the first word for
/// the 68020's longest operands, T0 set one time in eight, M one time in
/// two, a master stack of its own, and CACR's two bits and CAAR random.
fn case_68020(first: u16, random: &mut Random, trace: bool) -> Case {
    let mut case = case(first, random, trace);
    for _ in 0..8 {
        let word = random.word() & !1;
        case.regions[1].1.extend_from_slice(&word.to_be_bytes());
    }
    let t0 = random.below(8) == 0 && trace;
    let master = random.below(2) == 0;
    case.state.sr |= (t0 as u16) << 14 | (master as u16) << 12;
    case.state.msp = MASTER_STACK + 0x800 + random.below(0x100) * 2;
    case.state.cacr = random.below(4);
    case.state.caar = random.long();
    case.regions.push((MASTER_STACK, random.bytes(0x1000)));
    case
}

/// The stack pointer A7 is in `state`: the USP in user mode, and in
/// supervisor mode the ISP, or on a 68020 with M set the MSP.
fn a7_of(state: &State) -> u32 {
    match (state.sr & 0x2000 != 0, state.sr & 0x1000 != 0) {
        (false, _) => state.usp,
        (true, false) => state.ssp,
        (true, true) => state.msp,
    }
}

/// The Interpreter for a case, a 68010's.
#[cfg(feature = "moira")]
fn interpreter(case: &Case) -> Interpreter {
    interpreter_for(case, Processor::M68010)
}

/// The Interpreter for a case, `processor`'s.
fn interpreter_for(case: &Case, processor: Processor) -> Interpreter {
    let state = &case.state;
    let pointers = StackPointers {
        usp: state.usp,
        ssp: state.ssp,
        msp: state.msp,
    };
    let cpu = Cpu::of(processor)
        .with_state(state.d, state.a, pointers, state.sr)
        .with_control_registers(state.vbr, state.sfc, state.dfc)
        .with_cache_registers(state.cacr, state.caar);
    let mut words = Vec::new();
    for (start, bytes) in &case.regions {
        for (index, pair) in bytes.chunks_exact(2).enumerate() {
            let word = u16::from_be_bytes([pair[0], pair[1]]);
            if word != 0 {
                words.push((*start as usize + 2 * index, word));
            }
        }
    }
    Interpreter::from_dialect_state(processor, cpu, state.pc as usize, &words)
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
        msp: cpu.get_msp(),
        cacr: cpu.get_cacr(),
        caar: cpu.get_caar(),
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
        7 => a7_of(state),
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

/// Where Musashi's 68EC020 is not the 68020, or not s68k's, in its
/// supervisor model, which the 68020's runs of the 68010's instructions, of
/// its own and of its interrupts do not compare.
#[cfg(feature = "musashi")]
const RELAXATIONS_SUPERVISOR: &[Relaxation] = &[
    Relaxation {
        name: "address error",
        reason: "Musashi emulates address errors in 68000 mode only (its m68kconf.h); the \
                 68020 reads and writes data at odd addresses, which s68k's 68020 does and both \
                 compare, and its only address error is an instruction fetched from an odd \
                 address, a jump or a return to one, which Musashi runs from there. A case in \
                 which s68k takes one, vector 3, is not compared",
    },
    Relaxation {
        name: "trace after an exception",
        reason: "Musashi forgets a pending trace when the instruction takes an exception; after \
                 `trap`, `trapv`, `trapcc`, `chk`, `chk2` and a division by zero with T1 set, \
                 the 68020 takes the trace exception too, as WinUAE's has it from cputester and \
                 s68k does. A traced case that takes two exceptions is not compared",
    },
    Relaxation {
        name: "t0 where the 68020 and the 68040 differ",
        reason: "with T0 set and T1 clear, Musashi traces `nop`, `movec`, `moves`, `move` to \
                 USP, `stop`, `cas` and `cas2`, which the 68040 traces and the 68020 does not, \
                 and does not trace `andi`, `ori` and `eori` to CCR, which the 68020 traces, as \
                 WinUAE has them from cputester; such a case is not compared",
    },
    Relaxation {
        name: "line F",
        reason: "Musashi's 68EC020 runs line F words as the instructions of an FPU, and \
                 `cptrapcc` and the like of coprocessor 0 as instructions even where its \
                 disassembler says they are none; with no coprocessor the 68020 takes every \
                 line F word through vector 11 (M68020UM 6.3.7), as s68k does and as the hand \
                 tests check, and as the 68010's run compares with Musashi. A line F word is \
                 not compared on the 68020",
    },
    Relaxation {
        name: "t0 of move dn,sr",
        reason: "Musashi's `move Dn,sr` alone of the writes of the status register is not \
                 traced under T0, where its other modes, `andi`, `ori` and `eori` to SR and the \
                 68020 trace it; such a case is not compared",
    },
    Relaxation {
        name: "undefined flags",
        reason: "the flags of `chk`, `chk2` and a long division the 68020 leaves undefined, \
                 and s68k sets as WinUAE's 68020 has them from cputester (RELAXATIONS_68020, and \
                 RELAXATIONS for the 68000's instructions), are also in the status register the \
                 frame of `chk`, `chk.l` and `chk2` out of bounds and of a division by zero \
                 stacks, whose low byte is not compared, and in that of a trace frame, so a case \
                 of these instructions with T1 or T0 set is not compared",
    },
    Relaxation {
        name: "rte",
        reason: "Musashi raises the format error on the bus fault frames $A and $B, which s68k \
                 pops, and stacks the address after the `rte` for a format error, where the \
                 68020 and s68k stack the `rte`; a case is compared when the frame on top, or \
                 the one a throwaway frame on top leads to, is $0 or $2",
    },
    Relaxation {
        name: "CACR's C and CE",
        reason: "C and CE, bits 3 and 2 of CACR, clear the cache and an entry of it when they \
                 are written, and read as 0 (the M68020 User's Manual, 7.1.2); Musashi keeps \
                 them, and s68k, which has no cache, keeps E and F alone. Only those two are \
                 compared",
    },
];

/// Which of [`RELAXATIONS_SUPERVISOR`] keeps a case of `instruction` on the
/// 68020 from being run, by the state it starts from, if one does: `fresh`,
/// the Interpreter of the case, reads the frames an `rte` would pop.
fn relaxed_on_the_68020(
    instruction: &Instruction,
    state: &State,
    fresh: &Interpreter,
) -> Option<&'static str> {
    use crate::instructions::Operand;
    let t0_alone = state.sr & 0xC000 == 0x4000;
    match instruction {
        Instruction::NOP
        | Instruction::MOVEC { .. }
        | Instruction::MOVES { .. }
        | Instruction::STOP(_)
        | Instruction::MOVEUSP { to_usp: true, .. }
        | Instruction::CAS { .. }
        | Instruction::CAS2 { .. }
        | Instruction::ANDItoCCR(_)
        | Instruction::ORItoCCR(_)
        | Instruction::EORItoCCR(_)
            if t0_alone =>
        {
            Some("t0 where the 68020 and the 68040 differ")
        }
        Instruction::MOVEtoSR(Operand::Register(_)) if t0_alone => Some("t0 of move dn,sr"),
        Instruction::RTE if state.sr & 0x2000 != 0 => {
            let word = |address: u32| {
                let bytes = fresh
                    .get_memory()
                    .read_bytes(address as usize & 0xFF_FFFF, 2)
                    .unwrap();
                u16::from_be_bytes([bytes[0], bytes[1]])
            };
            let mut walked = *state;
            for _ in 0..4 {
                let sp = a7_of(&walked);
                match word(sp.wrapping_add(6)) >> 12 {
                    0 | 2 => return None,
                    1 => {
                        //the throwaway frame's status register, and the stack it names
                        let popped = sp.wrapping_add(8);
                        match (walked.sr & 0x2000 != 0, walked.sr & 0x1000 != 0) {
                            (false, _) => walked.usp = popped,
                            (true, false) => walked.ssp = popped,
                            (true, true) => walked.msp = popped,
                        }
                        walked.sr = word(sp) & 0xF71F;
                    }
                    _ => return Some("rte"),
                }
            }
            Some("rte")
        }
        _ => None,
    }
}

/// A 68010 to compare the Interpreter with: Musashi or Moira.
trait Reference {
    /// How a report names it.
    const NAME: &'static str;
    /// Whether it is Musashi, whose gaps [`RELAXATIONS`] lists; Moira has
    /// EASy68K's own words only, and its address errors are compared on their
    /// cycles.
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
    compare_on(reference, per_word, Processor::M68010)
}

/// [`compare`] on `processor`, the 68010 or the 68020, whose words of the
/// 68010's run from [`case_68020`]'s states, the 68020's supervisor model and
/// its exceptions among them, against a reference set to the same processor.
fn compare_on<R: Reference>(
    reference: &mut R,
    per_word: usize,
    processor: Processor,
) -> (BTreeMap<String, Tally>, BTreeMap<&'static str, usize>) {
    let m68020 = processor == Processor::M68020;
    let mut tallies: BTreeMap<String, Tally> = BTreeMap::new();
    let mut skipped: BTreeMap<&'static str, usize> = BTreeMap::new();
    for first in 0..=0xFFFFu16 {
        if !is_legal(first, Processor::M68010) {
            continue;
        }
        let mut random = Random(0x9E37_79B9_7F4A_7C15 ^ (first as u64) << 17 | 1);
        if m68020 {
            random = Random(0x6802_0E5C_A1E5_0001 ^ (first as u64) << 19 | 1);
        }
        for _ in 0..per_word {
            //Moira takes a pending trace exception at its next step, which is no step here
            let case = match m68020 {
                true => case_68020(first, &mut random, R::MUSASHI),
                false => case(first, &mut random, R::MUSASHI),
            };
            let code: Vec<u16> = case.regions[1]
                .1
                .chunks_exact(2)
                .map(|pair| u16::from_be_bytes([pair[0], pair[1]]))
                .collect();
            if let Some(relaxation) = relaxed(first, &code) {
                //a 68020 has the 68020's control registers, and its `rte` is compared by the
                //frames it pops ([`relaxed_on_the_68020`])
                let the_68010s = matches!(relaxation, "movec of a 68020 register" | "rte");
                if !(m68020 && the_68010s)
                    && (R::MUSASHI || ["trap #15", "simhalt"].contains(&relaxation))
                {
                    *skipped.entry(relaxation).or_default() += 1;
                    continue;
                }
            }
            if m68020 && first >= 0xF000 {
                *skipped.entry("line F").or_default() += 1;
                continue;
            }
            if !R::MUSASHI {
                if let Some(relaxation) = relaxed_for_moira(&code, &case.state) {
                    *skipped.entry(relaxation).or_default() += 1;
                    continue;
                }
            }
            let decoded = decode_words(&code, CODE, processor).unwrap();
            let bytes: Vec<u8> = code[..decoded.size as usize / 2]
                .iter()
                .flat_map(|word| word.to_be_bytes())
                .collect();
            let name = mnemonic(&decoded.instruction, &bytes);
            let mut ours = interpreter_for(&case, processor);
            if R::MUSASHI {
                let relaxation = match m68020 {
                    true => relaxed_on_the_68020(&decoded.instruction, &case.state, &ours),
                    false => None,
                };
                let relaxation =
                    relaxation.or_else(|| relaxed_data(&decoded.instruction, &case.state, &ours));
                if let Some(relaxation) = relaxation {
                    *skipped.entry(relaxation).or_default() += 1;
                    continue;
                }
            }
            let _ = ours.step();
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
            let mut flags =
                compared_flags(&decoded.instruction, &state_of(&ours), by_zero, R::MUSASHI);
            if m68020 {
                flags &= compared_flags_68020(&decoded.instruction, &state_of(&ours), &vectors);
            }
            //a trace frame stacks them, T1's or on the 68020 T0's
            if R::MUSASHI && case.state.sr & 0xC000 != 0 && flags != 0xFFFF {
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
            //the bits of CACR that are commands to a cache, which Musashi keeps
            theirs.cacr &= 3;
            let mut written = our_writes;
            written.extend(reference.written());
            written.sort_unstable();
            written.dedup();
            //the long `link a7` pushes, under A7 as it was
            if let (true, Instruction::LINK(RegisterOperand::Address(7), _)) =
                (R::MUSASHI, decoded.instruction)
            {
                let a7 = a7_of(&case.state);
                written.retain(|&at| !(a7.wrapping_sub(4)..a7).contains(&at));
            }
            //and the low byte of the status register a division by zero stacks, and on the 68020
            //`chk` and `chk2` out of bounds
            if flags != 0xFFFF && (by_zero || m68020 && !vectors.is_empty()) {
                written.retain(|&at| at != a7_of(&mine).wrapping_add(1));
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

/// A random Operand in one of the 68020's addressing modes, and the case to
/// run an instruction of it from: the brief extension word with a scale, or
/// the full one, with every base, a base suppressed one time in four, the
/// index one time in four, each displacement null, a word or a long, and
/// every indirection. The data registers are small, so that an index lands
/// near its base, and the data the address registers point into is a table
/// of pointers back into it, so that a memory indirect Operand reads one.
#[cfg(feature = "musashi")]
fn extended_case(random: &mut Random) -> (Operand, Case) {
    use crate::instructions::{
        DisplacementSize, ExtensionWord, IndexBase, IndexRegister, Indirection,
    };
    let mut case = case(0x4E71, random, false);
    for register in case.state.d.iter_mut() {
        *register = (random.below(0x40) as i32 - 0x20) as u32 & !1;
    }
    let mut pointers = Vec::with_capacity(0x1000);
    for _ in 0..0x400 {
        pointers.extend_from_slice(&(DATA + 0x200 + random.below(0x600) & !1).to_be_bytes());
    }
    case.regions[2] = (DATA, pointers);
    let size = |random: &mut Random| match random.below(3) {
        0 => DisplacementSize::Null,
        1 => DisplacementSize::Word,
        _ => DisplacementSize::Long,
    };
    let base = match random.below(8) {
        7 => IndexBase::PC,
        number => IndexBase::address(number as u8),
    };
    let index = IndexRegister {
        register: match random.below(2) {
            0 => RegisterOperand::Data(random.below(8) as u8),
            _ => RegisterOperand::Address(random.below(7) as u8),
        },
        size: match random.below(2) {
            0 => Size::Word,
            _ => Size::Long,
        },
    };
    let scale = 1 << random.below(4);
    //near the base, or for a base that adds nothing, in the data
    let near = |random: &mut Random, size: DisplacementSize, absolute: bool| match size {
        DisplacementSize::Null => 0,
        _ if absolute => (DATA + 0x200 + random.below(0x600)) as i32 & !1,
        _ => (random.below(0x80) as i32 - 0x40) & !1,
    };
    if random.below(4) == 0 {
        let byte = (random.below(0x80) as i32 - 0x40) & !1;
        let scale = 2 << random.below(3);
        let displacement = match base.is_pc() {
            true => (CODE + 2).wrapping_add(byte as u32) as i32,
            false => byte,
        };
        let word = ExtensionWord::brief(index, scale);
        return (
            Operand::Extended {
                base,
                word,
                displacement,
                outer: 0,
            },
            case,
        );
    }
    let base_suppressed = random.below(4) == 0;
    let index_suppressed = random.below(4) == 0;
    let base_size = size(random);
    let indirection = match (random.below(3), index_suppressed) {
        (0, _) => Indirection::None,
        (1, _) | (_, true) => Indirection::Preindexed,
        _ => Indirection::Postindexed,
    };
    let outer_size = match indirection {
        Indirection::None => DisplacementSize::Null,
        _ => size(random),
    };
    let (index, scale) = match index_suppressed {
        true => (
            IndexRegister {
                register: RegisterOperand::Data(0),
                size: Size::Word,
            },
            1,
        ),
        false => (index, scale),
    };
    let word = ExtensionWord::full(
        index,
        scale,
        base_suppressed,
        index_suppressed,
        base_size,
        indirection,
        outer_size,
    );
    let pc_relative = base.is_pc() && !base_suppressed;
    let base_displacement = match pc_relative {
        //the data, from the extension word
        true if base_size != DisplacementSize::Null => {
            (DATA + 0x200 + random.below(0x600) & !1) as i32
        }
        true => (CODE + 2) as i32,
        false => near(random, base_size, base_suppressed),
    };
    let outer = near(random, outer_size, false);
    (
        Operand::Extended {
            base,
            word,
            displacement: base_displacement,
            outer,
        },
        case,
    )
}

/// The 68020's addressing modes against Musashi's 68EC020, the 68020 with
/// s68k's 24-bit bus: random Operands of every mode ([`extended_case`]) in
/// `lea`, a `move` from them and to them, an `add` to them, `clr` and `jmp`,
/// from random states, the registers, the program counter and the bytes
/// written compared, where an exception's frame would be. Musashi models
/// every mode, and reads and writes data at an odd address as the 68020 and
/// s68k's 68020 do, so those cases are compared too. Left out is only the
/// address error of a jump to an odd address, the one exception these
/// cases raise, which s68k's 68020 takes and Musashi's does not, having no
/// address error on a 68020 (the first of [`RELAXATIONS_SUPERVISOR`]); the
/// runs of the supervisor model compare every other exception and its frame.
#[cfg(feature = "musashi")]
#[test]
#[ignore = "needs Musashi: scripts/fetch-musashi.sh, then --features musashi"]
fn the_68020s_addressing_modes_do_what_musashi_does() {
    use crate::machine::encode::encode;
    let mut musashi = Musashi::lock();
    musashi.set_cpu_type(CPU_TYPE_68EC020);
    let mut random = Random(0x6802_0C0D_E5A1_7E57);
    let (mut compared, mut skipped) = (0, 0);
    let mut differences = Vec::new();
    let (d0, d1) = (
        Operand::Register(RegisterOperand::Data(0)),
        Operand::Register(RegisterOperand::Data(1)),
    );
    for _ in 0..40_000 {
        let (operand, mut case) = extended_case(&mut random);
        let pc_relative = matches!(operand, Operand::Extended { base, .. } if base.is_pc());
        let instruction = match (random.below(6), pc_relative) {
            (0, _) => Instruction::LEA(operand, RegisterOperand::Address(1)),
            (1, _) | (2, true) | (3, true) => Instruction::MOVE(operand, d0, Size::Long),
            (2, false) => Instruction::MOVE(d1, operand, Size::Word),
            (3, false) => Instruction::ADD(d1, operand, Size::Long),
            (4, false) => Instruction::CLR(operand, Size::Byte),
            _ => Instruction::JMP(operand),
        };
        let words = encode(&instruction, CODE).expect("the operand encodes");
        case.regions[1] = (
            CODE,
            words.iter().flat_map(|word| word.to_be_bytes()).collect(),
        );
        let mut ours = interpreter_for(&case, Processor::M68020);
        let stepped = ours.step();
        let (our_writes, vectors) = interpreter_writes(&ours);
        let odd_jump = matches!(instruction, Instruction::JMP(_))
            && vectors.contains(&crate::machine::exception::ADDRESS_ERROR);
        if stepped.is_err() || odd_jump {
            skipped += 1;
            continue;
        }
        musashi.load(&case.state, &case.regions);
        musashi.step();
        compared += 1;
        let (mine, theirs) = (state_of(&ours), musashi.state());
        let mut written: Vec<u32> = our_writes.iter().map(|at| at & 0xFF_FFFF).collect();
        written.extend(musashi.written());
        written.sort_unstable();
        written.dedup();
        let memory = ours.get_memory();
        let differing: Vec<String> = written
            .iter()
            .filter_map(|&at| {
                let our = memory.read_bytes(at as usize, 1).unwrap()[0];
                let their = musashi.read(at, 1)[0];
                (our != their).then(|| format!("${at:06X}: {our:02X} vs {their:02X}"))
            })
            .collect();
        if (mine != theirs || !differing.is_empty()) && differences.len() < 20 {
            differences.push(format!(
                "{} ({words:04X?}):\n    s68k    {mine:X?}\n    Musashi {theirs:X?}\n    memory \
                 {differing:?}",
                crate::machine::disassemble::disassemble(&instruction, None),
            ));
        }
    }
    println!(
        "{compared} cases of the 68020's addressing modes compared, {skipped} jumps to an odd \
         address left out"
    );
    assert!(compared > 20_000, "only {compared} compared");
    assert!(
        differences.is_empty(),
        "{} cases differ:\n{}",
        differences.len(),
        differences.join("\n")
    );
}

/// Where Musashi's 68EC020 is not the 68020, or not s68k's, for the 68020's
/// own instructions, which are not compared there: the 68020 harness's skip
/// list.
#[cfg(feature = "musashi")]
const RELAXATIONS_68020: &[Relaxation] = &[
    Relaxation {
        name: "undefined flags",
        reason: "the flags Motorola leaves undefined, which s68k sets as the 68020 does, from \
                 WinUAE's cputester runs on the hardware, and Musashi does not: N after `chk` \
                 and `chk.l` in bounds, which the 68020 clears and Musashi leaves, and the four \
                 of them out of bounds, where the 68020 sets V and C; N and Z after a long \
                 division that overflows, with C, which Musashi leaves where the manual clears \
                 it, and the four after one by zero; and N and V after `chk2` and `cmp2`",
    },
    Relaxation {
        name: "pack and unpk in memory",
        reason: "Musashi reads `pack`'s two bytes, and writes `unpk`'s, the low one at the \
                 lower address, and moves A7 by four for `pack`; the 68020 reads and writes a \
                 word, the high byte first, as the manual draws it and WinUAE has it from \
                 cputester, and moves the source register by two",
    },
    Relaxation {
        name: "cas2 into one compare register",
        reason: "when both comparisons go into one register, the manual says it keeps the \
                 first operand, which s68k and WinUAE's 68020 write last; Musashi keeps the \
                 second, as the 68040 does",
    },
    Relaxation {
        name: "cas2 of one operand twice",
        reason: "the manual leaves undefined what `cas2` writes to operands that overlap; s68k \
                 writes the second first, as the 68020 does by WinUAE, Musashi the first",
    },
    Relaxation {
        name: "cas2.w through an address register",
        reason: "Musashi sign extends a word compared through an address register into the \
                 whole compare register, where the manual and WinUAE write its low word; the \
                 high words of the compare registers are not compared",
    },
    Relaxation {
        name: "a bit field at an offset of nearly -2^31",
        reason: "Musashi works out the byte of a field in memory as `-((7 - offset) / 8)` for a \
                 negative offset, which overflows for an offset from a register within eight of \
                 -2^31 and lands a byte too low; such a case is not compared",
    },
    Relaxation {
        name: "bfffo of a data register",
        reason: "`bfffo` writes the offset as written plus the distance into the field, a \
                 register's offset whole, as the manual says and WinUAE has it; Musashi takes a \
                 data register's offset modulo 32 first",
    },
    Relaxation {
        name: "chk2 of an address register below a long",
        reason: "the manual compares the whole address register with the bounds sign \
                 extended; Musashi compares its low byte or word",
    },
    Relaxation {
        name: "chk2 of bounds not read",
        reason: "the bounds decide whether Musashi's signed-only comparison is the 68020's, \
                 so they are read first, through `(An)`, `(d16,An)` and an absolute address; \
                 a case of another mode is not compared, nor one whose lower bound is above \
                 its upper one as numbers, which the 68020 reads as the unsigned range that \
                 wraps round and Musashi as signed, nor a byte bound relative to the program \
                 counter, which Musashi reads unsigned",
    },
    Relaxation {
        name: "muls.l into Dh:Dh",
        reason: "a 64-bit product into one register for both halves is undefined; the 68020 \
                 writes the low half first and leaves the high one, as WinUAE has it, and \
                 Musashi writes them the other way round, as the 68040 does",
    },
    Relaxation {
        name: "divs.l $80000000 by -1",
        reason: "Musashi gives a 32-bit `divs.l` of $80000000 by -1 a quotient of $80000000 \
                 with V clear; it overflows, as the quotient is no 32-bit signed number",
    },
    Relaxation {
        name: "link.l a7",
        reason: "`link.l a7,#d` pushes A7 as it was before it moves, as `link a7` does on the \
                 68000 and the 68010; Musashi pushes it four lower. The long pushed is not \
                 compared",
    },
];

/// The flags of the status register a 68020 instruction leaves undefined and
/// s68k and Musashi set differently ([`RELAXATIONS_68020`]), as a mask of
/// what is compared, given the state s68k left and the vectors it took.
fn compared_flags_68020(instruction: &Instruction, after: &State, vectors: &[u8]) -> u16 {
    const N: u16 = 8;
    const Z: u16 = 4;
    const V: u16 = 2;
    const C: u16 = 1;
    match instruction {
        Instruction::CHK(..) | Instruction::CHKL(..) if vectors.contains(&6) => !(N | Z | V | C),
        Instruction::CHK(..) | Instruction::CHKL(..) => !N,
        Instruction::DIVL { .. } if vectors.contains(&5) => !(N | Z | V | C),
        Instruction::DIVL { .. } if after.sr & V != 0 => !(N | Z | C),
        Instruction::CHK2(..) | Instruction::CMP2(..) => !(N | V),
        _ => 0xFFFF,
    }
}

/// Which relaxation keeps a case of the 68020's `instruction` from being
/// run, by the data it starts from, if one does.
#[cfg(feature = "musashi")]
fn relaxed_68020(
    instruction: &Instruction,
    state: &State,
    fresh: &Interpreter,
) -> Option<&'static str> {
    use crate::instructions::{AddressWidth, Operand, Sign};
    let a = |number: u8| match number {
        7 => a7_of(state),
        _ => state.a[number as usize],
    };
    let register = |register: RegisterOperand| match register {
        RegisterOperand::Data(number) => state.d[number as usize],
        RegisterOperand::Address(number) => a(number),
    };
    //a bound at `address`, sign extended from `size`
    let bound = |address: u32, size: Size| {
        let bytes = fresh
            .get_memory()
            .read_bytes(address as usize & 0xFF_FFFF, size.to_bytes())
            .unwrap();
        let value = bytes
            .iter()
            .fold(0u32, |value, byte| value << 8 | *byte as u32);
        crate::math::sign_extend_to_long(value, size)
    };
    match *instruction {
        Instruction::BF {
            operation,
            target,
            field:
                crate::instructions::BitField {
                    offset: crate::instructions::FieldValue::Register(number),
                    ..
                },
            ..
        } => {
            let offset = state.d[number as usize];
            match target {
                Operand::Register(_) => (operation
                    == crate::instructions::BitFieldOperation::FindFirstOne
                    && offset > 31)
                    .then_some("bfffo of a data register"),
                _ => ((offset as i32) < i32::MIN + 8)
                    .then_some("a bit field at an offset of nearly -2^31"),
            }
        }
        Instruction::PACK(Operand::PreIndirect(_), ..)
        | Instruction::UNPK(Operand::PreIndirect(_), ..) => Some("pack and unpk in memory"),
        Instruction::CAS2 { compare, .. } if compare.0 == compare.1 => {
            Some("cas2 into one compare register")
        }
        Instruction::CAS2 { pointers, .. } if register(pointers.0) == register(pointers.1) => {
            Some("cas2 of one operand twice")
        }
        Instruction::CHK2(_, RegisterOperand::Address(_), size)
        | Instruction::CMP2(_, RegisterOperand::Address(_), size)
            if size != Size::Long =>
        {
            Some("chk2 of an address register below a long")
        }
        Instruction::CHK2(bounds, _, size) | Instruction::CMP2(bounds, _, size) => {
            let address = match bounds {
                Operand::Indirect(number) => a(number),
                Operand::IndirectDisplacement {
                    offset,
                    base: RegisterOperand::Address(number),
                } => a(number).wrapping_add(offset as u32),
                Operand::Absolute {
                    address,
                    width: AddressWidth::Word | AddressWidth::Long,
                } => address as u32,
                _ => return Some("chk2 of bounds not read"),
            };
            let lower = bound(address, size);
            let upper = bound(address.wrapping_add(size.to_bytes() as u32), size);
            (lower > upper).then_some("chk2 of bounds not read")
        }
        Instruction::MULL {
            low,
            high: Some(high),
            ..
        } if low == high => Some("muls.l into Dh:Dh"),
        Instruction::DIVL {
            source,
            quotient,
            wide: false,
            sign: Sign::Signed,
            ..
        } if state.d[quotient as usize] == 0x8000_0000
            && match source {
                Operand::Register(RegisterOperand::Data(number)) => {
                    state.d[number as usize] == 0xFFFF_FFFF
                }
                Operand::Immediate(value) => value == 0xFFFF_FFFF,
                _ => false,
            } =>
        {
            Some("divs.l $80000000 by -1")
        }
        _ => None,
    }
}

/// The 68020's own instructions against Musashi's 68EC020: every first word
/// the decoder takes as an instruction of the 68020's and not of the 68010's,
/// `per_word` random cases each (`S68K_DIFFERENTIAL_CASES`, 64 by default),
/// one instruction from random states of the 68020's supervisor model
/// ([`case_68020`]), the registers, the three stack pointers, the status
/// register, the program counter, CACR and CAAR and the bytes written, the
/// frames of the exceptions among them, compared. The cycles are reported by
/// [`the_68020s_cycles_are_reported`].
#[cfg(feature = "musashi")]
#[test]
#[ignore = "needs Musashi: scripts/fetch-musashi.sh, then --features musashi"]
fn the_68020s_instructions_do_what_musashi_does() {
    let per_word = std::env::var("S68K_DIFFERENTIAL_CASES")
        .ok()
        .and_then(|cases| cases.parse().ok())
        .unwrap_or(64);
    let mut musashi = Musashi::lock();
    musashi.set_cpu_type(CPU_TYPE_68EC020);
    let (tallies, skipped) = compare_68020s_own(&mut musashi, per_word);
    let run: usize = tallies.values().map(|tally| tally.run).sum();
    let mut report = String::new();
    let mut differing = 0;
    for (name, tally) in &tallies {
        println!("{name}: {} cases", tally.run);
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
    println!("{run} cases of the 68020's instructions run, {skipped:?} skipped");
    for relaxation in RELAXATIONS_68020.iter().chain(RELAXATIONS_SUPERVISOR) {
        println!("relaxed, {}: {}", relaxation.name, relaxation.reason);
    }
    assert!(
        differing == 0,
        "{differing} of {run} cases differ:\n{report}"
    );
}

/// Every first word of the 68020's own instructions, `per_word` cases each,
/// against Musashi's 68EC020, set to it already: the tallies by mnemonic,
/// with their state differences and their cycles, and the skipped cases by
/// relaxation ([`the_68020s_instructions_do_what_musashi_does`]).
#[cfg(feature = "musashi")]
fn compare_68020s_own(
    musashi: &mut Musashi,
    per_word: usize,
) -> (BTreeMap<String, Tally>, BTreeMap<&'static str, usize>) {
    let mut tallies: BTreeMap<String, Tally> = BTreeMap::new();
    let mut skipped: BTreeMap<&'static str, usize> = BTreeMap::new();
    for first in 0..=0xFFFFu16 {
        if !is_legal(first, Processor::M68020) || is_legal(first, Processor::M68010) {
            continue;
        }
        let mut random = Random(0x6802_0000_0000_0001 ^ (first as u64) << 23 | 1);
        for _ in 0..per_word {
            let case = case_68020(first, &mut random, true);
            let code: Vec<u16> = case.regions[1]
                .1
                .chunks_exact(2)
                .map(|pair| u16::from_be_bytes([pair[0], pair[1]]))
                .collect();
            let decoded = decode_words(&code, CODE, Processor::M68020).unwrap();
            let bytes: Vec<u8> = code[..decoded.size as usize / 2]
                .iter()
                .flat_map(|word| word.to_be_bytes())
                .collect();
            let mut ours = interpreter_for(&case, Processor::M68020);
            if let Some(relaxation) = relaxed_68020(&decoded.instruction, &case.state, &ours)
                .or_else(|| relaxed_on_the_68020(&decoded.instruction, &case.state, &ours))
            {
                *skipped.entry(relaxation).or_default() += 1;
                continue;
            }
            let name = mnemonic(&decoded.instruction, &bytes);
            let stepped = ours.step();
            let (our_writes, vectors) = interpreter_writes(&ours);
            let halted = matches!(ours.get_termination(), Some(Termination::Halted { .. }));
            if vectors.contains(&3) || halted {
                *skipped.entry("address error").or_default() += 1;
                continue;
            }
            if let Err(error) = stepped {
                panic!("{error:?} from {:04X?}", &code[..decoded.size as usize / 2]);
            }
            if case.state.sr & 0x8000 != 0 && vectors.len() > 1 {
                *skipped.entry("trace after an exception").or_default() += 1;
                continue;
            }
            musashi.load(&case.state, &case.regions);
            let their_cycles = musashi.step();
            let tally = tallies.entry(name.clone()).or_default();
            tally.run += 1;
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
            let flags = compared_flags_68020(&decoded.instruction, &state_of(&ours), &vectors);
            //a trace frame stacks them, T1's or T0's
            if case.state.sr & 0xC000 != 0 && flags != 0xFFFF {
                *skipped.entry("undefined flags").or_default() += 1;
                continue;
            }
            let (mut mine, mut theirs) = (state_of(&ours), musashi.state());
            mine.sr &= flags;
            theirs.sr &= flags;
            theirs.cacr &= 3;
            //the high words of the compare registers of a `cas2.w` through an address register
            if let Instruction::CAS2 {
                compare,
                pointers,
                size: Size::Word,
                ..
            } = decoded.instruction
            {
                for (compare, pointer) in [(compare.0, pointers.0), (compare.1, pointers.1)] {
                    if let RegisterOperand::Address(_) = pointer {
                        let number = compare as usize;
                        theirs.d[number] = theirs.d[number] & 0xFFFF | mine.d[number] & 0xFFFF_0000;
                    }
                }
            }
            let mut written: Vec<u32> = our_writes.iter().map(|at| at & 0xFF_FFFF).collect();
            written.extend(musashi.written());
            written.sort_unstable();
            written.dedup();
            //the long `link.l a7` pushes, under A7 as it was
            if let Instruction::LINKL(RegisterOperand::Address(7), _) = decoded.instruction {
                let a7 = a7_of(&case.state);
                written.retain(|&at| !(a7.wrapping_sub(4)..a7).contains(&at));
            }
            //and the low byte of the status register `chk`, `chk.l` and `chk2` stack
            if flags != 0xFFFF && !vectors.is_empty() {
                written.retain(|&at| at != a7_of(&mine).wrapping_add(1));
            }
            let memory = ours.get_memory();
            let differing: Vec<String> = written
                .iter()
                .filter_map(|&at| {
                    let our = memory.read_bytes(at as usize, 1).unwrap()[0];
                    let their = musashi.read(at, 1)[0];
                    (our != their).then(|| format!("${at:06X}: {our:02X} vs {their:02X}"))
                })
                .collect();
            if mine != theirs || !differing.is_empty() {
                if tally.differences.len() < 4 {
                    tally.differences.push(format!(
                        "{} ({:04X?}):\n    s68k    {mine:X?}\n    Musashi {theirs:X?}\n    \
                         memory {differing:?}",
                        disassemble_words(&decoded.instruction, &bytes, None),
                        &code[..decoded.size as usize / 2],
                    ));
                } else {
                    tally.differences.push(String::new());
                }
            }
        }
    }
    (tallies, skipped)
}

/// The row of the M68020 User's Manual's timing tables that an Operand of the
/// 68020's own modes is on, for a report: `(d8,An,Xn)` for the brief
/// extension word with a scale, and `(B)`, `([d16,B],I,d32)` and their kin
/// for the full one.
#[cfg(feature = "musashi")]
fn manual_row(operand: &Operand) -> String {
    use crate::instructions::{DisplacementSize, Indirection};
    let Operand::Extended { word, .. } = operand else {
        return "a 68000 mode".to_string();
    };
    if !word.is_full() {
        return "(d8,An,Xn)".to_string();
    }
    let base = match word.base_displacement_size() {
        DisplacementSize::Null => "",
        DisplacementSize::Word => "d16,",
        DisplacementSize::Long => "d32,",
    };
    let outer = match word.outer_displacement_size() {
        DisplacementSize::Null => "",
        DisplacementSize::Word => ",d16",
        DisplacementSize::Long => ",d32",
    };
    match word.indirection() {
        Indirection::None => format!("({base}B)"),
        _ => format!("([{base}B],I{outer})"),
    }
}

/// The cycles of the 68020's estimate, the manual's cache case, against
/// Musashi's 68EC020, whose table is the cache case too (the asm-editor's
/// `docs/research/m68k-cpu-variants.md`, 3.3), by instruction: the 68000's
/// and the 68010's instructions on the 68020, two cases of each first word,
/// the 68020's own, sixteen of each, and its addressing modes in `lea`, a
/// `move` from them and to them, an `add` to them, `clr` and `jmp`, by the
/// manual's row. They are reported and not asserted: Musashi keeps one
/// number per opcode and departs from the manual in places, and both are an
/// estimate.
#[cfg(feature = "musashi")]
#[test]
#[ignore = "needs Musashi: scripts/fetch-musashi.sh, then --features musashi"]
fn the_68020s_cycles_are_reported() {
    use crate::machine::encode::encode;
    let mut musashi = Musashi::lock();
    musashi.set_cpu_type(CPU_TYPE_68EC020);
    let mut total = 0;
    let mut differing = 0;
    let mut report = |tallies: &BTreeMap<String, Tally>| {
        total += tallies.values().map(|tally| tally.run).sum::<usize>();
        differing += report_cycles::<Musashi>(tallies);
    };
    println!("The 68000's and the 68010's instructions on the 68020:");
    report(&compare_on(&mut musashi, 2, Processor::M68020).0);
    println!("The 68020's own instructions:");
    report(&compare_68020s_own(&mut musashi, 16).0);
    println!("The 68020's addressing modes:");
    let mut tallies: BTreeMap<String, Tally> = BTreeMap::new();
    let mut random = Random(0x6802_0C1C_1E5E_57A7);
    let (d0, d1) = (
        Operand::Register(RegisterOperand::Data(0)),
        Operand::Register(RegisterOperand::Data(1)),
    );
    for _ in 0..20_000 {
        let (operand, mut case) = extended_case(&mut random);
        let pc_relative = matches!(operand, Operand::Extended { base, .. } if base.is_pc());
        //named by kind, as the disassembler prints an extension word no assembler writes as words
        let (instruction, kind) = match (random.below(6), pc_relative) {
            (0, _) => (
                Instruction::LEA(operand, RegisterOperand::Address(1)),
                "lea",
            ),
            (1, _) | (2, true) | (3, true) => {
                (Instruction::MOVE(operand, d0, Size::Long), "move.l from")
            }
            (2, false) => (Instruction::MOVE(d1, operand, Size::Word), "move.w to"),
            (3, false) => (Instruction::ADD(d1, operand, Size::Long), "add.l to"),
            (4, false) => (Instruction::CLR(operand, Size::Byte), "clr.b"),
            _ => (Instruction::JMP(operand), "jmp"),
        };
        let words = encode(&instruction, CODE).expect("the operand encodes");
        let bytes: Vec<u8> = words.iter().flat_map(|word| word.to_be_bytes()).collect();
        case.regions[1] = (CODE, bytes.clone());
        let mut ours = interpreter_for(&case, Processor::M68020);
        let stepped = ours.step();
        if stepped.is_err() || !interpreter_writes(&ours).1.is_empty() {
            continue;
        }
        musashi.load(&case.state, &case.regions);
        let their_cycles = musashi.step();
        let name = format!("{kind} {}", manual_row(&operand));
        let tally = tallies.entry(name).or_default();
        tally.run += 1;
        let our_cycles = ours.get_cycles() as u32;
        if our_cycles == their_cycles {
            tally.cycles_agree += 1;
        } else {
            let (count, examples) = tally.cycles.entry((our_cycles, their_cycles)).or_default();
            *count += 1;
            if examples.len() < 3 {
                examples.push(disassemble_words(&instruction, &bytes, None));
            }
        }
    }
    report(&tallies);
    println!(
        "the 68020's cycles: {} of {total} cases agree with Musashi",
        total - differing
    );
}

/// The 68000's and the 68010's instructions on the 68020 against Musashi's
/// 68EC020: every first word the decoder takes as a 68010 instruction,
/// `per_word` cases each (`S68K_DIFFERENTIAL_CASES`, 2 by default), from
/// random states of the 68020's supervisor model ([`case_68020`]), T1, T0, S
/// and M among them, so that its exceptions, its frames, its stack pointers,
/// its tracing and its unaligned data are compared, outside the 68010's
/// relaxations and [`RELAXATIONS_SUPERVISOR`].
#[cfg(feature = "musashi")]
#[test]
#[ignore = "needs Musashi: scripts/fetch-musashi.sh, then --features musashi"]
fn the_68020s_supervisor_model_does_what_musashi_does() {
    let per_word = std::env::var("S68K_DIFFERENTIAL_CASES")
        .ok()
        .and_then(|cases| cases.parse().ok())
        .unwrap_or(2);
    let mut musashi = Musashi::lock();
    musashi.set_cpu_type(CPU_TYPE_68EC020);
    let (tallies, skipped) = compare_on(&mut musashi, per_word, Processor::M68020);
    for relaxation in RELAXATIONS_SUPERVISOR {
        println!(
            "relaxed on the 68020, {}: {}",
            relaxation.name, relaxation.reason
        );
    }
    assert_no_differences(&tallies, &skipped, true);
}

/// The 68020's interrupts against Musashi's 68EC020: a request at a random
/// level above the mask, or at 7, from random states of the 68020's
/// supervisor model, so that one taken with M set pushes its frame on the
/// master stack and the throwaway frame on the interrupt stack, and the
/// handler's first instruction run after it; the registers, the stack
/// pointers and the bytes written compared.
#[cfg(feature = "musashi")]
#[test]
#[ignore = "needs Musashi: scripts/fetch-musashi.sh, then --features musashi"]
fn the_68020s_interrupts_do_what_musashi_does() {
    let mut musashi = Musashi::lock();
    musashi.set_cpu_type(CPU_TYPE_68EC020);
    let mut random = Random(0x6802_01A7_E5C1_0001);
    let (mut compared, mut throwaway) = (0, 0);
    let mut differences = Vec::new();
    for _ in 0..20_000 {
        let mut case = case_68020(0x4E71, &mut random, true);
        let mask = (case.state.sr >> 8) & 7;
        let level = match mask {
            7 => 7,
            _ => (mask + 1 + random.below(7 - mask as u32) as u16) as u8,
        };
        //the instruction interrupted runs after the handler's `rte`
        case.regions[1].1.truncate(2);
        let mut ours = interpreter_for(&case, Processor::M68020);
        ours.raise_irq(level).unwrap();
        ours.step().unwrap();
        ours.step().unwrap();
        musashi.load(&case.state, &case.regions);
        musashi.set_irq(level);
        musashi.step();
        musashi.set_irq(0);
        compared += 1;
        throwaway += (case.state.sr & 0x1000 != 0) as usize;
        let (mine, mut theirs) = (state_of(&ours), musashi.state());
        theirs.cacr &= 3;
        let (our_writes, _) = interpreter_writes(&ours);
        let mut written: Vec<u32> = our_writes.iter().map(|at| at & 0xFF_FFFF).collect();
        let mut irq = ours.get_last_steps(2);
        irq.reverse();
        for step in irq {
            for mutation in step.get_mutations() {
                if let MutationOperation::WriteMemory { address, size, .. } = mutation {
                    written.extend((0..size.to_bytes() as u32).map(|byte| *address as u32 + byte));
                }
            }
        }
        written.extend(musashi.written());
        written.sort_unstable();
        written.dedup();
        let memory = ours.get_memory();
        let differing: Vec<String> = written
            .iter()
            .filter_map(|&at| {
                let our = memory.read_bytes(at as usize, 1).unwrap()[0];
                let their = musashi.read(at, 1)[0];
                (our != their).then(|| format!("${at:06X}: {our:02X} vs {their:02X}"))
            })
            .collect();
        if (mine != theirs || !differing.is_empty()) && differences.len() < 20 {
            differences.push(format!(
                "IRQ {level} from {:X?}:\n    s68k    {mine:X?}\n    Musashi {theirs:X?}\n    \
                 memory {differing:?}",
                case.state
            ));
        }
    }
    println!("{compared} interrupts compared, {throwaway} in the master state");
    assert!(
        differences.is_empty(),
        "{} cases differ:\n{}",
        differences.len(),
        differences.join("\n")
    );
}

/// Writes `words` at `address` into the region of `case` that holds it.
#[cfg(feature = "musashi")]
fn put_words(case: &mut Case, address: u32, words: &[u16]) {
    let (start, bytes) = case
        .regions
        .iter_mut()
        .find(|(start, bytes)| (*start..*start + bytes.len() as u32).contains(&address))
        .expect("a region holds it");
    let at = (address - *start) as usize;
    for (index, word) in words.iter().enumerate() {
        bytes[at + 2 * index..at + 2 * index + 2].copy_from_slice(&word.to_be_bytes());
    }
}

/// The 68020's `rte` against Musashi's 68EC020, of the frames both pop: a
/// format `$0` or `$2` frame on the stack A7 is, or a throwaway frame, format
/// `$1`, whose status register names a stack, the user's, the interrupt's or
/// the master's, that holds one of the two, from random states of the
/// 68020's supervisor model in supervisor mode; the registers, the three
/// stack pointers and the status register compared. The random cases of the
/// 68020's supervisor model rarely find such a frame.
#[cfg(feature = "musashi")]
#[test]
#[ignore = "needs Musashi: scripts/fetch-musashi.sh, then --features musashi"]
fn the_68020s_rte_does_what_musashi_does() {
    let mut musashi = Musashi::lock();
    musashi.set_cpu_type(CPU_TYPE_68EC020);
    let mut random = Random(0x6802_07E0_F0A3_0001);
    let (mut compared, mut skipped, mut throwaway) = (0, 0, 0);
    let mut differences = Vec::new();
    for _ in 0..20_000 {
        let mut case = case_68020(0x4E73, &mut random, true);
        case.state.sr |= 0x2000;
        //a frame of format `format` at `at`: the status register, the program counter, which
        //is even, the format word, and for `$2` an instruction address
        let frame = |random: &mut Random, format: u16, sr: u16| {
            let pc = CODE + random.below(0x100) * 2;
            let address = random.long();
            let mut words = vec![sr, (pc >> 16) as u16, pc as u16, format << 12 | 4 * 5];
            if format == 2 {
                words.extend([(address >> 16) as u16, address as u16]);
            }
            words
        };
        let format = [0u16, 2, 1][random.below(3) as usize];
        let sr = random.word() & 0xF71F;
        let top = a7_of(&case.state);
        if format == 1 {
            //the throwaway frame names a stack, on which the frame `rte` returns from is
            let named = sr | 0x2000;
            let words = frame(&mut random, 1, named);
            put_words(&mut case, top, &words);
            let mut after = case.state;
            match after.sr & 0x1000 != 0 {
                true => after.msp = top + 8,
                false => after.ssp = top + 8,
            }
            after.sr = named;
            let inner = [0u16, 2][random.below(2) as usize];
            let inner_sr = random.word() & 0xF71F;
            let words = frame(&mut random, inner, inner_sr);
            put_words(&mut case, a7_of(&after), &words);
            throwaway += 1;
        } else {
            let words = frame(&mut random, format, sr);
            put_words(&mut case, top, &words);
        }
        let mut ours = interpreter_for(&case, Processor::M68020);
        let _ = ours.step();
        let (our_writes, vectors) = interpreter_writes(&ours);
        //an odd stack pointer of the throwaway frame's stack, or a trace frame Musashi forgets
        if !vectors.is_empty() && vectors != [9] {
            skipped += 1;
            continue;
        }
        musashi.load(&case.state, &case.regions);
        musashi.step();
        compared += 1;
        let (mine, mut theirs) = (state_of(&ours), musashi.state());
        theirs.cacr &= 3;
        let mut written: Vec<u32> = our_writes.iter().map(|at| at & 0xFF_FFFF).collect();
        written.extend(musashi.written());
        written.sort_unstable();
        written.dedup();
        let memory = ours.get_memory();
        let differing: Vec<String> = written
            .iter()
            .filter_map(|&at| {
                let our = memory.read_bytes(at as usize, 1).unwrap()[0];
                let their = musashi.read(at, 1)[0];
                (our != their).then(|| format!("${at:06X}: {our:02X} vs {their:02X}"))
            })
            .collect();
        if (mine != theirs || !differing.is_empty()) && differences.len() < 20 {
            differences.push(format!(
                "rte of format ${format:X} from {:X?}:\n    s68k    {mine:X?}\n    Musashi \
                 {theirs:X?}\n    memory {differing:?}",
                case.state
            ));
        }
    }
    println!("{compared} rte compared, {throwaway} through a throwaway frame, {skipped} left out");
    assert!(compared > 15_000, "only {compared} compared");
    assert!(
        differences.is_empty(),
        "{} cases differ:\n{}",
        differences.len(),
        differences.join("\n")
    );
}

/// The decoder and Musashi agree on which first words are 68020
/// instructions, but for the groups where Musashi is not s68k's 68EC020:
/// `illegal`, the coprocessor's line F words, which s68k takes as line F as a
/// 68020 with no coprocessor does, and the 68040's `move16`; `callm` and
/// `rtm`, which s68k leaves out.
#[cfg(feature = "musashi")]
#[test]
#[ignore = "needs Musashi: scripts/fetch-musashi.sh, then --features musashi"]
fn the_legal_first_words_are_musashis_68020_words() {
    let musashi = Musashi::lock();
    let mut exceptions: BTreeMap<&str, usize> = BTreeMap::new();
    let mut unexplained = Vec::new();
    for word in 0..=0xFFFFu16 {
        let ours = is_legal(word, Processor::M68020);
        let theirs = musashi.is_valid(word, CPU_TYPE_68EC020);
        if ours == theirs {
            continue;
        }
        let group = match word {
            0x4AFC => "illegal",
            0x06C0..=0x06CF if !ours => "rtm",
            0x06D0..=0x06FF if !ours => "callm",
            0xF000..=0xFFFF => "line F",
            _ => {
                unexplained.push(format!(
                    "{word:04X}: s68k {ours}, Musashi {theirs} ({})",
                    disassemble_words(
                        &decode_words(
                            &[word, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
                            CODE,
                            Processor::M68020
                        )
                        .unwrap()
                        .instruction,
                        &word.to_be_bytes(),
                        None
                    )
                ));
                continue;
            }
        };
        *exceptions.entry(group).or_default() += 1;
    }
    assert!(
        unexplained.is_empty(),
        "{} words differ:\n{}",
        unexplained.len(),
        unexplained[..unexplained.len().min(60)].join("\n")
    );
    assert_eq!(
        exceptions,
        BTreeMap::from([("callm", 28), ("illegal", 1), ("line F", 2305), ("rtm", 16)])
    );
}
