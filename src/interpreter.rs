//! The Interpreter: what runs an assembled
//! [`Program`](crate::assembler::program).
//!
//! It holds the registers, the 16 MB of memory, the condition codes and the
//! step, run, undo and interrupt operations, and it never reads source: it
//! reaches a line only through the source [`Location`](crate::assembler::source)
//! the Assembler stored with each instruction (CONTEXT.md, "Interpreter").
//!
//! A failure of a running program is a **runtime error** and not a Diagnostic:
//! it is attributed to the instruction's Location and answered as an
//! [`InterpreterStatus`], while everything the Assembler found was reported
//! before the Program was built.
//!
//! Much of this module predates the assembler rewrite and its public items are
//! not all documented yet; everything the rewrite added or changed is.

/*
    Some of the implementations were inspired/taken from here, especially the complex flag handling and some mathematical operations
    https://github.com/transistorfet/moa/blob/main/emulator/cpus/m68k/src/execute.rs
*/

/*TODO
    Currently side effects are applied both when reading and storing the result of an operation.
    Those operations should be run only once, for example when reading to a postincrement register, and then stored
    to the same incremented register, 3 increments are applied, when only 1 should be applied.
    There needs to be added a way to only apply the side effect once, and then store the result to the register.
*/
use core::panic;
use std::collections::HashSet;

use bitflags::bitflags;
use serde::{Deserialize, Serialize};
use wasm_bindgen::{prelude::wasm_bindgen, JsValue};

use crate::assembler::program::{AssembledInstruction, MemoryContent, Program};
use crate::assembler::source::Location;
use crate::debugger::PrettyStackFrame;
use crate::instructions::TargetDirection;
use crate::{
    debugger::{Debugger, ExecutionStep, MutationOperation},
    instructions::{
        Condition, IndexRegister, Instruction, Interrupt, InterruptResult, KeyStateRequest,
        KeyStateResult, Operand, RegisterOperand, ShiftDirection, Sign, Size,
        EXTENSION_WORD_OFFSET,
    },
    math::*,
};

#[derive(Debug, Clone, Copy, PartialEq)]
enum Used {
    Once,
    Twice,
}

bitflags! {
    #[wasm_bindgen]
    #[derive(Serialize, Copy, Clone, Debug)]
    pub struct Flags: u16 {
        const Carry    = 1<<1;
        const Overflow = 1<<2;
        const Zero     = 1<<3;
        const Negative = 1<<4;
        const Extend   = 1<<5;
    }
}
impl Default for Flags {
    fn default() -> Self {
        Self::new()
    }
}

impl Flags {
    pub fn new() -> Self {
        Flags::empty()
    }
    pub fn clear(&mut self) {
        *self = Flags::empty();
    }
    /// The five flags as the 68000's own condition code byte: extend 16,
    /// negative 8, zero 4, overflow 2, carry 1.
    ///
    /// The bits of this type are s68k's own and are one place to the left of
    /// the processor's; the editor reads them
    /// ([`Interpreter::wasm_get_flags_as_number`]) and they are not moved. This
    /// is the conversion the status register needs, and the one every
    /// instruction that reads or writes `ccr` goes through.
    pub fn to_ccr_byte(&self) -> u8 {
        let mut byte = 0u8;
        if self.contains(Flags::Extend) {
            byte |= 0b1_0000;
        }
        if self.contains(Flags::Negative) {
            byte |= 0b1000;
        }
        if self.contains(Flags::Zero) {
            byte |= 0b100;
        }
        if self.contains(Flags::Overflow) {
            byte |= 0b10;
        }
        if self.contains(Flags::Carry) {
            byte |= 0b1;
        }
        byte
    }
    /// The flags a condition code byte names, the inverse of
    /// [`Flags::to_ccr_byte`]. Bits 5 to 7 of the byte are not condition codes
    /// and are dropped.
    pub fn from_ccr_byte(byte: u8) -> Flags {
        let mut flags = Flags::empty();
        flags.set(Flags::Extend, byte & 0b1_0000 != 0);
        flags.set(Flags::Negative, byte & 0b1000 != 0);
        flags.set(Flags::Zero, byte & 0b100 != 0);
        flags.set(Flags::Overflow, byte & 0b10 != 0);
        flags.set(Flags::Carry, byte & 0b1 != 0);
        flags
    }
    pub fn get_status(&self) -> String {
        format!(
            "X:{} N:{} Z:{} V:{} C:{}",
            self.contains(Flags::Extend) as u8,
            self.contains(Flags::Negative) as u8,
            self.contains(Flags::Zero) as u8,
            self.contains(Flags::Overflow) as u8,
            self.contains(Flags::Carry) as u8
        )
    }
}

pub enum MemoryCell {
    Byte(u8),
    Word(u16),
    Long(u32),
}

impl MemoryCell {
    pub fn get_long(&self) -> u32 {
        match self {
            MemoryCell::Byte(b) => *b as u32,
            MemoryCell::Word(w) => *w as u32,
            MemoryCell::Long(l) => *l,
        }
    }
    pub fn get_word(&self) -> u16 {
        match self {
            MemoryCell::Byte(b) => *b as u16,
            MemoryCell::Word(w) => *w,
            MemoryCell::Long(l) => *l as u16,
        }
    }
    pub fn get_byte(&self) -> u8 {
        match self {
            MemoryCell::Byte(b) => *b,
            MemoryCell::Word(w) => *w as u8,
            MemoryCell::Long(l) => *l as u8,
        }
    }
}

#[derive(Debug)]
#[wasm_bindgen]
pub struct Memory {
    data: Vec<u8>,
}

impl Memory {
    pub fn new() -> Self {
        Self {
            data: vec![255; 0x01000000], //16mb
        }
    }

    pub fn push(&mut self, data: &MemoryCell, mut sp: usize) -> RuntimeResult<usize> {
        match data {
            MemoryCell::Byte(byte) => {
                sp -= 1;
                self.write_byte(sp, *byte)?
            }
            MemoryCell::Word(word) => {
                sp -= 2;
                self.write_word(sp, *word)?
            }
            MemoryCell::Long(long) => {
                sp -= 4;
                self.write_long(sp, *long)?
            }
        }
        Ok(sp)
    }
    pub fn pop_empty_long(&self, mut sp: usize) -> RuntimeResult<usize> {
        sp += 4;
        Ok(sp)
    }
    pub fn pop(&mut self, size: Size, mut sp: usize) -> RuntimeResult<(MemoryCell, usize)> {
        let result = match size {
            Size::Byte => {
                let byte = self.read_byte(sp)?;
                MemoryCell::Byte(byte)
            }
            Size::Word => {
                let word = self.read_word(sp)?;
                sp += 2;
                MemoryCell::Word(word)
            }
            Size::Long => {
                let long = self.read_long(sp)?;
                sp += 4;
                MemoryCell::Long(long)
            }
        };
        Ok((result, sp))
    }
    pub fn read_long(&self, address: usize) -> RuntimeResult<u32> {
        let address = self.verify_address(address, Size::Long)?;

        Ok(u32::from_be_bytes(
            self.data[address..address + 4].try_into().unwrap(),
        ))
    }
    pub fn read_word(&self, address: usize) -> RuntimeResult<u16> {
        let address = self.verify_address(address, Size::Word)?;
        Ok(u16::from_be_bytes(
            self.data[address..address + 2].try_into().unwrap(),
        ))
    }
    pub fn read_byte(&self, address: usize) -> RuntimeResult<u8> {
        let address = self.verify_address(address, Size::Byte)?;
        Ok(u8::from_be_bytes(
            self.data[address..address + 1].try_into().unwrap(),
        ))
    }
    pub fn read_size(&self, address: usize, size: Size) -> RuntimeResult<u32> {
        match size {
            Size::Byte => {
                let byte = self.read_byte(address)?;
                Ok(byte as u32)
            }
            Size::Word => {
                let word = self.read_word(address)?;
                Ok(word as u32)
            }
            Size::Long => {
                let long = self.read_long(address)?;
                Ok(long)
            }
        }
    }
    pub fn write_size(&mut self, address: usize, size: Size, data: u32) -> RuntimeResult<()> {
        match size {
            Size::Byte => self.write_byte(address, data as u8)?,
            Size::Word => self.write_word(address, data as u16)?,
            Size::Long => self.write_long(address, data)?,
        }
        Ok(())
    }

    #[inline(always)]
    pub fn verify_address_bounds(&self, address: usize, length: usize) -> RuntimeResult<usize> {
        //m68k does not use the last 2 bytes of the address space, clamp it to 24 bits
        let address = address & 0x00ffffff;
        let end_address = address.wrapping_add(length);
        //+1 because the end address is exclusive
        if end_address > self.data.len() {
            return Err(RuntimeError::OutOfBounds(format!(
                "Memory out of bounds at address: 0x{:x} + {}, maximum: 0x{:x}",
                address,
                length,
                self.data.len()
            )));
        }
        Ok(address)
    }
    #[inline(always)]
    pub fn verify_address(&self, address: usize, size: Size) -> RuntimeResult<usize> {
        let address = self.verify_address_bounds(address, size.to_bytes())?;
        let odd = address & 1 != 0;
        if odd && size != Size::Byte {
            return Err(RuntimeError::AddressError { address, size });
        }
        Ok(address)
    }
    pub fn write_long(&mut self, address: usize, value: u32) -> RuntimeResult<()> {
        let address = self.verify_address(address, Size::Long)?;
        self.data[address..address + 4].copy_from_slice(&value.to_be_bytes());
        Ok(())
    }
    pub fn write_word(&mut self, address: usize, value: u16) -> RuntimeResult<()> {
        let address = self.verify_address(address, Size::Word)?;
        self.data[address..address + 2].copy_from_slice(&value.to_be_bytes());
        Ok(())
    }
    pub fn write_byte(&mut self, address: usize, value: u8) -> RuntimeResult<()> {
        let address = self.verify_address(address, Size::Byte)?;
        self.data[address] = value;
        Ok(())
    }
    pub fn write_bytes(&mut self, address: usize, bytes: &[u8]) -> RuntimeResult<()> {
        let address = self.verify_address_bounds(address, bytes.len())?;
        self.data[address..address + bytes.len()].copy_from_slice(bytes);
        Ok(())
    }
    pub fn read_bytes(&self, address: usize, length: usize) -> RuntimeResult<&[u8]> {
        let address = self.verify_address_bounds(address, length)?;
        Ok(&self.data[address..address + length])
    }
}

#[wasm_bindgen]
impl Memory {
    pub fn wasm_read_bytes(&self, address: usize, size: usize) -> Vec<u8> {
        match self.read_bytes(address, size) {
            Ok(bytes) => bytes.to_vec(),
            Err(_) => vec![],
        }
    }
}

#[wasm_bindgen]
#[derive(Debug, Clone, Copy)]
pub struct Register {
    data: u32,
}

impl Default for Register {
    fn default() -> Self {
        Self::new()
    }
}

impl Register {
    pub fn new() -> Self {
        Self { data: 0 }
    }
    pub fn store_long(&mut self, data: u32) {
        self.data = data;
    }
    pub fn store_word(&mut self, data: u16) {
        self.data = (self.data & 0xFFFF0000) | u32::from(data);
    }
    pub fn store_byte(&mut self, data: u8) {
        self.data = (self.data & 0xFFFFFF00) | u32::from(data);
    }

    pub fn get_long(&self) -> u32 {
        self.data
    }
    pub fn get_word(&self) -> u16 {
        (self.data & 0xFFFF) as u16
    }
    pub fn get_byte(&self) -> u8 {
        (self.data & 0xFF) as u8
    }
    pub fn get_size(&self, size: Size) -> u32 {
        match size {
            Size::Byte => self.get_byte() as u32,
            Size::Word => self.get_word() as u32,
            Size::Long => self.get_long(),
        }
    }
    pub fn store_size(&mut self, size: Size, data: u32) {
        match size {
            Size::Byte => self.store_byte(data as u8),
            Size::Word => self.store_word(data as u16),
            Size::Long => self.store_long(data),
        }
    }
    pub fn clear(&mut self) {
        self.data = 0;
    }
}

#[wasm_bindgen]
impl Register {
    pub fn wasm_get_long(&self) -> u32 {
        self.get_long()
    }
    pub fn wasm_get_word(&self) -> u16 {
        self.get_word()
    }
    pub fn wasm_get_byte(&self) -> u8 {
        self.get_byte()
    }
}

/// The status register a program starts with: `$2700`, EASy68K's own
/// (`SIMHELP/Exceptions.htm`, "the supervisor bit is set on") — supervisor,
/// interrupt mask 7, no trace and no condition code set.
pub const INITIAL_STATUS_REGISTER: u16 = 0x2700;

#[derive(Debug, Clone, Copy)]
#[wasm_bindgen]
pub struct Cpu {
    d_reg: [Register; 8],
    a_reg: [Register; 8],
    ccr: Flags,
    /// The high byte of the status register: trace, supervisor and the
    /// interrupt mask.
    ///
    /// It is stored and readable and has no effect at all — s68k runs every
    /// program as supervisor, which is what EASy68K's simulator starts in (the
    /// design record, "Instructions"). The low byte is [`Cpu::ccr`], so the
    /// whole register is [`Cpu::get_sr`].
    system_byte: u8,
}

impl Default for Cpu {
    fn default() -> Self {
        Self::new()
    }
}

impl Cpu {
    pub fn new() -> Self {
        Self {
            d_reg: [Register::new(); 8],
            a_reg: [Register::new(); 8],
            ccr: Flags::from_ccr_byte(INITIAL_STATUS_REGISTER as u8),
            system_byte: (INITIAL_STATUS_REGISTER >> 8) as u8,
        }
    }

    /// The whole status register: the system byte, then the condition codes as
    /// the processor numbers them.
    pub fn get_sr(&self) -> u16 {
        ((self.system_byte as u16) << 8) | self.ccr.to_ccr_byte() as u16
    }

    /// Sets the whole status register, condition codes included.
    pub fn set_sr(&mut self, value: u16) {
        self.system_byte = (value >> 8) as u8;
        self.ccr = Flags::from_ccr_byte(value as u8);
    }

    /// The condition codes as the low byte of the status register.
    pub fn get_ccr_byte(&self) -> u8 {
        self.ccr.to_ccr_byte()
    }

