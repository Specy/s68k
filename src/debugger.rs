//! The Debugger: what the Interpreter records while it runs.
//!
//! One [`ExecutionStep`] per instruction — the mutations it made, the program
//! counter and the condition codes before it — which is what undo replays
//! backwards, and the call stack of [`CallStackFrame`]s that `jsr` and `bsr`
//! push. A step is also what one Poke becomes: the values the host wrote
//! between two instructions, journaled through a [`PokeJournal`] and recorded
//! as a step of [`kind`](ExecutionStepKind) `poke` with its own id. Every step and every frame carries the
//! source [`Location`](crate::assembler::source) of the line it came from,
//! which is what the editor highlights.
//!
//! The `StackFrame` TypeScript declaration lives in `src/ts_types.rs` with the
//! other hand-written declarations. Items older than the assembler rewrite are
//! not all documented yet.

use std::collections::{BTreeMap, HashMap};

use serde::Serialize;
use wasm_bindgen::prelude::wasm_bindgen;

use crate::{
    assembler::{program::ProgramSymbol, source::Location, symbols::SymbolKind},
    instructions::{InputSettings, RegisterOperand, Size},
    interpreter::{CodeWrite, Flags, InterpreterStatus, StackPointers, StackTops},
    machine::{decode::MAX_68000_SIZE, interrupts::MachineState},
};

/// One thing a step changed, with the value it replaced beside the value it
/// wrote.
///
/// Every write carries both sides, read where the write happened and never
/// reconstructed afterwards: `old` is what the Core found there and `new` is
/// what the store left behind. A register write reports the whole register on
/// both sides, because a sized store changes only part of one and the register
/// is what the panels draw; a memory write reports the value at the width it
/// was made. `PushCall`, `PopCall`, `PushException` and `PopException` write
/// no value and carry neither: they are the call stack's. `SetInputSettings` is
/// tasks 12 and 16, which change how input is shown and nothing a panel draws,
/// and `SetMachineState` the IRQs' state.
///
/// The status register is not a mutation: a step carries it whole, before and
/// after, and undo writes it back first, as it was, without moving a stack
/// pointer ([`Cpu::restore_sr`](crate::interpreter::Cpu::restore_sr)). What a
/// change of S does to A7 is journaled as `WriteStackPointers`.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", content = "value")]
pub enum MutationOperation {

