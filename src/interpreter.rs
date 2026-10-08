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
use crate::{c_runtime, charset};
use crate::{
    debugger::{
        Debugger, ExecutionStep, ExecutionStepKind, MutationOperation, PokeJournal, PokeTarget,
        PokeWrite,
    },
    instructions::{
        Bytes, Condition, FileDialogMode, FileExistence, IndexRegister, InputSettings, Instruction,
        Interrupt, InterruptResult, KeyStateRequest, KeyStateResult, Operand, RegisterOperand,
        ShiftDirection, Sign, Size, EXTENSION_WORD_OFFSET,
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

/// Which bytes of the address space an instruction takes up.
///
/// The instructions are not in memory: the Interpreter looks them up in the
/// Program by address, and the bytes under them are whatever [`Memory::new`]
/// filled them with. A program that reads or writes there is reading or
/// writing an instruction that is not there, which MARS and RARS refuse as an
/// access to the text segment, and so does s68k.
///
/// Every load and store of the program asks it, so it is one bit per byte over
/// the addresses from just before the first instruction to the end of the
/// last, and an access outside them, the stack's and most data's, is answered
/// by one comparison.
#[derive(Debug, Default)]
struct InstructionMap {
    /// The address of the first bit, three bytes before the first instruction
    /// so that an access of up to four bytes that ends inside an instruction
    /// starts inside the map.
    base: usize,
    /// How many addresses from `base` the map covers.
    span: usize,
    /// One bit per address from `base`, low bit first, and a spare byte at the
    /// end so that the two bytes [`holds`](Self::holds) reads are always there.
    bits: Vec<u8>,
}

impl InstructionMap {
    fn new(instructions: &[AssembledInstruction]) -> Self {
        let start = instructions.iter().map(|ins| ins.address).min();
        let end = instructions.iter().map(|ins| ins.address + ins.size).max();
        let (Some(start), Some(end)) = (start, end) else {
            return Self::default();
        };
        let base = start.saturating_sub(3);
        let span = end - base;
        let mut bits = vec![0; span / 8 + 2];
        for ins in instructions {
            for address in ins.address..ins.address + ins.size {
                let offset = address - base;
                bits[offset >> 3] |= 1 << (offset & 7);
            }
        }
        Self { base, span, bits }
    }
    /// Whether any of the `length` bytes at `address`, one to four, is an
    /// instruction's.
    #[inline(always)]
    fn holds(&self, address: usize, length: usize) -> bool {
        //an address before `base` wraps round to a huge offset and is outside too
        let offset = address.wrapping_sub(self.base);
        if offset >= self.span {
            return false;
        }
        let at = offset >> 3;
        let window = u16::from_le_bytes([self.bits[at], self.bits[at + 1]]);
        (window >> (offset & 7)) & ((1 << length) - 1) != 0
    }
    /// Whether any of the `length` bytes at `address` is an instruction's, for
    /// a run of any length.
    fn holds_any(&self, address: usize, length: usize) -> bool {
        let start = address.max(self.base);
        let end = address.saturating_add(length).min(self.base + self.span);
        (start..end).any(|address| {
            let offset = address - self.base;
            self.bits[offset >> 3] & (1 << (offset & 7)) != 0
        })
    }
}

#[derive(Debug)]
#[wasm_bindgen]
pub struct Memory {
    data: Vec<u8>,
    instructions: InstructionMap,
}

impl Memory {
    pub fn new() -> Self {
        Self {
            data: vec![255; 0x01000000], //16mb
            instructions: InstructionMap::default(),
        }
    }
    /// Marks the bytes `instructions` take up, which the program's loads and
    /// stores are refused from then on.
    pub fn set_instructions(&mut self, instructions: &[AssembledInstruction]) {
        self.instructions = InstructionMap::new(instructions);
    }
    /// Refuses a run of `length` bytes at `address` that touches an
    /// instruction, for an access the typed reads and writes do not go
    /// through: a trap task's string or buffer, or a Poke.
    pub fn verify_not_instruction(
        &self,
        address: usize,
        length: usize,
        write: bool,
    ) -> RuntimeResult<()> {
        let address = address & 0x00ffffff;
        if self.instructions.holds_any(address, length) {
            return Err(RuntimeError::InstructionAccess { address, write });
        }
        Ok(())
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
        let address = self.verify_access(address, Size::Long, false)?;

        Ok(u32::from_be_bytes(
            self.data[address..address + 4].try_into().unwrap(),
        ))
    }
    pub fn read_word(&self, address: usize) -> RuntimeResult<u16> {
        let address = self.verify_access(address, Size::Word, false)?;
        Ok(u16::from_be_bytes(
            self.data[address..address + 2].try_into().unwrap(),
        ))
    }
    pub fn read_byte(&self, address: usize) -> RuntimeResult<u8> {
        let address = self.verify_access(address, Size::Byte, false)?;
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

    /// Stores `data` as [`write_size`](Self::write_size) does and answers the
    /// value it replaced, read at the same size: a store that journals what it
    /// overwrote, checked once, and as a store.
    pub fn replace_size(&mut self, address: usize, size: Size, data: u32) -> RuntimeResult<u32> {
        let address = self.verify_access(address, size, true)?;
        let bytes = &mut self.data[address..address + size.to_bytes()];
        let old = bytes.iter().fold(0, |old, byte| (old << 8) | *byte as u32);
        bytes.copy_from_slice(&data.to_be_bytes()[4 - size.to_bytes()..]);
        Ok(old)
    }

    #[inline(always)]
    pub fn verify_address_bounds(&self, address: usize, length: usize) -> RuntimeResult<usize> {
        //m68k does not use the last 2 bytes of the address space, clamp it to 24 bits
        let address = address & 0x00ffffff;
        //a sum that overflows is past the end too: on wasm32 a length from JavaScript near
        //4 GB wrapped round to a small end, passed, and panicked slicing
        let end_address = address.checked_add(length).unwrap_or(usize::MAX);
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
    /// [`verify_address`](Self::verify_address) for a load or store of the
    /// program, which is also refused when it touches an instruction.
    #[inline(always)]
    fn verify_access(&self, address: usize, size: Size, write: bool) -> RuntimeResult<usize> {
        let address = self.verify_address(address, size)?;
        if self.instructions.holds(address, size.to_bytes()) {
            return Err(RuntimeError::InstructionAccess { address, write });
        }
        Ok(address)
    }
    pub fn write_long(&mut self, address: usize, value: u32) -> RuntimeResult<()> {
        let address = self.verify_access(address, Size::Long, true)?;
        self.data[address..address + 4].copy_from_slice(&value.to_be_bytes());
        Ok(())
    }
    pub fn write_word(&mut self, address: usize, value: u16) -> RuntimeResult<()> {
        let address = self.verify_access(address, Size::Word, true)?;
        self.data[address..address + 2].copy_from_slice(&value.to_be_bytes());
        Ok(())
    }
    pub fn write_byte(&mut self, address: usize, value: u8) -> RuntimeResult<()> {
        let address = self.verify_access(address, Size::Byte, true)?;
        self.data[address] = value;
        Ok(())
    }
    /// Writes a run of bytes as they are, instructions or not: the Program's
    /// initial contents and undo. A write of the program's or the host's checks
    /// [`verify_not_instruction`](Self::verify_not_instruction) first.
    pub fn write_bytes(&mut self, address: usize, bytes: &[u8]) -> RuntimeResult<()> {
        let address = self.verify_address_bounds(address, bytes.len())?;
        self.data[address..address + bytes.len()].copy_from_slice(bytes);
        Ok(())
    }
    /// Reads a run of bytes as they are, instructions or not, which is how the
    /// host inspects memory.
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
    /// Data register `index`. It throws an `InvalidArgument` for an index past
    /// 7, where it used to panic and leave the module unusable.
    pub fn wasm_get_d_reg(&self, index: usize) -> Result<Register, JsValue> {
        self.d_reg.get(index).copied().ok_or_else(|| {
            js(&RuntimeError::InvalidArgument(format!(
                "there is no data register d{}",
                index
            )))
        })
    }
    pub fn wasm_get_d_regs_value(&self) -> Vec<u32> {
        self.d_reg.iter().map(|reg| reg.get_long()).collect()
    }

    pub fn wasm_get_a_regs_value(&self) -> Vec<u32> {
        self.a_reg.iter().map(|reg| reg.get_long()).collect()
    }
    /// Address register `index`. It throws an `InvalidArgument` for an index
    /// past 7.
    pub fn wasm_get_a_reg(&self, index: usize) -> Result<Register, JsValue> {
        self.a_reg.get(index).copied().ok_or_else(|| {
            js(&RuntimeError::InvalidArgument(format!(
                "there is no address register a{}",
                index
            )))
        })
    }
    pub fn wasm_get_ccr(&self) -> Flags {
        self.ccr
    }
    /// The whole status register, `$2700` before a program has run.
    pub fn wasm_get_sr(&self) -> u16 {
        self.get_sr()
    }
}

/// What stopped a running program, or what the host asked of the Interpreter
/// that it could not do.
///
/// **An error an instruction raised ends the program.** s68k keeps no
/// exception vectors and no supervisor stack frame, so where a 68000 would jump
/// through a vector (`SIMHELP/Exceptions.htm`) the run ends with
/// [`InterpreterStatus::TerminatedWithException`], the error is the
/// [`Termination`]'s cause, and undoing the step that raised it brings the
/// program back to the instruction, running. That covers a memory fault, a
/// division by zero, `chk`, `trapv` and `illegal`, and every `trap #15` task
/// the Interpreter cannot carry out:
/// [`UnsupportedTrapTask`](RuntimeError::UnsupportedTrapTask) and
/// [`InvalidTrapArgument`](RuntimeError::InvalidTrapArgument).
///
/// The others are the host's: an answer refused
/// ([`NoPendingInterrupt`](RuntimeError::NoPendingInterrupt),
/// [`InvalidAnswer`](RuntimeError::InvalidAnswer)), a value from JavaScript
/// that is not what a call takes ([`InvalidArgument`](RuntimeError::InvalidArgument)),
/// the run's own [`ExecutionLimit`](RuntimeError::ExecutionLimit), and
/// [`Raw`](RuntimeError::Raw) for a call made at the wrong time, such as a step
/// after the end. None of them changes the program.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", content = "value")]
pub enum RuntimeError {
    Raw(String),
    ExecutionLimit(usize),
    OutOfBounds(String),
    AddressError {
        address: usize,
        size: Size,
    },
    /// A load or store that touches the bytes of an instruction. The
    /// instructions are not in memory (see [`Memory::set_instructions`]), so
    /// there is nothing there to read or to change.
    InstructionAccess {
        /// The first byte of the access.
        address: usize,
        /// Whether it was a store.
        write: bool,
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
    /// A `trap #15` task s68k does not carry out, which is every number in
    /// D0.B that is not one of EASy68K's tasks and the tasks a page in a
    /// browser cannot do faithfully: the printer (10), the text window's font
    /// and contents (21, 22, 25), the cycle counter (30, 31), the hardware
    /// window (32), the serial ports (40 to 43), the interrupt requests (60,
    /// 62) and the network (100 to 107). The host says why from the number.
    UnsupportedTrapTask {
        /// The task, D0.B.
        task: u8,
    },
    /// A `trap #15` task given a value it cannot take: a base outside 2 to 36
    /// for task 15, a mode task 16, 58, 61 or 92 does not have, a string with
    /// no NUL. EASy68K does nothing for most of these; s68k stops and says why.
    InvalidTrapArgument {
        /// The task, D0.B.
        task: u8,
        /// What was wrong, in a sentence that names the register.
        reason: String,
    },
    /// An answer when no interrupt is waiting for one.
    NoPendingInterrupt,
    /// An answer the pending interrupt cannot take: one of another name, one
    /// whose value the task cannot take, such as more bytes than a read asked
    /// for, or a value that is not an answer at all. The interrupt still waits.
    InvalidAnswer {
        /// The name of the pending interrupt.
        interrupt: String,
        /// What was wrong with the answer.
        reason: String,
    },
    /// A value from the host that is not what the call takes: a register that
    /// does not exist, a breakpoint that is not `{ file, line }`.
    InvalidArgument(String),
}

pub type RuntimeResult<T> = Result<T, RuntimeError>;

/// Why a program ended: what [`Interpreter::get_termination`] answers once the
/// Interpreter has [terminated](Interpreter::has_terminated).
///
/// Undoing the step that ended the program takes it back, with the end.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", content = "value")]
pub enum Termination {
    /// Task 9, which ends the program on purpose.
    TerminateTask,
    /// The program counter left the last instruction, or the Program had no
    /// instruction to run.
    EndOfProgram,
    /// The host answered an interrupt with [`InterruptResult::Terminate`].
    TerminatedByHost,
    /// An instruction raised a runtime error, and the program ended with an
    /// exception: [`InterpreterStatus::TerminatedWithException`].
    Exception(RuntimeError),
}

/// The most characters tasks 0 and 1 display: EASy68K copies the string into a
/// buffer of 256 bytes and ends it at the 255th character at the latest
/// (`CODE9.CPP`).
const DISPLAY_LIMIT: usize = 255;

/// The most characters a line read by tasks 2, 4 and 18 keeps: EASy68K ends a
/// line at the 80th key and keeps the 79 typed before it (`simIOu.cpp`,
/// `FormKeyPress`).
pub(crate) const LINE_LIMIT: usize = 79;

/// The bytes a line typed for tasks 2, 4 and 18 leaves in EASy68K's input
/// buffer: its characters up to the first line terminator, because Enter ends
/// a line and is never part of it, at most [`LINE_LIMIT`] of them, in
/// Windows-1252 with `?` for a character that has no byte.
fn typed_line(line: &str) -> Vec<u8> {
    line.chars()
        .take_while(|character| !matches!(character, '\r' | '\n'))
        .take(LINE_LIMIT)
        .map(charset::typed_byte)
        .collect()
}

/// The byte a key typed for task 5 is stored as.
///
/// EASy68K stores the character Windows hands it, which is `'\r'` for Enter, so
/// a program waits for `$0D`; a host that types Enter as `'\n'` gets the same
/// `$0D`.
fn typed_key(key: char) -> u8 {
    match key {
        '\n' => b'\r',
        _ => charset::typed_byte(key),
    }
}

/// The bytes of memory there are: the 16 MB of the 68000's 24 bit address
/// space, EASy68K's `MEMSIZE` (`def.h`).
const MEMORY_SIZE: usize = 0x0100_0000;

/// The most characters EASy68K takes of a file name, a dialog's title and
/// filter, or a sound file name: it copies each into a buffer of 256 bytes and
/// ends it at the 255th character (`CODE9.CPP`, `strncpy(buf, inStr, 255)`).
const NAME_LIMIT: usize = 255;

/// How many bytes task 58 writes at (A3): the chosen path, at most
/// [`NAME_LIMIT`] characters, and NULs to the end of a 256 byte buffer, which
/// is what `strncpy(inPath, name, 255)` and `inPath[255] = '\0'` leave
/// (`simIOu.cpp`, `displayFileDialog`).
const DIALOG_PATH_BYTES: usize = NAME_LIMIT + 1;

/// How many files can be open at once, numbered 0 to 7: EASy68K's `MAXFILES`
/// (`def.h`).
const FILE_HANDLES: u32 = 8;

/// The longest NUL terminated string the text tasks read: EASy68K reads a
/// string for as long as it goes, and 16384 bytes with no NUL is a missing
/// terminator rather than a string anybody wrote.
const STRING_LIMIT: usize = 16384;

/// EASy68K's results for the file tasks, written to D0.W (`def.h`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FileResult {
    Success = 0,
    EndOfFile = 1,
    Error = 2,
    ReadOnly = 3,
}

/// The path at an address, as the file and sound tasks take it: at most
/// [`NAME_LIMIT`] characters, decoded from Windows-1252, and with `\` written as
/// `/`, because Windows takes either as a separator and a host need only know
/// the one.
fn as_path(name: String) -> String {
    name.replace('\\', "/")
}

/// The bytes task 58 writes for a chosen path: the path in Windows-1252, `?`
/// for a character with no byte, at most [`NAME_LIMIT`] of them, and NULs to
/// [`DIALOG_PATH_BYTES`].
fn dialog_path_bytes(path: &str) -> Vec<u8> {
    let mut bytes: Vec<u8> = path
        .chars()
        .take(NAME_LIMIT)
        .map(charset::typed_byte)
        .collect();
    bytes.resize(DIALOG_PATH_BYTES, 0);
    bytes
}

/// Whether `count` bytes from `address` fit below the end of memory, which is
/// EASy68K's check before a file read or write. The address is the 24 bits the
/// 68000 puts on its bus, and the sum is taken without wrapping, where
/// EASy68K's 32 bit sum wraps for a count near 4 GB and goes on to read past
/// its memory.
fn fits_in_memory(address: u32, count: u32) -> bool {
    (address as usize & (MEMORY_SIZE - 1)) + count as usize <= MEMORY_SIZE
}

/// Whether `answer` can answer the `pending` interrupt: it has to be the
/// variant of the same name, or `Terminate`, and carry a value the task can
/// take. The task decides where an answer is written and how it is read, so an
/// answer for another task is never a near miss to make the best of.
fn check_answer(pending: &Interrupt, answer: &InterruptResult) -> RuntimeResult<()> {
    let refuse = |reason: String| {
        Err(RuntimeError::InvalidAnswer {
            interrupt: pending.name().to_string(),
            reason,
        })
    };
    if matches!(answer, InterruptResult::Terminate) {
        return Ok(());
    }
    if answer.name() != pending.name() {
        return refuse(format!(
            "a {} answer cannot answer a {} interrupt",
            answer.name(),
            pending.name()
        ));
    }
    match (pending, answer) {
        (
            Interrupt::GetKeyState(KeyStateRequest::Keys(_)),
            InterruptResult::GetKeyState(KeyStateResult::LastKeys { .. }),
        ) => refuse(
            "the program asked whether four keys are down, and the answer is the last keys"
                .to_string(),
        ),
        (
            Interrupt::GetKeyState(KeyStateRequest::LastKeys),
            InterruptResult::GetKeyState(KeyStateResult::Keys(_)),
        ) => refuse(
            "the program asked for the last keys, and the answer is whether four keys are down"
                .to_string(),
        ),
        (Interrupt::ReadFile { count, .. }, InterruptResult::ReadFile(Some(bytes)))
            if bytes.as_slice().len() > *count as usize =>
        {
            refuse(format!(
                "the program asked for at most {} bytes, and the answer carries {}",
                count,
                bytes.as_slice().len()
            ))
        }
        (_, InterruptResult::OpenFile(Some(file))) if file.handle as u32 >= FILE_HANDLES => {
            refuse(format!(
                "file number {} is not one of EASy68K's eight, 0 to 7",
                file.handle
            ))
        }
        (_, InterruptResult::NewFile(Some(handle))) if *handle as u32 >= FILE_HANDLES => {
            refuse(format!(
                "file number {} is not one of EASy68K's eight, 0 to 7",
                handle
            ))
        }
        _ => Ok(()),
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Copy)]
#[wasm_bindgen]
pub enum InterpreterStatus {
    Running,
    Interrupt,
    Terminated,
    TerminatedWithException,
    /// Execution stopped at `simhalt`. Calling any run or step method resumes
    /// at the following instruction.
    ///
    /// This variant is last because wasm-bindgen exports this enum as numbers;
    /// keeping the older variants in place preserves their public values.
    Paused,
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
    stack_top: usize,
    memory: Memory,
    cpu: Cpu,
    pc: usize,
    /// What is being run. The instructions are looked up in it by address
    /// rather than kept in memory, which is less like a real processor and
    /// keeps a wild write from rewriting the program.
    program: Program,
    debugger: Debugger,
    keep_history: bool,
    /// The open Poke, if the host has begun one: what its writes have journaled
    /// so far, which [`end_poke`](Interpreter::end_poke) turns into one step of
    /// the history.
    poke: Option<PokeJournal>,
    /// Whether an instruction is running: from the moment `step` hands it to
    /// `execute_instruction` until it is finished, which for an instruction
    /// that raised an Interrupt is when the host answers it.
    ///
    /// It is what tells a write the program made from a write the host made:
    /// only the first belongs in the mutations of the step being executed.
    executing: bool,
    /// The address of the instruction being executed, and after the step, of
    /// the one that has just run. It is 0 before the first step, when none has.
    current_instruction_address: usize,
    /// One past the last byte of the last instruction: a program counter that
    /// reaches it has walked off the bottom of the program.
    end_address: usize,
    current_interrupt: Option<Interrupt>,
    status: InterpreterStatus,
    /// Why the program ended, once it has: set with the terminated status and
    /// taken back with it by undo.
    termination: Option<Termination>,
    /// How the read tasks show what is typed, which tasks 12 and 16 change.
    input_settings: InputSettings,
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
        memory.set_instructions(program.instructions());
        let runnable = !program.is_empty() && start < end;
        let mut interpreter = Self {
            stack_top: sp,
            memory,
            cpu: Cpu::new(),
            pc: start,
            end_address: end,
            keep_history: options.keep_history,
            poke: None,
            executing: false,
            current_instruction_address: 0,
            debugger: Debugger::new(options.history_size, program.symbols()),
            current_interrupt: None,
            status: if runnable {
                InterpreterStatus::Running
            } else {
                InterpreterStatus::Terminated
            },
            termination: if runnable {
                None
            } else {
                Some(Termination::EndOfProgram)
            },
            input_settings: InputSettings::default(),
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

    /// Ends the program, for `cause`: [`InterpreterStatus::TerminatedWithException`]
    /// for an [exception](Termination::Exception) and
    /// [`InterpreterStatus::Terminated`] for the rest.
    ///
    /// It is the one way a program ends, so the status and its cause never
    /// disagree. A program that has ended already keeps the cause it ended
    /// with.
    fn terminate(&mut self, cause: Termination) {
        if self.has_terminated() {
            return;
        }
        self.status = match cause {
            Termination::Exception(_) => InterpreterStatus::TerminatedWithException,
            _ => InterpreterStatus::Terminated,
        };
        self.termination = Some(cause);
    }

    /// Ends the run with an exception and answers the error that says why.
    ///
    /// A 68000 would push a stack frame and jump through the vector of
    /// `SIMHELP/Exceptions.htm`; s68k has neither, so every error an
    /// instruction raises — an address error, `chk`, `trapv`, `illegal`, a
    /// `trap` task it cannot carry out — stops the program:
    /// [`InterpreterStatus::TerminatedWithException`], with the
    /// [`RuntimeError`] naming the cause as the [`Termination`], and the
    /// Location of the line is the one the step recorded.
    fn end_with_an_exception(&mut self, error: RuntimeError) -> RuntimeError {
        self.terminate(Termination::Exception(error.clone()));
        error
    }

    /// Why the program ended, or `None` while it has not.
    pub fn get_termination(&self) -> Option<&Termination> {
        self.termination.as_ref()
    }

    /// How the read tasks show what is typed: the echo of task 12 and the
    /// prompt and line feed of task 16, all on when a program starts.
    pub fn get_input_settings(&self) -> InputSettings {
        self.input_settings
    }

    /// Replaces the [`InputSettings`], journaling the ones it replaced into the
    /// step being executed so that undo puts them back. Settings equal to the
    /// ones in force change nothing and journal nothing.
    fn set_input_settings(&mut self, settings: InputSettings) {
        if settings == self.input_settings {
            return;
        }
        if self.executing && self.keep_history {
            self.debugger
                .add_mutation(MutationOperation::SetInputSettings {
                    old: self.input_settings,
                    new: settings,
                });
        }
        self.input_settings = settings;
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
    /// Whether no execution can follow. A paused Interpreter has not
    /// terminated and can be resumed by a run or step method.
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

    /// Runs one instruction. If the Interpreter is paused, this resumes at the
    /// instruction after the `simhalt` that paused it.
    ///
    /// An instruction that raises a runtime error ends the program with an
    /// exception, [`InterpreterStatus::TerminatedWithException`], and the error
    /// is answered as well as recorded as the [`Termination`]. The step stays in
    /// the history, so undoing it brings the program back to the instruction
    /// that failed, running.
    pub fn step(&mut self) -> RuntimeResult<InterpreterStatus> {
        let sp_before = self.get_sp();
        let old_status = self.status;
        self.verify_can_run()?;
        if old_status == InterpreterStatus::Paused {
            if self.has_reached_bottom() {
                self.terminate(Termination::EndOfProgram);
                return Ok(self.status);
            }
            self.set_status(InterpreterStatus::Running);
        }
        if self.keep_history {
            self.debugger.add_step(ExecutionStep::new(
                self.pc,
                self.cpu.ccr,
                self.cpu.get_sr(),
                old_status,
            ));
        }
        self.current_instruction_address = self.pc;
        let instruction = self
            .get_instruction_at(self.pc)
            .map(|i| (i.size, i.instruction));
        match instruction {
            Some((size, ins)) => {
                if self.keep_history {
                    //cloned only when a history is kept: a Location holds the path of its File,
                    //and a step nobody can undo should not pay for it
                    let location = self.get_instruction_at(self.pc).map(|i| i.location.clone());
                    self.debugger.set_location(location);
                }
                self.increment_pc(size);
                self.executing = true;
                let executed = self.execute_instruction(&ins);
                let sp = self.get_sp();
                let top = if sp.abs_diff(sp_before) > 4096 || sp > self.stack_top { sp } else { self.stack_top };
                if top != self.stack_top {
                    if self.keep_history { self.debugger.record_stack_top(self.stack_top); }
                    self.stack_top = top;
                }
                //an instruction that raised an Interrupt is not finished until the host answers
                //it, and what `answer_interrupt` writes belongs to this step
                self.executing = self.status == InterpreterStatus::Interrupt;
                if self.keep_history {
                    self.debugger.set_new_ccr(self.cpu.ccr);
                    self.debugger.set_new_sr(self.cpu.get_sr());
                }
                if let Err(error) = executed {
                    return Err(self.end_with_an_exception(error));
                }
                if self.status == InterpreterStatus::Running && self.has_reached_bottom() {
                    self.terminate(Termination::EndOfProgram);
                }
                Ok(self.status)
            }
            None if self.pc < self.end_address => Err(self.end_with_an_exception(
                RuntimeError::OutOfBounds(format!("Invalid instruction address: {}", self.pc,)),
            )),
            None => {
                Err(self
                    .end_with_an_exception(RuntimeError::Raw("Program has terminated".to_string())))
            }
        }
    }
    pub fn get_pretty_call_stack(&self) -> Vec<PrettyStackFrame> {
        self.debugger.to_call_stack()
    }
    pub fn undo(&mut self) -> RuntimeResult<ExecutionStep> {
        match self.debugger.pop_step() {
            Some(step) => {
                if let Some(top) = step.old_stack_top { self.stack_top = top; }
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
                            new: _,
                            size: _,
                        } => match register {
                            RegisterOperand::Address(reg) => {
                                self.cpu.a_reg[*reg as usize].store_long(*old)
                            }
                            RegisterOperand::Data(reg) => {
                                self.cpu.d_reg[*reg as usize].store_long(*old)
                            }
                        },
                        MutationOperation::WriteMemory {
                            address,
                            old,
                            new: _,
                            size,
                        } => {
                            self.memory.write_size(*address, *size, *old)?;
                        }
                        MutationOperation::WriteMemoryBytes {
                            address,
                            old,
                            new: _,
                        } => {
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
                        MutationOperation::SetInputSettings { old, new: _ } => {
                            self.input_settings = *old;
                        }
                    }
                }
                //whatever an instruction left behind is gone with it: the interrupt an undone trap
                //was waiting on, the instruction that was executing until it was answered, and the
                //end of the program it caused. A step never begins while an interrupt waits, so
                //the status it recorded is never one. A Poke is made between instructions and
                //takes back only what it wrote
                self.status = step.get_interpreter_status();
                if step.get_kind() == ExecutionStepKind::Instruction {
                    self.current_interrupt = None;
                    self.executing = false;
                }
                if !self.has_terminated() {
                    self.termination = None;
                }
                Ok(step)
            }
            None => Err(RuntimeError::Raw("No more steps to undo".to_string())),
        }
    }

    /// Whether there is a step to undo, instruction or Poke alike.
    pub fn can_undo(&self) -> bool {
        self.debugger.can_undo()
    }

    /// The newest `count` steps of the history, newest first, a Poke among them
    /// in the place it was made.
    pub fn get_last_steps(&self, count: usize) -> Vec<&ExecutionStep> {
        self.debugger.get_last_steps(count)
    }

    /// The id of the newest step, or 0 when there is none. A Poke has an id of
    /// its own, so this moves past it.
    pub fn get_last_step_id(&self) -> u64 {
        self.debugger
            .get_last_step()
            .map_or(0, |step| step.get_id())
    }

    /// Whether an instruction is running, which is also true while an Interrupt
    /// it raised waits for its answer.
    #[inline(always)]
    pub fn is_executing(&self) -> bool {
        self.executing || self.status == InterpreterStatus::Interrupt
    }

    /// Whether a Poke is open.
    pub fn is_poking(&self) -> bool {
        self.poke.is_some()
    }

    /// Opens a Poke: the host is about to change register or memory values
    /// between two instructions, and everything
    /// [`set_register_value`](Interpreter::set_register_value) and
    /// [`write_memory_bytes`](Interpreter::write_memory_bytes) write until
    /// [`end_poke`](Interpreter::end_poke) becomes one step of the history.
    ///
    /// It refuses a Poke inside a Poke, and a Poke while an instruction is
    /// running: those writes are the program's own and belong to its step.
    pub fn begin_poke(&mut self) -> RuntimeResult<()> {
        if self.poke.is_some() {
            return Err(RuntimeError::Raw(
                "A poke is already open, end it before beginning another".to_string(),
            ));
        }
        if self.is_executing() {
            return Err(RuntimeError::Raw(
                "Cannot begin a poke while an instruction is executing".to_string(),
            ));
        }
        self.poke = Some(PokeJournal::new());
        Ok(())
    }

    /// Closes the open Poke and records it as one step of the history, and
    /// answers whether it recorded one.
    ///
    /// A Poke that wrote nothing — no write at all, or only writes that left
    /// the value they found — records no step and answers `false`, and so does
    /// one made by an Interpreter that keeps no history. The step it does
    /// record has an id of its own, the mutations that undo replays and the old
    /// and new value of everything the Poke wrote.
    pub fn end_poke(&mut self) -> RuntimeResult<bool> {
        let journal = match self.poke.take() {
            Some(journal) => journal,
            None => {
                return Err(RuntimeError::Raw(
                    "No poke is open, begin one before ending it".to_string(),
                ))
            }
        };
        if journal.is_empty() || !self.keep_history {
            return Ok(false);
        }
        let (mutations, targets) = journal.into_parts();
        //the new value is what the Poke leaves behind, read now, so that a register written
        //twice reports the value it ends on and not one it passed through
        let writes = targets
            .into_iter()
            .map(|target| match target {
                PokeTarget::Register { register, old } => PokeWrite::Register {
                    name: register.name(),
                    old,
                    new: self.get_register_value(register, Size::Long),
                },
                PokeTarget::Memory { address, old } => {
                    let new = self
                        .memory
                        .read_bytes(address, old.len())
                        .map(|bytes| bytes.to_vec())
                        .unwrap_or_default();
                    PokeWrite::Memory { address, old, new }
                }
            })
            .collect::<Vec<PokeWrite>>();
        //no instruction ran, so the step carries the program counter, the condition codes and
        //the status register as they are: undoing it puts back only what the Poke wrote
        let mut step =
            ExecutionStep::new_poke(self.pc, self.cpu.ccr, self.cpu.get_sr(), self.status);
        for mutation in mutations {
            step.add_mutation(mutation);
        }
        step.set_writes(writes);
        self.debugger.add_step(step);
        Ok(true)
    }
    /// Answers the pending interrupt, writes what the answer carries, and
    /// finishes the instruction that raised it.
    ///
    /// The answer has to be the pending interrupt's own: the variant of the
    /// same name, with a value the task can take — the request form task 19
    /// asked for, no more bytes than task 53 asked for, a file number 0 to 7.
    /// It is refused, changing nothing and leaving the interrupt pending, with
    /// [`RuntimeError::NoPendingInterrupt`] when nothing waits for it, with
    /// [`RuntimeError::InvalidAnswer`] when it is not the pending interrupt's,
    /// with the [`RuntimeError::OutOfBounds`] of the write when what it writes
    /// does not fit in memory, and with [`RuntimeError::InstructionAccess`]
    /// when it would write over an instruction.
    ///
    /// [`InterruptResult::Terminate`] answers any task, and ends the program:
    /// it is how a host that cannot do a task stops the run.
    pub fn answer_interrupt(&mut self, answer: InterruptResult) -> RuntimeResult<()> {
        let pending = match self.current_interrupt.take() {
            Some(interrupt) if self.status == InterpreterStatus::Interrupt => interrupt,
            other => {
                self.current_interrupt = other;
                return Err(RuntimeError::NoPendingInterrupt);
            }
        };
        let terminates = matches!(answer, InterruptResult::Terminate);
        let answered = check_answer(&pending, &answer).and_then(|()| self.apply_answer(answer));
        if let Err(error) = answered {
            self.current_interrupt = Some(pending);
            return Err(error);
        }
        self.executing = false;
        //an answer of Terminate ends the program, and so does the answer to an interrupt the
        //last instruction raised
        if terminates {
            self.terminate(Termination::TerminatedByHost);
        } else if self.has_reached_bottom() {
            self.terminate(Termination::EndOfProgram);
        } else {
            self.status = InterpreterStatus::Running;
        }
        Ok(())
    }

    /// Writes what an answer [`check_answer`] took carries, as EASy68K writes
    /// it for the task. It fails, having written nothing, only when a run of
    /// bytes does not fit in memory or lands on an instruction.
    fn apply_answer(&mut self, answer: InterruptResult) -> RuntimeResult<()> {
        match answer {
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
            | InterruptResult::DisplayStringAndNumber
            | InterruptResult::LoadSound => {}
            InterruptResult::ReadKeyboardString(line) => {
                //EASy68K writes the characters and the NUL after them, and the count through a
                //`long *` to D1, which is the whole long (`simIOu.cpp`, `FormKeyPress`)
                let typed = typed_line(&line);
                let mut buffer = Vec::with_capacity(typed.len() + 1);
                buffer.extend_from_slice(&typed);
                buffer.push(0);
                let address = self.cpu.a_reg[1].get_long() as usize;
                self.set_memory_bytes(address, &buffer)?;
                self.set_register_value(RegisterOperand::Data(1), typed.len() as u32, Size::Long);
            }
            InterruptResult::ReadNumber(line)
            | InterruptResult::DisplayStringAndReadNumber(line) => {
                let number = c_runtime::atoi(&typed_line(&line));
                self.set_register_value(RegisterOperand::Data(1), number as u32, Size::Long);
            }
            InterruptResult::ReadChar(key) => {
                //only the low byte, as EASy68K writes the key through a `char *` to D1
                self.set_register_value(
                    RegisterOperand::Data(1),
                    typed_key(key).into(),
                    Size::Byte,
                );
            }
            InterruptResult::GetTime(time) => {
                self.set_register_value(RegisterOperand::Data(1), time, Size::Long);
            }
            //it ends the program in `answer_interrupt`, which sets the status of every answer
            InterruptResult::Terminate => {}
            InterruptResult::GetPixelColor(color) => {
                self.set_register_value(RegisterOperand::Data(0), color, Size::Long);
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
            // ---- files: what the host's file system did, as EASy68K's result codes
            // (`SIMOPS2.CPP`), D0.W through a `short *` and the file number through a `long *`
            InterruptResult::CloseAllFiles(done)
            | InterruptResult::WriteFile(done)
            | InterruptResult::PositionFile(done)
            | InterruptResult::CloseFile(done)
            | InterruptResult::DeleteFile(done) => {
                self.set_file_result(if done {
                    FileResult::Success
                } else {
                    FileResult::Error
                });
            }
            InterruptResult::OpenFile(opened) => match opened {
                Some(file) => {
                    self.set_file_number(file.handle as u32);
                    self.set_file_result(if file.read_only {
                        FileResult::ReadOnly
                    } else {
                        FileResult::Success
                    });
                }
                None => {
                    self.set_file_number(u32::MAX);
                    self.set_file_result(FileResult::Error);
                }
            },
            InterruptResult::NewFile(handle) => match handle {
                Some(handle) => {
                    self.set_file_number(handle as u32);
                    self.set_file_result(FileResult::Success);
                }
                None => {
                    self.set_file_number(u32::MAX);
                    self.set_file_result(FileResult::Error);
                }
            },
            //`fread` answering fewer bytes than asked for is a success that says how many it
            //read; none at all, with the end of the file reached, is the end of file and leaves
            //D2.L alone (`readFile`)
            InterruptResult::ReadFile(read) => match read {
                None => self.set_file_result(FileResult::Error),
                Some(bytes) if bytes.as_slice().is_empty() => {
                    self.set_file_result(FileResult::EndOfFile)
                }
                Some(bytes) => {
                    let address = self.cpu.a_reg[1].get_long() as usize;
                    self.set_memory_bytes(address, bytes.as_slice())?;
                    self.set_register_value(
                        RegisterOperand::Data(2),
                        bytes.as_slice().len() as u32,
                        Size::Long,
                    );
                    self.set_file_result(FileResult::Success);
                }
            },
            //D0.W is 0 whatever the user did, 2 only when the name does not fit, and D1.L says
            //whether a file was chosen (`displayFileDialog`)
            InterruptResult::FileDialog(chosen) => match chosen {
                None => {
                    self.set_register_value(RegisterOperand::Data(1), 0, Size::Long);
                    self.set_file_result(FileResult::Success);
                }
                Some(path) => {
                    let address = self.cpu.a_reg[3].get_long();
                    if fits_in_memory(address, DIALOG_PATH_BYTES as u32) {
                        self.set_memory_bytes(address as usize, &dialog_path_bytes(&path))?;
                        self.set_register_value(RegisterOperand::Data(1), 1, Size::Long);
                        self.set_file_result(FileResult::Success);
                    } else {
                        self.set_file_result(FileResult::Error);
                    }
                }
            },
            InterruptResult::FileExists(existence) => self.set_file_result(match existence {
                FileExistence::Writable => FileResult::Success,
                FileExistence::ReadOnly => FileResult::ReadOnly,
                FileExistence::Missing => FileResult::Error,
            }),
            // ---- sound: 1 in D0.W when it happened, 0 when it did not
            InterruptResult::PlaySound(done)
            | InterruptResult::PlayLoadedSound(done)
            | InterruptResult::PlaySoundDirectX(done)
            | InterruptResult::LoadSoundDirectX(done)
            | InterruptResult::PlayLoadedSoundDirectX(done)
            | InterruptResult::ControlSound(done)
            | InterruptResult::ControlSoundDirectX(done) => {
                self.set_register_value(RegisterOperand::Data(0), done as u32, Size::Word);
            }
        };
        Ok(())
    }

    /// Writes a file task's result to D0.W, as EASy68K does through a
    /// `short *` to D0.
    fn set_file_result(&mut self, result: FileResult) {
        self.set_register_value(RegisterOperand::Data(0), result as u32, Size::Word);
    }

    /// Writes a file number, or -1 for none, to the whole of D1.L, as EASy68K
    /// does through a `long *` to D1.
    fn set_file_number(&mut self, number: u32) {
        self.set_register_value(RegisterOperand::Data(1), number, Size::Long);
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
            None => Err(RuntimeError::NoPendingInterrupt),
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
                        //the return address, which is what the push below stores there
                        new: self.pc as u32,
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
                        //the return address, which is what the push below stores there
                        new: self.pc as u32,
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
                    //the push writes below the stack pointer, so the value it replaces is the
                    //one at that address and not the one the pointer is on
                    let old_address = self.get_sp().wrapping_sub(4);
                    let old_value = self.memory.read_long(old_address)?;
                    self.debugger.add_mutation(MutationOperation::WriteMemory {
                        address: old_address,
                        old: old_value,
                        new: addr,
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
            // `simhalt` pauses after the instruction and touches no register.
            // `step` advances the PC before executing it, so the next run or
            // step call resumes at the following instruction.
            Instruction::SIMHALT => self.set_status(InterpreterStatus::Paused),
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
            Instruction::TRAP(vector) => match vector {
                15 => {
                    let task = self.cpu.d_reg[0].get_byte();
                    //a task the Interpreter does itself, a setting or a file result it can decide
                    //alone, raises no interrupt and finishes here
                    if let Some(interrupt) = self.trap_task(task)? {
                        //task 9 ends the program, and is left as the interrupt it was
                        match &interrupt {
                            Interrupt::Terminate => self.terminate(Termination::TerminateTask),
                            _ => self.set_status(InterpreterStatus::Interrupt),
                        }
                        self.current_interrupt = Some(interrupt);
                    }
                }
                //the Assembler refuses every other vector, as s68k has no exception vectors
                _ => {
                    return Err(RuntimeError::Raw(format!(
                        "Unknown trap: {}, only IO with #15 allowed",
                        vector
                    )))
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
        //what the program writes belongs to the step being executed; what the host writes
        //belongs to the open Poke, if there is one, and to nothing at all if there is not
        if self.executing {
            if self.keep_history {
                //the whole register the store leaves behind, read back the way the old value
                //was read, because a sized store changes only part of one
                let new_value = self.get_register_value(register, Size::Long);
                self.debugger
                    .add_mutation(MutationOperation::WriteRegister {
                        register,
                        old: old_value,
                        new: new_value,
                        size,
                    });
            }
        } else if self.poke.is_some() {
            //a write that changed nothing journals nothing
            let new_value = self.get_register_value(register, Size::Long);
            if new_value == old_value {
                return;
            }
            let poke = self.poke.as_mut().expect("a poke is open");
            poke.add_mutation(MutationOperation::WriteRegister {
                register,
                old: old_value,
                new: new_value,
                size,
            });
            poke.add_register_target(register, old_value);
        }
    }

    pub fn set_memory_value(
        &mut self,
        address: usize,
        size: Size,
        value: u32,
    ) -> RuntimeResult<()> {
        let old_value = self.memory.replace_size(address, size, value)?;
        if self.keep_history {
            self.debugger.add_mutation(MutationOperation::WriteMemory {
                address,
                old: old_value,
                //what the store puts there: `write_size` keeps the low bytes of the value, so
                //the two sides are read at the same width
                new: get_value_sized(value, size),
                size,
            });
        }
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

    /// Writes bytes to memory on behalf of the host.
    ///
    /// Inside a Poke it journals the bytes it overwrote, so that undoing the
    /// Poke puts them back; outside one it is a direct write and records
    /// nothing, which is what a Testcase's preset memory needs. Bytes equal to
    /// the ones already there are no write at all and journal nothing.
    pub fn write_memory_bytes(&mut self, address: usize, bytes: &[u8]) -> RuntimeResult<()> {
        self.memory
            .verify_not_instruction(address, bytes.len(), true)?;
        if self.poke.is_none() {
            return self.memory.write_bytes(address, bytes);
        }
        let old = self.memory.read_bytes(address, bytes.len())?.to_vec();
        if old == bytes {
            return Ok(());
        }
        self.memory.write_bytes(address, bytes)?;
        let poke = self.poke.as_mut().expect("a poke is open");
        poke.add_mutation(MutationOperation::WriteMemoryBytes {
            address,
            old: old.clone(),
            new: bytes.to_vec(),
        });
        poke.add_memory_target(address, old);
        Ok(())
    }

    pub fn set_memory_bytes(&mut self, address: usize, bytes: &[u8]) -> RuntimeResult<()> {
        self.memory
            .verify_not_instruction(address, bytes.len(), true)?;
        if self.keep_history {
            let old_bytes = self.memory.read_bytes(address, bytes.len())?;
            self.debugger
                .add_mutation(MutationOperation::WriteMemoryBytes {
                    address,
                    old: old_bytes.to_vec(),
                    new: bytes.to_vec(),
                });
        }
        self.memory.write_bytes(address, bytes)
    }
    /// The instruction the program counter is on, which is the one the next
    /// step will run.
    pub fn get_next_instruction(&self) -> Option<&AssembledInstruction> {
        self.get_instruction_at(self.pc)
    }
    /// The text of at most `limit` bytes at `address`, up to the first NUL,
    /// decoded from Windows-1252: what tasks 0 and 1 display.
    ///
    /// The bytes are read one at a time, so a string that ends before `limit`
    /// reads nothing past its NUL.
    fn read_string(&self, address: usize, limit: usize) -> RuntimeResult<String> {
        let mut bytes = Vec::new();
        for offset in 0..limit {
            let byte = self.memory.read_byte(address.wrapping_add(offset))?;
            if byte == 0x00 {
                break;
            }
            bytes.push(byte);
        }
        Ok(charset::decode(&bytes))
    }

    /// The NUL terminated string at `address`, decoded from Windows-1252: what
    /// tasks 13, 14, 17, 18 and 95 take at (A1).
    ///
    /// Every byte decodes to a character, so no string is refused for what it
    /// holds. EASy68K reads a string for as long as it goes; this one stops at
    /// [`STRING_LIMIT`] bytes with no NUL, which is a missing terminator rather
    /// than a string anybody wrote, and is an argument error of `task`.
    fn read_null_terminated_string(&self, address: usize, task: u8) -> RuntimeResult<String> {
        let mut bytes = Vec::new();
        loop {
            let byte = self.memory.read_byte(address.wrapping_add(bytes.len()))?;
            if byte == 0x00 {
                break;
            }
            if bytes.len() == STRING_LIMIT {
                return Err(RuntimeError::InvalidTrapArgument {
                    task,
                    reason: format!(
                        "the string at (A1) has no NUL in its first {} bytes",
                        STRING_LIMIT
                    ),
                });
            }
            bytes.push(byte);
        }
        Ok(charset::decode(&bytes))
    }

    /// The path at `address`, as the file and sound tasks take it: the NUL
    /// terminated string there, at most [`NAME_LIMIT`] characters, decoded from
    /// Windows-1252, with `\` written as `/` ([`as_path`]).
    fn read_path(&self, address: u32) -> RuntimeResult<String> {
        Ok(as_path(self.read_string(address as usize, NAME_LIMIT)?))
    }

    /// Carries out `trap #15` task `task`, and answers the interrupt it raises
    /// for the host, or `None` when the Interpreter did the whole task itself:
    /// the input settings of tasks 12 and 16, and a file task whose result it
    /// can decide without the host's file system.
    ///
    /// Every error it answers is the program's, and ends it.
    fn trap_task(&mut self, task: u8) -> RuntimeResult<Option<Interrupt>> {
        match task {
            12 | 16 => {
                self.set_input_setting(task)?;
                Ok(None)
            }
            50..=59 => self.file_task(task),
            _ => self.get_trap(task).map(Some),
        }
    }

    /// Tasks 12 and 16: the echo, the input prompt and the line feed after
    /// Enter, from D1.B (`CODE9.CPP`). Task 12 turns the echo off for 0 and on
    /// for anything else; task 16 takes 0 to 3 and does nothing in EASy68K for
    /// any other value, which s68k reports instead, like the base of task 15.
    fn set_input_setting(&mut self, task: u8) -> RuntimeResult<()> {
        let value = self.cpu.d_reg[1].get_byte();
        let mut settings = self.input_settings;
        match (task, value) {
            (12, 0) => settings.echo = false,
            (12, _) => settings.echo = true,
            (_, 0) => settings.prompt = false,
            (_, 1) => settings.prompt = true,
            (_, 2) => settings.line_feed = false,
            (_, 3) => settings.line_feed = true,
            (_, value) => {
                return Err(RuntimeError::InvalidTrapArgument {
                    task,
                    reason: format!(
                        "D1.B is {}, and task 16 takes 0 or 1 to turn the input prompt off or on, \
                         and 2 or 3 to turn the line feed after Enter off or on",
                        value
                    ),
                })
            }
        }
        self.set_input_settings(settings);
        Ok(())
    }

    /// The file number in D1.L, when it is one of EASy68K's eight: `readFile`
    /// and the other tasks that take one answer 2 for any other at once
    /// (`SIMOPS2.CPP`, `fn < 0 || fn >= MAXFILES`).
    fn file_handle(&self) -> Option<u8> {
        let number = self.cpu.d_reg[1].get_long();
        (number < FILE_HANDLES).then_some(number as u8)
    }

    /// Tasks 50 to 59, the files (`CODE9.CPP` and `SIMOPS2.CPP`).
    ///
    /// The host's file system does the work, so each task is an interrupt
    /// carrying its arguments decoded, and the answer is turned into EASy68K's
    /// result in D0.W. What EASy68K decides before it touches a file is
    /// decided here, with 2 in D0.W and no interrupt: a file number outside 0
    /// to 7 for tasks 53 to 56, a read or a write of no bytes, which `fread`
    /// and `fwrite` fail, a buffer that runs past the end of memory, and a
    /// negative position, which `fseek` refuses.
    fn file_task(&mut self, task: u8) -> RuntimeResult<Option<Interrupt>> {
        let handle = self.file_handle();
        let address = self.cpu.a_reg[1].get_long();
        let interrupt = match task {
            50 => Some(Interrupt::CloseAllFiles),
            51 => Some(Interrupt::OpenFile(self.read_path(address)?)),
            52 => Some(Interrupt::NewFile(self.read_path(address)?)),
            53 | 54 => {
                let count = self.cpu.d_reg[2].get_long();
                match handle {
                    Some(handle) if count > 0 && fits_in_memory(address, count) => {
                        Some(if task == 53 {
                            Interrupt::ReadFile { handle, count }
                        } else {
                            self.memory.verify_not_instruction(
                                address as usize,
                                count as usize,
                                false,
                            )?;
                            let bytes = self
                                .memory
                                .read_bytes(address as usize, count as usize)?
                                .to_vec();
                            Interrupt::WriteFile {
                                handle,
                                bytes: Bytes(bytes),
                            }
                        })
                    }
                    _ => None,
                }
            }
            55 => {
                //`fseek(fp, offset, SEEK_SET)` with the offset an `int`
                let offset = self.cpu.d_reg[2].get_long() as i32;
                match handle {
                    Some(handle) if offset >= 0 => Some(Interrupt::PositionFile {
                        handle,
                        offset: offset as u32,
                    }),
                    _ => None,
                }
            }
            56 => handle.map(Interrupt::CloseFile),
            57 => Some(Interrupt::DeleteFile(self.read_path(address)?)),
            58 => {
                //`switch (*mode)` over the whole of D1.L, and a title or a filter only when its
                //register is not 0 (`simIOu.cpp`, `displayFileDialog`)
                let mode = match self.cpu.d_reg[1].get_long() {
                    0 => FileDialogMode::Open,
                    1 => FileDialogMode::Save,
                    other => {
                        return Err(RuntimeError::InvalidTrapArgument {
                            task,
                            reason: format!(
                                "D1.L is {}, and task 58 takes 0 for the dialog that opens a \
                                 file or 1 for the one that saves one",
                                other
                            ),
                        })
                    }
                };
                let title = match address {
                    0 => String::new(),
                    _ => self.read_string(address as usize, NAME_LIMIT)?,
                };
                let filter = match self.cpu.a_reg[2].get_long() {
                    0 => String::new(),
                    filter => self.read_string(filter as usize, NAME_LIMIT)?,
                };
                let path = self.read_path(self.cpu.a_reg[3].get_long())?;
                Some(Interrupt::FileDialog {
                    mode,
                    title,
                    filter,
                    path,
                })
            }
            //EASy68K reserves D1.L for other operations and reads none of it, so neither does
            //this: any D1.L asks whether the file exists (`fileOp`)
            59 => Some(Interrupt::FileExists(self.read_path(address)?)),
            _ => return Err(RuntimeError::UnsupportedTrapTask { task }),
        };
        if interrupt.is_none() {
            self.set_file_result(FileResult::Error);
        }
        Ok(interrupt)
    }

    /// Every other task: the text and number tasks, the keyboard, the mouse, the
    /// screen and the sound, each an interrupt for the host.
    fn get_trap(&mut self, value: u8) -> RuntimeResult<Interrupt> {
        match value {
            0 | 1 => {
                //EASy68K copies at most 255 characters and ends the copy at the D1.W'th,
                //`strncpy(buf, (A1), 255)` then `buf[(short)D1] = 0`, so both tasks stop at a NUL
                //too. A D1.W of $8000 or more is a negative index there, which writes outside the
                //buffer and leaves all 255 characters of the copy: it is read as more than 255,
                //which shows what EASy68K shows for a string with a NUL in its first 255 bytes.
                let address = self.cpu.a_reg[1].get_long() as usize;
                let length = (self.cpu.d_reg[1].get_word() as usize).min(DISPLAY_LIMIT);
                let text = self.read_string(address, length)?;
                if value == 0 {
                    Ok(Interrupt::DisplayStringWithCRLF(text))
                } else {
                    Ok(Interrupt::DisplayStringWithoutCRLF(text))
                }
            }
            2 => Ok(Interrupt::ReadKeyboardString),
            3 => Ok(Interrupt::DisplayNumber(c_runtime::decimal(
                self.cpu.d_reg[1].get_long(),
            ))),
            4 => Ok(Interrupt::ReadNumber),
            5 => Ok(Interrupt::ReadChar),
            6 => Ok(Interrupt::DisplayChar(charset::character(
                self.cpu.d_reg[1].get_byte(),
            ))),
            7 => Ok(Interrupt::CheckKeyboardInput),
            8 => Ok(Interrupt::GetTime),
            //the program ends where the interrupt is raised (`execute_instruction`)
            9 => Ok(Interrupt::Terminate),
            13 | 14 => {
                let address = self.cpu.a_reg[1].get_long() as usize;
                let str = self.read_null_terminated_string(address, value)?;
                if value == 13 {
                    Ok(Interrupt::DisplayStringWithCRLF(str))
                } else {
                    Ok(Interrupt::DisplayStringWithoutCRLF(str))
                }
            }
            15 => {
                //Display the unsigned number in D1.L converted to the number base (2 through 36)
                //in D2.B: to display D1.L in base 16, put 16 in D2.B. EASy68K displays nothing for
                //a base outside 2 to 36; this reports it instead, a documented deviation.
                let number = self.cpu.d_reg[1].get_long();
                let base = self.cpu.d_reg[2].get_byte() as u32;
                match c_runtime::in_base(number, base) {
                    Some(text) => Ok(Interrupt::DisplayNumberInBase(text)),
                    None => Err(RuntimeError::InvalidTrapArgument {
                        task: value,
                        reason: format!("D2.B is {}, and a base is 2 to 36", base),
                    }),
                }
            }
            17 | 18 => {
                //tasks 14 and 3, or 14 and 4, in a single trap
                let address = self.cpu.a_reg[1].get_long() as usize;
                let string = self.read_null_terminated_string(address, value)?;
                if value == 17 {
                    let number = c_runtime::decimal(self.cpu.d_reg[1].get_long());
                    Ok(Interrupt::DisplayStringAndNumber(string + &number))
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
                //`sprintf(buf, "%*d", (char)D2, D1)`: the width is a signed byte
                let value = self.cpu.d_reg[1].get_long();
                let width = self.cpu.d_reg[2].get_byte();
                Ok(Interrupt::DisplaySignedNumberInField(c_runtime::in_field(
                    value, width,
                )))
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
                    _ => Err(RuntimeError::InvalidTrapArgument {
                        task: value,
                        reason: format!(
                            "D1.B is {}, and task 61 reads the mouse with 0 (its state now), 1 (at \
                             the last button up) or 2 (at the last button down)",
                            mode
                        ),
                    }),
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
                    //the bitwise raster modes draw against the background color, which this screen
                    //does not implement
                    _ => Err(RuntimeError::InvalidTrapArgument {
                        task: value,
                        reason: format!(
                            "D1.B is {}, and the drawing modes are 2 (move without drawing), 4 \
                             (draw), 16 (double buffering off) and 17 (double buffering on); the \
                             bitwise modes are not supported",
                            mode
                        ),
                    }),
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
                let str = self.read_null_terminated_string(address, value)?;
                let x = self.cpu.d_reg[1].get_word();
                let y = self.cpu.d_reg[2].get_word();
                Ok(Interrupt::DrawText(x as i16 as i32, y as i16 as i32, str))
            }
            // Sound: the arguments, for a host with a sound device to play them on
            70 => Ok(Interrupt::PlaySound(
                self.read_path(self.cpu.a_reg[1].get_long())?,
            )),
            71 => Ok(Interrupt::LoadSound {
                path: self.read_path(self.cpu.a_reg[1].get_long())?,
                index: self.cpu.d_reg[1].get_byte(),
            }),
            72 => Ok(Interrupt::PlayLoadedSound(self.cpu.d_reg[1].get_byte())),
            73 => Ok(Interrupt::PlaySoundDirectX(
                self.read_path(self.cpu.a_reg[1].get_long())?,
            )),
            74 => Ok(Interrupt::LoadSoundDirectX {
                path: self.read_path(self.cpu.a_reg[1].get_long())?,
                index: self.cpu.d_reg[1].get_byte(),
            }),
            75 => Ok(Interrupt::PlayLoadedSoundDirectX(
                self.cpu.d_reg[1].get_byte(),
            )),
            76 => Ok(Interrupt::ControlSound {
                index: self.cpu.d_reg[1].get_byte(),
                control: self.cpu.d_reg[2].get_long(),
            }),
            77 => Ok(Interrupt::ControlSoundDirectX {
                index: self.cpu.d_reg[1].get_byte(),
                control: self.cpu.d_reg[2].get_long(),
            }),
            _ => Err(RuntimeError::UnsupportedTrapTask { task: value }),
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
            //the whole register after the sized store, as `set_register_value` reports it
            let new_value = self.cpu.a_reg[reg as usize].get_long();
            self.debugger
                .add_mutation(MutationOperation::WriteRegister {
                    register: RegisterOperand::Address(reg),
                    old: old_value,
                    new: new_value,
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
    /// Runs until `simhalt` pauses the Program, an interrupt needs an answer,
    /// or the Program terminates. Calling it while paused resumes the Program.
    pub fn run(&mut self) -> RuntimeResult<InterpreterStatus> {
        self.verify_can_run()?;
        if self.status == InterpreterStatus::Paused {
            self.step()?;
        }
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

    /// Runs until a breakpoint, `simhalt`, the end of the program, an interrupt
    /// or `limit` instructions. Calling it while paused resumes the Program.
    ///
    /// `skip_breakpoint_at_pc` decides what a breakpoint on the instruction the
    /// program counter is *already* on does. `true` runs it anyway, which is
    /// what makes "continue" from a breakpoint move; `false` stops before it,
    /// having run nothing, which is what a caller that has just arrived here
    /// some other way wants. A caller that answers an interrupt and runs on has
    /// to pass `false`, or the instruction after every interrupt is unbreakable:
    /// each call resumes mid-program with the program counter on the next
    /// instruction, and that one has not run yet.
    ///
    /// Only the instruction the run starts on is affected; every breakpoint the
    /// run reaches after that stops it before the instruction executes.
    pub fn run_with_breakpoints(
        &mut self,
        breakpoints: &[Breakpoint],
        limit: Option<usize>,
        skip_breakpoint_at_pc: bool,
    ) -> RuntimeResult<InterpreterStatus> {
        self.verify_can_run()?;
        if self.status == InterpreterStatus::Paused && self.has_reached_bottom() {
            self.terminate(Termination::EndOfProgram);
            return Ok(self.status);
        }
        let resuming_after_pause = self.status == InterpreterStatus::Paused;
        let addresses = self.get_breakpoint_addresses(breakpoints);
        let mut iterations = 0;
        let limit = limit.unwrap_or(usize::MAX);
        let mut limit_counter = limit;
        while (self.status == InterpreterStatus::Running
            || (resuming_after_pause && iterations == 0))
            && limit_counter > 0
        {
            let at_starting_pc = iterations == 0;
            if !(at_starting_pc && skip_breakpoint_at_pc) && addresses.contains(&self.pc) {
                //a run that stops on a breakpoint is running, not paused: the
                //status a `simhalt` left behind belongs to the halt, not to the
                //breakpoint the resumed run stopped at
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
        if self.status == InterpreterStatus::Paused && self.has_reached_bottom() {
            self.terminate(Termination::EndOfProgram);
            return Ok(self.status);
        }
        let resuming_after_pause = self.status == InterpreterStatus::Paused;
        let mut iterations = 0;
        while (self.status == InterpreterStatus::Running
            || (resuming_after_pause && iterations == 0))
            && limit_counter > 0
        {
            self.step()?;
            limit_counter -= 1;
            iterations += 1;
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

/// A value as the plain JavaScript data the boundary answers with.
///
/// Serialising one of this crate's own shapes cannot fail, but a panic in
/// WebAssembly leaves the module unusable for every call after it, so a
/// failure is answered as a message instead of unwrapped.
fn js(value: &impl Serialize) -> JsValue {
    serde_wasm_bindgen::to_value(value)
        .unwrap_or_else(|error| JsValue::from_str(&format!("cannot serialise: {}", error)))
}

/// A register named from JavaScript, refused when its number is not 0 to 7:
/// `{type: 'Data', value: 9}` deserialises, and would index past the eight
/// registers.
fn register_from_js(register: JsValue) -> Result<RegisterOperand, JsValue> {
    let parsed: RegisterOperand = serde_wasm_bindgen::from_value(register).map_err(|error| {
        js(&RuntimeError::InvalidArgument(format!(
            "a register is {{type: 'Data' | 'Address', value: 0 to 7}}: {}",
            error
        )))
    })?;
    match parsed {
        RegisterOperand::Data(number) | RegisterOperand::Address(number) if number < 8 => {
            Ok(parsed)
        }
        RegisterOperand::Data(number) | RegisterOperand::Address(number) => Err(js(
            &RuntimeError::InvalidArgument(format!("there is no register number {}", number)),
        )),
    }
}

/// Every method below throws a [`RuntimeError`] as a plain object,
/// `{type, value}`, and none of them panics: a panic would leave the
/// WebAssembly module unusable for every call after it.
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
        self.write_memory_bytes(address, &bytes).map_err(|e| js(&e))
    }
    pub fn wasm_get_cpu_snapshot(&self) -> Cpu {
        self.cpu
    }
    pub fn wasm_get_pc(&self) -> usize {
        self.get_pc()
    }
    pub fn wasm_get_stack_top(&self) -> usize { self.stack_top }
    pub fn wasm_get_sp(&self) -> usize {
        self.get_sp()
    }
    pub fn wasm_get_instruction_at(&self, address: usize) -> JsValue {
        match self.get_instruction_at(address) {
            Some(ins) => js(ins),
            None => JsValue::NULL,
        }
    }
    pub fn wasm_can_undo(&self) -> bool {
        self.can_undo()
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
        self.step().map_err(|e| js(&e))
    }
    pub fn wasm_run(&mut self) -> Result<InterpreterStatus, JsValue> {
        self.run().map_err(|e| js(&e))
    }
    /// `breakpoints` is an array of `{ file, line }`, a Location without its
    /// columns. `skip_breakpoint_at_pc` defaults to `true`, the "continue"
    /// behaviour described on [`Interpreter::run_with_breakpoints`].
    pub fn wasm_run_with_breakpoints(
        &mut self,
        breakpoints: JsValue,
        limit: Option<usize>,
        skip_breakpoint_at_pc: Option<bool>,
    ) -> Result<InterpreterStatus, JsValue> {
        let breakpoints: Vec<Breakpoint> =
            serde_wasm_bindgen::from_value(breakpoints).map_err(|e| {
                js(&RuntimeError::InvalidArgument(format!(
                    "breakpoints are an array of {{file, line}}: {}",
                    e
                )))
            })?;
        self.run_with_breakpoints(&breakpoints, limit, skip_breakpoint_at_pc.unwrap_or(true))
            .map_err(|e| js(&e))
    }
    pub fn wasm_get_call_stack(&self) -> JsValue {
        js(&self.get_pretty_call_stack())
    }
    pub fn wasm_run_with_limit(&mut self, limit: usize) -> Result<InterpreterStatus, JsValue> {
        self.run_with_limit(limit).map_err(|e| js(&e))
    }
    pub fn wasm_get_next_instruction(&self) -> JsValue {
        match self.get_next_instruction() {
            Some(ins) => js(ins),
            None => JsValue::NULL,
        }
    }
    pub fn wasm_get_previous_mutations(&self) -> JsValue {
        match self.debugger.get_previous_mutations() {
            Some(m) => js(m),
            None => JsValue::NULL,
        }
    }
    /// The newest `count` steps, newest first: the instructions that have run
    /// and the Pokes made between them, each saying which it is in its `kind`.
    pub fn wasm_get_undo_history(&self, count: usize) -> JsValue {
        js(&self.get_last_steps(count))
    }

    pub fn wasm_get_last_step_id(&self) -> f64 {
        self.get_last_step_id() as f64
    }
    pub fn wasm_get_status(&self) -> InterpreterStatus {
        *self.get_status()
    }
    /// Why the program ended, a [`Termination`] as `{type, value}`, or `null`
    /// while it has not.
    pub fn wasm_get_termination(&self) -> JsValue {
        match self.get_termination() {
            Some(termination) => js(termination),
            None => JsValue::NULL,
        }
    }
    /// The [`InputSettings`] of tasks 12 and 16, `{echo, prompt, line_feed}`.
    pub fn wasm_get_input_settings(&self) -> JsValue {
        js(&self.get_input_settings())
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
        self.undo().map(|step| js(&step)).map_err(|e| js(&e))
    }
    /// Opens a Poke, which is `beginPoke()`. It throws when one is already
    /// open and when an instruction is executing.
    pub fn wasm_begin_poke(&mut self) -> Result<(), JsValue> {
        self.begin_poke().map_err(|e| js(&e))
    }
    /// Closes the open Poke, which is `endPoke()`, and answers whether it
    /// recorded a step. It throws when no Poke is open.
    pub fn wasm_end_poke(&mut self) -> Result<bool, JsValue> {
        self.end_poke().map_err(|e| js(&e))
    }
    pub fn wasm_get_last_step(&self) -> JsValue {
        match self.debugger.get_last_step() {
            Some(step) => js(step),
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
    /// The register's value at `size`. It throws an `InvalidArgument` for a
    /// register that is not `{type: 'Data' | 'Address', value: 0 to 7}`.
    pub fn wasm_get_register_value(&self, reg: JsValue, size: Size) -> Result<u32, JsValue> {
        let register = register_from_js(reg)?;
        Ok(self.get_register_value(register, size))
    }
    /// Writes the register at `size`, into the open Poke if there is one. It
    /// throws an `InvalidArgument` for a register that is not one.
    pub fn wasm_set_register_value(
        &mut self,
        reg: JsValue,
        value: u32,
        size: Size,
    ) -> Result<(), JsValue> {
        let register = register_from_js(reg)?;
        self.set_register_value(register, value, size);
        Ok(())
    }
    pub fn wasm_has_reached_bottom(&self) -> bool {
        self.has_reached_bottom()
    }
    pub fn wasm_has_terminated(&self) -> bool {
        self.has_terminated()
    }
    /// The pending [`Interrupt`] as `{type, value}`, or `null` when there is
    /// none. A file write's bytes are a `Uint8Array`.
    pub fn wasm_get_current_interrupt(&self) -> JsValue {
        match &self.current_interrupt {
            Some(interrupt) => js(interrupt),
            None => JsValue::NULL,
        }
    }
    /// Answers the pending interrupt, which is `answerInterrupt(answer)`.
    ///
    /// It throws a [`RuntimeError`] and changes nothing whenever
    /// [`answer_interrupt`](Interpreter::answer_interrupt) refuses the answer,
    /// and with an `InvalidAnswer` when the value is not an `InterruptResult`
    /// at all — a number where a typed line is a string, say. The interrupt
    /// still waits after a refusal. A file read's bytes are taken as a
    /// `Uint8Array`, or an array of numbers.
    pub fn wasm_answer_interrupt(&mut self, value: JsValue) -> Result<(), JsValue> {
        let pending = match &self.current_interrupt {
            Some(interrupt) if self.status == InterpreterStatus::Interrupt => interrupt.name(),
            _ => return Err(js(&RuntimeError::NoPendingInterrupt)),
        };
        let answer: InterruptResult = serde_wasm_bindgen::from_value(value).map_err(|e| {
            js(&RuntimeError::InvalidAnswer {
                interrupt: pending.to_string(),
                reason: format!("the answer is not an interrupt answer: {}", e),
            })
        })?;
        self.answer_interrupt(answer).map_err(|e| js(&e))
    }

    /// The [`Location`] of the instruction the program counter is on, as
    /// `{ file, line, column, end_column }`, or `null` when it is on none.
    pub fn wasm_get_current_location(&self) -> JsValue {
        match self.get_current_location() {
            Some(location) => js(location),
            None => JsValue::NULL,
        }
    }
}