    /// Sets the condition codes from a byte, leaving the system byte where it
    /// is.
    pub fn set_ccr_byte(&mut self, byte: u8) {
        self.ccr = Flags::from_ccr_byte(byte);
    }

    pub fn get_register_values(&self) -> Vec<u32> {
        self.d_reg
            .iter()
            .map(|reg| reg.get_long())
            .chain(self.a_reg.iter().map(|reg| reg.get_long()))
            .collect()
    }
}

#[wasm_bindgen]
impl Cpu {
    pub fn wasm_get_d_reg(&self, index: usize) -> Register {
        self.d_reg[index]
    }
    pub fn wasm_get_d_regs_value(&self) -> Vec<u32> {
        self.d_reg.iter().map(|reg| reg.get_long()).collect()
    }

    pub fn wasm_get_a_regs_value(&self) -> Vec<u32> {
        self.a_reg.iter().map(|reg| reg.get_long()).collect()
    }
    pub fn wasm_get_a_reg(&self, index: usize) -> Register {
        self.a_reg[index]
    }
    pub fn wasm_get_ccr(&self) -> Flags {
        self.ccr
    }
    /// The whole status register, `$2700` before a program has run.
    pub fn wasm_get_sr(&self) -> u16 {
        self.get_sr()
    }
}

/// What stopped a running program.
///
/// The three exception variants below are the instructions that end a run on
/// purpose (`SIMHELP/Exceptions.htm`, group 2 and the Illegal exception): s68k
/// keeps no exception vectors and no supervisor stack frame, so the run ends
/// with [`InterpreterStatus::TerminatedWithException`] where a 68000 would jump
/// through a vector, and the error names the instruction and its cause.
#[derive(Debug, Serialize)]
#[serde(tag = "type", content = "value")]
pub enum RuntimeError {
    Raw(String),
    ExecutionLimit(usize),
    OutOfBounds(String),
    AddressError {
        address: usize,
        size: Size,
    },
    DivisionByZero,
    IncorrectAddressingMode(String),
    Unimplemented,
    /// `chk` found the register outside 0 to the bound it was given.
    ChkOutOfBounds {
        /// The low word of the register, read as a signed number.
        value: i32,
        /// The bound the operand held, read as a signed number.
        bound: i32,
    },
    /// `trapv` with the overflow flag set.
    OverflowException,
    /// The `illegal` instruction, which always ends the run.
    IllegalInstruction,
}

pub type RuntimeResult<T> = Result<T, RuntimeError>;

#[derive(Debug, Clone, PartialEq, Serialize, Copy)]
#[wasm_bindgen]
pub enum InterpreterStatus {
    Running,
    Interrupt,
    Terminated,
    TerminatedWithException,
}

#[derive(Serialize, Deserialize)]
pub struct InterpreterOptions {
    pub keep_history: bool,
    pub history_size: usize,
}

impl InterpreterOptions {
    pub fn new() -> Self {
        Self {
            keep_history: false,
            history_size: 100,
        }
    }
}

impl Default for InterpreterOptions {
    fn default() -> Self {
        Self::new()
    }
}

/// A breakpoint: a Source line of a File, which is a [`Location`] without the
/// columns.
///
/// A breakpoint is set on a whole line, so it carries no column; and it names
/// its File, because a Program is assembled from several of them and two Files
/// both have a line 12. The Interpreter turns them into the addresses of the
/// instructions those lines assembled to.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Breakpoint {
    /// The root-relative path of the File, as a [`Location`] writes it.
    pub file: String,
    /// 0-based index of the Source line.
    pub line: usize,
}

impl Breakpoint {
    /// A breakpoint on a line of a File.
    pub fn new(file: impl Into<String>, line: usize) -> Self {
        Self {
            file: file.into(),
            line,
        }
    }
}

/// Writes the initial contents of memory: every run of bytes the Program
/// carries, and nothing where it only reserves room.
///
/// `ds` reserves memory and writes nothing to it (`Directives/ds.htm`), so a
/// [`MemoryContent::Reserved`] run leaves the fill the Interpreter starts with
/// where it is.
fn prepare_memory(memory: &mut Memory, program: &Program) -> RuntimeResult<()> {
    for run in program.memory() {
        match &run.content {
            MemoryContent::Bytes { bytes } => memory.write_bytes(run.address, bytes)?,
            MemoryContent::Reserved { .. } => {}
        }
    }
    Ok(())
}

#[wasm_bindgen]
pub struct Interpreter {
    memory: Memory,
    cpu: Cpu,
    pc: usize,
    /// What is being run. The instructions are looked up in it by address
    /// rather than kept in memory, which is less like a real processor and
    /// keeps a wild write from rewriting the program.
    program: Program,
    debugger: Debugger,
    keep_history: bool,
    /// The address of the instruction being executed, and after the step, of
    /// the one that has just run. It is 0 before the first step, when none has.
    current_instruction_address: usize,
    /// One past the last byte of the last instruction: a program counter that
    /// reaches it has walked off the bottom of the program.
    end_address: usize,
    current_interrupt: Option<Interrupt>,
    status: InterpreterStatus,
}

impl Interpreter {
    /// An Interpreter ready to run `program` from its Entry point.
    ///
    /// Memory is the 16 MB of the address space, filled as [`Memory::new`]
    /// leaves it and then written with the Program's initial contents; the
    /// stack pointer starts at the top of it. A Program with no instruction in
    /// it, or whose Entry point is past the last of them, is
    /// [`Terminated`](InterpreterStatus::Terminated) before it starts.
    pub fn new(program: Program, options: Option<InterpreterOptions>) -> Self {
        let sp = 0x01000000;
        let start = program.entry();
        let end = program.end_address();
        let options = options.unwrap_or_default();
        let mut memory = Memory::new();
        if let Err(e) = prepare_memory(&mut memory, &program) {
            //the Layout refuses to place anything past the address space, so this cannot
            //happen for a Program the Assembler built
            panic!("Error preparing memory: {:?}", e);
        }
        let mut interpreter = Self {
            memory,
            cpu: Cpu::new(),
            pc: start,
            end_address: end,
            keep_history: options.keep_history,
            current_instruction_address: 0,
            debugger: Debugger::new(options.history_size, program.symbols()),
            current_interrupt: None,
            status: if !program.is_empty() && start < end {
                InterpreterStatus::Running
            } else {
                InterpreterStatus::Terminated
            },
            program,
        };
        interpreter.cpu.a_reg[7].store_long(sp as u32);
        interpreter
    }

    /// The Program being run.
    pub fn get_program(&self) -> &Program {
        &self.program
    }

    #[inline(always)]
    pub fn get_cpu(&self) -> &Cpu {
        &self.cpu
    }

    #[inline(always)]
    pub fn get_memory(&self) -> &Memory {
        &self.memory
    }

    #[inline(always)]
    pub fn get_pc(&self) -> usize {
        self.pc
    }

    #[inline(always)]
    pub fn get_status(&self) -> &InterpreterStatus {
        &self.status
    }
    fn set_status(&mut self, status: InterpreterStatus) {
        // ignore if status is the same, helps with the assertion below
        if status == self.status {
            return;
        }
        match self.status {
            InterpreterStatus::Terminated | InterpreterStatus::TerminatedWithException => {
                panic!("Cannot change status of terminated program")
            }
            _ => self.status = status,
        }
    }
    /// The whole status register: the system byte, then the condition codes.
    ///
    /// It is `$2700` before a program has run, as in EASy68K, and its high byte
    /// has no effect on anything (the design record, "Instructions").
    #[inline(always)]
    pub fn get_sr(&self) -> u16 {
        self.cpu.get_sr()
    }

    /// Sets the whole status register, condition codes included.
    pub fn set_sr(&mut self, value: u16) {
        self.cpu.set_sr(value);
    }

    /// Ends the run with an exception and answers the error that says why.
    ///
    /// A 68000 would push a stack frame and jump through the vector of
    /// `SIMHELP/Exceptions.htm`; s68k has neither, so `chk`, `trapv` and
    /// `illegal` stop the program where an address error already stopped it —
    /// [`InterpreterStatus::TerminatedWithException`] and a
    /// [`RuntimeError`] naming the instruction — and the Location of the line
    /// is the one the step recorded.
    fn end_with_an_exception(&mut self, error: RuntimeError) -> RuntimeError {
        self.set_status(InterpreterStatus::TerminatedWithException);
        error
    }

    pub fn get_flags_as_array(&self) -> Vec<u8> {
        vec![
            self.cpu.ccr.contains(Flags::Carry) as u8,
            self.cpu.ccr.contains(Flags::Overflow) as u8,
            self.cpu.ccr.contains(Flags::Zero) as u8,
            self.cpu.ccr.contains(Flags::Negative) as u8,
            self.cpu.ccr.contains(Flags::Extend) as u8,
        ]
    }
    pub fn has_terminated(&self) -> bool {
        self.status == InterpreterStatus::Terminated
            || self.status == InterpreterStatus::TerminatedWithException
    }

    /// Whether the program counter has walked off the bottom of the program.
    ///
    /// The program ends one byte past the last instruction, which is that
    /// instruction's address plus the size stored with it.
    #[inline(always)]
    pub fn has_reached_bottom(&self) -> bool {
        self.pc >= self.end_address
    }

    pub fn step(&mut self) -> RuntimeResult<InterpreterStatus> {
        if self.keep_history {
            self.debugger
                .add_step(ExecutionStep::new(self.pc, self.cpu.ccr, self.cpu.get_sr()));
        }
        self.current_instruction_address = self.pc;
        let instruction = self
            .get_instruction_at(self.pc)
            .map(|i| (i.size, i.instruction));
        match instruction {
            _ if self.status == InterpreterStatus::Terminated
                || self.status == InterpreterStatus::TerminatedWithException =>
            {
                Err(RuntimeError::Raw(
                    "Attempt to run terminated program".to_string(),
                ))
            }
            _ if self.status == InterpreterStatus::Interrupt => Err(RuntimeError::Raw(
                "Attempted to step while interrupt is pending".to_string(),
            )),

            Some((size, ins)) => {
                if self.keep_history {
                    //cloned only when a history is kept: a Location holds the path of its File,
                    //and a step nobody can undo should not pay for it
                    let location = self.get_instruction_at(self.pc).map(|i| i.location.clone());
                    self.debugger.set_location(location);
                }
                self.increment_pc(size);
                self.execute_instruction(&ins)?;
                let status = self.get_status();
                //TODO not sure if doing this before or after running the instruction
                if self.has_reached_bottom() && *status != InterpreterStatus::Interrupt {
                    self.set_status(InterpreterStatus::Terminated);
                }
                if self.keep_history {
                    self.debugger.set_new_ccr(self.cpu.ccr);
                    self.debugger.set_new_sr(self.cpu.get_sr());
                }
                Ok(self.status)
            }
            None if self.pc < self.end_address => {
                self.set_status(InterpreterStatus::TerminatedWithException);
                Err(RuntimeError::OutOfBounds(format!(
                    "Invalid instruction address: {}",
                    self.pc,
                )))
            }
            None => {
                self.set_status(InterpreterStatus::TerminatedWithException);
                Err(RuntimeError::Raw("Program has terminated".to_string()))
            }
        }
    }
    pub fn get_pretty_call_stack(&self) -> Vec<PrettyStackFrame> {
        self.debugger.to_call_stack()
    }
    pub fn undo(&mut self) -> RuntimeResult<ExecutionStep> {
        match self.debugger.pop_step() {
            Some(step) => {
                self.pc = step.get_pc();
                //the whole status register, which is the condition codes and the system byte
                //`move #n,sr` and its kind can have changed
                self.cpu.set_sr(step.get_sr());
                //doing from right to left because mutations are added from left to right
                for mutation in step.get_mutations().iter().rev() {
                    match mutation {
                        MutationOperation::WriteRegister {
                            register,
                            old,
                            size: _,
                        } => match register {
                            RegisterOperand::Address(reg) => {
                                self.cpu.a_reg[*reg as usize].store_long(*old)
                            }
                            RegisterOperand::Data(reg) => {
                                self.cpu.d_reg[*reg as usize].store_long(*old)
                            }
                        },
                        MutationOperation::WriteMemory { address, old, size } => {
                            self.memory.write_size(*address, *size, *old)?;
                        }
                        MutationOperation::WriteMemoryBytes { address, old } => {
                            self.memory.write_bytes(*address, old)?;
                        }
                        MutationOperation::PopCall { to, from } => {
                            //try to get the address of the function that popped the call: the
                            //return address is one past the instruction that called it, and each
                            //instruction stores how many bytes it takes up
                            let ins = self.program.instruction_ending_at(*to);
                            let callee_address = match ins {
                                Some(ins) => match &ins.instruction {
                                    Instruction::BSR(address) => *address as usize,
                                    Instruction::JSR(operand) => {
                                        self.get_operand_address(&operand.clone())? as usize
                                    }
                                    _ => 0,
                                },
                                None => 0,
                            };
                            self.debugger.push_call(
                                callee_address,
                                *from,
                                self.cpu.get_register_values(),
                            );
                        }
                        MutationOperation::PushCall { to: _, from: _ } => {
                            self.debugger.pop_call();
                        }
                    }
                }
                Ok(step)
            }
            None => Err(RuntimeError::Raw("No more steps to undo".to_string())),
        }
    }
    pub fn answer_interrupt(&mut self, interrupt_result: InterruptResult) -> RuntimeResult<()> {
        match interrupt_result {
            InterruptResult::DisplayNumber
            | InterruptResult::DisplayNumberInBase
            | InterruptResult::DisplayStringWithCRLF
            | InterruptResult::DisplayStringWithoutCRLF
            | InterruptResult::DisplayChar
            | InterruptResult::Delay
            | InterruptResult::SetPenColor
            | InterruptResult::SetFillColor
            | InterruptResult::DrawPixel
            | InterruptResult::DrawLine
            | InterruptResult::DrawLineTo
            | InterruptResult::MoveTo
            | InterruptResult::DrawRectangle
            | InterruptResult::DrawEllipse
            | InterruptResult::FloodFill
            | InterruptResult::DrawUnfilledRectangle
            | InterruptResult::DrawUnfilledEllipse
            | InterruptResult::SetDrawingMode
            | InterruptResult::SetPenWidth
            | InterruptResult::Repaint
            | InterruptResult::DrawText
            | InterruptResult::SetScreenSize
            | InterruptResult::SetScreenMode
            | InterruptResult::ClearScreen
            | InterruptResult::SetTextCursorPosition
            | InterruptResult::SetSimulatorShortcuts
            | InterruptResult::DisplaySignedNumberInField
            | InterruptResult::DisplayStringAndNumber => {}
            InterruptResult::ReadKeyboardString(str) => {
                let safe_len = std::cmp::min(str.len(), 80);
                let safe_str_bytes = &str.as_bytes()[..safe_len];
                let mut buffer = Vec::with_capacity(safe_len + 1);
                buffer.extend_from_slice(safe_str_bytes);
                buffer.push(0);
                let address = self.cpu.a_reg[1].get_long() as usize;
                self.set_memory_bytes(address, &buffer)?;
                self.set_register_value(RegisterOperand::Data(1), safe_len as u32, Size::Word);
            }
            InterruptResult::ReadNumber(num) => {
                self.set_register_value(RegisterOperand::Data(1), num as u32, Size::Long);
            }
            InterruptResult::ReadChar(char) => {
                self.set_register_value(RegisterOperand::Data(1), char as u8 as u32, Size::Byte);
            }
            InterruptResult::GetTime(time) => {
                self.set_register_value(RegisterOperand::Data(1), time, Size::Long);
            }
            InterruptResult::Terminate => {
                self.set_status(InterpreterStatus::Terminated);
            }
            InterruptResult::GetPixelColor(color) => {
                self.set_register_value(RegisterOperand::Data(0), color, Size::Long);
            }
            InterruptResult::DisplayStringAndReadNumber(num) => {
                self.set_register_value(RegisterOperand::Data(1), num as u32, Size::Long);
            }
            InterruptResult::CheckKeyboardInput(has_input) => {
                self.set_register_value(RegisterOperand::Data(1), has_input as u32, Size::Byte);
            }
            InterruptResult::GetKeyState(state) => {
                let value = match state {
                    //EASy68K answers a key state with $FF or $00 in the byte the key code was given in
                    KeyStateResult::Keys(keys) => keys.iter().fold(0u32, |acc, down| {
                        (acc << 8) | if *down { 0xFF } else { 0x00 }
                    }),
                    KeyStateResult::LastKeys { up, down } => ((up as u32) << 16) | down as u32,
                };
                self.set_register_value(RegisterOperand::Data(1), value, Size::Long);
            }
            InterruptResult::ReadMouse { flags, x, y } => {
                self.set_register_value(RegisterOperand::Data(0), flags as u32, Size::Long);
                self.set_register_value(
                    RegisterOperand::Data(1),
                    ((y as u32) << 16) | x as u32,
                    Size::Long,
                );
            }
            InterruptResult::GetPenPosition(x, y) => {
                self.set_register_value(RegisterOperand::Data(1), x as u32, Size::Word);
                self.set_register_value(RegisterOperand::Data(2), y as u32, Size::Word);
            }
            InterruptResult::GetScreenSize(width, height) => {
                self.set_register_value(
                    RegisterOperand::Data(1),
                    (width << 16) | (height & 0xFFFF),
                    Size::Long,
                );
            }
            InterruptResult::GetTextCursorPosition(column, row) => {
                self.set_register_value(
                    RegisterOperand::Data(1),
                    ((column & 0xFF) << 8) | (row & 0xFF),
                    Size::Word,
                );
            }
        };
        self.current_interrupt = None;
        //edge case if the last instruction is an interrupt
        self.status = if self.has_reached_bottom() {
            InterpreterStatus::Terminated
        } else {
            InterpreterStatus::Running
        };
        Ok(())
    }
    #[inline(always)]
    fn increment_pc(&mut self, amount: usize) {
        self.pc += amount;
    }