    WriteRegister {
        register: RegisterOperand,
        /// The whole register before the store.
        old: u32,
        /// The whole register after it, sized store or not.
        new: u32,
        size: Size,
    },
    WriteMemory {
        address: usize,
        /// The value the write replaced, at `size`.
        old: u32,
        /// The value it stored, at `size`.
        new: u32,
        size: Size,
    },
    WriteMemoryBytes {
        address: usize,
        /// The bytes the write replaced.
        old: Vec<u8>,
        /// The bytes it stored.
        new: Vec<u8>,
    },
    PushCall {
        to: usize,
        from: usize,
    },
    PopCall {
        to: usize,
        from: usize,
        /// The frame the return took off the call stack, which undoing it puts
        /// back as it was: the callee, the return address and the registers at
        /// the call, none of which can be read again from the program.
        #[serde(skip)]
        frame: Option<CallStackFrame>,
    },
    /// The [`InputSettings`] tasks 12 and 16 replaced, and the ones they left.
    SetInputSettings {
        old: InputSettings,
        new: InputSettings,
    },
    /// The two stack pointers before and after a change of one of them that
    /// is not a write of A7, or of S: `move An,usp`, a Poke of the USP or the
    /// SSP, and every write of the status register that changes S, which
    /// makes A7 the other stack pointer. The values are unchanged by such a
    /// swap; what moved is which of them A7 is, which the step's status
    /// register says. Undo puts both back, A7 being the one of the mode
    /// undo has restored.
    WriteStackPointers {
        old: StackPointers,
        new: StackPointers,
    },
    /// An Exception was taken: the processor pushed a frame and jumped
    /// through `vector` to the handler at `handler`, from the instruction at
    /// `from`. The call stack gains a frame for it.
    PushException {
        vector: u8,
        from: usize,
        handler: usize,
    },
    /// `rte` returned from an Exception, from the `rte` at `from` to `to`.
    PopException {
        to: usize,
        from: usize,
        /// The exception frame `rte` took off the call stack, if the frame on
        /// top was one, which undoing it puts back as it was.
        #[serde(skip)]
        frame: Option<CallStackFrame>,
    },
    /// The [`MachineState`] before and after the step changed it: an IRQ
    /// taken, `stop`, and tasks 32, 60 and 62 setting up the interrupt
    /// requests. Undo puts back what the program set up and whether it was
    /// stopped as `old` has them, and makes pending again the request the
    /// step took, keeping the requests that came after it
    /// ([`MachineState::undone`]). Boxed, as it is rare and the two states
    /// would make every mutation larger.
    SetMachineState {
        old: Box<MachineState>,
        new: Box<MachineState>,
    },
    /// A control register of the 68010 that is no stack pointer, VBR, SFC or
    /// DFC, before and after `movec` or a Poke wrote it. Undo puts `old` back.
    WriteControlRegister {
        register: SystemRegister,
        old: u32,
        new: u32,
    },
    /// The 68010's loop mode entered or left: the address of the `DBcc`
    /// whose loop runs in it, before and after. It changes cycles only, and
    /// undo puts it back so that a step run again takes what it took.
    SetLoopMode {
        old: Option<usize>,
        new: Option<usize>,
    },
}
/// What a step of the history is: an instruction the program ran, a
/// [`Poke`](crate::interpreter::Interpreter::begin_poke) the host made between
/// two of them, or an IRQ taken between two of them.
///
/// It is written on every step, so a reader never has to guess what an absent
/// field meant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionStepKind {
    /// One instruction of the program.
    Instruction,
    /// One Poke: the register and memory values the host wrote between two
    /// instructions, in one transaction.
    Poke,
    /// One IRQ taken: the frame pushed and the jump through the autovector of
    /// its level, before the instruction at the step's program counter, which
    /// has not run. It runs no instruction, so it has no location and runs
    /// nothing; its mutations say which level it was.
    Irq,
}

/// One value a Poke wrote, as the editor shows it: what was there before the
/// write and what is there when the transaction closes.
///
/// A register is named the way the editor spells it (`d0`, `a7`); memory
/// carries the bytes themselves. This is the readable half of a Poke — the
/// [`MutationOperation`]s of the same step are what undo replays.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PokeWrite {
    Register {
        name: String,
        old: u32,
        new: u32,
    },
    Memory {
        address: usize,
        old: Vec<u8>,
        new: Vec<u8>,
    },
}

/// One value an open Poke has written, with what was there before it.
///
/// The new value is read when the transaction closes, so that a register
/// written twice reports the value the Poke leaves behind and not the one it
/// passed through.
pub enum PokeTarget {
    Register { register: RegisterOperand, old: u32 },
    Memory { address: usize, old: Vec<u8> },
    /// The status register or one of the two stack pointers, the System
    /// register file's.
    System { register: SystemRegister, old: u32 },
}

/// The registers of the System register file: the status register and the two
/// stack pointers, and the 68010's control registers VBR, SFC and DFC, which a
/// Poke can write beside the CPU file's D0 to A7.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SystemRegister {
    /// The status register.
    Sr,
    /// The user stack pointer.
    Usp,
    /// The supervisor stack pointer.
    Ssp,
    /// The vector base register, the 68010's.
    Vbr,
    /// The source function code, the 68010's.
    Sfc,
    /// The destination function code, the 68010's.
    Dfc,
}

impl SystemRegister {
    /// How the editor spells it: `sr`, `usp`, `ssp`, `vbr`, `sfc` or `dfc`.
    pub fn name(self) -> &'static str {
        match self {
            SystemRegister::Sr => "sr",
            SystemRegister::Usp => "usp",
            SystemRegister::Ssp => "ssp",
            SystemRegister::Vbr => "vbr",
            SystemRegister::Sfc => "sfc",
            SystemRegister::Dfc => "dfc",
        }
    }

    /// Whether it is one of the 68010's control registers that are no stack
    /// pointer: VBR, SFC or DFC, which a 68000 does not have.
    pub fn is_control(self) -> bool {
        matches!(
            self,
            SystemRegister::Vbr | SystemRegister::Sfc | SystemRegister::Dfc
        )
    }
}

