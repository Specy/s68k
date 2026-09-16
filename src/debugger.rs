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

use std::collections::{BTreeMap, HashMap, LinkedList};

use serde::Serialize;
use wasm_bindgen::prelude::wasm_bindgen;

use crate::{
    assembler::{program::ProgramSymbol, source::Location, symbols::SymbolKind},
    instructions::{RegisterOperand, Size},
    interpreter::{Flags, InterpreterStatus},
};

/// One thing a step changed, with the value it replaced beside the value it
/// wrote.
///
/// Every write carries both sides, read where the write happened and never
/// reconstructed afterwards: `old` is what the Core found there and `new` is
/// what the store left behind. A register write reports the whole register on
/// both sides, because a sized store changes only part of one and the register
/// is what the panels draw; a memory write reports the value at the width it
/// was made. `PushCall` and `PopCall` write no value and carry neither.
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
    },
}
/// What a step of the history is: an instruction the program ran, or a
/// [`Poke`](crate::interpreter::Interpreter::begin_poke) the host made between
/// two of them.
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
}

/// What an open Poke transaction has collected so far.
///
/// [`mutations`](PokeJournal::add_mutation) is what undo replays, in the order
/// the writes happened; `targets` is one entry per value written, which becomes
/// the step's [`PokeWrite`] list. A value written twice keeps the old value of
/// the first write, because that is what undoing the whole Poke puts back.
#[derive(Default)]
pub struct PokeJournal {
    mutations: Vec<MutationOperation>,
    targets: Vec<PokeTarget>,
}