    #[inline(always)]
    pub fn get_sp(&self) -> usize {
        self.cpu.a_reg[7].get_long() as usize
    }
    #[inline(always)]
    pub fn set_sp(&mut self, sp: usize) {
        self.set_register_value(RegisterOperand::Address(7), sp as u32, Size::Long);
    }
    /// The instruction laid out at `address`, if one is.
    #[inline(always)]
    pub fn get_instruction_at(&self, address: usize) -> Option<&AssembledInstruction> {
        self.program.instruction_at(address)
    }

    /// The address of the instruction being executed, and after the step, of
    /// the one that has just run. It is 0 before the first step.
    #[inline(always)]
    pub fn get_current_instruction_address(&self) -> usize {
        self.current_instruction_address
    }

    /// Where in the source the instruction about to run was written, if the
    /// program counter is on one.
    ///
    /// This is what the editor highlights while a program is stopped: a
    /// [`Location`] and not a line number, because a Program is assembled from
    /// several Files.
    pub fn get_current_location(&self) -> Option<&Location> {
        self.program
            .instruction_at(self.pc)
            .map(|instruction| &instruction.location)
    }
    pub fn get_current_interrupt(&self) -> RuntimeResult<Interrupt> {
        match &self.current_interrupt {
            Some(interrupt) => Ok(interrupt.clone()),
            None => Err(RuntimeError::Raw("No interrupt pending".to_string())),
        }
    }
    fn execute_instruction(&mut self, ins: &Instruction) -> RuntimeResult<()> {
        match ins {
            Instruction::MOVE(source, dest, size) => {
                let source_value = self.get_operand_value(source, *size, Used::Once)?;
                self.set_logic_flags(source_value, *size);
                self.store_operand_value(dest, source_value, *size, Used::Once)?;
            }
            Instruction::MOVEA(source, dest, size) => {
                let source_value = self.get_operand_value(source, *size, Used::Once)?;
                let source_value = sign_extend_to_long(source_value, *size) as u32;
                self.set_register_value(*dest, source_value, Size::Long);
            }
            Instruction::MOVEQ(value, dest) => {
                let value = sign_extend_to_long(*value as u32, Size::Byte) as u32;
                self.set_logic_flags(value, Size::Long);
                self.set_register_value(*dest, value, Size::Long);
            }
            Instruction::MOVEM {
                registers_mask,
                direction,
                target,
                size,
            } => {
                let addr = self.get_operand_address(target)?;
                let post_addr = match target {
                    Operand::PostIndirect(_) => {
                        if *direction != TargetDirection::FromMemory {
                            return Err(RuntimeError::Raw(
                                "MOVEM to postindirect not allowed".to_string(),
                            ));
                        }
                        self.move_memory_to_registers(addr as usize, *size, *registers_mask)?
                    }
                    Operand::PreIndirect(_) => {
                        if *direction != TargetDirection::ToMemory {
                            return Err(RuntimeError::Raw(
                                "MOVEM from preindirect not allowed".to_string(),
                            ));
                        }
                        self.move_registers_to_memory_reverse(
                            addr as usize,
                            *size,
                            *registers_mask,
                        )?
                    }
                    _ => match direction {
                        TargetDirection::ToMemory => {
                            self.move_registers_to_memory(addr as usize, *size, *registers_mask)?
                        }
                        TargetDirection::FromMemory => {
                            self.move_memory_to_registers(addr as usize, *size, *registers_mask)?
                        }
                    },
                };
                match target {
                    Operand::PostIndirect(reg) | Operand::PreIndirect(reg) => {
                        self.set_register_value(
                            RegisterOperand::Address(*reg),
                            post_addr,
                            Size::Long,
                        );
                    }
                    _ => {}
                }
            }
            Instruction::SUB(source, dest, size) => {
                let source_value = self.get_operand_value(source, *size, Used::Once)?;
                let dest_value = self.get_operand_value(dest, *size, Used::Twice)?;
                let (result, carry) = overflowing_sub_sized(dest_value, source_value, *size);
                let overflow = has_sub_overflowed(dest_value, source_value, result, *size);
                self.set_compare_flags(result, *size, carry, overflow);
                self.set_flag(Flags::Extend, carry);
                self.store_operand_value(dest, result, *size, Used::Twice)?;
            }
            Instruction::SUBA(source, dest, size) => {
                let source_value =
                    sign_extend_to_long(self.get_operand_value(source, *size, Used::Once)?, *size)
                        as u32;
                let dest_value = self.get_register_value(*dest, Size::Long);
                let (result, _) = overflowing_sub_sized(dest_value, source_value, Size::Long);
                self.set_register_value(*dest, result, Size::Long);
            }
            Instruction::SUBQ(value, dest, size) => {
                match dest {
                    Operand::Register(RegisterOperand::Address(reg)) => {
                        //if the destination is an address register, it is always treated as long and doesn't set the flags
                        match size {
                            Size::Byte => {
                                return Err(RuntimeError::Raw(
                                    "SUBQ.B not allowed on address register".to_string(),
                                ));
                            }
                            Size::Word | Size::Long => {
                                let dest_value = self
                                    .get_register_value(RegisterOperand::Address(*reg), Size::Long);
                                let (result, _) =
                                    overflowing_sub_sized(dest_value, *value as u32, Size::Long);
                                self.set_register_value(
                                    RegisterOperand::Address(*reg),
                                    result,
                                    Size::Long,
                                );
                            }
                        }
                    }
                    _ => {
                        let source_value = *value as u32;
                        let dest_value = self.get_operand_value(dest, *size, Used::Twice)?;
                        let (result, carry) =
                            overflowing_sub_sized(dest_value, source_value, *size);
                        let overflow = has_sub_overflowed(dest_value, source_value, result, *size);
                        self.set_compare_flags(result, *size, carry, overflow);
                        self.set_flag(Flags::Extend, carry);
                        self.store_operand_value(dest, result, *size, Used::Twice)?;
                    }
                }
            }
            Instruction::SUBI(source_value, dest, size) => {
                let dest_value = self.get_operand_value(dest, *size, Used::Twice)?;
                let (result, carry) = overflowing_sub_sized(dest_value, *source_value, *size);
                let overflow = has_sub_overflowed(dest_value, *source_value, result, *size);
                self.set_compare_flags(result, *size, carry, overflow);
                self.set_flag(Flags::Extend, carry);
                self.store_operand_value(dest, result, *size, Used::Twice)?;
            }
            Instruction::ADD(source, dest, size) => {
                let source_value = self.get_operand_value(source, *size, Used::Once)?;
                let dest_value = self.get_operand_value(dest, *size, Used::Twice)?;
                let (result, carry) = overflowing_add_sized(dest_value, source_value, *size);
                let overflow = has_add_overflowed(dest_value, source_value, result, *size);
                self.set_compare_flags(result, *size, carry, overflow);
                self.set_flag(Flags::Extend, carry);
                self.store_operand_value(dest, result, *size, Used::Twice)?;
            }
            // `addx` and `subx` add or subtract the extend flag as well, and
            // their flags are the multi-precision ones: X and C alike, N and V
            // from the result, and Z cleared when the result is not zero and
            // left alone when it is (`Reference/68ks5e.htm`,
            // `Reference/68ks5v.htm`). The source is read before the
            // destination, so `addx -(a0),-(a1)` decrements `a0` first, as the
            // 68000 does.
            Instruction::ADDX(source, dest, size) => {
                let source_value = self.get_operand_value(source, *size, Used::Once)?;
                let dest_value = self.get_operand_value(dest, *size, Used::Twice)?;
                let extend = self.cpu.ccr.contains(Flags::Extend);
                let (result, carry) = add_with_extend(dest_value, source_value, extend, *size);
                let overflow = has_add_overflowed(dest_value, source_value, result, *size);
                self.store_operand_value(dest, result, *size, Used::Twice)?;
                self.set_extended_arithmetic_flags(result, *size, carry, overflow);
            }
            Instruction::SUBX(source, dest, size) => {
                let source_value = self.get_operand_value(source, *size, Used::Once)?;
                let dest_value = self.get_operand_value(dest, *size, Used::Twice)?;
                let extend = self.cpu.ccr.contains(Flags::Extend);
                let (result, borrow) = sub_with_extend(dest_value, source_value, extend, *size);
                let overflow = has_sub_overflowed(dest_value, source_value, result, *size);
                self.store_operand_value(dest, result, *size, Used::Twice)?;
                self.set_extended_arithmetic_flags(result, *size, borrow, overflow);
            }
            Instruction::ADDA(source, dest, size) => {
                let source_value =
                    sign_extend_to_long(self.get_operand_value(source, *size, Used::Once)?, *size)
                        as u32;
                let dest_value = self.get_register_value(*dest, Size::Long);
                let (result, _) = overflowing_add_sized(dest_value, source_value, Size::Long);
                self.set_register_value(*dest, result, Size::Long);
            }
            Instruction::ADDI(source_value, dest, size) => {
                let dest_value = self.get_operand_value(dest, *size, Used::Twice)?;
                let (result, carry) = overflowing_add_sized(dest_value, *source_value, *size);
                let overflow = has_add_overflowed(dest_value, *source_value, result, *size);
                self.set_compare_flags(result, *size, carry, overflow);
                self.set_flag(Flags::Extend, carry);
                self.store_operand_value(dest, result, *size, Used::Twice)?;
            }
            Instruction::ADDQ(value, dest, size) => {
                match dest {
                    Operand::Register(RegisterOperand::Address(reg)) => {
                        //if the destination is an address register, it is always treated as long and doesn't set the flags
                        match size {
                            Size::Byte => {
                                return Err(RuntimeError::Raw(
                                    "ADDQ.B not allowed on address register".to_string(),
                                ));
                            }
                            Size::Word | Size::Long => {
                                let dest_value = self
                                    .get_register_value(RegisterOperand::Address(*reg), Size::Long);
                                let (result, _) =
                                    overflowing_add_sized(dest_value, *value as u32, Size::Long);
                                self.set_register_value(
                                    RegisterOperand::Address(*reg),
                                    result,
                                    Size::Long,
                                );
                            }
                        }
                    }
                    _ => {
                        let source_value = *value as u32;
                        let dest_value = self.get_operand_value(dest, *size, Used::Twice)?;
                        let (result, carry) =
                            overflowing_add_sized(dest_value, source_value, *size);
                        let overflow = has_add_overflowed(dest_value, source_value, result, *size);
                        self.set_compare_flags(result, *size, carry, overflow);
                        self.set_flag(Flags::Extend, carry);
                        self.store_operand_value(dest, result, *size, Used::Twice)?;
                    }
                }
            }

            Instruction::MULx(source, dest, sign) => {
                let source_value = self.get_operand_value(source, Size::Word, Used::Once)?;
                let dest_value =
                    get_value_sized(self.get_register_value(*dest, Size::Long), Size::Word);
                let result = match sign {
                    Sign::Signed => {
                        ((((dest_value as u16) as i16) as i64)
                            * (((source_value as u16) as i16) as i64))
                            as u64
                    }
                    Sign::Unsigned => dest_value as u64 * source_value as u64,
                };
                self.set_compare_flags(result as u32, Size::Long, false, false);
                self.set_register_value(*dest, result as u32, Size::Long);
            }

            Instruction::BRA(address) => {
                //instead of using the absolute address, the original language uses pc + 2 + offset
                self.pc = *address as usize;
            }
            Instruction::BSR(address) => {
                if self.keep_history {
                    let old_address = self.get_sp().wrapping_sub(4);
                    let old_value = self.memory.read_long(old_address)?;
                    self.debugger.add_mutation(MutationOperation::WriteMemory {
                        address: old_address,
                        old: old_value,
                        size: Size::Long,
                    });
                    self.debugger.add_mutation(MutationOperation::PushCall {
                        to: *address as usize,
                        //the pc is incremented before the instruction is executed, so the address
                        //of the call is the one the step recorded
                        from: self.current_instruction_address,
                    });
                }
                let new_sp = self
                    .memory
                    .push(&MemoryCell::Long(self.pc as u32), self.get_sp())?;
                self.set_sp(new_sp);
                let caller_address = self.pc;
                self.pc = *address as usize;
                self.debugger
                    .push_call(self.pc, caller_address, self.cpu.get_register_values());
            }
            Instruction::JSR(source) => {
                let address = self.get_operand_address(source)?;
                if self.keep_history {
                    let old_address = self.get_sp().wrapping_sub(4);
                    let old_value = self.memory.read_long(old_address)?;
                    self.debugger.add_mutation(MutationOperation::WriteMemory {
                        address: old_address,
                        old: old_value,
                        size: Size::Long,
                    });
                    self.debugger.add_mutation(MutationOperation::PushCall {
                        to: address as usize,
                        from: self.current_instruction_address,
                    });
                }
                let new_sp = self
                    .memory
                    .push(&MemoryCell::Long(self.pc as u32), self.get_sp())?;
                self.set_sp(new_sp);
                let caller_address = self.pc;
                self.pc = address as usize;
                self.debugger
                    .push_call(self.pc, caller_address, self.cpu.get_register_values());
            }
            Instruction::JMP(op) => {
                let addr = self.get_operand_address(op)?;
                self.pc = addr as usize;
            }
            Instruction::LEA(source, dest) => {
                let addr = self.get_operand_address(source)?;
                self.set_register_value(*dest, addr, Size::Long);
            }
            Instruction::PEA(source) => {
                let addr = self.get_operand_address(source)?;
                if self.keep_history {
                    let old_value = self.memory.read_long(self.get_sp())?;
                    self.debugger.add_mutation(MutationOperation::WriteMemory {
                        address: self.get_sp().wrapping_sub(4),
                        old: old_value,
                        size: Size::Long,
                    })
                }
                let new_sp = self.memory.push(&MemoryCell::Long(addr), self.get_sp())?;
                self.set_sp(new_sp);
            }
            Instruction::BCHG(bit_source, dest) => {
                let bit = self.get_operand_value(bit_source, Size::Byte, Used::Once)?;
                let limited_bit = self.limit_bit_size(bit, dest)?;
                let size = match dest {
                    Operand::Register(_) => Ok(Size::Long),
                    _ => Ok(Size::Byte),
                }?;
                let source_value = self.get_operand_value(dest, size, Used::Twice)?;
                let mask = self.set_bit_test_flags(source_value, limited_bit, size);
                let source_value = (source_value & !mask) | (!(source_value & mask) & mask);
                self.store_operand_value(dest, source_value, size, Used::Twice)?;
            }
            Instruction::BCLR(bit_source, dest) => {
                let bit = self.get_operand_value(bit_source, Size::Byte, Used::Once)?;
                let limited_bit = self.limit_bit_size(bit, dest)?;
                let size = match dest {
                    Operand::Register(_) => Ok(Size::Long),
                    _ => Ok(Size::Byte),
                }?;
                let src_val = self.get_operand_value(dest, size, Used::Twice)?;
                let mask = self.set_bit_test_flags(src_val, limited_bit, size);
                let src_val = src_val & !mask;
                self.store_operand_value(dest, src_val, size, Used::Twice)?;
            }
            Instruction::BSET(bit_source, dest) => {
                let bit = self.get_operand_value(bit_source, Size::Byte, Used::Once)?;
                let size = match dest {
                    Operand::Register(_) => Ok(Size::Long),
                    _ => Ok(Size::Byte),
                }?;
                let limited_bit = self.limit_bit_size(bit, dest)?;
                let value = self.get_operand_value(dest, size, Used::Twice)?;
                let mask = self.set_bit_test_flags(value, limited_bit, size);
                let value = value | mask;
                self.store_operand_value(dest, value, size, Used::Twice)?;
            }

            Instruction::BTST(bit, op2) => {
                let bit = self.get_operand_value(bit, Size::Byte, Used::Once)?;
                let limited_bit = self.limit_bit_size(bit, op2)?;
                let size = match op2 {
                    Operand::Register(_) => Ok(Size::Long),
                    _ => Ok(Size::Byte),
                }?;
                let value = self.get_operand_value(op2, size, Used::Once)?;
                self.set_bit_test_flags(value, limited_bit, size);
            }
            Instruction::ASd(amount, dest, direction, size) => {
                let amount_value = self.get_operand_value(amount, *size, Used::Once)? % 64;
                let dest_value = self.get_operand_value(dest, *size, Used::Twice)?;
                let mut has_overflowed = false;
                let (mut value, mut msb) = (dest_value, false);
                let mut previous_msb = get_sign(value, *size);
                for _ in 0..amount_value {
                    (value, msb) = shift(direction, value, *size, true);
                    if get_sign(value, *size) != previous_msb {
                        has_overflowed = true;
                    }
                    previous_msb = get_sign(value, *size);
                }
                self.store_operand_value(dest, value, *size, Used::Twice)?;

                let carry = match direction {
                    ShiftDirection::Left => msb,
                    ShiftDirection::Right => {
                        if amount_value < size.to_bits() as u32 {
                            msb
                        } else {
                            false
                        }
                    }
                };
                self.set_logic_flags(value, *size);
                self.set_flag(Flags::Overflow, has_overflowed);
                if amount_value != 0 {
                    self.set_flag(Flags::Extend, carry);
                    self.set_flag(Flags::Carry, carry);
                } else {
                    self.set_flag(Flags::Carry, false);
                }
            }
            Instruction::LSd(amount_source, dest, direction, size) => {
                let amount = self.get_operand_value(amount_source, *size, Used::Once)? % 64;
                let (mut value, mut msb) =
                    (self.get_operand_value(dest, *size, Used::Twice)?, false);
                for _ in 0..amount {
                    (value, msb) = shift(direction, value, *size, false);
                }
                self.store_operand_value(dest, value, *size, Used::Twice)?;
                self.set_logic_flags(value, *size);
                self.set_flag(Flags::Overflow, false);
                if amount != 0 {
                    self.set_flag(Flags::Extend, msb);
                    self.set_flag(Flags::Carry, msb);
                } else {
                    self.set_flag(Flags::Carry, false);
                }
            }
            Instruction::ROd(amount, dest, direction, size) => {
                let count = self.get_operand_value(amount, *size, Used::Once)? % 64;
                let (mut value, mut carry) =
                    (self.get_operand_value(dest, *size, Used::Twice)?, false);
                for _ in 0..count {
                    (value, carry) = rotate(direction, value, *size);
                }
                self.store_operand_value(dest, value, *size, Used::Twice)?;
                self.set_logic_flags(value, *size);
                if carry {
                    self.set_flag(Flags::Carry, true);
                }
            }

            // A rotation through the extend flag: nine, seventeen or
            // thirty-three bits wide, so the bit that leaves the operand goes
            // to X and the bit X held comes in at the other end. With a count
            // of zero X is left alone and C answers it, which is the one place
            // a rotate's carry is not the bit it moved
            // (`Reference/68ks7g.htm`, `Reference/68ks7h.htm`).
            Instruction::ROXd(amount, dest, direction, size) => {
                let count = self.get_operand_value(amount, *size, Used::Once)? % 64;
                let mut value = self.get_operand_value(dest, *size, Used::Twice)?;
                let mut extend = self.cpu.ccr.contains(Flags::Extend);
                for _ in 0..count {
                    (value, extend) = rotate_with_extend(direction, value, *size, extend);
                }
                self.store_operand_value(dest, value, *size, Used::Twice)?;
                self.set_logic_flags(value, *size);
                self.set_flag(Flags::Extend, extend);
                self.set_flag(Flags::Carry, extend);
            }
            Instruction::AND(source, dest, size) => {
                let source_value = self.get_operand_value(source, *size, Used::Once)?;
                let dest_value = self.get_operand_value(dest, *size, Used::Twice)?;
                let result = get_value_sized(dest_value & source_value, *size);
                self.store_operand_value(dest, result, *size, Used::Twice)?;
                self.set_logic_flags(result, *size);
            }
            Instruction::OR(source, dest, size) => {
                let source_value = self.get_operand_value(source, *size, Used::Once)?;
                let dest_value = self.get_operand_value(dest, *size, Used::Twice)?;
                let result = get_value_sized(dest_value | source_value, *size);
                self.store_operand_value(dest, result, *size, Used::Twice)?;
                self.set_logic_flags(result, *size);
            }
            Instruction::EOR(source, dest, size) => {
                let source_value = self.get_operand_value(source, *size, Used::Once)?;
                let dest_value = self.get_operand_value(dest, *size, Used::Twice)?;
                let result = get_value_sized(dest_value ^ source_value, *size);
                self.store_operand_value(dest, result, *size, Used::Twice)?;
                self.set_logic_flags(result, *size);
            }
            Instruction::NOT(op, size) => {
                //watchout for the "!"
                let value = !self.get_operand_value(op, *size, Used::Twice)?;
                let value = get_value_sized(value, *size);
                self.store_operand_value(op, value, *size, Used::Twice)?;
                self.set_logic_flags(value, *size);
            }
            Instruction::ANDI(source_value, dest, size) => {
                let dest_value = self.get_operand_value(dest, *size, Used::Twice)?;
                let result = get_value_sized(dest_value & source_value, *size);
                self.store_operand_value(dest, result, *size, Used::Twice)?;
                self.set_logic_flags(result, *size);
            }
            Instruction::ORI(source_value, dest, size) => {
                let dest_value = self.get_operand_value(dest, *size, Used::Twice)?;
                let result = get_value_sized(dest_value | source_value, *size);
                self.store_operand_value(dest, result, *size, Used::Twice)?;
                self.set_logic_flags(result, *size);
            }
            Instruction::EORI(source_value, dest, size) => {
                let dest_value = self.get_operand_value(dest, *size, Used::Twice)?;
                let result = get_value_sized(dest_value ^ source_value, *size);
                self.store_operand_value(dest, result, *size, Used::Twice)?;
                self.set_logic_flags(result, *size);
            }
            Instruction::NEG(source, size) => {
                let original = self.get_operand_value(source, *size, Used::Twice)?;
                let (result, overflow) = overflowing_sub_signed_sized(0, original, *size);
                let carry = result != 0;
                self.store_operand_value(source, result, *size, Used::Twice)?;
                self.set_compare_flags(result, *size, carry, overflow);
                self.set_flag(Flags::Extend, carry);
            }
            // `negx` is `neg` with the extend flag taken away as well, and it
            // carries the same multi-precision Z rule
            // (`Reference/68ks5q.htm`, whose flag table reads "Z - Set if the
            // result is not zero, else unaffected": that sentence is the one
            // typing slip of the three pages, and `SUBX`'s own table on
            // `68ks5v.htm` — "Cleared if the result is not zero, else
            // unaffected" — is the rule the 68000 has and the one implemented
            // here).
            Instruction::NEGX(destination, size) => {
                let value = self.get_operand_value(destination, *size, Used::Twice)?;
                let extend = self.cpu.ccr.contains(Flags::Extend);
                let (result, borrow) = sub_with_extend(0, value, extend, *size);
                let overflow = has_sub_overflowed(0, value, result, *size);
                self.store_operand_value(destination, result, *size, Used::Twice)?;
                self.set_extended_arithmetic_flags(result, *size, borrow, overflow);
            }
            // The three binary coded decimal instructions. Each works on one
            // byte, carries the extend flag in, and leaves N and V exactly
            // where they were, because the help calls both of them undefined
            // (`Reference/68ks8e.htm`, `68ks8g.htm`, `68ks8f.htm`).
            Instruction::ABCD(source, dest) => {
                let source_value = self.get_operand_value(source, Size::Byte, Used::Once)?;
                let dest_value = self.get_operand_value(dest, Size::Byte, Used::Twice)?;
                let extend = self.cpu.ccr.contains(Flags::Extend);
                let (result, carry) = add_decimal(dest_value, source_value, extend);
                self.store_operand_value(dest, result, Size::Byte, Used::Twice)?;
                self.set_decimal_flags(result, carry);
            }
            Instruction::SBCD(source, dest) => {
                let source_value = self.get_operand_value(source, Size::Byte, Used::Once)?;
                let dest_value = self.get_operand_value(dest, Size::Byte, Used::Twice)?;
                let extend = self.cpu.ccr.contains(Flags::Extend);
                let (result, borrow) = subtract_decimal(dest_value, source_value, extend);
                self.store_operand_value(dest, result, Size::Byte, Used::Twice)?;
                self.set_decimal_flags(result, borrow);
            }
            // The tens complement is zero less the value, which is the
            // subtraction `sbcd` does from a destination of zero: "The tens
            // complement to 01 is 99" (`Reference/68ks8f.htm`).
            Instruction::NBCD(destination) => {
                let value = self.get_operand_value(destination, Size::Byte, Used::Twice)?;
                let extend = self.cpu.ccr.contains(Flags::Extend);
                let (result, borrow) = subtract_decimal(0, value, extend);
                self.store_operand_value(destination, result, Size::Byte, Used::Twice)?;
                self.set_decimal_flags(result, borrow);
            }
            Instruction::DIVx(source, dest, sign) => {
                let source_value = self.get_operand_value(source, Size::Word, Used::Once)?;
                if source_value == 0 {
                    return Err(RuntimeError::DivisionByZero);
                }
                let dest_value = self.get_register_value(*dest, Size::Long);
                let dest_value = get_value_sized(dest_value, Size::Long);
                let (remainder, quotient, has_overflowed) = match sign {
                    Sign::Signed => {
                        let dest_value = dest_value as i32;
                        let source_value = sign_extend_to_long(source_value, Size::Word);
                        let quotient = dest_value / source_value;
                        (
                            (dest_value % source_value) as u32,
                            quotient as u32,
                            quotient > i16::MAX as i32 || quotient < i16::MIN as i32,
                        )
                    }
                    Sign::Unsigned => {
                        let quotient = dest_value / source_value;
                        (
                            dest_value % source_value,
                            quotient,
                            (quotient & 0xFFFF0000) != 0,
                        )
                    }
                };
                if !has_overflowed {
                    self.set_compare_flags(quotient, Size::Word, false, false);
                    self.set_register_value(
                        *dest,
                        (remainder << 16) | (0xFFFF & quotient),
                        Size::Long,
                    );
                } else {
                    self.set_flag(Flags::Carry, false);
                    self.set_flag(Flags::Overflow, true);
                }
            }
            Instruction::EXG(reg1, reg2) => {
                let reg1_value = self.get_register_value(*reg1, Size::Long);
                let reg2_value = self.get_register_value(*reg2, Size::Long);
                self.set_register_value(*reg1, reg2_value, Size::Long);
                self.set_register_value(*reg2, reg1_value, Size::Long);
            }
            Instruction::EXT(reg, from, to) => {
                let input = get_value_sized(self.get_register_value(*reg, Size::Long), *from);
                let result = match (from, to) {
                    (Size::Byte, Size::Word) => ((((input as u8) as i8) as i16) as u16) as u32,
                    (Size::Word, Size::Long) => (((input as u16) as i16) as i32) as u32,
                    (Size::Byte, Size::Long) => (((input as u8) as i8) as i32) as u32,
                    _ => {
                        return Err(RuntimeError::Raw(
                            "Invalid size for EXT instruction".to_string(),
                        ));
                    }
                };
                self.set_register_value(*reg, result, *to);
                self.set_logic_flags(result, *to);
            }
            Instruction::SWAP(reg) => {
                let value = self.get_register_value(*reg, Size::Long);
                let new_value = ((value & 0x0000FFFF) << 16) | ((value & 0xFFFF0000) >> 16);
                self.set_register_value(*reg, new_value, Size::Long);
                self.set_logic_flags(new_value, Size::Long);
            }
            Instruction::TST(source, size) => {
                let value = self.get_operand_value(source, *size, Used::Once)?;
                self.set_logic_flags(value, *size);
            }
            Instruction::CMP(source, dest, size) => {
                //TODO revise this, should i strict it to only data registers?
                let source_value = self.get_operand_value(source, *size, Used::Once)?;
                let dest_value = self.get_register_value(*dest, *size);
                let (result, carry) = overflowing_sub_sized(dest_value, source_value, *size);
                let overflow = has_sub_overflowed(dest_value, source_value, result, *size);
                self.set_compare_flags(result, *size, carry, overflow);
            }
            Instruction::CMPA(source, dest, size) => {
                let source_value =
                    sign_extend_to_long(self.get_operand_value(source, *size, Used::Once)?, *size)
                        as u32;
                let dest_value = self.get_register_value(*dest, Size::Long);
                let (result, carry) = overflowing_sub_sized(dest_value, source_value, Size::Long);
                let overflow = has_sub_overflowed(dest_value, source_value, result, Size::Long);
                self.set_compare_flags(result, Size::Long, carry, overflow);
            }
            Instruction::CMPI(source_value, dest, size) => {
                let dest_value = self.get_operand_value(dest, *size, Used::Once)?;
                let (result, carry) = overflowing_sub_sized(dest_value, *source_value, *size);
                let overflow = has_sub_overflowed(dest_value, *source_value, result, *size);
                self.set_compare_flags(result, *size, carry, overflow);
            }
            Instruction::CMPM(source, dest, size) => {
                let source_value = self.get_operand_value(source, *size, Used::Once)?;
                let dest_value = self.get_operand_value(dest, *size, Used::Once)?;
                let (result, carry) = overflowing_sub_sized(dest_value, source_value, *size);
                let overflow = has_sub_overflowed(dest_value, source_value, result, *size);
                self.set_compare_flags(result, *size, carry, overflow);
            }
            Instruction::Bcc(address, condition) => {
                if self.get_condition_value(condition) {
                    self.pc = *address as usize;
                }
            }
            Instruction::CLR(dest, size) => {
                self.store_operand_value(dest, 0, *size, Used::Once)?;
                let extend = self.get_flag(Flags::Extend);
                self.cpu.ccr.clear();
                self.set_flag(Flags::Zero, true);
                self.set_flag(Flags::Extend, extend);
            }
            Instruction::Scc(op, condition) => {
                if self.get_condition_value(condition) {
                    self.store_operand_value(op, 0xFF, Size::Byte, Used::Once)?;
                } else {
                    self.store_operand_value(op, 0x00, Size::Byte, Used::Once)?;
                }
            }
            Instruction::DBcc(reg, address, cond) => {
                if !self.get_condition_value(cond) {
                    let next = (self.get_register_value(*reg, Size::Word) as i16).wrapping_sub(1);
                    self.set_register_value(*reg, next as u32, Size::Word);
                    if next != -1 {
                        self.pc = *address as usize;
                    }
                }
            }
            Instruction::LINK(reg, offset) => {
                let sp = self.get_sp().wrapping_sub(4);
                self.set_sp(sp);
                let value = self.get_register_value(*reg, Size::Long);
                self.set_memory_value(sp, Size::Long, value)?;
                self.set_register_value(*reg, sp as u32, Size::Long);
                self.set_sp((sp as i32).wrapping_add(*offset as i32) as usize)
            }
            Instruction::UNLK(reg) => {
                let value = self.get_register_value(*reg, Size::Long);
                let (value, new_sp) = self.memory.pop(Size::Long, value as usize)?;
                self.set_register_value(*reg, value.get_long(), Size::Long);
                self.set_sp(new_sp);
            }
            Instruction::NOP => {}
            // `simhalt` is the EASy68K directive that halts the simulator
            // (`Directives/simhalt.htm`): the run ends where the Terminate task
            // ends it and no register is touched. EASy68K's Pause button lets a
            // run carry on from the instruction after it; s68k does not offer
            // that, and a terminated Interpreter stays terminated.
            Instruction::SIMHALT => self.set_status(InterpreterStatus::Terminated),
            Instruction::RTS => {
                let (value, new_sp) = self.memory.pop(Size::Long, self.get_sp())?;
                if self.keep_history {
                    self.debugger.add_mutation(MutationOperation::PopCall {
                        to: value.get_long() as usize,
                        from: self.current_instruction_address,
                    })
                }
                self.set_sp(new_sp);
                self.pc = value.get_long() as usize;
                self.debugger.pop_call();
            }
            Instruction::TRAP(value) => match value {
                15 => {
                    let task = self.cpu.d_reg[0].get_byte();
                    let interrupt = self.get_trap(task)?;

                    // TODO should i check if the interrupt is the Terminate one or if it terminated?
                    match &interrupt {
                        Interrupt::Terminate => self.set_status(InterpreterStatus::Terminated),
                        _ => self.set_status(InterpreterStatus::Interrupt),
                    }
                    self.current_interrupt = Some(interrupt);
                }
                _ => {
                    return Err(RuntimeError::Raw(format!(
                        "Unknown trap: {}, only IO with #15 allowed",
                        value
                    )));
                }
            },
            Instruction::MOVEP {
                direction,
                size,
                register,
                target,
            } => {
                //the bytes of the register go to every second address, most significant first
                //(`Reference/68ks4g.htm`); the displacement operand is the only mode `movep` takes
                let address = self.get_operand_address(target)? as usize;
                let count = size.to_bytes();
                match direction {
                    TargetDirection::ToMemory => {
                        let value = self.get_register_value(*register, *size);
                        for index in 0..count {
                            let byte = (value >> (8 * (count - 1 - index))) & 0xFF;
                            self.set_memory_value(address + index * 2, Size::Byte, byte)?;
                        }
                    }
                    TargetDirection::FromMemory => {
                        let mut value = 0u32;
                        for index in 0..count {
                            value =
                                (value << 8) | self.memory.read_byte(address + index * 2)? as u32;
                        }
                        self.set_register_value(*register, value, *size);
                    }
                }
            }
            //`move <ea>,ccr` reads a word and keeps its low byte; the flags are the byte moved
            //and not the result of moving it (`Reference/68ks4d.htm`)
            Instruction::MOVEtoCCR(source) => {
                let value = self.get_operand_value(source, Size::Word, Used::Once)?;
                self.cpu.set_ccr_byte(value as u8);
            }
            Instruction::MOVEtoSR(source) => {
                let value = self.get_operand_value(source, Size::Word, Used::Once)?;
                self.cpu.set_sr(value as u16);
            }
            Instruction::MOVEfromSR(destination) => {
                let value = self.cpu.get_sr() as u32;
                self.store_operand_value(destination, value, Size::Word, Used::Once)?;
            }
            Instruction::MOVEfromCCR(destination) => {
                let value = self.cpu.get_ccr_byte() as u32;
                self.store_operand_value(destination, value, Size::Word, Used::Once)?;
            }
            Instruction::ANDItoCCR(value) => {
                let byte = self.cpu.get_ccr_byte() & value;
                self.cpu.set_ccr_byte(byte);
            }
            Instruction::ORItoCCR(value) => {
                let byte = self.cpu.get_ccr_byte() | value;
                self.cpu.set_ccr_byte(byte);
            }
            Instruction::EORItoCCR(value) => {
                let byte = self.cpu.get_ccr_byte() ^ value;
                self.cpu.set_ccr_byte(byte);
            }
            Instruction::ANDItoSR(value) => {
                let word = self.cpu.get_sr() & value;
                self.cpu.set_sr(word);
            }
            Instruction::ORItoSR(value) => {
                let word = self.cpu.get_sr() | value;
                self.cpu.set_sr(word);
            }
            Instruction::EORItoSR(value) => {
                let word = self.cpu.get_sr() ^ value;
                self.cpu.set_sr(word);
            }
            Instruction::TAS(destination) => {
                //the flags are the byte *before* the operation, and bit 7 is set after
                //(`Reference/68ks5w.htm`); `set_logic_flags` is N and Z with V and C cleared
                //and X kept, which is the help's table
                let value = self.get_operand_value(destination, Size::Byte, Used::Twice)?;
                self.set_logic_flags(value, Size::Byte);
                self.store_operand_value(destination, value | 0x80, Size::Byte, Used::Twice)?;
            }
            Instruction::RTR => {
                //a word first, of which the low byte is the condition codes, then the return
                //address (`Reference/68ks9f.htm`); the stack pointer goes up by six
                let (status, sp) = self.memory.pop(Size::Word, self.get_sp())?;
                let (address, sp) = self.memory.pop(Size::Long, sp)?;
                if self.keep_history {
                    self.debugger.add_mutation(MutationOperation::PopCall {
                        to: address.get_long() as usize,
                        from: self.current_instruction_address,
                    })
                }
                self.cpu.set_ccr_byte(status.get_word() as u8);
                self.set_sp(sp);
                self.pc = address.get_long() as usize;
                self.debugger.pop_call();
            }
            Instruction::CHK(source, register) => {
                //the low word of the register against the operand, both signed
                //(`Reference/68ks10a.htm`)
                let bound = sign_extend_to_long(
                    self.get_operand_value(source, Size::Word, Used::Once)?,
                    Size::Word,
                );
                let value =
                    sign_extend_to_long(self.get_register_value(*register, Size::Word), Size::Word);
                if value < 0 || value > bound {
                    //"N - Set if the data register is less than zero, cleared if the data
                    //register is greater than the higher limit"; the other flags the help
                    //leaves undefined and s68k leaves alone
                    self.set_flag(Flags::Negative, value < 0);
                    return Err(
                        self.end_with_an_exception(RuntimeError::ChkOutOfBounds { value, bound })
                    );
                }
            }
            Instruction::TRAPV => {
                if self.get_flag(Flags::Overflow) {
                    return Err(self.end_with_an_exception(RuntimeError::OverflowException));
                }
            }
            Instruction::ILLEGAL => {
                return Err(self.end_with_an_exception(RuntimeError::IllegalInstruction));
            }
        };
        Ok(())
    }
    #[rustfmt::skip]
    pub fn debug_status(&self) {
        println!("\n-----INTERPRETER DEBUG-----\n");
        println!("PC: {:#010X} ({})", self.pc, self.pc);
        println!("D0: {:#010X} ({})", self.cpu.d_reg[0].get_long(), self.cpu.d_reg[0].get_long());
        println!("D1: {:#010X} ({})", self.cpu.d_reg[1].get_long(), self.cpu.d_reg[1].get_long());
        println!("D2: {:#010X} ({})", self.cpu.d_reg[2].get_long(), self.cpu.d_reg[2].get_long());
        println!("D3: {:#010X} ({})", self.cpu.d_reg[3].get_long(), self.cpu.d_reg[3].get_long());
        println!("D4: {:#010X} ({})", self.cpu.d_reg[4].get_long(), self.cpu.d_reg[4].get_long());
        println!("D5: {:#010X} ({})", self.cpu.d_reg[5].get_long(), self.cpu.d_reg[5].get_long());
        println!("D6: {:#010X} ({})", self.cpu.d_reg[6].get_long(), self.cpu.d_reg[6].get_long());
        println!("D7: {:#010X} ({})", self.cpu.d_reg[7].get_long(), self.cpu.d_reg[7].get_long());
        println!("A0: {:#010X} ({})", self.cpu.a_reg[0].get_long(), self.cpu.a_reg[0].get_long());
        println!("A1: {:#010X} ({})", self.cpu.a_reg[1].get_long(), self.cpu.a_reg[1].get_long());
        println!("A2: {:#010X} ({})", self.cpu.a_reg[2].get_long(), self.cpu.a_reg[2].get_long());
        println!("A3: {:#010X} ({})", self.cpu.a_reg[3].get_long(), self.cpu.a_reg[3].get_long());
        println!("A4: {:#010X} ({})", self.cpu.a_reg[4].get_long(), self.cpu.a_reg[4].get_long());
        println!("A5: {:#010X} ({})", self.cpu.a_reg[5].get_long(), self.cpu.a_reg[5].get_long());
        println!("A6: {:#010X} ({})", self.cpu.a_reg[6].get_long(), self.cpu.a_reg[6].get_long());
        println!("A7: {:#010X} ({})", self.cpu.a_reg[7].get_long(), self.cpu.a_reg[7].get_long());
        let ccr = self.cpu.ccr.get_status();
        println!("SR: {:#06X}", self.cpu.get_sr());
        println!("{}", ccr);
    }