/// What an open Poke transaction has collected so far.
///
/// [`mutations`](PokeJournal::add_mutation) is what undo replays, in the order
/// the writes happened; `targets` is one entry per value written, which becomes
/// the step's [`PokeWrite`] list. A value written twice keeps the old value of
/// the first write, because that is what undoing the whole Poke puts back.
///
/// The status register as the Poke found it is kept too: it is the step's
/// status register before, which undo writes back as it does an instruction's.
/// So is the Interpreter's status, which a Poke of the status register can
/// change, waking a stopped processor or leaving it waiting.
pub struct PokeJournal {
    mutations: Vec<MutationOperation>,
    targets: Vec<PokeTarget>,
    sr: u16,
    status: InterpreterStatus,
    /// The tops of the stacks before a Poke of S swapped them, or of the
    /// stack pointer A7 is not on moved one.
    stack_tops: Option<StackTops>,
}

impl PokeJournal {
    /// An empty journal, opened with the status register at `sr` and the
    /// Interpreter in `status`.
    pub fn new(sr: u16, status: InterpreterStatus) -> Self {
        Self {
            mutations: Vec::new(),
            targets: Vec::new(),
            sr,
            status,
            stack_tops: None,
        }
    }
    /// Records the tops of the stacks before the Poke changed them, the first
    /// time it does.
    pub fn record_stack_tops(&mut self, old: StackTops) {
        self.stack_tops.get_or_insert(old);
    }
    /// The tops of the stacks before the Poke changed them, if it did.
    pub fn stack_tops(&self) -> Option<StackTops> {
        self.stack_tops
    }
    /// The status register when the Poke was opened.
    pub fn sr(&self) -> u16 {
        self.sr
    }
    /// The Interpreter's status when the Poke was opened.
    pub fn status(&self) -> InterpreterStatus {
        self.status
    }
    /// Whether nothing has been written into it, which is a Poke that records
    /// no step.
    pub fn is_empty(&self) -> bool {
        self.mutations.is_empty() && self.targets.is_empty()
    }
    pub fn add_mutation(&mut self, mutation: MutationOperation) {
        self.mutations.push(mutation);
    }
    /// Names a register the Poke has written, keeping the old value of its
    /// first write.
    pub fn add_register_target(&mut self, register: RegisterOperand, old: u32) {
        let already_written = self.targets.iter().any(|target| match target {
            PokeTarget::Register {
                register: other, ..
            } => other == &register,
            _ => false,
        });
        if !already_written {
            self.targets.push(PokeTarget::Register { register, old });
        }
    }
    /// Names a register of the System file the Poke has written, keeping the
    /// old value of its first write.
    pub fn add_system_target(&mut self, register: SystemRegister, old: u32) {
        let already_written = self.targets.iter().any(|target| match target {
            PokeTarget::System {
                register: other, ..
            } => *other == register,
            _ => false,
        });
        if !already_written {
            self.targets.push(PokeTarget::System { register, old });
        }
    }
    /// Names a run of memory the Poke has written, keeping the old bytes of its
    /// first write.
    pub fn add_memory_target(&mut self, address: usize, old: Vec<u8>) {
        let already_written = self.targets.iter().any(|target| match target {
            PokeTarget::Memory {
                address: other,
                old: other_old,
            } => other == &address && other_old.len() == old.len(),
            _ => false,
        });
        if !already_written {
            self.targets.push(PokeTarget::Memory { address, old });
        }
    }
    /// The mutations undo replays and the values written, in the order they
    /// were written.
    pub fn into_parts(self) -> (Vec<MutationOperation>, Vec<PokeTarget>) {
        (self.mutations, self.targets)
    }
}

/// Where the instruction of a step was written, kept in the step's slot even
/// while the step has none.
///
/// The slot keeps the File path it was last written with, so a reused slot
/// copies into that buffer instead of allocating a new one. It serialises as
/// the `Option<Location>` it stands for: the Location, or `null`.
#[derive(Clone)]
struct StepLocation {
    location: Location,
    present: bool,
}

impl Default for StepLocation {
    fn default() -> Self {
        Self {
            location: Location {
                file: String::new(),
                line: 0,
                column: 0,
                end_column: 0,
            },
            present: false,
        }
    }
}

impl Serialize for StepLocation {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        if self.present {
            self.location.serialize(serializer)
        } else {
            serializer.serialize_none()
        }
    }
}