impl PokeJournal {
    pub fn new() -> Self {
        Self::default()
    }
    /// Whether nothing has been written into it, which is a Poke that records
    /// no step.
    pub fn is_empty(&self) -> bool {
        self.mutations.is_empty()
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

#[derive(Serialize)]
pub struct ExecutionStep {
    /// Identifies this execution, including repeated visits to the same PC. Never reused by undo.
    id: u64,
    /// Whether the step is an instruction or a Poke. Always written.
    kind: ExecutionStepKind,
    mutations: Vec<MutationOperation>,
    /// The values a Poke wrote, with their old and new values; empty on an
    /// instruction.
    writes: Vec<PokeWrite>,
    pc: usize,
    /// Where the instruction that ran was written, or `None` when the program
    /// counter was on no instruction.
    location: Option<Location>,
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
    /// The Interpreter state before the step. This is internal history state:
    /// undo needs it to put a resumed instruction back behind the pause it
    /// crossed, but it is not part of the public ExecutionStep shape.
    #[serde(skip)]
    old_interpreter_status: InterpreterStatus,
}

impl ExecutionStep {
    pub fn new(pc: usize, ccr: Flags, sr: u16, interpreter_status: InterpreterStatus) -> Self {
        Self {
            id: 0,
            kind: ExecutionStepKind::Instruction,
            mutations: vec![],
            writes: vec![],
            pc,
            old_ccr: ccr,
            new_ccr: ccr,
            old_sr: sr,
            new_sr: sr,
            location: None,
            old_interpreter_status: interpreter_status,
        }
    }
    /// A step of a Poke: it ran no instruction, so the program counter, the
    /// condition codes and the status register are the ones the Interpreter
    /// already holds and undoing it puts back exactly what it found.
    pub fn new_poke(pc: usize, ccr: Flags, sr: u16, interpreter_status: InterpreterStatus) -> Self {
        Self {
            kind: ExecutionStepKind::Poke,
            ..Self::new(pc, ccr, sr, interpreter_status)
        }
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
    /// The Interpreter state before the step, which is what undo restores.
    pub fn get_interpreter_status(&self) -> InterpreterStatus {
        self.old_interpreter_status
    }
    /// Where the instruction this step ran was written.
    pub fn get_location(&self) -> Option<&Location> {
        self.location.as_ref()
    }
}

/// One frame of the call stack: a subroutine that has been entered and not yet
/// returned from.
pub struct CallStackFrame {
    address: usize,
    source_address: usize,
    registers: Vec<u32>,
}

impl CallStackFrame {
    pub fn new(address: usize, source_address: usize, registers: Vec<u32>) -> Self {
        Self {
            address,
            source_address,
            registers,
        }
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

#[wasm_bindgen]
pub struct Debugger {
    next_step_id: u64,
    history: LinkedList<ExecutionStep>,
    history_size: usize,
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
        //include at least one to prevent initialization errors when pushing history state
        let mut empty_history: LinkedList<ExecutionStep> = LinkedList::new();
        empty_history.push_front(ExecutionStep::new(
            0,
            Flags::empty(),
            crate::interpreter::INITIAL_STATUS_REGISTER,
            InterpreterStatus::Running,
        ));
        Self {
            next_step_id: 1,
            history: empty_history,
            history_size,
            call_stack: vec![],
            labels: labels_map,
        }
    }
    pub fn add_step(&mut self, mut step: ExecutionStep) {
        step.id = self.next_step_id;
        self.next_step_id += 1;
        self.history.push_back(step);
        if self.history.len() > self.history_size {
            self.history.pop_front();
        }
    }
    pub fn pop_step(&mut self) -> Option<ExecutionStep> {
        self.history.pop_back()
    }
    pub fn get_previous_mutations(&self) -> Option<&Vec<MutationOperation>> {
        match self.history.back() {
            Some(step) => Some(step.get_mutations()),
            None => None,
        }
    }
    pub fn can_undo(&self) -> bool {
        !self.history.is_empty()
    }
    pub fn get_last_step(&self) -> Option<&ExecutionStep> {
        self.history.back()
    }
    pub fn set_new_ccr(&mut self, ccr: Flags) {
        self.history
            .back_mut()
            .expect("No history to set new ccr")
            .new_ccr = ccr;
    }
    /// Records the whole status register the step left behind.
    pub fn set_new_sr(&mut self, sr: u16) {
        self.history
            .back_mut()
            .expect("No history to set new sr")
            .new_sr = sr;
    }
    /// Records where the instruction of the step being executed was written.
    pub fn set_location(&mut self, location: Option<Location>) {
        self.history
            .back_mut()
            .expect("No history to set the location of")
            .location = location;
    }
    pub fn add_mutation(&mut self, operation: MutationOperation) {
        self.history
            .back_mut()
            .expect("No history to add mutation to")
            .add_mutation(operation);
    }
    pub fn get_history(&self) -> &LinkedList<ExecutionStep> {
        &self.history
    }
    pub fn get_last_steps(&self, count: usize) -> Vec<&ExecutionStep> {
        self.history
            .iter()
            .rev()
            .take(count)
            .collect::<Vec<&ExecutionStep>>()
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
                },
                None => PrettyStackFrame {
                    address: frame.address,
                    source_address: frame.source_address,
                    registers: frame.registers.clone(),
                    label_name: "Unknown".to_string(),
                    label_address: frame.address,
                    label_location: None,
                },
            })
            .collect()
    }
}

/// One frame of the call stack, with the Label of the routine it is in already
/// looked up: what the editor draws.
#[derive(Debug, Clone, Serialize)]
pub struct PrettyStackFrame {
    /// The address the routine starts at.
    pub address: usize,
    /// The address of the instruction that called it.
    pub source_address: usize,
    /// The registers as they were when it was called.
    pub registers: Vec<u32>,

    /// The name of the Label on `address`, or `Unknown` when it has none.
    pub label_name: String,
    /// The address that Label stands for.
    pub label_address: usize,
    /// Where that Label was written, or `None` when the address has none.
    pub label_location: Option<Location>,
}
// The TypeScript declaration of this shape is `IStackFrame` in
// `src/ts_types.rs`, with every other hand-written one.