    #[inline]
    pub fn get_register_value(&self, register: RegisterOperand, size: Size) -> u32 {
        match register {
            RegisterOperand::Address(num) => self.cpu.a_reg[num as usize].get_size(size),
            RegisterOperand::Data(num) => self.cpu.d_reg[num as usize].get_size(size),
        }
    }

    #[inline]
    pub fn set_register_value(&mut self, register: RegisterOperand, value: u32, size: Size) {
        let old_value = match register {
            RegisterOperand::Address(num) => {
                //TODO i could probably make this a bit more efficient by not having to do the get_size even if the history is not being kept
                let old_value = self.cpu.a_reg[num as usize].get_long();
                self.cpu.a_reg[num as usize].store_size(size, value);
                old_value
            }
            RegisterOperand::Data(num) => {
                let old_value = self.cpu.d_reg[num as usize].get_long();
                self.cpu.d_reg[num as usize].store_size(size, value);
                old_value
            }
        };
        if self.keep_history {
            self.debugger
                .add_mutation(MutationOperation::WriteRegister {
                    register,
                    old: old_value,
                    size,
                });
        }
    }

    pub fn set_memory_value(
        &mut self,
        address: usize,
        size: Size,
        value: u32,
    ) -> RuntimeResult<()> {
        if self.keep_history {
            let old_value = self.memory.read_size(address, size)?;
            self.debugger.add_mutation(MutationOperation::WriteMemory {
                address,
                old: old_value,
                size,
            });
        }
        self.memory.write_size(address, size, value)?;
        Ok(())
    }