#[derive(Clone, Serialize)]
pub struct ExecutionStep {
    #[serde(skip)]
    /// The tops of the two stacks before the step changed either, the one
    /// A7 was on first, which undo puts back.
    pub(crate) old_stack_tops: Option<StackTops>,
    /// Identifies this execution, including repeated visits to the same PC. Never reused by undo.
    id: u64,
    /// Whether the step is an instruction or a Poke. Always written.
    kind: ExecutionStepKind,
    mutations: Vec<MutationOperation>,
    /// The values a Poke wrote, with their old and new values; empty on an
    /// instruction.
    writes: Vec<PokeWrite>,
    pc: usize,
    /// Where the instruction that ran was written, or absent when the program
    /// counter was on no instruction.
    location: StepLocation,
    old_ccr: Flags,
    new_ccr: Flags,
    /// The whole status register before the step, condition codes included.
    ///
    /// It is what undo puts back, because the high byte of the status register
    /// — trace, supervisor, interrupt mask — is state a step can change
    /// (`move #n,sr`, `andi #n,sr`) and `old_ccr` does not carry it. The two
    /// overlap on the condition codes on purpose: `old_ccr` is the shape the
    /// editor has always read, in this crate's own flag bits, and this is the
    /// register as the processor numbers it (the implementation notes, phase 3).
    old_sr: u16,
    /// The whole status register after the step.
    new_sr: u16,
    /// The cycle counter before the step, which undo puts back.
    #[serde(skip)]
    old_cycles: u64,
    /// What the counter is measured from for [`cycles`](ExecutionStep::cycles):
    /// `old_cycles`, moved down by what task 30 cleared during the step, so
    /// that a step that clears the counter costs what it took on either side
    /// of the clear. It wraps.
    #[serde(skip)]
    cycles_from: u64,
    /// The cycles the step took: an instruction's, with the exceptions it
    /// raised; an IRQ's 44; nothing for a Poke.
    cycles: u32,
    /// The [code write](CodeWrite) the instruction made: the first instruction
    /// whose bytes it changed, or `None`. Undoing the step takes it back with
    /// the bytes.
    #[serde(rename = "wroteCode")]
    wrote_code: Option<CodeWrite>,
    /// The words of the instruction the step ran, as memory held them when it
    /// was fetched, which is what the decode cache ran: a later write into
    /// code does not change them. The Interpreter turns them into the step's
    /// `ran` line for the host
    /// ([`Interpreter::ran_by`](crate::interpreter::Interpreter::ran_by)),
    /// which is why they are not serialised here.
    #[serde(skip)]
    ran: Ran,
    /// The Interpreter state before the step. This is internal history state:
    /// undo needs it to put a resumed instruction back behind the pause it
    /// crossed, but it is not part of the public ExecutionStep shape.
    #[serde(skip)]
    old_interpreter_status: InterpreterStatus,
}

/// The most bytes an instruction the Interpreter decodes takes up, which is
/// what a step keeps of the one it ran.
pub const MAX_INSTRUCTION_BYTES: usize = MAX_68000_SIZE as usize;

/// The words of the instruction a step ran ([`ExecutionStep::get_ran`]),
/// recorded without allocating anything, and for the Program's own
/// instructions without reading memory: the step is on the path of every
/// instruction that runs with a history.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Ran {
    /// Nothing ran: a Poke, or an instruction fetched from an odd address.
    #[default]
    Nothing,
    /// The Program's instruction of this index, in the order of
    /// [`Program::instructions`](crate::assembler::program::Program::instructions),
    /// whose bytes memory held: the decode cache runs one only until a write
    /// changes them.
    Assembled(u32),
    /// Words decoded from memory as the program ran, the first `len` of
    /// `bytes`: code the program reached in its data, or code a write had
    /// changed.
    Decoded {
        /// The words, from the instruction's address on.
        bytes: [u8; MAX_INSTRUCTION_BYTES],
        /// How many of `bytes` are the instruction's.
        len: u8,
    },
}