    pub fn move_registers_to_memory(
        &mut self,
        mut addr: usize,
        size: Size,
        mut mask: u16,
    ) -> RuntimeResult<u32> {
        for i in 0..8 {
            if (mask & 0x01) != 0 {
                self.set_memory_value(addr, size, self.cpu.d_reg[i].get_long())?;
                addr += size.to_bytes();
            }
            mask >>= 1;
        }
        for i in 0..8 {
            if (mask & 0x01) != 0 {
                let value = self.cpu.a_reg[i].get_long();
                self.set_memory_value(addr, size, value)?;
                addr += size.to_bytes();
            }
            mask >>= 1;
        }
        Ok(addr as u32)
    }
    pub fn move_registers_to_memory_reverse(
        &mut self,
        mut addr: usize,
        size: Size,
        mut mask: u16,
    ) -> RuntimeResult<u32> {
        for i in (0..8).rev() {
            if (mask & 0x01) != 0 {
                let value = self.cpu.a_reg[i].get_long();
                addr -= size.to_bytes();
                self.set_memory_value(addr, size, value)?;
            }
            mask >>= 1;
        }
        for i in (0..8).rev() {
            if (mask & 0x01) != 0 {
                addr -= size.to_bytes();
                self.set_memory_value(addr, size, self.cpu.d_reg[i].get_long())?;
            }
            mask >>= 1;
        }
        Ok(addr as u32)
    }
    pub fn move_memory_to_registers(
        &mut self,
        addr: usize,
        size: Size,
        mut mask: u16,
    ) -> RuntimeResult<u32> {
        let size_bytes = size.to_bytes() as u32;
        let mut addr = addr as u32;
        for i in 0..8 {
            if (mask & 0x01) != 0 {
                let val =
                    sign_extend_to_long(self.memory.read_size(addr as usize, size)?, size) as u32;
                self.set_register_value(RegisterOperand::Data(i), val, size);
                (addr, _) = overflowing_add_sized(addr, size_bytes, Size::Long);
            }
            mask >>= 1;
        }
        for i in 0..8 {
            if (mask & 0x01) != 0 {
                let val =
                    sign_extend_to_long(self.memory.read_size(addr as usize, size)?, size) as u32;
                self.set_register_value(RegisterOperand::Address(i), val, Size::Long);
                (addr, _) = overflowing_add_sized(addr, size_bytes, Size::Long);
            }
            mask >>= 1;
        }
        Ok(addr)
    }

    pub fn set_memory_bytes(&mut self, address: usize, bytes: &[u8]) -> RuntimeResult<()> {
        if self.keep_history {
            let old_bytes = self.memory.read_bytes(address, bytes.len())?;
            self.debugger
                .add_mutation(MutationOperation::WriteMemoryBytes {
                    address,
                    old: old_bytes.to_vec(),
                });
        }
        self.memory.write_bytes(address, bytes)
    }
    /// The instruction the program counter is on, which is the one the next
    /// step will run.
    pub fn get_next_instruction(&self) -> Option<&AssembledInstruction> {
        self.get_instruction_at(self.pc)
    }
    /// Tasks 13, 14, 17, 18 and 95 all take the string at (A1), terminated by a null byte.
    fn read_null_terminated_string(&self, address: usize) -> RuntimeResult<String> {
        let max = 16384; //to prevent infinite loop
        let mut bytes = Vec::new();
        let mut i = 0;
        loop {
            let byte = self.memory.read_byte(address + i)?;
            if byte == 0x00 {
                break;
            }
            bytes.push(byte);
            i += 1;
            if i > max {
                return Err(RuntimeError::Raw(format!(
                    "Invalid String read, reached max length of {} bytes",
                    max
                )));
            }
        }
        String::from_utf8(bytes.to_vec()).map_err(|e| {
            RuntimeError::Raw(format!(
                "Invalid String read, received: {:?}, expected UTF-8",
                e.into_bytes()
            ))
        })
    }
    fn get_trap(&mut self, value: u8) -> RuntimeResult<Interrupt> {
        match value {
            0 | 1 => {
                //TODO not sure if this is correct or if it should read untill 0x00
                let address = self.cpu.a_reg[1].get_long();
                let length = self.cpu.d_reg[1].get_word() as i32;
                if !(0..=255).contains(&length) {
                    //only for interrupt 1 and 2, check bounds
                    Err(RuntimeError::Raw(format!("Invalid String read, length of string in d1 register is: {}, expected between 0 and 255", length)))
                } else {
                    let mut bytes = self
                        .memory
                        .read_bytes(address as usize, length as usize)?
                        .to_vec();
                    if value == 0 {
                        //get all bytes until 0x00
                        match bytes.iter().position(|&x| x == 0x00) {
                            Some(pos) => bytes = bytes[..pos].to_vec(),
                            None => {}
                        }
                    }
                    //TODO implement call to interrupt handler
                    match String::from_utf8(bytes.to_vec()) {
                        Ok(str) if value == 0 => Ok(Interrupt::DisplayStringWithCRLF(str)),
                        Ok(str) if value == 1 => Ok(Interrupt::DisplayStringWithoutCRLF(str)),
                        Err(_) | Ok(_) => Err(RuntimeError::Raw(format!(
                            "Invalid String read, received: {:?}, expected UTF-8",
                            bytes
                        ))),
                    }
                }
            }
            2 => Ok(Interrupt::ReadKeyboardString),
            3 => {
                let value = self.cpu.d_reg[1].get_long();
                Ok(Interrupt::DisplayNumber(value as i32))
            }
            4 => Ok(Interrupt::ReadNumber),
            5 => Ok(Interrupt::ReadChar),
            6 => {
                let value = self.cpu.d_reg[1].get_byte();
                Ok(Interrupt::DisplayChar(value as char))
            }
            7 => Ok(Interrupt::CheckKeyboardInput),
            8 => Ok(Interrupt::GetTime),
            9 => {
                self.set_status(InterpreterStatus::Terminated);
                Ok(Interrupt::Terminate)
            }
            13 | 14 => {
                let address = self.cpu.a_reg[1].get_long() as usize;
                let str = self.read_null_terminated_string(address)?;
                if value == 13 {
                    Ok(Interrupt::DisplayStringWithCRLF(str))
                } else {
                    Ok(Interrupt::DisplayStringWithoutCRLF(str))
                }
            }
            15 => {
                //Display the unsigned number in D1.L converted to the number base (2 through 36)
                //in D2.B: to display D1.L in base 16, put 16 in D2.B. EASy68K ignores a base
                //outside 2 to 36; this reports it instead.
                let value = self.cpu.d_reg[1].get_long();
                let base = self.cpu.d_reg[2].get_byte() as u32;
                if !(2..=36).contains(&base) {
                    return Err(RuntimeError::Raw(format!(
                        "Invalid base for display number: {} in register D2.b, expected between 2 and 36",
                        base
                    )));
                };
                Ok(Interrupt::DisplayNumberInBase {
                    value,
                    base: base as u8,
                })
            }
            17 | 18 => {
                //tasks 14 and 3, or 14 and 4, in a single trap
                let address = self.cpu.a_reg[1].get_long() as usize;
                let string = self.read_null_terminated_string(address)?;
                if value == 17 {
                    let number = self.cpu.d_reg[1].get_long() as i32;
                    Ok(Interrupt::DisplayStringAndNumber { string, number })
                } else {
                    Ok(Interrupt::DisplayStringAndReadNumber(string))
                }
            }
            19 => {
                //D1.L holds up to four key codes, one per byte, or zero to ask for the last keys
                let request = self.cpu.d_reg[1].get_long();
                Ok(Interrupt::GetKeyState(if request == 0 {
                    KeyStateRequest::LastKeys
                } else {
                    KeyStateRequest::Keys(request.to_be_bytes())
                }))
            }
            20 => {
                let value = self.cpu.d_reg[1].get_long() as i32;
                let width = self.cpu.d_reg[2].get_byte();
                Ok(Interrupt::DisplaySignedNumberInField { value, width })
            }
            23 => {
                let time = self.cpu.d_reg[1].get_long();
                Ok(Interrupt::Delay(time))
            }
            24 => {
                //the focused screen already receives every key, so shortcut control changes nothing here
                let value = self.cpu.d_reg[1].get_long();
                Ok(Interrupt::SetSimulatorShortcuts(value))
            }
            61 => {
                let mode = self.cpu.d_reg[1].get_byte();
                match mode {
                    0..=2 => Ok(Interrupt::ReadMouse(mode)),
                    _ => Err(RuntimeError::Raw(format!(
                        "Invalid mouse read mode: {} in register D1.b, expected 0 (current state), 1 (last button up) or 2 (last button down)",
                        mode
                    ))),
                }
            }
            // Graphics interrupts
            11 => {
                //EASy68K reserves two values of D1.W and packs the column in the high byte and the row in the low byte of the rest
                let request = self.cpu.d_reg[1].get_word();
                match request {
                    0xFF00 => Ok(Interrupt::ClearScreen),
                    0x00FF => Ok(Interrupt::GetTextCursorPosition),
                    _ => Ok(Interrupt::SetTextCursorPosition(
                        (request >> 8) as u32,
                        (request & 0xFF) as u32,
                    )),
                }
            }
            33 => {
                //D1.L holds the width in the high word and the height in the low word, with 0, 1 and 2 reserved
                let request = self.cpu.d_reg[1].get_long();
                match request {
                    0 => Ok(Interrupt::GetScreenSize),
                    1 | 2 => Ok(Interrupt::SetScreenMode(request as u8)),
                    _ => Ok(Interrupt::SetScreenSize(request >> 16, request & 0xFFFF)),
                }
            }
            80 => {
                let color = self.cpu.d_reg[1].get_long();
                Ok(Interrupt::SetPenColor(color))
            }
            81 => {
                let color = self.cpu.d_reg[1].get_long();
                Ok(Interrupt::SetFillColor(color))
            }
            82 => {
                let x = self.cpu.d_reg[1].get_word();
                let y = self.cpu.d_reg[2].get_word();
                Ok(Interrupt::DrawPixel(x as i16 as i32, y as i16 as i32))
            }
            83 => {
                let x = self.cpu.d_reg[1].get_word();
                let y = self.cpu.d_reg[2].get_word();
                Ok(Interrupt::GetPixelColor(x as i16 as i32, y as i16 as i32))
            }
            84 => {
                let x1 = self.cpu.d_reg[1].get_word();
                let y1 = self.cpu.d_reg[2].get_word();
                let x2 = self.cpu.d_reg[3].get_word();
                let y2 = self.cpu.d_reg[4].get_word();
                Ok(Interrupt::DrawLine(
                    x1 as i16 as i32,
                    y1 as i16 as i32,
                    x2 as i16 as i32,
                    y2 as i16 as i32,
                ))
            }
            85 => {
                let x = self.cpu.d_reg[1].get_word();
                let y = self.cpu.d_reg[2].get_word();
                Ok(Interrupt::DrawLineTo(x as i16 as i32, y as i16 as i32))
            }
            86 => {
                let x = self.cpu.d_reg[1].get_word();
                let y = self.cpu.d_reg[2].get_word();
                Ok(Interrupt::MoveTo(x as i16 as i32, y as i16 as i32))
            }
            87 => {
                let left_x = self.cpu.d_reg[1].get_word();
                let upper_y = self.cpu.d_reg[2].get_word();
                let right_x = self.cpu.d_reg[3].get_word();
                let lower_y = self.cpu.d_reg[4].get_word();
                Ok(Interrupt::DrawRectangle(
                    left_x as i16 as i32,
                    upper_y as i16 as i32,
                    right_x as i16 as i32,
                    lower_y as i16 as i32,
                ))
            }
            88 => {
                let left_x = self.cpu.d_reg[1].get_word();
                let upper_y = self.cpu.d_reg[2].get_word();
                let right_x = self.cpu.d_reg[3].get_word();
                let lower_y = self.cpu.d_reg[4].get_word();
                Ok(Interrupt::DrawEllipse(
                    left_x as i16 as i32,
                    upper_y as i16 as i32,
                    right_x as i16 as i32,
                    lower_y as i16 as i32,
                ))
            }
            89 => {
                let x = self.cpu.d_reg[1].get_word();
                let y = self.cpu.d_reg[2].get_word();
                Ok(Interrupt::FloodFill(x as i16 as i32, y as i16 as i32))
            }
            90 => {
                let left_x = self.cpu.d_reg[1].get_word();
                let upper_y = self.cpu.d_reg[2].get_word();
                let right_x = self.cpu.d_reg[3].get_word();
                let lower_y = self.cpu.d_reg[4].get_word();
                Ok(Interrupt::DrawUnfilledRectangle(
                    left_x as i16 as i32,
                    upper_y as i16 as i32,
                    right_x as i16 as i32,
                    lower_y as i16 as i32,
                ))
            }
            91 => {
                let left_x = self.cpu.d_reg[1].get_word();
                let upper_y = self.cpu.d_reg[2].get_word();
                let right_x = self.cpu.d_reg[3].get_word();
                let lower_y = self.cpu.d_reg[4].get_word();
                Ok(Interrupt::DrawUnfilledEllipse(
                    left_x as i16 as i32,
                    upper_y as i16 as i32,
                    right_x as i16 as i32,
                    lower_y as i16 as i32,
                ))
            }
            92 => {
                let mode = self.cpu.d_reg[1].get_byte();
                match mode {
                    2 | 4 | 16 | 17 => Ok(Interrupt::SetDrawingMode(mode)),
                    //the bitwise raster modes draw against the background color, which this screen does not implement
                    _ => Err(RuntimeError::Raw(format!(
                        "Unsupported drawing mode: {} in register D1.b, expected 2 (move without drawing), 4 (draw normally), 16 (double buffering off) or 17 (double buffering on)",
                        mode
                    ))),
                }
            }
            93 => {
                let width = self.cpu.d_reg[1].get_byte();
                Ok(Interrupt::SetPenWidth(width as u32))
            }
            94 => Ok(Interrupt::Repaint),
            96 => Ok(Interrupt::GetPenPosition),
            95 => {
                let address = self.cpu.a_reg[1].get_long() as usize;
                let str = self.read_null_terminated_string(address)?;
                let x = self.cpu.d_reg[1].get_word();
                let y = self.cpu.d_reg[2].get_word();
                Ok(Interrupt::DrawText(x as i16 as i32, y as i16 as i32, str))
            }
            _ => Err(RuntimeError::Raw(format!("Unknown interrupt: {}", value))),
        }
    }
    /**
    Some instructions limit inputs to 8 bits if the destination is
    not a register.
     */
    fn limit_bit_size(&mut self, bit: u32, dest: &Operand) -> RuntimeResult<u32> {
        match dest {
            Operand::Register(_) => Ok(bit),
            _ => Ok(bit % 8),
        }
    }
    fn get_a_reg_sized(&self, reg: u8, size: Size) -> u32 {
        self.cpu.a_reg[reg as usize].get_size(size)
    }
    fn set_a_reg_sized(&mut self, reg: u8, value: u32, size: Size) {
        let old_value = self.cpu.a_reg[reg as usize].get_long();
        self.cpu.a_reg[reg as usize].store_size(size, value);
        if self.keep_history {
            self.debugger
                .add_mutation(MutationOperation::WriteRegister {
                    register: RegisterOperand::Address(reg),
                    old: old_value,
                    size,
                });
        }
    }
    fn get_operand_value(&mut self, op: &Operand, size: Size, used: Used) -> RuntimeResult<u32> {
        match op {
            Operand::Immediate(v) => Ok(*v),
            Operand::Register(op) => Ok(self.get_register_value(*op, size)),
            Operand::Absolute(address) => Ok(self.memory.read_size(*address, size)?),

            Operand::Indirect(reg) => {
                let address = self.get_a_reg_sized(*reg, Size::Long);
                Ok(self.memory.read_size(address as usize, size)?)
            }
            Operand::PreIndirect(op) => {
                let address = self.get_a_reg_sized(*op, Size::Long);
                let address = (address).wrapping_sub(size.to_bytes() as u32);
                //in this case the read should always decrement the address
                self.set_a_reg_sized(*op, address, Size::Long);
                Ok(self.memory.read_size(address as usize, size)?)
            }
            Operand::PostIndirect(op) => {
                let address = self.get_a_reg_sized(*op, Size::Long);
                if used != Used::Twice {
                    //if the value is used twice, give precedence of increment to the setter
                    let new_address = address.wrapping_add(size.to_bytes() as u32);
                    self.set_a_reg_sized(*op, new_address, Size::Long);
                }
                Ok(self.memory.read_size(address as usize, size)?)
            }
            Operand::IndirectDisplacement { offset, base } => {
                //TODO not sure if this works fine with full 32bits
                let address = self.get_register_value(*base, Size::Long) as i32;
                let address = address.wrapping_add(*offset);
                Ok(self.memory.read_size(address as usize, size)?)
            }
            Operand::IndirectIndex {
                offset,
                base,
                index,
            } => {
                //TODO not sure if this is how it should work
                //TODO should this be i32?
                let base_value = self.get_register_value(*base, Size::Long) as i32;
                let index_value = self.get_register_value(index.register, index.size);
                let index_value = sign_extend_to_long(index_value, index.size);
                let final_address = base_value.wrapping_add(*offset).wrapping_add(index_value);
                Ok(self.memory.read_size(final_address as usize, size)?)
            }
            Operand::PcDisplacement { offset } => {
                let address = self.pc_relative_address(*offset, None);
                Ok(self.memory.read_size(address as usize, size)?)
            }
            Operand::PcIndex { offset, index } => {
                let address = self.pc_relative_address(*offset, Some(*index));
                Ok(self.memory.read_size(address as usize, size)?)
            }
        }
    }
    /// The address a PC-relative Operand names, which is the other half of the
    /// round trip the Assembler started.
    ///
    /// The Assembler stored `label - (address of this instruction +
    /// EXTENSION_WORD_OFFSET)`, so adding the two back gives the label again —
    /// whatever the instruction is and wherever the program counter has got
    /// to, because the address used is the instruction being executed and not
    /// the one after it.
    fn pc_relative_address(&self, offset: i32, index: Option<IndexRegister>) -> u32 {
        let base = (self.current_instruction_address as u32)
            .wrapping_add(EXTENSION_WORD_OFFSET as u32)
            .wrapping_add(offset as u32);
        match index {
            None => base,
            Some(index) => {
                let value = self.get_register_value(index.register, index.size);
                base.wrapping_add(sign_extend_to_long(value, index.size) as u32)
            }
        }
    }
    fn get_operand_address(&mut self, op: &Operand) -> RuntimeResult<u32> {
        match op {
            Operand::PreIndirect(op) | Operand::PostIndirect(op) => {
                Ok(self.get_a_reg_sized(*op, Size::Long))
            }
            Operand::Indirect(reg) => Ok(self.get_a_reg_sized(*reg, Size::Long)),
            Operand::IndirectDisplacement { offset, base } => {
                //TODO not sure if this works fine with full 32bits
                let address = self.get_register_value(*base, Size::Long) as i32;
                let address = address.wrapping_add(*offset);
                Ok(address as u32)
            }
            Operand::IndirectIndex {
                offset,
                base,
                index,
            } => {
                //TODO not sure if this is how it should work
                let base_value = self.get_register_value(*base, Size::Long) as i32;
                let index_value = self.get_register_value(index.register, index.size);
                let index_value = sign_extend_to_long(index_value, index.size);
                let final_address = base_value.wrapping_add(*offset).wrapping_add(index_value);
                Ok(final_address as u32)
            }
            Operand::Absolute(address) => Ok(*address as u32),
            Operand::PcDisplacement { offset } => Ok(self.pc_relative_address(*offset, None)),
            Operand::PcIndex { offset, index } => {
                Ok(self.pc_relative_address(*offset, Some(*index)))
            }
            _ => Err(RuntimeError::IncorrectAddressingMode(
                "Attempted to get address of non address addressing mode".to_string(),
            )),
        }
    }
    fn store_operand_value(
        &mut self,
        op: &Operand,
        value: u32,
        size: Size,
        used: Used,
    ) -> RuntimeResult<()> {
        match op {
            Operand::Immediate(_) => Err(RuntimeError::IncorrectAddressingMode(
                "Attempted to store to immediate value".to_string(),
            )),
            Operand::Register(op) => {
                self.set_register_value(*op, value, size);
                Ok(())
            }
            Operand::Absolute(address) => Ok(self.set_memory_value(*address, size, value)?),
            Operand::Indirect(reg) => {
                let address = self.get_a_reg_sized(*reg, Size::Long);
                Ok(self.set_memory_value(address as usize, size, value)?)
            }

            Operand::PreIndirect(op) => {
                //give priority to the getter to decrement
                let address = if used == Used::Twice {
                    //if it's used twice, just get the address
                    //as it was already decremented by the get
                    self.get_a_reg_sized(*op, Size::Long)
                } else {
                    //if it's not used twice, then decrement the value
                    let a = self.get_a_reg_sized(*op, Size::Long);
                    let a = (a).wrapping_sub(size.to_bytes() as u32);
                    self.set_a_reg_sized(*op, a, Size::Long);
                    a
                };

                Ok(self.set_memory_value(address as usize, size, value)?)
            }
            Operand::PostIndirect(op) => {
                let address = self.get_a_reg_sized(*op, Size::Long);
                let new_address = (address).wrapping_add(size.to_bytes() as u32);
                //give priority to increment to the setter
                self.set_a_reg_sized(*op, new_address, Size::Long);
                Ok(self.set_memory_value(address as usize, size, value)?)
            }
            Operand::IndirectDisplacement { offset, base } => {
                //TODO not sure if this works fine with full 32bits
                let address = self.get_register_value(*base, Size::Long) as i32;
                let address = address.wrapping_add(*offset);
                Ok(self.set_memory_value(address as usize, size, value)?)
            }
            Operand::IndirectIndex {
                offset,
                index,
                base,
            } => {
                let base_value = self.get_register_value(*base, Size::Long) as i32;
                let index_value = self.get_register_value(index.register, index.size);
                let index_value = sign_extend_to_long(index_value, index.size);
                let final_address = base_value.wrapping_add(*offset).wrapping_add(index_value);
                Ok(self.set_memory_value(final_address as usize, size, value)?)
            }
            // Nothing is written through the program counter: the analyzer
            // refuses a PC-relative Operand wherever the instruction writes
            // one (`Modes::ALTERABLE` holds neither of them), so this is
            // unreachable from any assembled Program and is an error rather
            // than a store to the address it names.
            Operand::PcDisplacement { .. } | Operand::PcIndex { .. } => {
                Err(RuntimeError::IncorrectAddressingMode(
                    "Attempted to store through a PC-relative operand".to_string(),
                ))
            }
        }
    }
    pub fn verify_can_run(&mut self) -> RuntimeResult<()> {
        if self.status == InterpreterStatus::Terminated
            || self.status == InterpreterStatus::TerminatedWithException
        {
            return Err(RuntimeError::Raw(
                "Attempted to run terminated emulator".to_string(),
            ));
        }
        if self.status == InterpreterStatus::Interrupt {
            return Err(RuntimeError::Raw(
                "Attempted to run emulator with pending interrupt".to_string(),
            ));
        }
        Ok(())
    }
    pub fn run(&mut self) -> RuntimeResult<InterpreterStatus> {
        self.verify_can_run()?;
        while self.status == InterpreterStatus::Running {
            self.step()?;
        }
        Ok(self.status)
    }