impl ExecutionStep {
    pub fn new(
        kind: ExecutionStepKind,
        pc: usize,
        ccr: Flags,
        sr: u16,
        cycles: u64,
        interpreter_status: InterpreterStatus,
    ) -> Self {
        Self {
            old_stack_tops: None,
            id: 0,
            kind,
            mutations: vec![],
            writes: vec![],
            pc,
            old_ccr: ccr,
            new_ccr: ccr,
            old_sr: sr,
            new_sr: sr,
            old_cycles: cycles,
            cycles_from: cycles,
            cycles: 0,
            location: StepLocation::default(),
            wrote_code: None,
            ran: Ran::default(),
            old_interpreter_status: interpreter_status,
        }
    }
    /// Makes this slot a new step, emptying what the step it held had written
    /// and keeping the capacity of its buffers, so that a history which is full
    /// reuses its slots and allocates nothing for a step.
    fn reset(
        &mut self,
        kind: ExecutionStepKind,
        pc: usize,
        ccr: Flags,
        sr: u16,
        cycles: u64,
        interpreter_status: InterpreterStatus,
    ) {
        self.old_stack_tops = None;
        self.kind = kind;
        self.mutations.clear();
        self.writes.clear();
        self.pc = pc;
        self.old_ccr = ccr;
        self.new_ccr = ccr;
        self.old_sr = sr;
        self.new_sr = sr;
        self.old_cycles = cycles;
        self.cycles_from = cycles;
        self.cycles = 0;
        self.location.present = false;
        self.wrote_code = None;
        self.ran = Ran::Nothing;
        self.old_interpreter_status = interpreter_status;
    }
    pub fn add_mutation(&mut self, mutation: MutationOperation) {
        self.mutations.push(mutation);
    }
    /// Records the values a Poke wrote.
    pub fn set_writes(&mut self, writes: Vec<PokeWrite>) {
        self.writes = writes;
    }
    pub fn set_pc(&mut self, pc: usize) {
        self.pc = pc;
    }
    pub fn set_ccr(&mut self, ccr: Flags) {
        self.old_ccr = ccr;
    }
    pub fn get_mutations(&self) -> &Vec<MutationOperation> {
        &self.mutations
    }
    /// Whether this step is an instruction or a Poke.
    pub fn get_kind(&self) -> ExecutionStepKind {
        self.kind
    }
    /// The values a Poke wrote; empty on an instruction.
    pub fn get_writes(&self) -> &Vec<PokeWrite> {
        &self.writes
    }
    pub fn get_pc(&self) -> usize {
        self.pc
    }
    pub fn get_id(&self) -> u64 {
        self.id
    }
    pub fn get_ccr(&self) -> Flags {
        self.old_ccr
    }
    /// The whole status register before the step, which is what undo restores.
    pub fn get_sr(&self) -> u16 {
        self.old_sr
    }
    /// The cycle counter before the step, which is what undo restores.
    pub fn get_old_cycles(&self) -> u64 {
        self.old_cycles
    }
    /// The cycles the step took: an instruction's own and those of the
    /// exceptions it raised, 44 for an IRQ, and nothing for a Poke.
    pub fn get_cycles(&self) -> u32 {
        self.cycles
    }
    /// The Interpreter state before the step, which is what undo restores.
    pub fn get_interpreter_status(&self) -> InterpreterStatus {
        self.old_interpreter_status
    }
    /// Where the instruction this step ran was written.
    pub fn get_location(&self) -> Option<&Location> {
        self.location.present.then_some(&self.location.location)
    }
    /// The code write the instruction made, if it made one.
    pub fn get_code_write(&self) -> Option<CodeWrite> {
        self.wrote_code
    }
    /// The words of the instruction this step ran, as they were when it ran:
    /// [`Ran::Nothing`] for a Poke and for a step that ran nothing.
    pub fn get_ran(&self) -> Ran {
        self.ran
    }
}

/// What a frame of the call stack was entered by.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FrameKind {
    /// `bsr` or `jsr`, which `rts` or `rtr` returns from.
    Call,
    /// An Exception through a vector, which `rte` returns from.
    Exception,
}

/// One frame of the call stack: a subroutine that has been entered and not yet
/// returned from, or an Exception handler that has not executed its `rte`.
#[derive(Debug, Clone)]
pub struct CallStackFrame {
    address: usize,
    source_address: usize,
    registers: Vec<u32>,
    kind: FrameKind,
    /// The vector of an exception frame.
    vector: Option<u8>,
}