    /// The addresses the given breakpoints stop at: every instruction whose
    /// Location is one of those (File, line) pairs.
    ///
    /// A line that assembled to several instructions contributes all of them,
    /// and a line that assembled to none — a comment, a Directive, a Label on
    /// its own — contributes nothing and stops the run nowhere.
    pub fn get_breakpoint_addresses(&self, breakpoints: &[Breakpoint]) -> HashSet<usize> {
        let lines: HashSet<(&str, usize)> = breakpoints
            .iter()
            .map(|breakpoint| (breakpoint.file.as_str(), breakpoint.line))
            .collect();
        self.program
            .instructions()
            .iter()
            .filter(|instruction| {
                lines.contains(&(
                    instruction.location.file.as_str(),
                    instruction.location.line,
                ))
            })
            .map(|instruction| instruction.address)
            .collect()
    }

    /// Runs until a breakpoint, the end of the program, an interrupt or
    /// `limit` instructions.
    ///
    /// A breakpoint on the line the program counter is already on does not stop
    /// it again, which is what makes "continue" from a breakpoint move.
    pub fn run_with_breakpoints(
        &mut self,
        breakpoints: &[Breakpoint],
        limit: Option<usize>,
    ) -> RuntimeResult<InterpreterStatus> {
        self.verify_can_run()?;
        let addresses = self.get_breakpoint_addresses(breakpoints);
        let mut iterations = 0;
        let limit = limit.unwrap_or(usize::MAX);
        let mut limit_counter = limit;
        while self.status == InterpreterStatus::Running && limit_counter > 0 {
            //skip the first iteration if the pc is on a breakpoint
            if iterations > 0 && addresses.contains(&self.pc) {
                self.status = InterpreterStatus::Running;
                break;
            }
            self.step()?;
            limit_counter -= 1;
            iterations += 1;
        }
        if limit_counter == 0 {
            return Err(RuntimeError::ExecutionLimit(limit));
        }
        Ok(self.status)
    }