impl CallStackFrame {
    pub fn new(address: usize, source_address: usize, registers: Vec<u32>) -> Self {
        Self {
            address,
            source_address,
            registers,
            kind: FrameKind::Call,
            vector: None,
        }
    }
    /// The frame of an Exception through `vector` into the handler at
    /// `address`, which returns to `source_address`.
    pub fn exception(
        address: usize,
        source_address: usize,
        registers: Vec<u32>,
        vector: u8,
    ) -> Self {
        Self {
            address,
            source_address,
            registers,
            kind: FrameKind::Exception,
            vector: Some(vector),
        }
    }
    /// What entered the frame.
    pub fn get_kind(&self) -> FrameKind {
        self.kind
    }
    /// The vector of an exception frame, `None` for a call.
    pub fn get_vector(&self) -> Option<u8> {
        self.vector
    }
    pub fn get_address(&self) -> usize {
        self.address
    }
    pub fn get_source_address(&self) -> usize {
        self.source_address
    }
    pub fn get_registers(&self) -> Vec<u32> {
        self.registers.clone()
    }
}

/// A Label of the Program, as the call stack names it: which routine a frame
/// is in, and where that name was written.
#[derive(Debug, Clone, Serialize)]
pub struct StackFrameLabel {
    /// The full name, so a Local label reads as `start:loop`.
    pub name: String,
    /// The address the Label stands for.
    pub address: usize,
    /// Where the Label was written.
    pub location: Location,
}

/// Overwrites `into` with `from`, reusing the buffer of `into`'s File path,
/// which is left alone when it names the same File already, as it nearly
/// always does: it is the File of the step the slot held before.
///
/// It is kept out of line: inlined into every run's loop, the comparison and
/// the copy, which only a run that keeps a history makes, slowed the loop of
/// a run that keeps none.
#[inline(never)]
fn copy_location(into: &mut Location, from: &Location) {
    if into.file != from.file {
        into.file.clone_from(&from.file);
    }
    into.line = from.line;
    into.column = from.column;
    into.end_column = from.end_column;
}

#[wasm_bindgen]
pub struct Debugger {
    next_step_id: u64,
    /// The slots of the history, in no particular order: `oldest` says where the
    /// history starts. A slot is made the first time the history grows to it
    /// and is reused after that, so a full history overwrites its oldest step in
    /// place and a step allocates nothing once the buffer has filled.
    history: Vec<ExecutionStep>,
    /// The index in `history` of the oldest step.
    oldest: usize,
    /// How many steps the history holds, never more than `capacity`.
    len: usize,
    /// How many steps the history keeps, which is at least one.
    capacity: usize,
    call_stack: Vec<CallStackFrame>,
    labels: HashMap<usize, StackFrameLabel>,
}

impl Debugger {
    /// A Debugger that keeps `history_size` steps and names the addresses of
    /// `symbols` in the call stack.
    ///
    /// Only Labels are names of addresses; a Constant, a Variable and a
    /// Register list name values and never a frame. Where two Labels sit on one
    /// address the first in name order is the one the call stack shows, so that
    /// it always shows the same one.
    pub fn new(history_size: usize, symbols: &BTreeMap<String, ProgramSymbol>) -> Self {
        let mut labels_map: HashMap<usize, StackFrameLabel> = HashMap::new();
        for symbol in symbols.values() {
            if symbol.kind != SymbolKind::Label {
                continue;
            }
            labels_map
                .entry(symbol.value as usize)
                .or_insert_with(|| StackFrameLabel {
                    name: symbol.name.clone(),
                    address: symbol.value as usize,
                    location: symbol.location.clone(),
                });
        }
        //empty until the first step: a program that has not run has nothing to undo
        Self {
            next_step_id: 1,
            history: Vec::new(),
            oldest: 0,
            len: 0,
            capacity: history_size.max(1),
            call_stack: vec![],
            labels: labels_map,
        }
    }
    /// Starts a step as the newest of the history: an instruction about to run,
    /// or a Poke. The step gets the next id, which undo never gives back.
    pub fn begin_step(
        &mut self,
        kind: ExecutionStepKind,
        pc: usize,
        ccr: Flags,
        sr: u16,
        cycles: u64,
        interpreter_status: InterpreterStatus,
    ) {
        let id = self.next_step_id;
        self.next_step_id += 1;
        let index = self.claim_slot();
        if index == self.history.len() {
            self.history.push(ExecutionStep::new(
                kind,
                pc,
                ccr,
                sr,
                cycles,
                interpreter_status,
            ));
        } else {
            self.history[index].reset(kind, pc, ccr, sr, cycles, interpreter_status);
        }
        self.history[index].id = id;
    }
    /// The slot the newest step is written into. While the history is growing
    /// that is the next free one; once it is full, it is the oldest step's,
    /// which the new step replaces.
    fn claim_slot(&mut self) -> usize {
        if self.len < self.capacity {
            let index = self.wrap(self.oldest + self.len);
            self.len += 1;
            index
        } else {
            let index = self.oldest;
            self.oldest = self.wrap(self.oldest + 1);
            index
        }
    }
    /// The index in `history` of the step `age` steps before the newest, where
    /// age 0 is the newest. `age` must be less than `len`.
    fn index_of(&self, age: usize) -> usize {
        self.wrap(self.oldest + self.len - 1 - age)
    }
    /// `position % capacity` for a `position` below twice the capacity, which
    /// every index into the history is, without a division: it is on the path
    /// of every step that keeps a history, several times.
    #[inline(always)]
    fn wrap(&self, position: usize) -> usize {
        if position >= self.capacity {
            position - self.capacity
        } else {
            position
        }
    }
    /// The step being recorded, or None before the first one: what is written
    /// then belongs to no step, and there is nothing it could be undone to.
    fn newest_mut(&mut self) -> Option<&mut ExecutionStep> {
        if self.len == 0 {
            return None;
        }
        let index = self.index_of(0);
        Some(&mut self.history[index])
    }
    /// Takes the newest step out of the history, answering a copy of it. Its
    /// slot keeps its buffers for the next step to reuse.
    pub fn pop_step(&mut self) -> Option<ExecutionStep> {
        let step = self.get_last_step()?.clone();
        self.len -= 1;
        Some(step)
    }
    pub fn get_previous_mutations(&self) -> Option<&Vec<MutationOperation>> {
        self.get_last_step().map(|step| step.get_mutations())
    }
    pub fn can_undo(&self) -> bool {
        self.len > 0
    }
    pub fn get_last_step(&self) -> Option<&ExecutionStep> {
        if self.len == 0 {
            None
        } else {
            Some(&self.history[self.index_of(0)])
        }
    }
    /// Records the tops of the stacks before the step being recorded changed
    /// them, the first time it does: a step can follow A7's and swap them.
    pub fn record_stack_tops(&mut self, old: StackTops) {
        if let Some(step) = self.newest_mut() {
            step.old_stack_tops.get_or_insert(old);
        }
    }
    /// Records what the step being recorded left behind: the condition
    /// codes, the whole status register, and the cycle counter, from which
    /// the step's own cycles are worked out.
    pub fn set_new_state(&mut self, ccr: Flags, sr: u16, cycles: u64) {
        if let Some(step) = self.newest_mut() {
            step.new_ccr = ccr;
            step.new_sr = sr;
            step.cycles = cycles.wrapping_sub(step.cycles_from) as u32;
        }
    }
    /// Records that task 30 cleared the cycle counter from `cleared` during
    /// the step being recorded, so that the step's cycles count what it took
    /// before the clear and after it.
    pub fn record_cycles_cleared(&mut self, cleared: u64) {
        if let Some(step) = self.newest_mut() {
            step.cycles_from = step.cycles_from.wrapping_sub(cleared);
        }
    }
    /// Records the code write of the step being executed.
    pub fn set_code_write(&mut self, write: CodeWrite) {
        if let Some(step) = self.newest_mut() {
            step.wrote_code = Some(write);
        }
    }
    /// Records where the instruction of the step being executed was written,
    /// and the words it runs.
    #[inline(always)]
    pub fn set_instruction(&mut self, location: Option<&Location>, ran: Ran) {
        let Some(step) = self.newest_mut() else {
            return;
        };
        step.ran = ran;
        Self::write_location(step, location);
    }
    #[inline(always)]
    fn write_location(step: &mut ExecutionStep, location: Option<&Location>) {
        match location {
            Some(location) => {
                copy_location(&mut step.location.location, location);
                step.location.present = true;
            }
            None => step.location.present = false,
        }
    }
    pub fn add_mutation(&mut self, operation: MutationOperation) {
        if let Some(step) = self.newest_mut() {
            step.add_mutation(operation);
        }
    }
    /// Records the values a Poke wrote.
    pub fn set_writes(&mut self, writes: Vec<PokeWrite>) {
        if let Some(step) = self.newest_mut() {
            step.set_writes(writes);
        }
    }
    /// Every step of the history, oldest first.
    pub fn get_history(&self) -> impl Iterator<Item = &ExecutionStep> {
        (0..self.len).map(move |position| &self.history[self.wrap(self.oldest + position)])
    }
    pub fn get_last_steps(&self, count: usize) -> Vec<&ExecutionStep> {
        (0..count.min(self.len))
            .map(|age| &self.history[self.index_of(age)])
            .collect()
    }
    /// The Label of every address that has one, which is what the call stack
    /// reads.
    pub fn get_labels(&self) -> &HashMap<usize, StackFrameLabel> {
        &self.labels
    }
    pub fn push_call(&mut self, address: usize, source_address: usize, registers: Vec<u32>) {
        self.call_stack
            .push(CallStackFrame::new(address, source_address, registers));
    }
    pub fn pop_call(&mut self) -> Option<CallStackFrame> {
        self.call_stack.pop()
    }
    /// Pushes the frame of an Exception through `vector` into the handler at
    /// `address`, which returns to `source_address`.
    pub fn push_exception(
        &mut self,
        address: usize,
        source_address: usize,
        registers: Vec<u32>,
        vector: u8,
    ) {
        self.call_stack.push(CallStackFrame::exception(
            address,
            source_address,
            registers,
            vector,
        ));
    }
    /// Takes the frame on top of the call stack off when it is an exception
    /// frame, which is what `rte` returns from; a call frame on top, or none,
    /// is left as it is.
    pub fn pop_exception(&mut self) -> Option<CallStackFrame> {
        match self.call_stack.last() {
            Some(frame) if frame.kind == FrameKind::Exception => self.call_stack.pop(),
            _ => None,
        }
    }
    /// Puts back a frame [`pop_call`](Self::pop_call) took, as undoing a return
    /// does.
    pub fn restore_call(&mut self, frame: CallStackFrame) {
        self.call_stack.push(frame);
    }
    /// The call stack as the editor shows it: every frame with the name of the
    /// routine it is in and where that name was written.
    pub fn to_call_stack(&self) -> Vec<PrettyStackFrame> {
        self.call_stack
            .iter()
            .map(|frame| match self.labels.get(&frame.address) {
                Some(label) => PrettyStackFrame {
                    address: frame.address,
                    source_address: frame.source_address,
                    registers: frame.registers.clone(),
                    label_name: label.name.clone(),
                    label_address: label.address,
                    label_location: Some(label.location.clone()),
                    kind: frame.kind,
                    vector: frame.vector,
                },
                None => PrettyStackFrame {
                    address: frame.address,
                    source_address: frame.source_address,
                    registers: frame.registers.clone(),
                    label_name: "Unknown".to_string(),
                    label_address: frame.address,
                    label_location: None,
                    kind: frame.kind,
                    vector: frame.vector,
                },
            })
            .collect()
    }
}

/// One frame of the call stack, with the Label of the routine it is in already
/// looked up: what the editor draws.
#[derive(Debug, Clone, Serialize)]
pub struct PrettyStackFrame {
    /// The address the routine starts at: the handler, for an exception
    /// frame.
    pub address: usize,
    /// The address the frame returns to: after the call, or the program
    /// counter the exception stacked.
    pub source_address: usize,
    /// The registers as they were when it was called.
    pub registers: Vec<u32>,

    /// The name of the Label on `address`, or `Unknown` when it has none.
    pub label_name: String,
    /// The address that Label stands for.
    pub label_address: usize,
    /// Where that Label was written, or `None` when the address has none.
    pub label_location: Option<Location>,
    /// A call, or an Exception.
    pub kind: FrameKind,
    /// The vector of an exception frame, `None` for a call.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vector: Option<u8>,
}
// The TypeScript declaration of this shape is `IStackFrame` in
// `src/ts_types.rs`, with every other hand-written one.