    pub fn run_with_limit(&mut self, limit: usize) -> RuntimeResult<InterpreterStatus> {
        let mut limit_counter = limit;
        self.verify_can_run()?;
        while self.status == InterpreterStatus::Running && limit_counter > 0 {
            self.step()?;
            limit_counter -= 1;
        }
        if limit_counter == 0 {
            return Err(RuntimeError::ExecutionLimit(limit));
        }
        Ok(self.status)
    }

    pub fn get_flag(&self, flag: Flags) -> bool {
        self.cpu.ccr.contains(flag)
    }
    fn set_flag(&mut self, flag: Flags, value: bool) {
        self.cpu.ccr.set(flag, value)
    }
    fn set_logic_flags(&mut self, value: u32, size: Size) {
        let mut flags = Flags::new();
        if get_sign(value, size) {
            flags |= Flags::Negative;
        }
        if value == 0 {
            flags |= Flags::Zero;
        }
        if self.cpu.ccr.contains(Flags::Extend) {
            flags |= Flags::Extend;
        }
        self.cpu.ccr = flags;
    }
    fn set_bit_test_flags(&mut self, value: u32, bitnum: u32, size: Size) -> u32 {
        let mask = 0x1 << (bitnum % size.to_bits() as u32);
        self.set_flag(Flags::Zero, (value & mask) == 0);
        mask
    }
    fn set_compare_flags(&mut self, value: u32, size: Size, carry: bool, overflow: bool) {
        let value = sign_extend_to_long(value, size);
        let mut flags = Flags::new();
        if value < 0 {
            flags |= Flags::Negative;
        }
        if value == 0 {
            flags |= Flags::Zero;
        }
        if carry {
            flags |= Flags::Carry;
        }
        if overflow {
            flags |= Flags::Overflow;
        }
        if self.cpu.ccr.contains(Flags::Extend) {
            flags |= Flags::Extend;
        }
        self.cpu.ccr = flags;
    }

    /// The flags of the three instructions that carry the extend flag into
    /// their arithmetic: `addx`, `subx` and `negx`.
    ///
    /// N and V are the result's, as they are for `add` and `sub`, and X and C
    /// are the one carry, set alike. **Z is the multi-precision rule**
    /// ([`Interpreter::clear_zero_if_the_result_is_not_zero`]).
    fn set_extended_arithmetic_flags(
        &mut self,
        result: u32,
        size: Size,
        carry: bool,
        overflow: bool,
    ) {
        self.set_flag(Flags::Negative, get_sign(result, size));
        self.clear_zero_if_the_result_is_not_zero(result, size);
        self.set_flag(Flags::Overflow, overflow);
        self.set_flag(Flags::Extend, carry);
        self.set_flag(Flags::Carry, carry);
    }

    /// The flags of `abcd`, `sbcd` and `nbcd`: the decimal carry in X and C,
    /// the same Z rule, and **N and V left exactly as they were**.
    ///
    /// The help calls both of those undefined for all three instructions, and
    /// s68k leaves an undefined flag alone rather than inventing a value for
    /// it, which is what `chk` already does with the flags its own page calls
    /// undefined.
    fn set_decimal_flags(&mut self, result: u32, carry: bool) {
        self.clear_zero_if_the_result_is_not_zero(result, Size::Byte);
        self.set_flag(Flags::Extend, carry);
        self.set_flag(Flags::Carry, carry);
    }

    /// The Z flag of every instruction that carries the extend flag: **cleared
    /// when the result is not zero, and left exactly as it was when it is**.
    ///
    /// It is what makes a multi-precision number testable in one go: the
    /// program sets Z, adds or subtracts the pieces from the least significant
    /// up, and Z is still set at the end only if every piece came out zero
    /// ("The Z flag works in another way now, making it possible to check if a
    /// big number (much bigger than 32 bits) is zero. You must set the zero
    /// flag before making the addition though", `Reference/68ks5e.htm`). An
    /// instruction that set Z from its own result would lose the answer of the
    /// piece below it, which is the mistake this rule exists to prevent.
    fn clear_zero_if_the_result_is_not_zero(&mut self, result: u32, size: Size) {
        if get_value_sized(result, size) != 0 {
            self.set_flag(Flags::Zero, false);
        }
    }

    pub fn get_condition_value(&self, cond: &Condition) -> bool {
        match cond {
            Condition::True => true,
            Condition::False => false,
            Condition::High => !self.get_flag(Flags::Carry) && !self.get_flag(Flags::Zero),
            Condition::LowOrSame => self.get_flag(Flags::Carry) || self.get_flag(Flags::Zero),
            Condition::CarryClear => !self.get_flag(Flags::Carry),
            Condition::CarrySet => self.get_flag(Flags::Carry),
            Condition::NotEqual => !self.get_flag(Flags::Zero),
            Condition::Equal => self.get_flag(Flags::Zero),
            Condition::OverflowClear => !self.get_flag(Flags::Overflow),
            Condition::OverflowSet => self.get_flag(Flags::Overflow),
            Condition::Plus => !self.get_flag(Flags::Negative),
            Condition::Minus => self.get_flag(Flags::Negative),
            Condition::GreaterThanOrEqual => {
                (self.get_flag(Flags::Negative) && self.get_flag(Flags::Overflow))
                    || (!self.get_flag(Flags::Negative) && !self.get_flag(Flags::Overflow))
            }
            Condition::LessThan => {
                (self.get_flag(Flags::Negative) && !self.get_flag(Flags::Overflow))
                    || (!self.get_flag(Flags::Negative) && self.get_flag(Flags::Overflow))
            }
            Condition::GreaterThan => {
                (self.get_flag(Flags::Negative)
                    && self.get_flag(Flags::Overflow)
                    && !self.get_flag(Flags::Zero))
                    || (!self.get_flag(Flags::Negative)
                        && !self.get_flag(Flags::Overflow)
                        && !self.get_flag(Flags::Zero))
            }
            Condition::LessThanOrEqual => {
                self.get_flag(Flags::Zero)
                    || (self.get_flag(Flags::Negative) && !self.get_flag(Flags::Overflow))
                    || (!self.get_flag(Flags::Negative) && self.get_flag(Flags::Overflow))
            }
        }
    }
}

#[wasm_bindgen]
impl Interpreter {
    pub fn wasm_read_memory_bytes(&self, address: usize, size: usize) -> Vec<u8> {
        match self.memory.read_bytes(address, size) {
            Ok(bytes) => bytes.to_vec(),
            Err(_) => vec![],
        }
    }
    pub fn wasm_write_memory_bytes(
        &mut self,
        address: usize,
        bytes: Vec<u8>,
    ) -> Result<(), JsValue> {
        match self.memory.write_bytes(address, &bytes) {
            Ok(_) => Ok(()),
            Err(e) => Err(serde_wasm_bindgen::to_value(&e).unwrap()),
        }
    }
    pub fn wasm_get_cpu_snapshot(&self) -> Cpu {
        self.cpu
    }
    pub fn wasm_get_pc(&self) -> usize {
        self.get_pc()
    }
    pub fn wasm_get_sp(&self) -> usize {
        self.get_sp()
    }
    pub fn wasm_get_instruction_at(&self, address: usize) -> JsValue {
        match self.get_instruction_at(address) {
            Some(ins) => serde_wasm_bindgen::to_value(ins).unwrap(),
            None => JsValue::NULL,
        }
    }
    pub fn wasm_can_undo(&self) -> bool {
        self.debugger.can_undo()
    }
    /// Run one instruction and answer the status the Interpreter is left in.
    ///
    /// 1.4.2 serialised that status through `serde` and declared the result a
    /// `[instruction, status]` pair, which it never was: the pair had gone
    /// before the type was written, and a caller that destructured it read the
    /// second character of the string `"Running"`. It answers the same
    /// `InterpreterStatus` as [`wasm_run`](Interpreter::wasm_run) now, and
    /// `wasm_step_only_status`, which existed to work around it, is gone.
    pub fn wasm_step(&mut self) -> Result<InterpreterStatus, JsValue> {
        match self.step() {
            Ok(status) => Ok(status),
            Err(e) => Err(serde_wasm_bindgen::to_value(&e).unwrap()),
        }
    }
    pub fn wasm_run(&mut self) -> Result<InterpreterStatus, JsValue> {
        match self.run() {
            Ok(status) => Ok(status),
            Err(e) => Err(serde_wasm_bindgen::to_value(&e).unwrap()),
        }
    }
    /// `breakpoints` is an array of `{ file, line }`, a Location without its
    /// columns.
    pub fn wasm_run_with_breakpoints(
        &mut self,
        breakpoints: JsValue,
        limit: Option<usize>,
    ) -> Result<InterpreterStatus, JsValue> {
        let breakpoints: Vec<Breakpoint> = serde_wasm_bindgen::from_value(breakpoints)
            .map_err(|e| JsValue::from_str(&format!("Invalid breakpoints: {}", e)))?;
        match self.run_with_breakpoints(&breakpoints, limit) {
            Ok(status) => Ok(status),
            Err(e) => Err(serde_wasm_bindgen::to_value(&e).unwrap()),
        }
    }
    pub fn wasm_get_call_stack(&self) -> JsValue {
        serde_wasm_bindgen::to_value(&self.get_pretty_call_stack()).unwrap()
    }
    pub fn wasm_run_with_limit(&mut self, limit: usize) -> Result<InterpreterStatus, JsValue> {
        match self.run_with_limit(limit) {
            Ok(status) => Ok(status),
            Err(e) => Err(serde_wasm_bindgen::to_value(&e).unwrap()),
        }
    }
    pub fn wasm_get_next_instruction(&self) -> JsValue {
        match self.get_next_instruction() {
            Some(ins) => serde_wasm_bindgen::to_value(ins).unwrap(),
            None => JsValue::NULL,
        }
    }
    pub fn wasm_get_previous_mutations(&self) -> JsValue {
        match self.debugger.get_previous_mutations() {
            Some(m) => serde_wasm_bindgen::to_value(&m).unwrap(),
            None => JsValue::NULL,
        }
    }
    pub fn wasm_get_undo_history(&self, count: usize) -> JsValue {
        serde_wasm_bindgen::to_value(&self.debugger.get_last_steps(count)).unwrap()
    }

    pub fn wasm_get_last_step_id(&self) -> f64 {
        self.debugger
            .get_last_step()
            .map_or(0, |step| step.get_id()) as f64
    }
    pub fn wasm_get_status(&self) -> InterpreterStatus {
        *self.get_status()
    }
    pub fn wasm_get_flag(&self, flag: Flags) -> bool {
        self.get_flag(flag)
    }
    pub fn wasm_get_flags_as_number(&self) -> u16 {
        self.cpu.ccr.bits()
    }
    /// The whole status register, `$2700` before a program has run.
    ///
    /// Its low byte is the condition codes as the processor numbers them
    /// (extend 16, negative 8, zero 4, overflow 2, carry 1), which is **not**
    /// the bitfield [`Interpreter::wasm_get_flags_as_number`] answers: that one
    /// is this crate's own and the editor has always read it.
    pub fn wasm_get_sr(&self) -> u16 {
        self.get_sr()
    }
    pub fn wasm_undo(&mut self) -> Result<JsValue, JsValue> {
        match self.undo() {
            Ok(step) => Ok(serde_wasm_bindgen::to_value(&step).unwrap()),
            Err(e) => Err(serde_wasm_bindgen::to_value(&e).unwrap()),
        }
    }
    pub fn wasm_get_last_step(&self) -> JsValue {
        match self.debugger.get_last_step() {
            Some(step) => serde_wasm_bindgen::to_value(step).unwrap(),
            None => JsValue::NULL,
        }
    }
    pub fn wasm_get_flags_as_array(&self) -> Vec<u8> {
        self.get_flags_as_array()
    }
    pub fn wasm_get_condition_value(&self, cond: Condition) -> bool {
        self.get_condition_value(&cond)
    }
    pub fn wasm_get_last_line_address(&self) -> usize {
        self.get_current_instruction_address()
    }
    pub fn wasm_get_last_instruction(&self) -> JsValue {
        self.wasm_get_instruction_at(self.current_instruction_address)
    }
    pub fn wasm_get_register_value(&self, reg: JsValue, size: Size) -> Result<u32, String> {
        match serde_wasm_bindgen::from_value(reg.clone()) {
            Ok(reg) => Ok(self.get_register_value(reg, size)),
            Err(e) => Err(format!(
                "Cannot get register, invalid register {:?}, {}",
                reg, e
            )),
        }
    }
    pub fn wasm_set_register_value(
        &mut self,
        reg: JsValue,
        value: u32,
        size: Size,
    ) -> Result<(), String> {
        match serde_wasm_bindgen::from_value(reg.clone()) {
            Ok(parsed) => self.set_register_value(parsed, value, size),
            Err(e) => {
                return Err(format!(
                    "Cannot set register, invalid register {:?}, {}",
                    reg, e
                ));
            }
        }
        Ok(())
    }
    pub fn wasm_has_reached_bottom(&self) -> bool {
        self.has_reached_bottom()
    }
    pub fn wasm_has_terminated(&self) -> bool {
        self.has_terminated()
    }
    pub fn wasm_get_current_interrupt(&self) -> Result<JsValue, String> {
        match &self.get_current_interrupt() {
            Ok(interrupt) => match serde_wasm_bindgen::to_value(interrupt) {
                Ok(value) => Ok(value),
                Err(e) => Err(format!("Error converting interrupt to js value {:?}", e)),
            },
            Err(_) => Ok(JsValue::NULL),
        }
    }
    pub fn wasm_answer_interrupt(&mut self, value: JsValue) -> Result<(), String> {
        match serde_wasm_bindgen::from_value(value.clone()) {
            Ok(answer) => self.answer_interrupt(answer).unwrap(),
            Err(e) => {
                return Err(format!("Invalid interrupt answer: {:?}, {}", value, e));
            }
        }
        Ok(())
    }

    /// The [`Location`] of the instruction the program counter is on, as
    /// `{ file, line, column, end_column }`, or `null` when it is on none.
    pub fn wasm_get_current_location(&self) -> JsValue {
        match self.get_current_location() {
            Some(location) => serde_wasm_bindgen::to_value(location).unwrap(),
            None => JsValue::NULL,
        }
    }
}
