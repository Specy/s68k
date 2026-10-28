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
//! before the Program was built. One the 68000 has a vector for is an
//! **Exception**, taken into the program's handler as a 68000 takes it, in
//! supervisor mode on the supervisor stack, and it ends the program only when
//! the vector holds no handler (ADR 0008, and [`crate::machine::exception`]).
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
use std::convert::Infallible;

use bitflags::bitflags;
use serde::{Deserialize, Serialize};
use wasm_bindgen::{prelude::wasm_bindgen, JsValue};

use crate::assembler::program::{
    AssembledInstruction, MemoryContent, MemoryRange, MemoryRangeKind, Program,
};
use crate::assembler::source::Location;
#[cfg(any(test, feature = "conformance"))]
use crate::assembler::symbols::SymbolTable;
use crate::debugger::{PrettyStackFrame, SystemRegister};
use crate::instructions::TargetDirection;
use crate::machine::cache::{self, DecodeCache, Slot};
use crate::machine::decode::{self, Decoded};
use crate::machine::exception::{self, Frame};
use crate::machine::memory_map::{Kinds, MemoryMap, Verdict};
use crate::machine::{disassemble, Processor};
use crate::{c_runtime, charset};
use crate::{
    debugger::{
        Debugger, ExecutionStep, ExecutionStepKind, MutationOperation, PokeJournal, PokeTarget,
        PokeWrite, Ran, MAX_INSTRUCTION_BYTES,
    },
    instructions::{
        AddressWidth, Bytes, Condition, FileDialogMode, FileExistence, IndexRegister,
        InputSettings, Instruction, Interrupt, InterruptResult, KeyStateRequest, KeyStateResult,
        Operand, RegisterOperand, Sign, Size, SoundOutcome,
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

/// The mask of the 24 bits of the 68000's address bus: what every address is
/// reduced to before it reaches memory.
pub const ADDRESS_MASK: usize = 0x00ff_ffff;

/// The 16 MB of memory, the decode cache of the instructions in it, and the
/// Memory map the program's accesses are checked against.
///
/// Every write passes the cache (ADR 0006): a typed store of the program, a
/// run of bytes of a trap task, an answer, the host or undo, and the
/// Program's initial contents. Each clears the instructions it covers, so the
/// next fetch decodes what memory holds then.
///
/// The program's typed accesses pass the [memory map](crate::machine::memory_map)
/// too: its reads ([`read_size`](Memory::read_size), [`pop`](Memory::pop),
/// [`read_program`](Memory::read_program)) and its stores
/// ([`replace_size`](Memory::replace_size)). Everything else is the host's or
/// undo's and passes it by: runs of bytes, [`write_size`](Memory::write_size),
/// which undo writes with, and the Program's initial contents. With no range
/// declared an access asks one flag, `mapped`, and goes on.
#[derive(Debug)]
#[wasm_bindgen]
pub struct Memory {
    data: Vec<u8>,
    cache: DecodeCache,
    map: MemoryMap,
    /// Whether the map declares any range: the one thing an access asks when
    /// it does not.
    mapped: bool,
    /// Whether the processor is in supervisor mode, which a protected range
    /// reads: the Interpreter keeps it equal to the S bit whenever it writes
    /// the status register.
    supervisor: bool,
}

impl Memory {
    pub fn new() -> Self {
        Self {
            data: vec![255; MEMORY_SIZE], //16mb
            cache: DecodeCache::new(),
            map: MemoryMap::default(),
            mapped: false,
            supervisor: true,
        }
    }

    /// Checks the program's accesses against the Memory map of `ranges`. Call
    /// it before [`load_code`](Memory::load_code), which keeps no instruction
    /// whose fetch the map checks.
    pub fn set_map(&mut self, ranges: &[MemoryRange]) {
        self.map = MemoryMap::new(ranges);
        self.mapped = !self.map.is_empty();
    }

    /// Whether the decode cache keeps the instruction of `size` bytes at `pc`,
    /// an even address of the 24 bits of the bus: every instruction but one a
    /// word of which an invalid or a protected range reaches, whose fetch has
    /// to be checked each time, in the mode the processor is in then. Such
    /// an instruction is decoded at every fetch, and a write into it is no
    /// Code write, as there is no decoded instruction to clear.
    #[inline(always)]
    pub fn keeps(&self, pc: usize, size: usize) -> bool {
        !self.mapped || !self.is_fetch_checked(pc, size)
    }

    #[cold]
    #[inline(never)]
    fn is_fetch_checked(&self, pc: usize, size: usize) -> bool {
        let last = pc + size.max(1) - 1;
        self.map
            .fetched_ranges()
            .any(|(start, end)| last >= start && pc <= end)
    }

    /// The Memory map the program's accesses are checked against.
    pub fn map(&self) -> &MemoryMap {
        &self.map
    }

    /// Says whether the processor is in supervisor mode, which is what a
    /// protected range reads.
    #[inline(always)]
    fn set_supervisor(&mut self, supervisor: bool) {
        self.supervisor = supervisor;
    }

    /// The bus error of an access of `size` the program made at `address`,
    /// which the map's range of `range` refused.
    #[cold]
    #[inline(never)]
    fn bus_error(address: usize, size: Size, access: Access, range: MemoryRangeKind) -> RuntimeError {
        RuntimeError::BusError {
            address: address as u32 as usize,
            size,
            access,
            range,
        }
    }

    /// A read of the program's at `bus`, `address` as it wrote it, against the
    /// map, in the mode `supervisor` says.
    #[cold]
    #[inline(never)]
    fn check_read(
        &self,
        bus: usize,
        address: usize,
        size: Size,
        access: Access,
        supervisor: bool,
    ) -> RuntimeResult<()> {
        match self.map.read(bus, supervisor) {
            Verdict::Refused(range) => Err(Self::bus_error(address, size, access, range)),
            Verdict::Allowed | Verdict::Ignored => Ok(()),
        }
    }

    /// A store of the program's at `bus` against the map: whether a ROM range
    /// ignores it, or the bus error a range refuses it with.
    #[cold]
    #[inline(never)]
    fn check_write(&self, bus: usize, address: usize, size: Size) -> RuntimeResult<bool> {
        match self.map.write(bus, size.to_bytes(), self.supervisor) {
            Verdict::Allowed => Ok(false),
            Verdict::Ignored => Ok(true),
            Verdict::Refused(range) => Err(Self::bus_error(address, size, Access::Write, range)),
        }
    }

    /// The first word of the instruction of `size` bytes at `pc`, an even
    /// address of the 24 bits of the bus, that the map refuses to fetch, and
    /// the range that refuses it: EASy68K fetches each word as a read, so a
    /// word in an invalid range, or in a protected one in user mode, is a bus
    /// error. `None` when every word can be fetched.
    pub fn fetch_fault(&self, pc: usize, size: usize) -> Option<(usize, MemoryRangeKind)> {
        if !self.mapped {
            return None;
        }
        (0..size.max(2)).step_by(2).find_map(|offset| {
            let at = (pc + offset) & ADDRESS_MASK;
            match self.map.read(at, self.supervisor) {
                Verdict::Refused(range) => Some((at, range)),
                Verdict::Allowed | Verdict::Ignored => None,
            }
        })
    }

    /// What the map says of a buffer of `length` bytes at `address` that a
    /// `trap #15` task reads or writes, against `kinds`
    /// ([`MemoryMap::buffer`]).
    pub fn check_buffer(&self, kinds: Kinds, address: usize, length: usize) -> Verdict {
        if !self.mapped {
            return Verdict::Allowed;
        }
        self.map.buffer(kinds, address & ADDRESS_MASK, length)
    }

    /// The byte at `address`, wrapping round the 24 bits of the bus, read as
    /// the host reads memory, past the map: what the trap tasks read a string
    /// with, once they have checked it as EASy68K checks it.
    #[inline(always)]
    fn byte_at(&self, address: usize) -> u8 {
        self.data[address & ADDRESS_MASK]
    }

    /// Fills the decode cache with the Program's instructions, each from the
    /// [`AssembledInstruction`] the Assembler built, so that assembled code is
    /// never decoded at run time, but for those the memory map checks at every
    /// fetch ([`keeps`](Memory::keeps)). Their bytes are already in memory.
    pub fn load_code(&mut self, instructions: &[AssembledInstruction]) {
        for (index, instruction) in instructions.iter().enumerate() {
            let address = instruction.address & ADDRESS_MASK;
            if !self.keeps(address, instruction.size) {
                continue;
            }
            self.cache.store(
                address,
                Slot::assembled(instruction.instruction, instruction.size, index),
            );
        }
    }

    /// The decode cache.
    pub fn cache(&self) -> &DecodeCache {
        &self.cache
    }

    /// What a step keeps of the instruction of `size` bytes it runs at `pc`,
    /// an address of the 24 bits of the bus, decoded from memory as the
    /// program ran: its bytes, wrapping round the bus as an instruction's
    /// words do. `size` is 0 for an instruction fetched from an odd address,
    /// which ran nothing.
    #[cold]
    #[inline(never)]
    fn ran_at(&self, pc: usize, size: usize) -> Ran {
        if size == 0 {
            return Ran::Nothing;
        }
        let mut bytes = [0; MAX_INSTRUCTION_BYTES];
        let len = size.min(MAX_INSTRUCTION_BYTES);
        for (offset, byte) in bytes[..len].iter_mut().enumerate() {
            *byte = self.data[(pc + offset) & ADDRESS_MASK];
        }
        Ran::Decoded {
            bytes,
            len: len as u8,
        }
    }

    /// The word at `address`, wrapping round the 24 bits of the bus, as an
    /// instruction is read: an odd address reads the two bytes there.
    #[inline(always)]
    fn word_at(&self, address: usize) -> u16 {
        let address = address & ADDRESS_MASK;
        u16::from_be_bytes([self.data[address], self.data[(address + 1) & ADDRESS_MASK]])
    }

    /// The instruction memory holds at `address` now, decoded for
    /// `processor`, its words wrapping round the 24 bits of the bus. Every
    /// word is an instruction or [`Instruction::IllegalWord`], so this never
    /// fails.
    pub fn decode_at(&self, address: usize, processor: Processor) -> Decoded {
        let decoded = decode::decode(address as u32, processor, |at| {
            Ok::<u16, Infallible>(self.word_at(at as usize))
        });
        match decoded {
            Ok(decoded) => decoded,
            Err(never) => match never {},
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
    /// The address a read of the program's of `size` reaches, in the 24 bits
    /// of the bus, or its address error, or the bus error the memory map
    /// refuses it with.
    #[inline(always)]
    fn verify_read(&self, address: usize, size: Size, access: Access) -> RuntimeResult<usize> {
        let bus = self.verify_address(address, size, access)?;
        if self.mapped {
            self.check_read(bus, address, size, access, self.supervisor)?;
        }
        Ok(bus)
    }
    pub fn read_long(&self, address: usize) -> RuntimeResult<u32> {
        self.read_long_as(address, Access::Read)
    }
    #[inline(always)]
    fn read_long_as(&self, address: usize, access: Access) -> RuntimeResult<u32> {
        let address = self.verify_read(address, Size::Long, access)?;
        Ok(u32::from_be_bytes(
            self.data[address..address + 4].try_into().unwrap(),
        ))
    }
    pub fn read_word(&self, address: usize) -> RuntimeResult<u16> {
        self.read_word_as(address, Access::Read)
    }
    #[inline(always)]
    fn read_word_as(&self, address: usize, access: Access) -> RuntimeResult<u16> {
        let address = self.verify_read(address, Size::Word, access)?;
        Ok(u16::from_be_bytes(
            self.data[address..address + 2].try_into().unwrap(),
        ))
    }
    pub fn read_byte(&self, address: usize) -> RuntimeResult<u8> {
        self.read_byte_as(address, Access::Read)
    }
    #[inline(always)]
    fn read_byte_as(&self, address: usize, access: Access) -> RuntimeResult<u8> {
        let address = self.verify_read(address, Size::Byte, access)?;
        Ok(self.data[address])
    }
    pub fn read_size(&self, address: usize, size: Size) -> RuntimeResult<u32> {
        self.read_size_as(address, size, Access::Read)
    }
    #[inline(always)]
    fn read_size_as(&self, address: usize, size: Size, access: Access) -> RuntimeResult<u32> {
        match size {
            Size::Byte => Ok(self.read_byte_as(address, access)? as u32),
            Size::Word => Ok(self.read_word_as(address, access)? as u32),
            Size::Long => self.read_long_as(address, access),
        }
    }

    /// The long at `address` read as the processor reads a vector, in
    /// supervisor mode whatever the program's mode is, and against the map: a
    /// vector in an invalid range is a bus error, which halts the processor
    /// as it takes an exception.
    fn read_vector(&self, address: usize) -> RuntimeResult<u32> {
        let bus = self.verify_address(address, Size::Long, Access::Read)?;
        if self.mapped {
            self.check_read(bus, address, Size::Long, Access::Read, true)?;
        }
        Ok(u32::from_be_bytes(
            self.data[bus..bus + 4].try_into().unwrap(),
        ))
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
    /// value it replaced, read at the same size: a store of the program's
    /// that journals what it overwrote, checked once, and as a store, the
    /// memory map included. `None` when a ROM range ignored it, which changes
    /// nothing and is no error.
    pub fn replace_size(
        &mut self,
        address: usize,
        size: Size,
        data: u32,
    ) -> RuntimeResult<Option<u32>> {
        let bus = self.verify_address(address, size, Access::Write)?;
        if self.mapped && self.check_write(bus, address, size)? {
            return Ok(None);
        }
        self.cache.invalidate(bus, size.to_bytes());
        let bytes = &mut self.data[bus..bus + size.to_bytes()];
        let old = bytes.iter().fold(0, |old, byte| (old << 8) | *byte as u32);
        bytes.copy_from_slice(&data.to_be_bytes()[4 - size.to_bytes()..]);
        Ok(Some(old))
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
    /// The address an access of `size` reaches, in the 24 bits of the bus, or
    /// the 68000's address error for a word or a long at an odd address, which
    /// names the address as the program wrote it and the `access`.
    #[inline(always)]
    pub fn verify_address(
        &self,
        address: usize,
        size: Size,
        access: Access,
    ) -> RuntimeResult<usize> {
        let bus = self.verify_address_bounds(address, size.to_bytes())?;
        let odd = bus & 1 != 0;
        if odd && size != Size::Byte {
            return Err(Self::address_error(address, size, access));
        }
        Ok(bus)
    }

    #[cold]
    #[inline(never)]
    fn address_error(address: usize, size: Size, access: Access) -> RuntimeError {
        RuntimeError::AddressError {
            address: address as u32 as usize,
            size,
            access,
        }
    }

    /// Reads a word or a long as [`read_size`](Self::read_size) does, in
    /// program space: an operand relative to the program counter, whose
    /// address error and bus error say so.
    pub fn read_program(&self, address: usize, size: Size) -> RuntimeResult<u32> {
        self.read_size_as(address, size, Access::Program)
    }
    pub fn write_long(&mut self, address: usize, value: u32) -> RuntimeResult<()> {
        let address = self.verify_address(address, Size::Long, Access::Write)?;
        self.cache.invalidate(address, 4);
        self.data[address..address + 4].copy_from_slice(&value.to_be_bytes());
        Ok(())
    }
    pub fn write_word(&mut self, address: usize, value: u16) -> RuntimeResult<()> {
        let address = self.verify_address(address, Size::Word, Access::Write)?;
        self.cache.invalidate(address, 2);
        self.data[address..address + 2].copy_from_slice(&value.to_be_bytes());
        Ok(())
    }
    pub fn write_byte(&mut self, address: usize, value: u8) -> RuntimeResult<()> {
        let address = self.verify_address(address, Size::Byte, Access::Write)?;
        self.cache.invalidate(address, 1);
        self.data[address] = value;
        Ok(())
    }
    /// Writes a run of bytes as they are, code or data, past the memory map:
    /// the Program's initial contents, undo, a trap task's buffer and the
    /// host's writes.
    pub fn write_bytes(&mut self, address: usize, bytes: &[u8]) -> RuntimeResult<()> {
        let address = self.verify_address_bounds(address, bytes.len())?;
        self.cache.invalidate(address, bytes.len());
        self.data[address..address + bytes.len()].copy_from_slice(bytes);
        Ok(())
    }
    /// Reads a run of bytes as they are, past the memory map, which is how the
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

/// How far `-(An)` and `(An)+` move the address register for an access of
/// `size`: the size itself, except that a byte on A7 moves it by 2, so the
/// stack pointer stays even, as the 68000 Programmer's Reference Manual says of
/// both modes (2.2.4 and 2.2.5).
#[inline(always)]
fn address_step(register: u8, size: Size) -> u32 {
    if register == 7 && size == Size::Byte {
        2
    } else {
        size.to_bytes() as u32
    }
}

/// The address error of a long accessed through `-(An)` a word at a time, its
/// low word first, as `move.l` and `movem.l` write one and `addx.l` and
/// `subx.l` read one: the fault is on the low word, two bytes past the long.
#[cold]
#[inline(never)]
fn low_word_first(error: RuntimeError) -> RuntimeError {
    match error {
        RuntimeError::AddressError {
            address,
            size,
            access,
        } => RuntimeError::AddressError {
            address: (address as u32).wrapping_add(2) as usize,
            size,
            access,
        },
        other => other,
    }
}

/// The bits of the status register a 68000 has: trace, supervisor, the
/// interrupt mask and the five condition codes. The others always read as 0,
/// whatever is written to them, so `move #$FFFF,sr` reads back `$A71F`.
pub const SR_IMPLEMENTED_BITS: u16 = 0xA71F;

/// The trace bit of the status register, T: set, the processor takes a trace
/// exception after every instruction.
pub const TRACE_BIT: u16 = 0x8000;

/// The supervisor bit of the status register, S: set, the processor is in
/// supervisor mode and A7 is the supervisor stack pointer.
pub const SUPERVISOR_BIT: u16 = 0x2000;

/// The status register a program starts with: `$2000`, EASy68K's own
/// (`STARTSIM.CPP`): supervisor, interrupt mask 0, no trace and no condition
/// code set (the asm-editor's `docs/design/m68k-machine.md`, decision 2).
pub const INITIAL_STATUS_REGISTER: u16 = 0x2000;

/// The supervisor stack pointer a program starts with, which is A7 at the
/// start, since a program starts in supervisor mode: `$01000000`, the end of
/// memory, so the first push lands at `$00FFFFFC`.
pub const INITIAL_SUPERVISOR_STACK_POINTER: u32 = 0x0100_0000;

/// The user stack pointer a program starts with: `$00FF0000`, EASy68K's.
pub const INITIAL_USER_STACK_POINTER: u32 = 0x00FF_0000;

/// The two stack pointers, by the mode each belongs to: what
/// [`MutationOperation::WriteStackPointers`] journals.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct StackPointers {
    /// The user stack pointer, USP.
    pub usp: u32,
    /// The supervisor stack pointer, SSP.
    pub ssp: u32,
}

/// The System register file: the status register and both stack pointers,
/// what `getSystem()` answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct SystemRegisters {
    /// The whole status register.
    pub sr: u16,
    /// The user stack pointer.
    pub usp: u32,
    /// The supervisor stack pointer.
    pub ssp: u32,
}

/// The processor's registers.
///
/// A7 is the stack pointer of the mode the status register's S bit names: the
/// supervisor stack pointer (SSP) when it is set, the user stack pointer (USP)
/// when it is not. The other one is kept aside, so that every instruction
/// reads and writes A7 as an address register like any other, and only a
/// write of S, an exception and `rte` move the two.
#[derive(Debug, Clone, Copy)]
#[wasm_bindgen]
pub struct Cpu {
    d_reg: [Register; 8],
    a_reg: [Register; 8],
    ccr: Flags,
    /// The high byte of the status register: trace, supervisor and the
    /// interrupt mask. The low byte is [`Cpu::ccr`], so the whole register is
    /// [`Cpu::get_sr`].
    system_byte: u8,
    /// The stack pointer of the mode the processor is not in: the USP in
    /// supervisor mode, the SSP in user mode.
    inactive_stack_pointer: u32,
}

impl Default for Cpu {
    fn default() -> Self {
        Self::new()
    }
}

impl Cpu {
    /// The registers as a program starts with them, EASy68K's: every data and
    /// address register zero, supervisor mode with A7 the SSP at
    /// [`INITIAL_SUPERVISOR_STACK_POINTER`], the USP at
    /// [`INITIAL_USER_STACK_POINTER`] and SR [`INITIAL_STATUS_REGISTER`].
    pub fn new() -> Self {
        let mut a_reg = [Register::new(); 8];
        a_reg[7].store_long(INITIAL_SUPERVISOR_STACK_POINTER);
        Self {
            d_reg: [Register::new(); 8],
            a_reg,
            ccr: Flags::from_ccr_byte(INITIAL_STATUS_REGISTER as u8),
            system_byte: (INITIAL_STATUS_REGISTER >> 8) as u8,
            inactive_stack_pointer: INITIAL_USER_STACK_POINTER,
        }
    }

    /// The whole status register: the system byte, then the condition codes as
    /// the processor numbers them.
    pub fn get_sr(&self) -> u16 {
        ((self.system_byte as u16) << 8) | self.ccr.to_ccr_byte() as u16
    }

    /// Writes the whole status register as the 68000 does, condition codes
    /// included: reduced to [`SR_IMPLEMENTED_BITS`], and, when S changes, A7
    /// becomes the stack pointer of the new mode and the old one is kept
    /// aside. Answers whether it swapped them.
    pub fn set_sr(&mut self, value: u16) -> bool {
        let swaps = (value ^ self.get_sr()) & SUPERVISOR_BIT != 0;
        if swaps {
            let active = self.a_reg[7].get_long();
            self.a_reg[7].store_long(self.inactive_stack_pointer);
            self.inactive_stack_pointer = active;
        }
        self.restore_sr(value);
        swaps
    }

    /// Writes the whole status register and leaves the stack pointers where
    /// they are, even when S changes: what undo does, since it puts the stack
    /// pointers back itself ([`Cpu::restore_stack_pointers`]).
    pub fn restore_sr(&mut self, value: u16) {
        let value = value & SR_IMPLEMENTED_BITS;
        self.system_byte = (value >> 8) as u8;
        self.ccr = Flags::from_ccr_byte(value as u8);
    }

    /// Whether the processor is in supervisor mode: S is set.
    #[inline(always)]
    pub fn is_supervisor(&self) -> bool {
        self.system_byte & (SUPERVISOR_BIT >> 8) as u8 != 0
    }

    /// Whether T is set, so that a trace exception follows the next
    /// instruction.
    #[inline(always)]
    pub fn is_tracing(&self) -> bool {
        self.system_byte & (TRACE_BIT >> 8) as u8 != 0
    }

    /// The user stack pointer: A7 in user mode, the one kept aside in
    /// supervisor mode.
    pub fn get_usp(&self) -> u32 {
        match self.is_supervisor() {
            true => self.inactive_stack_pointer,
            false => self.a_reg[7].get_long(),
        }
    }

    /// The supervisor stack pointer: A7 in supervisor mode, the one kept aside
    /// in user mode.
    pub fn get_ssp(&self) -> u32 {
        match self.is_supervisor() {
            true => self.a_reg[7].get_long(),
            false => self.inactive_stack_pointer,
        }
    }

    /// Both stack pointers.
    pub fn stack_pointers(&self) -> StackPointers {
        StackPointers {
            usp: self.get_usp(),
            ssp: self.get_ssp(),
        }
    }

    /// Sets both stack pointers, A7 being the one of the mode S names now.
    pub fn restore_stack_pointers(&mut self, pointers: StackPointers) {
        let (active, inactive) = match self.is_supervisor() {
            true => (pointers.ssp, pointers.usp),
            false => (pointers.usp, pointers.ssp),
        };
        self.a_reg[7].store_long(active);
        self.inactive_stack_pointer = inactive;
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
    /// The whole status register, `$2000` before a program has run.
    pub fn wasm_get_sr(&self) -> u16 {
        self.get_sr()
    }
    /// The user stack pointer, which A7 is in user mode.
    pub fn wasm_get_usp(&self) -> u32 {
        self.get_usp()
    }
    /// The supervisor stack pointer, which A7 is in supervisor mode.
    pub fn wasm_get_ssp(&self) -> u32 {
        self.get_ssp()
    }
}

/// What stopped a running program, or what the host asked of the Interpreter
/// that it could not do.
///
/// **An error an instruction raised is an Exception when the 68000 has one
/// for it** (ADR 0008): the processor pushes a frame and jumps through the
/// error's vector to the program's handler, and the program goes on. That
/// covers a bus error, an address error, an illegal instruction (`illegal`, and a word that
/// is no instruction), a division by zero, `chk` and `trapv`, a privilege
/// violation, trace, the line A and line F words, `trap #0` to `#14`, and a
/// `trap #15` task EASy68K does not know. Only a vector that holds no handler
/// ends the program: one holding an odd address, which every vector does until
/// the program writes it, since memory starts as `$FF`. Then the run ends with
/// [`InterpreterStatus::TerminatedWithException`], the error is the
/// [`Termination`]'s cause with the vector beside it, and undoing the step that
/// raised it brings the program back to the instruction, running.
///
/// The others end the program with no exception to take: a `trap #15` task the
/// Interpreter cannot carry out ([`UnsupportedTrapTask`](RuntimeError::UnsupportedTrapTask)
/// for one of EASy68K's own tasks) or one given a value it cannot take
/// ([`InvalidTrapArgument`](RuntimeError::InvalidTrapArgument)), and `stop`
/// and `reset`, which are not run yet.
///
/// The rest are the host's: an answer refused
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
    /// A word or a long read or written at an odd address, and an instruction
    /// fetched from one, a jump to an odd address included: the 68000's
    /// address error, vector 3.
    AddressError {
        /// The address the program used, all 32 bits of it, as the 68000
        /// stacks it; the bus reads its low 24.
        address: usize,
        /// The size of the access.
        size: Size,
        /// What the access was.
        access: Access,
    },
    /// `divs` or `divu` by zero: vector 5.
    DivisionByZero,
    /// An access the Memory map refuses (the asm-editor's
    /// `docs/design/m68k-machine.md`, decision 15): a read or a write in an
    /// invalid range, any access in a protected one from user mode, a write
    /// in a read only one, and an instruction fetched from the first two, as
    /// EASy68K fetches each word as a read; a `trap #15` task's buffer, too,
    /// as EASy68K checks it. The 68000's bus error, vector 2.
    BusError {
        /// The address the program used, all 32 bits of it, as the 68000
        /// stacks it: for a `trap #15` task, the start of its buffer, as in
        /// EASy68K.
        address: usize,
        /// The size of the access.
        size: Size,
        /// What the access was.
        access: Access,
        /// The kind of range that refused it: read only, protected or
        /// invalid.
        range: MemoryRangeKind,
    },
    IncorrectAddressingMode(String),
    /// A 68000 instruction the Interpreter does not run yet, named in the
    /// text: `stop` and `reset`, which need interrupts (the plan's milestone
    /// C8). The Assembler refuses both, so only a decoded word reaches one.
    Unimplemented(String),
    /// `chk` found the register outside 0 to the bound it was given: vector 6.
    ChkOutOfBounds {
        /// The low word of the register, read as a signed number.
        value: i32,
        /// The bound the operand held, read as a signed number.
        bound: i32,
    },
    /// `trapv` with the overflow flag set: vector 7.
    OverflowException,
    /// The `illegal` instruction, or a word that is the first word of no
    /// instruction ([`Instruction::IllegalWord`]): vector 4.
    IllegalInstruction,
    /// A privileged instruction in user mode: `move <ea>,sr`, `andi`, `ori`
    /// and `eori` to `sr`, `rte`, `stop`, `reset` and `move usp`. Vector 8.
    PrivilegeViolation,
    /// The trace exception, after an instruction that started with T set:
    /// vector 9.
    Trace,
    /// A word of the form `$Axxx`, the line 1010 emulator: vector 10. The word
    /// is kept.
    LineA(u16),
    /// A word of the form `$Fxxx`, the line 1111 emulator: vector 11. The word
    /// is kept.
    LineF(u16),
    /// `trap #0` to `#14`, through vectors 32 to 46.
    UnhandledTrap {
        /// The vector, 32 and the trap's number.
        vector: u8,
    },
    /// A `trap #15` task s68k does not carry out, by its number in D0.B.
    ///
    /// A number that is none of EASy68K's tasks goes through vector 47, as in
    /// EASy68K, so a program can handle it. One of EASy68K's own tasks that a
    /// page in a browser cannot do faithfully ends the program with no
    /// exception: the printer (10), the text window's font and contents (21,
    /// 22, 25), the cycle counter (30, 31), the hardware window (32 but its
    /// `D1.B` of 5), the serial ports (40 to 43), the interrupt requests (60,
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

/// What an access that raised an [address error](RuntimeError::AddressError)
/// was, which is what the 68000 writes in the status word of its frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Access {
    /// A read of data.
    Read,
    /// A write.
    Write,
    /// A read in program space: an instruction's word, a jump's target, or an
    /// operand read relative to the program counter.
    Program,
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
    /// The program ran into memory it never placed: the instruction at
    /// `address` is `$FFFF $FFFF`, EASy68K's SIMHALT, and none of its bytes
    /// is one the Program placed (ADR 0009). Memory starts as `$FF`, so this is
    /// where a program that does not end itself ends, one past its last
    /// instruction when nothing follows it. `address` is the Entry point of a
    /// Program that placed nothing there, an empty one, which ends before it
    /// starts.
    EndOfProgram {
        /// Where the program counter was, in the 24 bits of the bus.
        address: usize,
    },
    /// The host answered an interrupt with [`InterruptResult::Terminate`].
    TerminatedByHost,
    /// An instruction raised a runtime error and the program ended with it:
    /// [`InterpreterStatus::TerminatedWithException`].
    Exception {
        /// What the instruction raised.
        cause: RuntimeError,
        /// The Exception's vector, when the 68000 has one for the error and
        /// the program could have handled it: the vector held no handler, an
        /// odd address (ADR 0008). `None` for an error the program cannot
        /// handle, such as a `trap #15` task s68k does not carry out.
        #[serde(skip_serializing_if = "Option::is_none")]
        vector: Option<u8>,
    },
    /// The processor halted, as a 68000 does when exception processing itself
    /// fails: a double fault. Taking the exception of vector `vector` raised
    /// `cause`: an address error for a frame pushed on an odd supervisor
    /// stack pointer, or a bus error for one pushed where the memory map
    /// refuses it, or for a vector read from an invalid range.
    /// [`InterpreterStatus::TerminatedWithException`] too.
    Halted {
        /// What taking the exception raised.
        cause: RuntimeError,
        /// The vector of the exception it was taking.
        vector: u8,
    },
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
    /// The instruction that has just run stored into the range
    /// [`Interpreter::watch_memory`] watches, and the run or step that ran it
    /// ended there, so that the host takes the writes
    /// ([`Interpreter::take_watched_writes`]) before the next instruction runs.
    /// The instruction is finished and in the history like any other, the
    /// program counter is on the next one, and calling any run or step method
    /// goes on from it, as from [`Paused`](InterpreterStatus::Paused).
    ///
    /// An error comes first: an instruction that stored into the range and
    /// then failed ends the program with its exception, and the writes it made
    /// are in the log all the same. Everything else comes after. An
    /// instruction that stored into the range and was the last one stops here,
    /// and the next call ends the program, as it does after a `simhalt` at the
    /// bottom. A breakpoint on the next instruction is reported by the call
    /// after this one, before that instruction runs, when the host goes on with
    /// `skip_breakpoint_at_pc` false. A run's `limit` running out on the same
    /// instruction answers this status and not the limit. Answering an
    /// interrupt with an answer that writes into the range (a line read into
    /// it, say) leaves this status too.
    ///
    /// Appended after `Paused` for the same reason.
    WatchedWrite,
    /// The instruction that has just run wrote into code, and the run that ran
    /// it was asked to stop there ([`RunOptions::stop_on_code_write`]): a
    /// [Code write](CodeWrite), which [`Interpreter::get_code_write`] and the
    /// step's `wrote_code` describe. Like a
    /// [`WatchedWrite`](InterpreterStatus::WatchedWrite) stop, the instruction
    /// is finished and in the history, the program counter is on the next one,
    /// and any run or step method goes on from it, the overwritten instruction
    /// running as its new bytes decode.
    ///
    /// A watched write by the same instruction comes first and stops the run as
    /// `WatchedWrite`; the code write is recorded all the same. An instruction
    /// that wrote into code and was the last one stops here, and the next call
    /// ends the program. Only a limited run asked to stop answers this status:
    /// [`step`](Interpreter::step), [`run`](Interpreter::run) and an answer to
    /// an interrupt record the code write and go on.
    ///
    /// Appended after `WatchedWrite` for the same reason.
    CodeWrite,
}

/// A **Code write**: a write of the program's own that changed the bytes of an
/// instruction the Interpreter had decoded, which it will decode again before
/// running it (ADR 0006). The program's stores and the trap tasks' writes on
/// its behalf are the program's; a Poke, a Testcase's starting memory and undo
/// are not, and never make one.
///
/// It names the first instruction the write changed, when one write changed
/// several. It is what the editor's pause on a write into code reads: "The
/// program wrote into its own code at $1008, line 4".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeWrite {
    /// The address the overwritten instruction started at.
    pub address: usize,
    /// The index of the Program instruction laid out at that address, in the
    /// order of [`Program::instructions`] (which is `getInstructionAddresses()`
    /// in ts-lib), or `None` when the instruction was not one the Program laid
    /// out there: code the program reached in data, or the middle of a line.
    pub instruction_index: Option<usize>,
}

/// What a limited run, [`Interpreter::run_with_breakpoints`], does besides
/// stopping at the breakpoints.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunOptions {
    /// What a breakpoint on the instruction the program counter is *already*
    /// on does: `true` runs it anyway, `false` stops before it, having run
    /// nothing. See [`Interpreter::run_with_breakpoints`].
    pub skip_breakpoint_at_pc: bool,
    /// Whether the run stops after an instruction that wrote into code, with
    /// [`InterpreterStatus::CodeWrite`]. Off, the run goes on and the write is
    /// recorded as it always is.
    pub stop_on_code_write: bool,
}

impl RunOptions {
    /// "Continue": run the instruction the program counter is on, whatever
    /// breakpoint names it, and do not stop on a code write. These are the
    /// defaults.
    pub const CONTINUE: RunOptions = RunOptions {
        skip_breakpoint_at_pc: true,
        stop_on_code_write: false,
    };

    /// Go on mid-program: stop before the instruction the program counter is
    /// on when a breakpoint names it, as after an interrupt, a watched write or
    /// a slice of the host's own; do not stop on a code write.
    pub const RESUME: RunOptions = RunOptions {
        skip_breakpoint_at_pc: false,
        stop_on_code_write: false,
    };

    /// These options, stopping on a code write.
    pub fn stopping_on_code_writes(self) -> RunOptions {
        RunOptions {
            stop_on_code_write: true,
            ..self
        }
    }
}

impl Default for RunOptions {
    fn default() -> Self {
        Self::CONTINUE
    }
}

/// One store into the range [`Interpreter::watch_memory`] watches, as
/// [`Interpreter::take_watched_writes`] answers it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WatchedWrite {
    /// The address of the first byte stored, in the 24 bits the 68000 puts on
    /// its bus. A store that straddles the start of the range begins before
    /// it.
    pub address: usize,
    /// How many bytes were stored: 1, 2 or 4.
    pub size: usize,
    /// The value stored, `size` bytes of it, read as memory holds it, most
    /// significant byte first.
    pub value: u32,
}

/// The addresses [`Interpreter::watch_memory`] watches, from `start` up to
/// `end`, `end` not included.
///
/// Nothing watched is the empty range `0..0`, so a store into memory asks one
/// comparison, `address < end`, which is always false, and goes on.
#[derive(Debug, Default, Clone, Copy)]
struct WatchedRange {
    start: usize,
    end: usize,
}

impl WatchedRange {
    /// Whether any of the `length` bytes at `address` is in the range.
    #[inline(always)]
    fn touches(&self, address: usize, length: usize) -> bool {
        address < self.end && address + length > self.start
    }
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

/// Writes the initial contents of memory: the encoding of every instruction,
/// every run of bytes the Program carries, and nothing where it only reserves
/// room.
///
/// `ds` reserves memory and writes nothing to it (`Directives/ds.htm`), so a
/// [`MemoryContent::Reserved`] run leaves the fill the Interpreter starts with
/// where it is, and so does the pad byte before an aligned item.
///
/// The instructions' bytes are in memory as a 68000 would hold them, and the
/// Interpreter runs them from there, through the decode cache
/// ([`Memory::load_code`] fills it after this).
fn prepare_memory(memory: &mut Memory, program: &Program) -> RuntimeResult<()> {
    for instruction in program.instructions() {
        memory.write_bytes(instruction.address, &instruction.bytes)?;
    }
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
    /// What is being run. The instructions run from memory, through the decode
    /// cache the Program filled; the Program is the address map the source
    /// lookups read, and says which bytes it placed.
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
    current_interrupt: Option<Interrupt>,
    status: InterpreterStatus,
    /// Why the program ended, once it has: set with the terminated status and
    /// taken back with it by undo.
    termination: Option<Termination>,
    /// How the read tasks show what is typed, which tasks 12 and 16 change.
    input_settings: InputSettings,
    /// The range whose stores are recorded, empty while nothing is watched.
    watched: WatchedRange,
    /// The stores into [`watched`](Interpreter::watched) the host has not
    /// taken yet, in the order they were made.
    watched_writes: Vec<WatchedWrite>,
    /// How many instructions have run since the Interpreter was made.
    executed_instructions: u64,
    /// The code write of the instruction that has just run, if it made one.
    code_write: Option<CodeWrite>,
    /// Whether the run in progress stops after a code write: set by
    /// [`run_with_breakpoints`](Interpreter::run_with_breakpoints) for its
    /// duration, and read by [`step`](Interpreter::step).
    stop_on_code_write: bool,
    /// Whether T takes a trace exception after an instruction. It always does
    /// but in the conformance harness, whose suite records the state before
    /// the trace exception ([`Interpreter::from_state`]).
    takes_trace: bool,
    /// Whether the instruction waiting on the host started with T set, so
    /// that the trace exception follows the answer.
    trace_after_answer: bool,
}

impl Interpreter {
    /// An Interpreter ready to run `program` from its Entry point.
    ///
    /// Memory is the 16 MB of the address space, filled as [`Memory::new`]
    /// leaves it and then written with the Program's initial contents, and the
    /// decode cache is filled with the Program's instructions; the stack
    /// pointer starts at the top of memory. A Program whose Entry point is on
    /// `$FFFF $FFFF` the Program did not place has
    /// [ended](Termination::EndOfProgram) before it starts, which is where an
    /// empty one is. A Program with no instruction line runs what it placed
    /// at its Entry point, `dc.w` words included, as EASy68K runs them.
    pub fn new(program: Program, options: Option<InterpreterOptions>) -> Self {
        let start = program.entry();
        let mut memory = Memory::new();
        if let Err(e) = prepare_memory(&mut memory, &program) {
            //the Layout refuses to place anything past the address space, so this cannot
            //happen for a Program the Assembler built
            panic!("Error preparing memory: {:?}", e);
        }
        memory.set_map(program.memory_map());
        memory.load_code(program.instructions());
        let mut interpreter = Self::with_memory(program, memory, options);
        if interpreter.ends_at_pc() {
            interpreter.terminate(Termination::EndOfProgram {
                address: start & ADDRESS_MASK,
            });
        }
        interpreter
    }

    /// An Interpreter over `memory` as it is, about to run `program` from its
    /// Entry point, with the registers as a program starts with them
    /// ([`Cpu::new`]) and nothing ended yet.
    fn with_memory(program: Program, memory: Memory, options: Option<InterpreterOptions>) -> Self {
        let sp = INITIAL_SUPERVISOR_STACK_POINTER as usize;
        let start = program.entry();
        let options = options.unwrap_or_default();
        let interpreter = Self {
            stack_top: sp,
            memory,
            cpu: Cpu::new(),
            pc: start,
            keep_history: options.keep_history,
            poke: None,
            executing: false,
            current_instruction_address: 0,
            debugger: Debugger::new(options.history_size, program.symbols()),
            current_interrupt: None,
            status: InterpreterStatus::Running,
            termination: None,
            input_settings: InputSettings::default(),
            watched: WatchedRange::default(),
            watched_writes: Vec::new(),
            executed_instructions: 0,
            code_write: None,
            stop_on_code_write: false,
            takes_trace: true,
            trace_after_answer: false,
            program,
        };
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
    /// It is `$2000` before a program has run, as in EASy68K: supervisor mode,
    /// interrupt mask 0.
    #[inline(always)]
    pub fn get_sr(&self) -> u16 {
        self.cpu.get_sr()
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
            Termination::Exception { .. } | Termination::Halted { .. } => {
                InterpreterStatus::TerminatedWithException
            }
            _ => InterpreterStatus::Terminated,
        };
        self.termination = Some(cause);
    }

    /// Ends the program with the error an instruction raised and answers the
    /// error: [`InterpreterStatus::TerminatedWithException`], with the error
    /// and its `vector`, when it had one the program left without a handler,
    /// as the [`Termination`]. The Location of the line is the one the step
    /// recorded.
    fn end_with_an_exception(&mut self, error: RuntimeError, vector: Option<u8>) -> RuntimeError {
        self.terminate(Termination::Exception {
            cause: error.clone(),
            vector,
        });
        error
    }

    /// Halts the processor, a double fault: pushing the frame of the
    /// exception through `vector` raised `fault`. The program ends with
    /// [`Termination::Halted`].
    #[cold]
    #[inline(never)]
    fn halt(&mut self, fault: RuntimeError, vector: u8) -> RuntimeError {
        self.terminate(Termination::Halted {
            cause: fault.clone(),
            vector,
        });
        fault
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

    /// Whether the next step ends the program: the program counter is on
    /// `$FFFF $FFFF`, EASy68K's SIMHALT, none of whose bytes the Program placed
    /// (ADR 0009). That is where a program that does not end itself gets to:
    /// one past its last instruction, when nothing follows it, since memory
    /// starts as `$FF`.
    ///
    /// A SIMHALT inside the bytes the Program placed is a `simhalt`, which
    /// pauses, and any other bytes run as what they decode to.
    pub fn has_reached_bottom(&self) -> bool {
        let pc = self.pc & ADDRESS_MASK;
        if pc & 1 != 0 {
            return false;
        }
        match self.memory.cache.get(pc) {
            Some(slot) => slot.ends_the_program(),
            None => self.is_an_end(pc, &self.memory.decode_at(pc, self.program.processor())),
        }
    }

    /// Whether `decoded`, the instruction at `pc`, ends the program rather than
    /// running: a SIMHALT none of whose four bytes the Program placed.
    fn is_an_end(&self, pc: usize, decoded: &Decoded) -> bool {
        matches!(decoded.instruction, Instruction::SIMHALT) && !self.program.places_any(pc, 4)
    }

    /// The instruction at `pc`, from the decode cache, decoded from memory and
    /// kept there on a miss.
    ///
    /// The address is reduced to the 24 bits of the bus. An odd one has none,
    /// and fetches the [empty](Slot::is_empty) slot: it is the 68000's address
    /// error, an instruction fetched from an odd address
    /// ([`odd_fetch`](Interpreter::odd_fetch)).
    ///
    /// The slot is copied once, out of the cache into the step that runs it. An
    /// `Option` around it would be copied again on the way, and for an
    /// instruction as cheap as an `addq` to a register, that second copy is a
    /// good part of the step.
    #[inline(always)]
    fn fetch(&mut self, pc: usize) -> Slot {
        let pc = pc & ADDRESS_MASK;
        if pc & 1 != 0 {
            return cache::EMPTY;
        }
        match self.memory.cache.get(pc) {
            Some(slot) => *slot,
            None => self.decode_into_the_cache(pc),
        }
    }

    /// The error of a [`fetch`](Interpreter::fetch) that found nothing at
    /// `pc`: the address error of an odd address, or the bus error of a word
    /// the memory map refuses to fetch, an access in program space, at the
    /// address of the first such word.
    #[cold]
    #[inline(never)]
    fn fetch_error(&self, pc: usize) -> RuntimeError {
        let bus = pc & ADDRESS_MASK;
        if bus & 1 == 0 {
            let size = self.memory.decode_at(bus, self.program.processor()).size as usize;
            if let Some((at, range)) = self.memory.fetch_fault(bus, size) {
                return RuntimeError::BusError {
                    address: (pc as u32).wrapping_add(at.wrapping_sub(bus) as u32) as usize,
                    size: Size::Word,
                    access: Access::Program,
                    range,
                };
            }
        }
        Self::jump_to_an_odd_address(pc as u32)
    }

    /// The address error of a jump to `target`, odd, which the 68000 raises in
    /// the jump's own prefetch from it.
    #[cold]
    #[inline(never)]
    fn jump_to_an_odd_address(target: u32) -> RuntimeError {
        RuntimeError::AddressError {
            address: target as usize,
            size: Size::Word,
            access: Access::Program,
        }
    }

    /// A miss of [`fetch`](Interpreter::fetch): bytes the Program did not place
    /// as code, code a write has changed since it was last decoded, or an
    /// instruction the memory map checks at every fetch, which is not kept
    /// ([`Memory::keeps`]).
    #[cold]
    #[inline(never)]
    fn decode_into_the_cache(&mut self, pc: usize) -> Slot {
        let decoded = self.memory.decode_at(pc, self.program.processor());
        //a word the memory map refuses to fetch is a bus error, which the step that fetches
        //it raises, as for an odd address (`fetch_error`); the cache keeps nothing there
        if self.memory.fetch_fault(pc, decoded.size as usize).is_some() {
            return cache::EMPTY;
        }
        let slot = Slot::decoded(decoded, self.is_an_end(pc, &decoded));
        if self.memory.keeps(pc, decoded.size as usize) {
            self.memory.cache.store(pc, slot);
        }
        slot
    }

    /// Whether the instruction at the program counter ends the program, as
    /// [`has_reached_bottom`](Interpreter::has_reached_bottom), read through the
    /// cache: what the next step will fetch, fetched now, though only the
    /// slot's index is read.
    #[inline(always)]
    fn ends_at_pc(&mut self) -> bool {
        let pc = self.pc & ADDRESS_MASK;
        if pc & 1 != 0 {
            return false;
        }
        match self.memory.cache.get(pc) {
            Some(slot) => slot.ends_the_program(),
            None => self.decode_into_the_cache(pc).ends_the_program(),
        }
    }

    /// Records the code write the cache saw since it was last asked, as the
    /// code write of the instruction executing, when it has not made one
    /// already.
    #[inline(always)]
    fn take_code_write(&mut self) {
        if self.memory.cache.has_cleared() {
            self.record_code_write();
        }
    }

    #[cold]
    #[inline(never)]
    fn record_code_write(&mut self) {
        let Some(cleared) = self.memory.cache.take_cleared() else {
            return;
        };
        if self.code_write.is_some() {
            return;
        }
        let write = CodeWrite {
            address: cleared.address,
            instruction_index: cleared
                .index
                .or_else(|| self.program.instruction_index_at(cleared.address)),
        };
        self.code_write = Some(write);
        if self.keep_history {
            self.debugger.set_code_write(write);
        }
    }

    /// Runs one instruction. If the Interpreter is paused, this resumes at the
    /// instruction after the `simhalt` that paused it, and after a
    /// [watched write](InterpreterStatus::WatchedWrite) or a
    /// [code write](InterpreterStatus::CodeWrite) stop at the instruction after
    /// the one that made it.
    ///
    /// The instruction is the one memory holds at the program counter, from the
    /// decode cache. On `$FFFF $FFFF` the Program never placed, the program
    /// ends there ([`Termination::EndOfProgram`]) and nothing runs; the end is
    /// already the step before's, when it got there by running on.
    ///
    /// An instruction that raises a runtime error ends the program with an
    /// exception, [`InterpreterStatus::TerminatedWithException`], and the error
    /// is answered as well as recorded as the [`Termination`]. The step stays in
    /// the history, so undoing it brings the program back to the instruction
    /// that failed, running. An instruction fetched from an odd address is such
    /// an error, the 68000's [address error](RuntimeError::AddressError).
    pub fn step(&mut self) -> RuntimeResult<InterpreterStatus> {
        self.verify_can_run()?;
        self.run_step()?;
        self.end_if_reached();
        Ok(self.status)
    }

    /// A [`step`](Interpreter::step) taken by a run, which has checked once
    /// that the program can run and takes a step only while it still can, and
    /// which leaves the end of the program to the next step: that step's fetch
    /// finds the end and ends the program before anything runs.
    ///
    /// That way each step looks its instruction up once, where a step on its own
    /// looks the next one up as well, to know whether it has ended the program.
    /// A run that stops while the program is running, on its limit or on a
    /// breakpoint, takes no next step and checks the end itself
    /// ([`end_if_reached`](Interpreter::end_if_reached)), so every run answers
    /// the status and the [`Termination`] a run of single steps would have.
    #[inline(always)]
    fn run_step(&mut self) -> RuntimeResult<()> {
        let sp_before = self.get_sp();
        let old_status = self.status;
        let pc = self.pc;
        let slot = self.fetch(pc);
        if slot.ends_the_program() {
            self.terminate(Termination::EndOfProgram {
                address: pc & ADDRESS_MASK,
            });
            return Ok(());
        }
        if self.is_resumable_stop() {
            self.set_status(InterpreterStatus::Running);
        }
        if self.keep_history {
            self.debugger.begin_step(
                ExecutionStepKind::Instruction,
                pc,
                self.cpu.ccr,
                self.cpu.get_sr(),
                old_status,
            );
        }
        self.current_instruction_address = pc;
        self.code_write = None;
        if self.keep_history {
            //the line the address map has there, which a change to the bytes does not move;
            //copied into the step's own slot, so a full history copies without allocating
            let location = match slot.program_index() {
                Some(index) => Some(&self.program.instructions()[index].location),
                None => self
                    .program
                    .instruction_at(pc & ADDRESS_MASK)
                    .map(|instruction| &instruction.location),
            };
            //with the words that run, which a later write into code does not change in the step:
            //the Program's own instruction's, or, decoded as the program ran, memory's
            let ran = match slot.program_index() {
                Some(index) => Ran::Assembled(index as u32),
                None => self.memory.ran_at(pc & ADDRESS_MASK, slot.size()),
            };
            self.debugger.set_instruction(location, ran);
        }
        self.increment_pc(slot.size());
        self.executing = true;
        self.executed_instructions += 1;
        let watched = self.watched_writes.len();
        //T as it was before the instruction, which is what decides a trace exception after it
        let traced = self.cpu.is_tracing();
        let mut executed = match slot.is_empty() {
            true => Err(self.fetch_error(pc)),
            false => self.execute_instruction(&slot.instruction),
        };
        if executed.is_err() || traced {
            //an empty slot is an instruction that could not be fetched, which ran nothing
            let fetched = (!slot.is_empty()).then_some(&slot.instruction);
            executed = self.take_exceptions(executed, fetched, pc, traced);
        }
        let sp = self.get_sp();
        let top = if sp.abs_diff(sp_before) > 4096 || sp > self.stack_top { sp } else { self.stack_top };
        if top != self.stack_top {
            if self.keep_history { self.debugger.record_stack_top(self.stack_top); }
            self.stack_top = top;
        }
        //an instruction that raised an Interrupt is not finished until the host answers
        //it, and what `answer_interrupt` writes belongs to this step
        self.executing = self.status == InterpreterStatus::Interrupt;
        self.take_code_write();
        if self.keep_history {
            self.debugger.set_new_ccr(self.cpu.ccr);
            self.debugger.set_new_sr(self.cpu.get_sr());
        }
        executed?;
        if self.status == InterpreterStatus::Running {
            //the host takes what the instruction stored before the next one runs, and sees the
            //code write it was asked to stop for; the end of the program, if this was the last
            //instruction, is the next call's after either
            if self.watched_writes.len() != watched {
                self.status = InterpreterStatus::WatchedWrite;
            } else if self.stop_on_code_write && self.code_write.is_some() {
                self.status = InterpreterStatus::CodeWrite;
            }
        }
        Ok(())
    }

    /// Ends the program when it is running and the next step would end it
    /// ([`ends_at_pc`](Interpreter::ends_at_pc)): what a step does after its
    /// instruction, so that the status after the last instruction already says
    /// the program has ended.
    fn end_if_reached(&mut self) {
        if self.status == InterpreterStatus::Running && self.ends_at_pc() {
            self.terminate(Termination::EndOfProgram {
                address: self.pc & ADDRESS_MASK,
            });
        }
    }
    /// What follows an instruction that raised an error or started with T
    /// set: the Exceptions it takes (ADR 0008).
    ///
    /// An error the 68000 has a vector for is taken through it
    /// ([`take_exception`](Interpreter::take_exception)), and one it has none
    /// for ends the program. After an instruction that started with T set, a
    /// trace exception follows, unless the instruction raised one of group 0
    /// or 1, which it did not finish: so after `trap`, `trapv`, `chk` or a
    /// division by zero, the trace frame is pushed on top of theirs and the
    /// trace handler runs first, returning into theirs, as on a 68000. An
    /// instruction that raised an Interrupt is traced when the host has
    /// answered it, and one that paused or ended the program is not.
    #[cold]
    #[inline(never)]
    fn take_exceptions(
        &mut self,
        executed: RuntimeResult<()>,
        instruction: Option<&Instruction>,
        at: usize,
        traced: bool,
    ) -> RuntimeResult<()> {
        let traced = traced && self.takes_trace;
        let error = match executed {
            Ok(()) => {
                return if traced {
                    self.trace_after_instruction()
                } else {
                    Ok(())
                }
            }
            Err(error) => error,
        };
        let Some(vector) = exception::vector_of(&error) else {
            return Err(self.end_with_an_exception(error, None));
        };
        let frame = self.frame_of(&error, vector, instruction, at);
        self.take_exception(vector, frame, error)?;
        if traced && exception::group(vector) == exception::Group::Two {
            self.trace_after_instruction()?;
        }
        Ok(())
    }

    /// The trace exception after an instruction that started with T set, or,
    /// for one waiting on the host, after its answer.
    fn trace_after_instruction(&mut self) -> RuntimeResult<()> {
        match self.status {
            InterpreterStatus::Running => self.take_exception(
                exception::TRACE,
                Frame::Short { pc: self.pc as u32 },
                RuntimeError::Trace,
            ),
            InterpreterStatus::Interrupt => {
                self.trace_after_answer = true;
                Ok(())
            }
            _ => Ok(()),
        }
    }

    /// The frame `error`, raised by `instruction` at `at`, pushes through
    /// `vector`: the long one of group 0 for an address error, with the
    /// program counter [`exception::address_error_pc`] says; the short one
    /// with the instruction's own address for groups 1, and with the next
    /// instruction's for group 2.
    fn frame_of(
        &self,
        error: &RuntimeError,
        vector: u8,
        instruction: Option<&Instruction>,
        at: usize,
    ) -> Frame {
        let at = at as u32;
        match *error {
            RuntimeError::AddressError {
                address, access, ..
            }
            | RuntimeError::BusError {
                address, access, ..
            } => {
                let address = address as u32;
                //an instruction that could not be fetched, from an odd address or one the memory
                //map refuses, is no instruction at all, and stacks its own address. A bus error
                //is the same abort as an address error at the same access, so the prefetch has
                //reached the same place
                let (pc, register_at) = match instruction {
                    None => (at, at),
                    Some(instruction) => {
                        let pc = exception::address_error_pc(
                            instruction,
                            at,
                            self.pc as u32,
                            access,
                            address,
                        );
                        let register_at =
                            exception::instruction_register_address(instruction, at, pc, access);
                        (pc, register_at)
                    }
                };
                let instruction_register = self.memory.word_at(register_at as usize);
                Frame::Group0 {
                    pc,
                    access_address: address,
                    instruction_register,
                    status_word: exception::status_word(
                        instruction_register,
                        access,
                        self.cpu.is_supervisor(),
                    ),
                }
            }
            _ => Frame::Short {
                pc: match exception::group(vector) {
                    exception::Group::Two => self.pc as u32,
                    exception::Group::Zero | exception::Group::One => at,
                },
            },
        }
    }

    /// Takes the Exception through `vector`, pushing `frame`, which `cause`
    /// raised: the one frame builder (the design's decision 10), which the
    /// dialects' frames will extend.
    ///
    /// The vector is read first, in supervisor mode, as the 68000 reads it,
    /// and against the memory map: one in an invalid range is a bus error, a
    /// double fault. An odd address there is no handler, which is
    /// what every vector holds until the program writes it, and the program
    /// ends with `cause` as it would have before exceptions were taken, the
    /// vector named beside it. Otherwise the processor copies the status
    /// register, sets S and clears T, which makes A7 the supervisor stack
    /// pointer, pushes the frame there with the copy, and jumps to the
    /// handler. A fault while it pushes the frame, which an odd supervisor
    /// stack pointer raises, or the memory map where it refuses the frame, is
    /// a double fault, and the processor halts.
    ///
    /// The call stack gains an exception frame, which `rte` takes off.
    fn take_exception(
        &mut self,
        vector: u8,
        frame: Frame,
        cause: RuntimeError,
    ) -> RuntimeResult<()> {
        let handler = match self
            .memory
            .read_vector(exception::vector_address(vector) as usize)
        {
            Ok(handler) => handler,
            Err(fault) => return Err(self.halt(fault, vector)),
        };
        if handler & 1 != 0 {
            return Err(self.end_with_an_exception(cause, Some(vector)));
        }
        let sr = self.cpu.get_sr();
        self.write_sr((sr | SUPERVISOR_BIT) & !TRACE_BIT);
        if let Err(fault) = self.push_frame(&frame, sr) {
            return Err(self.halt(fault, vector));
        }
        let from = self.current_instruction_address;
        self.pc = handler as usize;
        if self.keep_history {
            self.debugger
                .add_mutation(MutationOperation::PushException {
                    vector,
                    from,
                    handler: handler as usize,
                });
        }
        self.debugger.push_exception(
            handler as usize,
            frame.pc() as usize,
            self.cpu.get_register_values(),
            vector,
        );
        Ok(())
    }

    /// Pushes `frame`, which stacks `sr`, on the supervisor stack, which A7
    /// is: its words from the top down, as the 68000 pushes them, each a store
    /// of the program's.
    fn push_frame(&mut self, frame: &Frame, sr: u16) -> RuntimeResult<()> {
        let sp = (self.get_sp() as u32).wrapping_sub(frame.size());
        let words = frame.words(sr);
        for (index, word) in words.iter().enumerate().rev() {
            let address = sp.wrapping_add(index as u32 * 2) as usize;
            self.set_memory_value(address, Size::Word, *word as u32)?;
        }
        self.set_sp(sp as usize);
        Ok(())
    }

    /// Writes the status register for the program or the host, as the 68000
    /// does: when S changes, A7 becomes the other stack pointer
    /// ([`Cpu::set_sr`]), and the swap is journaled
    /// ([`MutationOperation::WriteStackPointers`]) into the step being executed
    /// or the open Poke, so that undo, which writes the status register back
    /// without moving the stack pointers, puts each back where it was.
    fn write_sr(&mut self, value: u16) {
        let pointers = self.cpu.stack_pointers();
        if self.cpu.set_sr(value) {
            self.journal_stack_pointers(pointers, pointers);
            self.memory.set_supervisor(self.cpu.is_supervisor());
        }
    }

    /// Journals a change of the stack pointers into the step being executed,
    /// or into the open Poke.
    fn journal_stack_pointers(&mut self, old: StackPointers, new: StackPointers) {
        let mutation = MutationOperation::WriteStackPointers { old, new };
        if self.executing {
            if self.keep_history {
                self.debugger.add_mutation(mutation);
            }
        } else if let Some(poke) = self.poke.as_mut() {
            poke.add_mutation(mutation);
        }
    }

    /// Sets the user stack pointer: A7 in user mode, the one kept aside in
    /// supervisor mode. Journaled like any write.
    fn write_usp(&mut self, value: u32) {
        let old = self.cpu.stack_pointers();
        let new = StackPointers { usp: value, ..old };
        self.cpu.restore_stack_pointers(new);
        if old != new {
            self.journal_stack_pointers(old, new);
        }
    }

    /// The user stack pointer.
    pub fn get_usp(&self) -> u32 {
        self.cpu.get_usp()
    }

    /// The supervisor stack pointer.
    pub fn get_ssp(&self) -> u32 {
        self.cpu.get_ssp()
    }

    /// The status register and the two stack pointers: the System register
    /// file.
    pub fn get_system(&self) -> SystemRegisters {
        SystemRegisters {
            sr: self.cpu.get_sr(),
            usp: self.cpu.get_usp(),
            ssp: self.cpu.get_ssp(),
        }
    }

    /// Writes a register of the System file on behalf of the host: inside a
    /// Poke it is journaled, so that undoing the Poke puts it back, and
    /// outside one it is direct and records nothing, as
    /// [`set_register_value`](Interpreter::set_register_value) is.
    fn poke_system(&mut self, register: SystemRegister, value: u32) {
        let old = match register {
            SystemRegister::Sr => self.cpu.get_sr() as u32,
            SystemRegister::Usp => self.cpu.get_usp(),
            SystemRegister::Ssp => self.cpu.get_ssp(),
        };
        match register {
            SystemRegister::Sr => self.write_sr(value as u16),
            SystemRegister::Usp => self.write_usp(value),
            SystemRegister::Ssp => {
                let pointers = self.cpu.stack_pointers();
                let new = StackPointers {
                    ssp: value,
                    ..pointers
                };
                self.cpu.restore_stack_pointers(new);
                if pointers != new {
                    self.journal_stack_pointers(pointers, new);
                }
            }
        }
        let now = match register {
            SystemRegister::Sr => self.cpu.get_sr() as u32,
            SystemRegister::Usp => self.cpu.get_usp(),
            SystemRegister::Ssp => self.cpu.get_ssp(),
        };
        if now != old {
            if let Some(poke) = self.poke.as_mut() {
                poke.add_system_target(register, old);
            }
        }
    }

    /// Writes the whole status register for the host, as `move #n,sr` would:
    /// reduced to [`SR_IMPLEMENTED_BITS`], and a change of S makes A7 the
    /// other stack pointer. Inside a Poke it is journaled; outside one it is
    /// direct.
    pub fn set_sr(&mut self, value: u16) {
        self.poke_system(SystemRegister::Sr, value as u32);
    }

    /// Writes the user stack pointer for the host, A7 itself in user mode.
    /// Inside a Poke it is journaled; outside one it is direct.
    pub fn set_usp(&mut self, value: u32) {
        self.poke_system(SystemRegister::Usp, value);
    }

    /// Writes the supervisor stack pointer for the host, A7 itself in
    /// supervisor mode. Inside a Poke it is journaled; outside one it is
    /// direct.
    pub fn set_ssp(&mut self, value: u32) {
        self.poke_system(SystemRegister::Ssp, value);
    }

    pub fn get_pretty_call_stack(&self) -> Vec<PrettyStackFrame> {
        self.debugger.to_call_stack()
    }
    /// Takes back the newest step of the history, instruction or Poke, and
    /// answers it. It empties the log of watched writes
    /// ([`take_watched_writes`](Interpreter::take_watched_writes)).
    ///
    /// The bytes it writes back pass the decode cache like any write, so an
    /// instruction the step overwrote is decoded again from its old bytes
    /// before it runs, and the step's code write goes with it:
    /// [`get_code_write`](Interpreter::get_code_write) is `None` after an undo.
    pub fn undo(&mut self) -> RuntimeResult<ExecutionStep> {
        match self.debugger.pop_step() {
            Some(step) => {
                if let Some(top) = step.old_stack_top { self.stack_top = top; }
                self.pc = step.get_pc();
                //the whole status register, which is the condition codes and the system byte
                //`move #n,sr`, an exception and `rte` can have changed. It is written back
                //without moving a stack pointer: every change of S journaled both of them, and
                //they are put back below, A7 being the one of this mode
                self.cpu.restore_sr(step.get_sr());
                self.memory.set_supervisor(self.cpu.is_supervisor());
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
                        MutationOperation::PopCall { frame, .. } => {
                            //a return from no call (an `rts` with nothing on the call stack)
                            //took nothing off it, and puts nothing back
                            if let Some(frame) = frame {
                                self.debugger.restore_call(frame.clone());
                            }
                        }
                        MutationOperation::PushCall { to: _, from: _ } => {
                            self.debugger.pop_call();
                        }
                        MutationOperation::SetInputSettings { old, new: _ } => {
                            self.input_settings = *old;
                        }
                        MutationOperation::WriteStackPointers { old, new: _ } => {
                            self.cpu.restore_stack_pointers(*old);
                        }
                        MutationOperation::PushException { .. } => {
                            self.debugger.pop_call();
                        }
                        MutationOperation::PopException { frame, .. } => {
                            if let Some(frame) = frame {
                                self.debugger.restore_call(frame.clone());
                            }
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
                    self.trace_after_answer = false;
                }
                if !self.has_terminated() {
                    self.termination = None;
                }
                //a write not taken yet may be one this undo has just taken back, so none is kept:
                //the host reads the watched range again, as memory now holds it
                self.watched_writes.clear();
                //the old bytes undo wrote back are no code write of the program's
                self.memory.cache.take_cleared();
                self.code_write = None;
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
        self.poke = Some(PokeJournal::new(self.cpu.get_sr()));
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
        let opened_with = journal.sr();
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
                PokeTarget::System { register, old } => PokeWrite::Register {
                    name: register.name().to_string(),
                    old,
                    new: match register {
                        SystemRegister::Sr => self.cpu.get_sr() as u32,
                        SystemRegister::Usp => self.cpu.get_usp(),
                        SystemRegister::Ssp => self.cpu.get_ssp(),
                    },
                },
            })
            .collect::<Vec<PokeWrite>>();
        //no instruction ran, so the step carries the program counter as it is, and the status
        //register as the Poke found it, which undo writes back: undoing it puts back only what
        //the Poke wrote
        self.debugger.begin_step(
            ExecutionStepKind::Poke,
            self.pc,
            Flags::from_ccr_byte(opened_with as u8),
            opened_with,
            self.status,
        );
        for mutation in mutations {
            self.debugger.add_mutation(mutation);
        }
        self.debugger.set_writes(writes);
        self.debugger.set_new_ccr(self.cpu.ccr);
        self.debugger.set_new_sr(self.cpu.get_sr());
        Ok(true)
    }

    /// Watches the addresses from `start` up to `end`, `end` not included, for
    /// a device the host keeps in memory: every store into them is recorded
    /// from now on, and an instruction that makes one ends the run or step
    /// that ran it with [`InterpreterStatus::WatchedWrite`].
    ///
    /// A store is recorded when any of its bytes is in the range, so one that
    /// straddles an edge counts. That is every store the program makes — a
    /// `move`, each register of a `movem`, the one write of an `addq`, `bset`
    /// or `clr` to memory, the long `bsr`, `jsr` and `pea` push — and every
    /// run of bytes written on its behalf (the answer to a read task) or by the
    /// host ([`write_memory_bytes`](Interpreter::write_memory_bytes), a Poke or
    /// not), a byte at a time and only the bytes in the range. The Program's
    /// initial contents and undo are not stores.
    ///
    /// One range at a time: this replaces the one watched before and forgets
    /// the writes recorded and not taken. An empty range, or one past the
    /// 16 MB of memory, is an [`InvalidArgument`](RuntimeError::InvalidArgument).
    pub fn watch_memory(&mut self, start: usize, end: usize) -> RuntimeResult<()> {
        if start >= end || end > MEMORY_SIZE {
            return Err(RuntimeError::InvalidArgument(format!(
                "a watched range is from its start up to its end, the end not included, inside \
                 the 16 MB of memory: ${:x} to ${:x} is not one",
                start, end
            )));
        }
        self.watched = WatchedRange { start, end };
        self.watched_writes.clear();
        Ok(())
    }

    /// How many instructions have run since the Interpreter was made, each
    /// counted once as it starts, the one that failed or raised an interrupt
    /// included. Undo does not take them back: this is the work done, which is
    /// what a host that splits a run in slices needs, since a run that stops
    /// early, at an interrupt, a pause or a watched write, says nothing of how
    /// far it got.
    pub fn get_executed_instructions(&self) -> u64 {
        self.executed_instructions
    }

    /// Stops watching, and forgets the writes recorded and not taken.
    pub fn unwatch_memory(&mut self) {
        self.watched = WatchedRange::default();
        self.watched_writes.clear();
    }

    /// The stores into the watched range since the last call, in the order
    /// they were made, and the log emptied.
    ///
    /// The log is the host's to drain, after every
    /// [`WatchedWrite`](InterpreterStatus::WatchedWrite) stop and after a
    /// write of its own; [`undo`](Interpreter::undo) empties it, since what it
    /// held may be what the undo took back.
    pub fn take_watched_writes(&mut self) -> Vec<WatchedWrite> {
        std::mem::take(&mut self.watched_writes)
    }

    /// Records a store of the program at `size` when it touches the watched
    /// range. It is on the path of every store, so watching nothing costs it
    /// the one comparison of [`WatchedRange::touches`].
    #[inline(always)]
    fn watch_store(&mut self, address: usize, size: Size, value: u32) {
        //the address the store went to, as `Memory` masks it
        let address = address & 0x00ffffff;
        if self.watched.touches(address, size.to_bytes()) {
            self.record_watched_store(address, size, value);
        }
    }

    #[cold]
    #[inline(never)]
    fn record_watched_store(&mut self, address: usize, size: Size, value: u32) {
        self.watched_writes.push(WatchedWrite {
            address,
            size: size.to_bytes(),
            value: get_value_sized(value, size),
        });
    }

    /// Records the bytes of a run written at `address` that are in the watched
    /// range, one write a byte.
    fn watch_bytes(&mut self, address: usize, bytes: &[u8]) {
        let address = address & 0x00ffffff;
        if !self.watched.touches(address, bytes.len()) {
            return;
        }
        let from = address.max(self.watched.start);
        let to = (address + bytes.len()).min(self.watched.end);
        self.watched_writes.extend((from..to).map(|at| WatchedWrite {
            address: at,
            size: 1,
            value: bytes[at - address] as u32,
        }));
    }

    /// Whether the Interpreter stopped between two instructions and the next
    /// run or step goes on from there: [`Paused`](InterpreterStatus::Paused)
    /// at a `simhalt`, or after a [watched write](InterpreterStatus::WatchedWrite)
    /// or a [code write](InterpreterStatus::CodeWrite).
    #[inline(always)]
    fn is_resumable_stop(&self) -> bool {
        matches!(
            self.status,
            InterpreterStatus::Paused | InterpreterStatus::WatchedWrite | InterpreterStatus::CodeWrite
        )
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
    /// and with the [`RuntimeError::OutOfBounds`] of the write when what it
    /// writes does not fit in memory.
    ///
    /// What the answer writes is the program's, on behalf of the instruction
    /// that raised the interrupt: a line read over code is a
    /// [code write](CodeWrite) of that instruction's, which
    /// [`get_code_write`](Interpreter::get_code_write) and the step answer
    /// after this. Nothing is stopped for it, as no run is in progress.
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
        let watched = self.watched_writes.len();
        let answered = check_answer(&pending, &answer).and_then(|()| self.apply_answer(answer));
        if let Err(error) = answered {
            self.memory.cache.take_cleared();
            self.current_interrupt = Some(pending);
            return Err(error);
        }
        self.take_code_write();
        //the instruction is finished now, and one that started with T set is traced: the trace
        //exception is its step's, and a vector 9 with no handler ends the program there
        if std::mem::take(&mut self.trace_after_answer) && !terminates {
            self.status = InterpreterStatus::Running;
            let traced = self.take_exception(
                exception::TRACE,
                Frame::Short { pc: self.pc as u32 },
                RuntimeError::Trace,
            );
            if self.keep_history {
                self.debugger.set_new_ccr(self.cpu.ccr);
                self.debugger.set_new_sr(self.cpu.get_sr());
            }
            if traced.is_err() {
                self.executing = false;
                return Ok(());
            }
        }
        self.executing = false;
        //an answer of Terminate ends the program, and so does the answer to an interrupt the
        //last instruction raised; an answer that wrote into the watched range stops first, as
        //the instruction would have
        if terminates {
            self.terminate(Termination::TerminatedByHost);
        } else if self.watched_writes.len() != watched {
            self.status = InterpreterStatus::WatchedWrite;
        } else if self.ends_at_pc() {
            self.terminate(Termination::EndOfProgram {
                address: self.pc & ADDRESS_MASK,
            });
        } else {
            self.status = InterpreterStatus::Running;
        }
        Ok(())
    }

    /// Writes what an answer [`check_answer`] took carries, as EASy68K writes
    /// it for the task. It fails, having written nothing, only when a run of
    /// bytes does not fit in memory.
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
            // ---- sound: what the player did, as EASy68K's result through a `short *` to D0
            //(`simIOu.cpp`). `playSound` and `playSoundMem` write nothing when the file or the
            //slot is missing, so D0 is left as it was
            InterruptResult::PlaySound(outcome) | InterruptResult::PlayLoadedSound(outcome) => {
                if outcome != SoundOutcome::Missing {
                    self.set_sound_result(outcome);
                }
            }
            InterruptResult::PlaySoundDirectX(outcome)
            | InterruptResult::LoadSoundDirectX(outcome)
            | InterruptResult::PlayLoadedSoundDirectX(outcome)
            | InterruptResult::ControlSound(outcome)
            | InterruptResult::ControlSoundDirectX(outcome) => self.set_sound_result(outcome),
        };
        Ok(())
    }

    /// Writes a file task's result to D0.W, as EASy68K does through a
    /// `short *` to D0.
    fn set_file_result(&mut self, result: FileResult) {
        self.set_register_value(RegisterOperand::Data(0), result as u32, Size::Word);
    }

    /// Writes a sound task's result to D0.W: 1 when it was done, 0 when it
    /// was not.
    fn set_sound_result(&mut self, outcome: SoundOutcome) {
        let done = outcome == SoundOutcome::Done;
        self.set_register_value(RegisterOperand::Data(0), done as u32, Size::Word);
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
    /// The instruction the Program laid out at `address`, if one is: the
    /// address map, which is the line as it was assembled even after the
    /// program or a Poke changed its bytes.
    /// [`disassemble_at`](Interpreter::disassemble_at) reads what memory holds
    /// there now.
    #[inline(always)]
    pub fn get_instruction_at(&self, address: usize) -> Option<&AssembledInstruction> {
        self.program.instruction_at(address & ADDRESS_MASK)
    }

    /// What the instruction laid out at `address` assembled as, when the
    /// Assembler chose something its line did not write
    /// ([`Program::assembled_as`]), with the Program's Labels for its
    /// addresses.
    pub fn get_assembled_as(&self, instruction: &AssembledInstruction) -> Option<String> {
        let labels = self.debugger.get_labels();
        let names = |address: u32| labels.get(&(address as usize)).map(|label| label.name.clone());
        self.program.assembled_as(instruction, Some(&names))
    }

    /// The instruction memory holds at `address` now, decoded for the
    /// Program's processor and printed in EASy68K's syntax with the Program's
    /// Labels: what runs there, whatever the Program laid out. Bytes that are
    /// no instruction, or an instruction of a later processor, print as
    /// `DC.W`, and so does an instruction no line writes, a `movem` of no
    /// register. It reads memory and leaves the decode cache alone.
    pub fn disassemble_at(&self, address: usize) -> disassemble::Line {
        let address = address & ADDRESS_MASK;
        let decoded = self.memory.decode_at(address, self.program.processor());
        let labels = self.debugger.get_labels();
        let names = |address: u32| labels.get(&(address as usize)).map(|label| label.name.clone());
        let bytes: Vec<u8> = (0..decoded.size as usize)
            .map(|offset| self.memory.data[(address + offset) & ADDRESS_MASK])
            .collect();
        disassemble::Line {
            address: address as u32,
            size: decoded.size,
            text: disassemble::disassemble_words(&decoded.instruction, &bytes, Some(&names)),
            bytes,
        }
    }

    /// What `step` ran: the instruction its words decode to at its address,
    /// for the Program's processor and with the Program's Labels, as
    /// [`disassemble_at`](Interpreter::disassemble_at) prints one, or `None`
    /// for a Poke and for a step that ran nothing, an instruction fetched from
    /// an odd address. The words are the step's own, kept when it ran, so a
    /// later write into code leaves it saying what really ran.
    pub fn ran_by(&self, step: &ExecutionStep) -> Option<disassemble::Line> {
        let bytes = match &step.get_ran() {
            Ran::Nothing => return None,
            Ran::Assembled(index) => self.program.instructions().get(*index as usize)?.bytes.clone(),
            Ran::Decoded { bytes, len } => bytes[..*len as usize].to_vec(),
        };
        let bytes = bytes.as_slice();
        let address = step.get_pc() & ADDRESS_MASK;
        let labels = self.debugger.get_labels();
        let names = |address: u32| labels.get(&(address as usize)).map(|label| label.name.clone());
        let mut lines = disassemble::disassemble_bytes(
            bytes,
            address as u32,
            self.program.processor(),
            Some(&names),
        );
        //the words are one instruction's, so they are one line
        (lines.len() == 1).then(|| lines.remove(0))
    }

    /// A step for the host: the step itself, with what it ran as `ran`
    /// ([`ran_by`](Interpreter::ran_by)), absent when it ran nothing.
    fn step_to_js(&self, step: &ExecutionStep) -> JsValue {
        let value = js(step);
        if let Some(line) = self.ran_by(step) {
            let _ = js_sys::Reflect::set(&value, &JsValue::from_str("ran"), &js(&line));
        }
        value
    }

    /// The [code write](CodeWrite) of the instruction that has just run, or of
    /// the one whose interrupt was answered, if it made one; `None` before the
    /// first step and after an undo.
    pub fn get_code_write(&self) -> Option<CodeWrite> {
        self.code_write
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
    ///
    /// It is the address map's ([`get_instruction_at`](Interpreter::get_instruction_at)),
    /// so a line the program changed is still highlighted as itself, and a
    /// program counter on no line the Program laid out, in data or in the
    /// middle of a line, is on none.
    pub fn get_current_location(&self) -> Option<&Location> {
        self.get_instruction_at(self.pc)
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
            //the flags are set after the store, which reads none of them, so that a store that
            //faults finds them as they were and sets what MAME's microcode had set by then
            Instruction::MOVE(source, dest, size) => {
                let source_value = self.get_operand_value(source, *size, Used::Once)?;
                if let Err(error) = self.store_operand_value(dest, source_value, *size, Used::Once)
                {
                    return Err(self.move_write_fault(error, source, dest, *size, source_value));
                }
                self.set_logic_flags(source_value, *size);
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
                        self.move_memory_to_registers(addr as usize, *size, *registers_mask, false)?
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
                        //relative to the program counter is in program space
                        TargetDirection::FromMemory => self.move_memory_to_registers(
                            addr as usize,
                            *size,
                            *registers_mask,
                            matches!(
                                target,
                                Operand::PcDisplacement { .. } | Operand::PcIndex { .. }
                            ),
                        )?,
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
                let (source_value, dest_value) = self.read_extended_pair(source, dest, *size)?;
                let extend = self.cpu.ccr.contains(Flags::Extend);
                let (result, carry) = add_with_extend(dest_value, source_value, extend, *size);
                let overflow = has_add_overflowed(dest_value, source_value, result, *size);
                self.store_operand_value(dest, result, *size, Used::Twice)?;
                self.set_extended_arithmetic_flags(result, *size, carry, overflow);
            }
            Instruction::SUBX(source, dest, size) => {
                let (source_value, dest_value) = self.read_extended_pair(source, dest, *size)?;
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

            //the target is kept as an address, so the size of the displacement does not matter here
            Instruction::BRA(address, _) => {
                if *address & 1 != 0 {
                    return Err(Self::jump_to_an_odd_address(*address));
                }
                self.pc = *address as usize;
            }
            Instruction::BSR(address, _) => {
                //the return address is pushed before the 68000 prefetches from the target, so a
                //branch to an odd address faults with it pushed
                let sp = self.get_sp().wrapping_sub(4);
                self.set_memory_value(sp, Size::Long, self.pc as u32)?;
                if *address & 1 != 0 {
                    self.set_sp(sp);
                    return Err(Self::jump_to_an_odd_address(*address));
                }
                if self.keep_history {
                    self.debugger.add_mutation(MutationOperation::PushCall {
                        to: *address as usize,
                        //the pc is incremented before the instruction is executed, so the address
                        //of the call is the one the step recorded
                        from: self.current_instruction_address,
                    });
                }
                self.set_sp(sp);
                let caller_address = self.pc;
                self.pc = *address as usize;
                self.debugger
                    .push_call(self.pc, caller_address, self.cpu.get_register_values());
            }
            Instruction::JSR(source) => {
                let address = self.get_operand_address(source)?;
                //the 68000 prefetches from the target before it pushes the return address
                if address & 1 != 0 {
                    return Err(Self::jump_to_an_odd_address(address));
                }
                let sp = self.get_sp().wrapping_sub(4);
                self.set_memory_value(sp, Size::Long, self.pc as u32)?;
                if self.keep_history {
                    self.debugger.add_mutation(MutationOperation::PushCall {
                        to: address as usize,
                        from: self.current_instruction_address,
                    });
                }
                self.set_sp(sp);
                let caller_address = self.pc;
                self.pc = address as usize;
                self.debugger
                    .push_call(self.pc, caller_address, self.cpu.get_register_values());
            }
            Instruction::JMP(op) => {
                let addr = self.get_operand_address(op)?;
                if addr & 1 != 0 {
                    return Err(Self::jump_to_an_odd_address(addr));
                }
                self.pc = addr as usize;
            }
            Instruction::LEA(source, dest) => {
                let addr = self.get_operand_address(source)?;
                self.set_register_value(*dest, addr, Size::Long);
            }
            Instruction::PEA(source) => {
                let addr = self.get_operand_address(source)?;
                //the push writes below the stack pointer, a store of the program's like any other
                let sp = self.get_sp().wrapping_sub(4);
                self.set_memory_value(sp, Size::Long, addr)?;
                self.set_sp(sp);
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
                //the carry is the last bit shifted out, whatever the count: an `asr` by the size
                //or more shifts out copies of the sign, so X and C are the sign (the 68000's
                //Programmer's Reference Manual, `asl`/`asr`; the suite's ASR files)
                let carry = msb;
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
                        //in 64 bits, as `$80000000 / -1` overflows 32 and would panic: it is an
                        //overflow of the 16 bit quotient like any other
                        let dest_value = dest_value as i32 as i64;
                        let source_value = sign_extend_to_long(source_value, Size::Word) as i64;
                        let quotient = dest_value / source_value;
                        (
                            (dest_value % source_value) as u32,
                            quotient as u32,
                            quotient > i16::MAX as i64 || quotient < i16::MIN as i64,
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
                    //an overflow leaves the register as it was, and the flags as MAME's microcode
                    //leaves them on every overflow of SingleStepTests/m68000: N and V set, Z and C
                    //clear, X kept. Motorola calls N and Z undefined there
                    self.set_flag(Flags::Negative, true);
                    self.set_flag(Flags::Zero, false);
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
                let source_value = self.read_compared(source, *size, 2)?;
                let dest_value = self.read_compared(dest, *size, 0)?;
                let (result, carry) = overflowing_sub_sized(dest_value, source_value, *size);
                let overflow = has_sub_overflowed(dest_value, source_value, result, *size);
                self.set_compare_flags(result, *size, carry, overflow);
            }
            Instruction::Bcc(address, condition, _) => {
                if self.get_condition_value(condition) {
                    if *address & 1 != 0 {
                        return Err(Self::jump_to_an_odd_address(*address));
                    }
                    self.pc = *address as usize;
                }
            }
            //the 68000 reads the destination before it clears it, which is the access an odd address
            //faults on
            Instruction::CLR(dest, size) => {
                let used = self.read_before_writing(dest, *size)?;
                self.store_operand_value(dest, 0, *size, used)?;
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
                    //a branch to an odd address faults before the counter is written, as MAME's
                    //microcode leaves it on every such DBcc of SingleStepTests/m68000
                    if next != -1 && *address & 1 != 0 {
                        return Err(Self::jump_to_an_odd_address(*address));
                    }
                    self.set_register_value(*reg, next as u32, Size::Word);
                    if next != -1 {
                        self.pc = *address as usize;
                    }
                }
            }
            //the register is read before the stack pointer moves, so `link a7,#n` pushes A7 as it
            //was, as MAME's microcode does on every `link a7` of SingleStepTests/m68000
            Instruction::LINK(reg, offset) => {
                let value = self.get_register_value(*reg, Size::Long);
                let sp = self.get_sp().wrapping_sub(4);
                self.set_sp(sp);
                self.set_memory_value(sp, Size::Long, value)?;
                self.set_register_value(*reg, sp as u32, Size::Long);
                self.set_sp((sp as i32).wrapping_add(*offset as i32) as usize)
            }
            //the stack pointer goes past the frame first and the register is loaded last, so
            //`unlk a7` leaves A7 the long it popped, as on every `unlk a7` of the suite
            Instruction::UNLK(reg) => {
                let value = self.get_register_value(*reg, Size::Long);
                let (value, new_sp) = self.memory.pop(Size::Long, value as usize)?;
                self.set_sp(new_sp);
                self.set_register_value(*reg, value.get_long(), Size::Long);
            }
            Instruction::NOP => {}
            // `simhalt` pauses after the instruction and touches no register.
            // `step` advances the PC before executing it, so the next run or
            // step call resumes at the following instruction.
            Instruction::SIMHALT => self.set_status(InterpreterStatus::Paused),
            Instruction::RTS => {
                let (value, new_sp) = self.memory.pop(Size::Long, self.get_sp())?;
                if value.get_long() & 1 != 0 {
                    self.set_sp(new_sp);
                    return Err(Self::jump_to_an_odd_address(value.get_long()));
                }
                let frame = self.debugger.pop_call();
                if self.keep_history {
                    self.debugger.add_mutation(MutationOperation::PopCall {
                        to: value.get_long() as usize,
                        from: self.current_instruction_address,
                        frame,
                    })
                }
                self.set_sp(new_sp);
                self.pc = value.get_long() as usize;
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
                //`trap #0` to `#14` go through vectors 32 to 46, the program's own services
                _ => {
                    return Err(RuntimeError::UnhandledTrap {
                        vector: exception::TRAP_0 + vector,
                    })
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
                self.check_privilege()?;
                let value = self.get_operand_value(source, Size::Word, Used::Once)?;
                self.write_sr(value as u16);
            }
            //the 68000 reads the destination before it writes it, which is the access an odd
            //address faults on
            Instruction::MOVEfromSR(destination) => {
                let value = self.cpu.get_sr() as u32;
                let used = self.read_before_writing(destination, Size::Word)?;
                self.store_operand_value(destination, value, Size::Word, used)?;
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
                self.check_privilege()?;
                let word = self.cpu.get_sr() & value;
                self.write_sr(word);
            }
            Instruction::ORItoSR(value) => {
                self.check_privilege()?;
                let word = self.cpu.get_sr() | value;
                self.write_sr(word);
            }
            Instruction::EORItoSR(value) => {
                self.check_privilege()?;
                let word = self.cpu.get_sr() ^ value;
                self.write_sr(word);
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
                if address.get_long() & 1 != 0 {
                    self.cpu.set_ccr_byte(status.get_word() as u8);
                    self.set_sp(sp);
                    return Err(Self::jump_to_an_odd_address(address.get_long()));
                }
                let frame = self.debugger.pop_call();
                if self.keep_history {
                    self.debugger.add_mutation(MutationOperation::PopCall {
                        to: address.get_long() as usize,
                        from: self.current_instruction_address,
                        frame,
                    })
                }
                self.cpu.set_ccr_byte(status.get_word() as u8);
                self.set_sp(sp);
                self.pc = address.get_long() as usize;
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
                //"N - Set if the data register is less than zero, cleared if the data register is
                //greater than the higher limit", and the rest undefined. A 68000 sets them in
                //bounds or not: N from the register's sign, V and C cleared, X kept, on every
                //test of SingleStepTests/m68000's CHK file; Z is the register's being zero, as
                //Musashi has it, which none of the suite's tests reaches
                self.set_flag(Flags::Negative, value < 0);
                self.set_flag(Flags::Zero, value == 0);
                self.set_flag(Flags::Overflow, false);
                self.set_flag(Flags::Carry, false);
                if value < 0 || value > bound {
                    return Err(RuntimeError::ChkOutOfBounds { value, bound });
                }
            }
            Instruction::TRAPV => {
                if self.get_flag(Flags::Overflow) {
                    return Err(RuntimeError::OverflowException);
                }
            }
            //a word that is no instruction is an illegal instruction on a 68000, as `illegal` is
            Instruction::ILLEGAL | Instruction::IllegalWord(_) => {
                return Err(RuntimeError::IllegalInstruction);
            }
            //the status register, then the program counter, off the supervisor stack, which A7 is;
            //an S of 0 in the word popped makes A7 the user stack pointer. A return to an odd
            //address faults in `rte`'s own prefetch, with the status register restored
            Instruction::RTE => {
                self.check_privilege()?;
                let (status, sp) = self.memory.pop(Size::Word, self.get_sp())?;
                let (address, sp) = self.memory.pop(Size::Long, sp)?;
                let target = address.get_long();
                self.set_sp(sp);
                self.write_sr(status.get_word());
                if target & 1 != 0 {
                    return Err(Self::jump_to_an_odd_address(target));
                }
                let frame = self.debugger.pop_exception();
                if self.keep_history {
                    self.debugger.add_mutation(MutationOperation::PopException {
                        to: target as usize,
                        from: self.current_instruction_address,
                        frame,
                    });
                }
                self.pc = target as usize;
            }
            //privileged, and run in the plan's milestone C8, with interrupts; the Assembler refuses
            //both, so only decoded words reach these
            Instruction::STOP(_) => {
                self.check_privilege()?;
                return Err(RuntimeError::Unimplemented("`stop`".to_string()));
            }
            Instruction::RESET => {
                self.check_privilege()?;
                return Err(RuntimeError::Unimplemented("`reset`".to_string()));
            }
            //the user stack pointer, which supervisor mode keeps aside
            Instruction::MOVEUSP { to_usp, register } => {
                self.check_privilege()?;
                match to_usp {
                    true => {
                        let value = self.get_register_value(*register, Size::Long);
                        self.write_usp(value);
                    }
                    false => {
                        let value = self.cpu.get_usp();
                        self.set_register_value(*register, value, Size::Long);
                    }
                }
            }
            Instruction::LINEA(word) => return Err(RuntimeError::LineA(*word)),
            Instruction::LINEF(word) => return Err(RuntimeError::LineF(*word)),
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
        println!("USP: {:#010X} SSP: {:#010X}", self.cpu.get_usp(), self.cpu.get_ssp());
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

    /// Stores `value` at `address` for the program, journaled, watched and
    /// checked against the memory map: a store a ROM range ignores changes
    /// nothing, and is neither journaled nor watched, as it is no store.
    pub fn set_memory_value(
        &mut self,
        address: usize,
        size: Size,
        value: u32,
    ) -> RuntimeResult<()> {
        let Some(old_value) = self.memory.replace_size(address, size, value)? else {
            return Ok(());
        };
        self.watch_store(address, size, value);
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
        //a long goes low word first, so an odd address faults on the low word
        let fault = |error| match size {
            Size::Long => low_word_first(error),
            Size::Byte | Size::Word => error,
        };
        for i in (0..8).rev() {
            if (mask & 0x01) != 0 {
                let value = self.cpu.a_reg[i].get_long();
                addr = addr.wrapping_sub(size.to_bytes());
                self.set_memory_value(addr, size, value).map_err(fault)?;
            }
            mask >>= 1;
        }
        for i in (0..8).rev() {
            if (mask & 0x01) != 0 {
                addr = addr.wrapping_sub(size.to_bytes());
                self.set_memory_value(addr, size, self.cpu.d_reg[i].get_long())
                    .map_err(fault)?;
            }
            mask >>= 1;
        }
        Ok(addr as u32)
    }
    /// `movem` from memory, `program` when the operand is relative to the
    /// program counter, which reads program space.
    pub fn move_memory_to_registers(
        &mut self,
        addr: usize,
        size: Size,
        mut mask: u16,
        program: bool,
    ) -> RuntimeResult<u32> {
        let size_bytes = size.to_bytes() as u32;
        let mut addr = addr as u32;
        let read = |memory: &Memory, address: u32| match program {
            true => memory.read_program(address as usize, size),
            false => memory.read_size(address as usize, size),
        };
        for i in 0..8 {
            if (mask & 0x01) != 0 {
                let val = sign_extend_to_long(read(&self.memory, addr)?, size) as u32;
                //a word fills the whole register, sign extended, as for an address register
                self.set_register_value(RegisterOperand::Data(i), val, Size::Long);
                (addr, _) = overflowing_add_sized(addr, size_bytes, Size::Long);
            }
            mask >>= 1;
        }
        for i in 0..8 {
            if (mask & 0x01) != 0 {
                let val = sign_extend_to_long(read(&self.memory, addr)?, size) as u32;
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
    ///
    /// Code takes it like data: an instruction it covers is decoded again
    /// before it runs, and runs as its new bytes say. It is the host's write,
    /// never a [code write](CodeWrite) of the program's.
    pub fn write_memory_bytes(&mut self, address: usize, bytes: &[u8]) -> RuntimeResult<()> {
        if self.poke.is_none() {
            self.memory.write_bytes(address, bytes)?;
            self.memory.cache.take_cleared();
            self.watch_bytes(address, bytes);
            return Ok(());
        }
        let old = self.memory.read_bytes(address, bytes.len())?.to_vec();
        if old == bytes {
            return Ok(());
        }
        self.memory.write_bytes(address, bytes)?;
        self.memory.cache.take_cleared();
        self.watch_bytes(address, bytes);
        let poke = self.poke.as_mut().expect("a poke is open");
        poke.add_mutation(MutationOperation::WriteMemoryBytes {
            address,
            old: old.clone(),
            new: bytes.to_vec(),
        });
        poke.add_memory_target(address, old);
        Ok(())
    }

    /// Writes a run of bytes on behalf of the program, a trap task's answer,
    /// journaled into the step being executed.
    pub fn set_memory_bytes(&mut self, address: usize, bytes: &[u8]) -> RuntimeResult<()> {
        if self.keep_history {
            let old_bytes = self.memory.read_bytes(address, bytes.len())?;
            self.debugger
                .add_mutation(MutationOperation::WriteMemoryBytes {
                    address,
                    old: old_bytes.to_vec(),
                    new: bytes.to_vec(),
                });
        }
        self.memory.write_bytes(address, bytes)?;
        self.watch_bytes(address, bytes);
        Ok(())
    }
    /// The instruction the Program laid out where the program counter is, as
    /// [`get_instruction_at`](Interpreter::get_instruction_at) reads it: the one
    /// the next step will run, unless its bytes have changed since.
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
            let byte = self.memory.byte_at(address.wrapping_add(offset));
            if byte == 0x00 {
                break;
            }
            bytes.push(byte);
        }
        Ok(charset::decode(&bytes))
    }

    /// The NUL terminated string at `address`, decoded from Windows-1252: what
    /// tasks 13, 14, 17, 18 and 95 take at (A1), checked against the memory map
    /// as EASy68K checks it ([`check_string`](Interpreter::check_string)).
    ///
    /// Every byte decodes to a character, so no string is refused for what it
    /// holds. EASy68K reads a string for as long as it goes; this one stops at
    /// [`STRING_LIMIT`] bytes with no NUL, which is a missing terminator rather
    /// than a string anybody wrote, and is an argument error of `task`.
    fn read_null_terminated_string(&self, address: usize, task: u8) -> RuntimeResult<String> {
        self.check_string(address)?;
        let mut bytes = Vec::new();
        loop {
            let byte = self.memory.byte_at(address.wrapping_add(bytes.len()));
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

    /// The path at `address` that a file or sound task names, checked against
    /// the memory map as EASy68K checks it ([`check_string`](Interpreter::check_string)):
    /// every task's path but the dialog's of task 58, which EASy68K does not
    /// check.
    fn read_checked_path(&self, address: u32) -> RuntimeResult<String> {
        self.check_string(address as usize)?;
        self.read_path(address)
    }

    /// EASy68K's check of a string a `trap #15` task reads at `address`,
    /// `memoryMapCheck(Invalid, A1, strlen(A1))` in its `CODE9.CPP`: its
    /// characters up to the NUL, at most [`STRING_LIMIT`] of them, against an
    /// invalid range. Protected and read only ranges are not checked, as
    /// EASy68K checks neither, and with no map nothing is read.
    fn check_string(&self, address: usize) -> RuntimeResult<()> {
        if !self.memory.mapped {
            return Ok(());
        }
        let length = (0..STRING_LIMIT)
            .take_while(|offset| self.memory.byte_at(address.wrapping_add(*offset)) != 0)
            .count();
        self.check_buffer(Kinds::INVALID, address, length, Access::Read)
            .map(|_| ())
    }

    /// EASy68K's check of the `length` bytes at `address` a `trap #15` task
    /// reads or writes, against `kinds` (`memoryMapCheck` in its `CODE9.CPP`):
    /// whether the task goes ahead, `false` when a ROM range takes the bytes
    /// and the task does nothing, or the bus error, for an `access` that
    /// starts at `address` as EASy68K reports it.
    fn check_buffer(
        &self,
        kinds: Kinds,
        address: usize,
        length: usize,
        access: Access,
    ) -> RuntimeResult<bool> {
        match self.memory.check_buffer(kinds, address, length) {
            Verdict::Allowed => Ok(true),
            Verdict::Ignored => Ok(false),
            Verdict::Refused(range) => Err(Memory::bus_error(address, Size::Byte, access, range)),
        }
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
            //task 32 with D1.B of 5 turns EASy68K's "Enable exceptions" on; s68k always takes
            //them (ADR 0008), so it is accepted and changes nothing. Its other parts are the
            //hardware window's, and stay refused
            32 if self.cpu.d_reg[1].get_byte() == 5 => Ok(None),
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
            51 => Some(Interrupt::OpenFile(self.read_checked_path(address)?)),
            52 => Some(Interrupt::NewFile(self.read_checked_path(address)?)),
            53 | 54 => {
                let count = self.cpu.d_reg[2].get_long();
                //EASy68K checks the bytes against the memory map once they fit in memory, before
                //it looks at the file: those task 53 reads a file into against invalid, read only
                //and ROM, which takes nothing and leaves D0 as it is, and those task 54 writes a
                //file from against invalid
                if self.memory.mapped && fits_in_memory(address, count) {
                    let (kinds, access) = match task {
                        53 => (Kinds::INVALID_READ_ROM, Access::Write),
                        _ => (Kinds::INVALID, Access::Read),
                    };
                    if !self.check_buffer(kinds, address as usize, count as usize, access)? {
                        return Ok(None);
                    }
                }
                match handle {
                    Some(handle) if count > 0 && fits_in_memory(address, count) => {
                        Some(if task == 53 {
                            Interrupt::ReadFile { handle, count }
                        } else {
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
            57 => Some(Interrupt::DeleteFile(self.read_checked_path(address)?)),
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
            59 => Some(Interrupt::FileExists(self.read_checked_path(address)?)),
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
                //EASy68K checks the whole string, to its NUL, whatever D1.W says
                self.check_string(address)?;
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
                self.read_checked_path(self.cpu.a_reg[1].get_long())?,
            )),
            71 => Ok(Interrupt::LoadSound {
                path: self.read_checked_path(self.cpu.a_reg[1].get_long())?,
                index: self.cpu.d_reg[1].get_byte(),
            }),
            72 => Ok(Interrupt::PlayLoadedSound(self.cpu.d_reg[1].get_byte())),
            73 => Ok(Interrupt::PlaySoundDirectX(
                self.read_checked_path(self.cpu.a_reg[1].get_long())?,
            )),
            74 => Ok(Interrupt::LoadSoundDirectX {
                path: self.read_checked_path(self.cpu.a_reg[1].get_long())?,
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
    /// Raises a privilege violation in user mode, for an instruction only
    /// supervisor mode runs.
    #[inline(always)]
    fn check_privilege(&self) -> RuntimeResult<()> {
        match self.cpu.is_supervisor() {
            true => Ok(()),
            false => Err(RuntimeError::PrivilegeViolation),
        }
    }

    /// Reads `destination` and throws the value away, when it is in memory, for
    /// the instructions the 68000 reads a destination before it writes it,
    /// `clr` and `move sr,<ea>`: the read is the access an odd address faults
    /// on, and the read moves a predecrement. Answers how the write that
    /// follows uses the operand.
    fn read_before_writing(&mut self, destination: &Operand, size: Size) -> RuntimeResult<Used> {
        match destination {
            Operand::Register(_) => Ok(Used::Once),
            _ => {
                self.get_operand_value(destination, size, Used::Twice)?;
                Ok(Used::Twice)
            }
        }
    }

    /// The two values `addx` and `subx` read, the source first. A long read
    /// through `-(An)` that faults leaves its register where it was, as MAME's
    /// microcode does for these two, which decrement by a word at a time.
    fn read_extended_pair(
        &mut self,
        source: &Operand,
        destination: &Operand,
        size: Size,
    ) -> RuntimeResult<(u32, u32)> {
        let source_value = self.read_extended(source, size, Used::Once)?;
        let destination_value = self.read_extended(destination, size, Used::Twice)?;
        Ok((source_value, destination_value))
    }

    fn read_extended(&mut self, operand: &Operand, size: Size, used: Used) -> RuntimeResult<u32> {
        match (operand, size) {
            (Operand::PreIndirect(register), Size::Long) => {
                let before = self.get_a_reg_sized(*register, Size::Long);
                self.get_operand_value(operand, size, used)
                    .map_err(|error| {
                        self.set_a_reg_sized(*register, before, Size::Long);
                        low_word_first(error)
                    })
            }
            _ => self.get_operand_value(operand, size, used),
        }
    }

    /// The fault of a `move` that read its source and could not write it,
    /// the flags still as they were: what MAME's microcode has set by then,
    /// as it sets them as it goes.
    ///
    /// A long written through `(An)` or `(An)+` faults with the flags as they
    /// were when it comes from a register or an immediate, and with N and Z set
    /// from its low word when it comes from memory, as it does to an absolute
    /// long address. Through a displacement or an index, one from a register
    /// or an immediate faults with N and Z set and V and C as they were. Every
    /// other `move` has set all four from the value.
    #[cold]
    #[inline(never)]
    fn move_write_fault(
        &mut self,
        error: RuntimeError,
        source: &Operand,
        destination: &Operand,
        size: Size,
        value: u32,
    ) -> RuntimeError {
        let from_memory = !matches!(source, Operand::Register(_) | Operand::Immediate(_));
        match (size, destination) {
            (
                Size::Long,
                Operand::Indirect(_)
                | Operand::PostIndirect(_)
                | Operand::Absolute {
                    width: AddressWidth::Long,
                    ..
                },
            ) if from_memory => self.set_logic_flags(value & 0xFFFF, Size::Word),
            (Size::Long, Operand::Indirect(_) | Operand::PostIndirect(_)) => {}
            (Size::Long, Operand::IndirectDisplacement { .. } | Operand::IndirectIndex { .. })
                if !from_memory =>
            {
                let kept = self.cpu.ccr & (Flags::Overflow | Flags::Carry);
                self.set_logic_flags(value, size);
                self.cpu.ccr |= kept;
            }
            _ => self.set_logic_flags(value, size),
        }
        error
    }

    /// One of the two reads of `cmpm`, through `(An)+`: a fault leaves the
    /// register moved by `moved_on_fault`, a word for the source and nothing
    /// for the destination, whatever the size, as MAME's microcode leaves them.
    fn read_compared(
        &mut self,
        operand: &Operand,
        size: Size,
        moved_on_fault: u32,
    ) -> RuntimeResult<u32> {
        let Operand::PostIndirect(register) = operand else {
            return self.get_operand_value(operand, size, Used::Once);
        };
        let before = self.get_a_reg_sized(*register, Size::Long);
        self.get_operand_value(operand, size, Used::Once)
            .inspect_err(|_| {
                let after = before.wrapping_add(moved_on_fault);
                if self.get_a_reg_sized(*register, Size::Long) != after {
                    self.set_a_reg_sized(*register, after, Size::Long);
                }
            })
    }

    /// A read through `(An)+` that faulted at `address`: the register has moved
    /// past a word, and not past a long, as MAME's microcode leaves it.
    #[cold]
    #[inline(never)]
    fn read_fault_after_postincrement(
        &mut self,
        error: RuntimeError,
        register: u8,
        address: u32,
        size: Size,
    ) -> RuntimeError {
        let after = match size {
            Size::Long => address,
            Size::Byte | Size::Word => address.wrapping_add(address_step(register, size)),
        };
        if self.get_a_reg_sized(register, Size::Long) != after {
            self.set_a_reg_sized(register, after, Size::Long);
        }
        error
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
            Operand::Absolute { address, .. } => Ok(self.memory.read_size(*address, size)?),

            Operand::Indirect(reg) => {
                let address = self.get_a_reg_sized(*reg, Size::Long);
                Ok(self.memory.read_size(address as usize, size)?)
            }
            Operand::PreIndirect(op) => {
                let address = self.get_a_reg_sized(*op, Size::Long);
                let address = (address).wrapping_sub(address_step(*op, size));
                //in this case the read should always decrement the address
                self.set_a_reg_sized(*op, address, Size::Long);
                Ok(self.memory.read_size(address as usize, size)?)
            }
            Operand::PostIndirect(op) => {
                let address = self.get_a_reg_sized(*op, Size::Long);
                if used != Used::Twice {
                    //if the value is used twice, give precedence of increment to the setter
                    let new_address = address.wrapping_add(address_step(*op, size));
                    self.set_a_reg_sized(*op, new_address, Size::Long);
                }
                match self.memory.read_size(address as usize, size) {
                    Ok(value) => Ok(value),
                    Err(error) => {
                        Err(self.read_fault_after_postincrement(error, *op, address, size))
                    }
                }
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
            //relative to the program counter is in program space
            Operand::PcDisplacement { target } => {
                let address = self.pc_relative_address(*target, None);
                Ok(self.memory.read_program(address as usize, size)?)
            }
            Operand::PcIndex { target, index } => {
                let address = self.pc_relative_address(*target, Some(*index));
                Ok(self.memory.read_program(address as usize, size)?)
            }
        }
    }
    /// The address a PC-relative Operand names.
    ///
    /// The Operand keeps the address its displacement reaches, worked out from
    /// the address of its extension word when it was assembled or decoded, so
    /// this is that address, plus the index register when there is one. It
    /// does not read the program counter.
    fn pc_relative_address(&self, target: u32, index: Option<IndexRegister>) -> u32 {
        match index {
            None => target,
            Some(index) => {
                let value = self.get_register_value(index.register, index.size);
                target.wrapping_add(sign_extend_to_long(value, index.size) as u32)
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
            Operand::Absolute { address, .. } => Ok(*address as u32),
            Operand::PcDisplacement { target } => Ok(self.pc_relative_address(*target, None)),
            Operand::PcIndex { target, index } => {
                Ok(self.pc_relative_address(*target, Some(*index)))
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
            Operand::Absolute { address, .. } => Ok(self.set_memory_value(*address, size, value)?),
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
                    let a = (a).wrapping_sub(address_step(*op, size));
                    self.set_a_reg_sized(*op, a, Size::Long);
                    a
                };
                match self.set_memory_value(address as usize, size, value) {
                    Ok(()) => Ok(()),
                    //a long written through -(An) at an odd address faults before the register
                    //moves, as MAME's microcode leaves it; a word after
                    Err(error) if size == Size::Long && used == Used::Once => {
                        let before = address.wrapping_add(address_step(*op, size));
                        self.set_a_reg_sized(*op, before, Size::Long);
                        Err(low_word_first(error))
                    }
                    Err(error) => Err(error),
                }
            }
            Operand::PostIndirect(op) => {
                let address = self.get_a_reg_sized(*op, Size::Long);
                let new_address = (address).wrapping_add(address_step(*op, size));
                //give priority to increment to the setter
                self.set_a_reg_sized(*op, new_address, Size::Long);
                match self.set_memory_value(address as usize, size, value) {
                    Ok(()) => Ok(()),
                    //a write through (An)+ at an odd address faults before the register moves
                    Err(error) => {
                        self.set_a_reg_sized(*op, address, Size::Long);
                        Err(error)
                    }
                }
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
    /// an instruction stores into the watched range, or the Program
    /// terminates. Calling it while paused, or after a watched write, resumes
    /// the Program.
    ///
    /// It never stops for a [code write](CodeWrite); a run that should is
    /// [`run_with_breakpoints`](Interpreter::run_with_breakpoints).
    pub fn run(&mut self) -> RuntimeResult<InterpreterStatus> {
        self.verify_can_run()?;
        if self.is_resumable_stop() {
            self.run_step()?;
        }
        //the step after the last instruction is the one that ends the program
        while self.status == InterpreterStatus::Running {
            self.run_step()?;
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

    /// Runs until a breakpoint, `simhalt`, the end of the program, an
    /// interrupt, a store into the watched range, a write into code when
    /// `options` asks for it, or `limit` instructions. Calling it while
    /// paused, after a watched write or after a code write, resumes the
    /// Program.
    ///
    /// With [`stop_on_code_write`](RunOptions::stop_on_code_write), the run
    /// stops after an instruction that wrote into code, with
    /// [`InterpreterStatus::CodeWrite`]; the next run goes on and the
    /// overwritten instruction runs as its new bytes decode.
    ///
    /// [`skip_breakpoint_at_pc`](RunOptions::skip_breakpoint_at_pc) decides
    /// what a breakpoint on the instruction the program counter is *already* on
    /// does. `true` runs it anyway, which is
    /// what makes "continue" from a breakpoint move; `false` stops before it,
    /// having run nothing, which is what a caller that has just arrived here
    /// some other way wants. A caller that answers an interrupt and runs on has
    /// to pass `false`, or the instruction after every interrupt is unbreakable:
    /// each call resumes mid-program with the program counter on the next
    /// instruction, and that one has not run yet. So does a caller that runs on
    /// after a [`WatchedWrite`](InterpreterStatus::WatchedWrite) stop.
    ///
    /// A stop made by the last of the `limit` instructions — an interrupt, a
    /// pause, a watched write, a code write or the end of the program — is
    /// answered as that status rather than as the
    /// [`ExecutionLimit`](RuntimeError::ExecutionLimit), so that the host
    /// answers the interrupt or sees the stop before anything else runs. The
    /// limit is the answer only when the program would run on.
    ///
    /// Only the instruction the run starts on is affected; every breakpoint the
    /// run reaches after that stops it before the instruction executes.
    pub fn run_with_breakpoints(
        &mut self,
        breakpoints: &[Breakpoint],
        limit: Option<usize>,
        options: RunOptions,
    ) -> RuntimeResult<InterpreterStatus> {
        self.stop_on_code_write = options.stop_on_code_write;
        let ran = self.run_to_breakpoints(breakpoints, limit, options.skip_breakpoint_at_pc);
        self.stop_on_code_write = false;
        ran
    }

    fn run_to_breakpoints(
        &mut self,
        breakpoints: &[Breakpoint],
        limit: Option<usize>,
        skip_breakpoint_at_pc: bool,
    ) -> RuntimeResult<InterpreterStatus> {
        self.verify_can_run()?;
        if self.is_resumable_stop() && self.ends_at_pc() {
            self.terminate(Termination::EndOfProgram {
                address: self.pc & ADDRESS_MASK,
            });
            return Ok(self.status);
        }
        let resuming_after_pause = self.is_resumable_stop();
        let addresses = self.get_breakpoint_addresses(breakpoints);
        let mut iterations = 0;
        let limit = limit.unwrap_or(usize::MAX);
        let mut limit_counter = limit;
        while (self.status == InterpreterStatus::Running
            || (resuming_after_pause && iterations == 0))
            && limit_counter > 0
        {
            let at_starting_pc = iterations == 0;
            if !(at_starting_pc && skip_breakpoint_at_pc)
                && addresses.contains(&(self.pc & ADDRESS_MASK))
            {
                //a run that stops on a breakpoint is running, not paused: the
                //status a `simhalt` left behind belongs to the halt, not to the
                //breakpoint the resumed run stopped at
                self.status = InterpreterStatus::Running;
                break;
            }
            self.run_step()?;
            limit_counter -= 1;
            iterations += 1;
        }
        //the end the last step reached, which no next step's fetch has found
        if iterations > 0 {
            self.end_if_reached();
        }
        //a stop the last instruction the limit allows made is reported, not the limit: the
        //host has an interrupt to answer, a pause or a watched write to see, or an end
        let stopped = iterations > 0 && self.status != InterpreterStatus::Running;
        if limit_counter == 0 && !stopped {
            return Err(RuntimeError::ExecutionLimit(limit));
        }
        Ok(self.status)
    }

    /// Runs until `simhalt`, the end of the program, an interrupt, a store into
    /// the watched range or `limit` instructions, which answers the
    /// [`ExecutionLimit`](RuntimeError::ExecutionLimit) only when the last of
    /// them stopped nothing, as [`run_with_breakpoints`](Interpreter::run_with_breakpoints)
    /// does. It never stops for a code write; `run_with_breakpoints` with no
    /// breakpoint and [`RunOptions::stopping_on_code_writes`] is the limited run
    /// that does.
    pub fn run_with_limit(&mut self, limit: usize) -> RuntimeResult<InterpreterStatus> {
        let mut limit_counter = limit;
        self.verify_can_run()?;
        if self.is_resumable_stop() && self.ends_at_pc() {
            self.terminate(Termination::EndOfProgram {
                address: self.pc & ADDRESS_MASK,
            });
            return Ok(self.status);
        }
        let resuming_after_pause = self.is_resumable_stop();
        let mut iterations = 0;
        while (self.status == InterpreterStatus::Running
            || (resuming_after_pause && iterations == 0))
            && limit_counter > 0
        {
            self.run_step()?;
            limit_counter -= 1;
            iterations += 1;
        }
        //the end the last step reached, which no next step's fetch has found
        if iterations > 0 {
            self.end_if_reached();
        }
        //a stop the last instruction the limit allows made is reported, not the limit: the
        //host has an interrupt to answer, a pause or a watched write to see, or an end
        let stopped = iterations > 0 && self.status != InterpreterStatus::Running;
        if limit_counter == 0 && !stopped {
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
/// An [`AssembledInstruction`] as JavaScript reads it, the `InstructionLine` of
/// `src/ts_types.rs`: its own fields, camelCase, and what it
/// [assembled as](Program::assembled_as), worked out when it is asked for.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct InstructionLine<'a> {
    address: usize,
    size: usize,
    bytes: &'a [u8],
    instruction: &'a Instruction,
    location: &'a Location,
    include_chain: &'a [Location],
    source: &'a str,
    assembled_as: Option<String>,
}

impl Interpreter {
    /// `instruction` as an `InstructionLine`, or `null`.
    fn instruction_line(&self, instruction: Option<&AssembledInstruction>) -> JsValue {
        match instruction {
            Some(instruction) => js(&InstructionLine {
                address: instruction.address,
                size: instruction.size,
                bytes: &instruction.bytes,
                instruction: &instruction.instruction,
                location: &instruction.location,
                include_chain: &instruction.include_chain,
                source: &instruction.source,
                assembled_as: self.get_assembled_as(instruction),
            }),
            None => JsValue::NULL,
        }
    }
}

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
    /// The instruction the Program laid out at `address`, as the
    /// `InstructionLine` of `src/ts_types.rs` with its `assembledAs`, or `null`.
    pub fn wasm_get_instruction_at(&self, address: usize) -> JsValue {
        self.instruction_line(self.get_instruction_at(address))
    }
    /// What memory holds at `address` now, `{address, size, bytes, text}`:
    /// [`disassemble_at`](Interpreter::disassemble_at).
    pub fn wasm_disassemble_at(&self, address: usize) -> JsValue {
        js(&self.disassemble_at(address))
    }
    /// The [code write](CodeWrite) of the instruction that has just run,
    /// `{address, instructionIndex}`, or `null`.
    pub fn wasm_get_code_write(&self) -> JsValue {
        match self.get_code_write() {
            Some(write) => js(&write),
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
    /// behaviour described on [`Interpreter::run_with_breakpoints`], and
    /// `stop_on_code_write` to `false`.
    pub fn wasm_run_with_breakpoints(
        &mut self,
        breakpoints: JsValue,
        limit: Option<usize>,
        skip_breakpoint_at_pc: Option<bool>,
        stop_on_code_write: Option<bool>,
    ) -> Result<InterpreterStatus, JsValue> {
        let breakpoints: Vec<Breakpoint> =
            serde_wasm_bindgen::from_value(breakpoints).map_err(|e| {
                js(&RuntimeError::InvalidArgument(format!(
                    "breakpoints are an array of {{file, line}}: {}",
                    e
                )))
            })?;
        let options = RunOptions {
            skip_breakpoint_at_pc: skip_breakpoint_at_pc.unwrap_or(true),
            stop_on_code_write: stop_on_code_write.unwrap_or(false),
        };
        self.run_with_breakpoints(&breakpoints, limit, options)
            .map_err(|e| js(&e))
    }
    pub fn wasm_get_call_stack(&self) -> JsValue {
        js(&self.get_pretty_call_stack())
    }
    pub fn wasm_run_with_limit(&mut self, limit: usize) -> Result<InterpreterStatus, JsValue> {
        self.run_with_limit(limit).map_err(|e| js(&e))
    }
    pub fn wasm_get_next_instruction(&self) -> JsValue {
        self.instruction_line(self.get_next_instruction())
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
        self.get_last_steps(count)
            .into_iter()
            .map(|step| self.step_to_js(step))
            .collect::<js_sys::Array>()
            .into()
    }

    pub fn wasm_get_last_step_id(&self) -> f64 {
        self.get_last_step_id() as f64
    }
    /// [`get_executed_instructions`](Interpreter::get_executed_instructions),
    /// as a number, exact up to 2^53.
    pub fn wasm_get_executed_instructions(&self) -> f64 {
        self.get_executed_instructions() as f64
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
    /// The whole status register, `$2000` before a program has run.
    ///
    /// Its low byte is the condition codes as the processor numbers them
    /// (extend 16, negative 8, zero 4, overflow 2, carry 1), which is **not**
    /// the bitfield [`Interpreter::wasm_get_flags_as_number`] answers: that one
    /// is this crate's own and the editor has always read it.
    pub fn wasm_get_sr(&self) -> u16 {
        self.get_sr()
    }
    /// The System register file, `{sr, usp, ssp}`:
    /// [`get_system`](Interpreter::get_system).
    pub fn wasm_get_system(&self) -> JsValue {
        js(&self.get_system())
    }
    /// Writes the status register, into the open Poke if there is one:
    /// [`set_sr`](Interpreter::set_sr).
    pub fn wasm_set_sr(&mut self, value: u16) {
        self.set_sr(value)
    }
    /// Writes the user stack pointer, into the open Poke if there is one.
    pub fn wasm_set_usp(&mut self, value: u32) {
        self.set_usp(value)
    }
    /// Writes the supervisor stack pointer, into the open Poke if there is
    /// one.
    pub fn wasm_set_ssp(&mut self, value: u32) {
        self.set_ssp(value)
    }
    pub fn wasm_undo(&mut self) -> Result<JsValue, JsValue> {
        self.undo()
            .map(|step| self.step_to_js(&step))
            .map_err(|e| js(&e))
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
    /// Watches `start` up to `end`, `end` not included, which is
    /// `watchMemory(start, end)`. It throws an `InvalidArgument` for an empty
    /// range and for one past the end of memory.
    pub fn wasm_watch_memory(&mut self, start: usize, end: usize) -> Result<(), JsValue> {
        self.watch_memory(start, end).map_err(|e| js(&e))
    }
    pub fn wasm_unwatch_memory(&mut self) {
        self.unwatch_memory()
    }
    /// The watched writes not taken yet, three numbers a write — its address,
    /// its size in bytes and the value stored — in one flat `Uint32Array`, which
    /// crosses the boundary as one copy; the log is emptied.
    pub fn wasm_take_watched_writes(&mut self) -> Vec<u32> {
        let mut flat = Vec::with_capacity(self.watched_writes.len() * 3);
        for write in self.watched_writes.drain(..) {
            flat.extend([write.address as u32, write.size as u32, write.value]);
        }
        flat
    }
    pub fn wasm_get_last_step(&self) -> JsValue {
        match self.debugger.get_last_step() {
            Some(step) => self.step_to_js(step),
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

/// Memory that starts as zeros, for the conformance harness: the
/// SingleStepTests/m68000 files list only the words a test reads or writes,
/// and read every other word as zero (`src/conformance/skips.rs`).
#[cfg(any(test, feature = "conformance"))]
impl Memory {
    /// The 16 MB, every byte zero, and an empty decode cache.
    ///
    /// The zeros come from the allocator, which maps fresh pages rather than
    /// writing them, so a harness that makes one of these per test pays only
    /// for the pages the test touches.
    pub fn zeroed() -> Self {
        Self {
            data: vec![0; MEMORY_SIZE],
            ..Self::new()
        }
    }

    /// Writes `word` at `address`, high byte first, each byte's address
    /// reduced to the 24 bits of the bus: how a test's RAM is laid down, with
    /// no address error for an odd address and no wrap refused at the top.
    pub fn put_word(&mut self, address: usize, word: u16) {
        let [high, low] = word.to_be_bytes();
        self.data[address & ADDRESS_MASK] = high;
        self.data[(address + 1) & ADDRESS_MASK] = low;
        self.cache.invalidate(address & ADDRESS_MASK, 2);
        self.cache.take_cleared();
    }
}

#[cfg(any(test, feature = "conformance"))]
impl Cpu {
    /// The registers of a test's state: the data registers, A0 to A6, the two
    /// stack pointers, A7 being the one of the mode the status register is
    /// in, and the status register, reduced to [`SR_IMPLEMENTED_BITS`].
    pub fn from_state(data: [u32; 8], address: [u32; 7], usp: u32, ssp: u32, sr: u16) -> Self {
        let mut cpu = Cpu::new();
        for (register, value) in cpu.d_reg.iter_mut().zip(data) {
            register.store_long(value);
        }
        for (register, value) in cpu.a_reg.iter_mut().zip(address) {
            register.store_long(value);
        }
        cpu.restore_sr(sr);
        cpu.restore_stack_pointers(StackPointers { usp, ssp });
        cpu
    }

    /// The registers of a test's state with A7 given as the stack pointer of
    /// the mode the status register is in, and the other stack pointer as a
    /// program starts with it.
    pub fn from_registers(data: [u32; 8], address: [u32; 8], sr: u16) -> Self {
        let (usp, ssp) = match sr & SUPERVISOR_BIT != 0 {
            true => (INITIAL_USER_STACK_POINTER, address[7]),
            false => (address[7], INITIAL_SUPERVISOR_STACK_POINTER),
        };
        let mut low = [0; 7];
        low.copy_from_slice(&address[..7]);
        Cpu::from_state(data, low, usp, ssp, sr)
    }
}

/// The conformance harness's two ways in (`src/conformance`, ADR 0010): an
/// Interpreter made from a machine state, to run one instruction of a
/// single-step test, and one made from a raw image, to run one of Musashi's
/// self-checking programs. Neither is for the editor, and both are behind the
/// `conformance` feature.
#[cfg(any(test, feature = "conformance"))]
impl Interpreter {
    /// An Interpreter ready to run the one instruction at `pc` from a state a
    /// single-step test gives: the registers in `cpu`, the RAM it lists as
    /// `(address, word)` pairs in `memory_words`, and the processor's two
    /// prefetched words, which are written at `pc` and `pc + 2` after the RAM
    /// (a 68000 runs what it prefetched, and an Interpreter runs memory).
    ///
    /// Memory is [zeroed](Memory::zeroed) apart from those words. The decode
    /// cache starts empty, so the instruction is decoded from memory when it is
    /// fetched, and the Program is an empty one whose Entry point is `pc`,
    /// which places nothing: the end rule of ADR 0009 holds as it does for
    /// any memory the program never placed. History is kept, a step deep, so
    /// that the step's writes can be read back from its mutations.
    ///
    /// The Interpreter takes no trace exception, whatever T says: the suite
    /// records the state a step leaves before trace is taken (its issue #2,
    /// a relaxation of `src/conformance/skips.rs`).
    pub fn from_state(
        cpu: Cpu,
        pc: usize,
        memory_words: &[(usize, u16)],
        prefetch: [u16; 2],
    ) -> Interpreter {
        let mut memory = Memory::zeroed();
        for &(address, word) in memory_words {
            memory.put_word(address, word);
        }
        memory.put_word(pc, prefetch[0]);
        memory.put_word(pc + 2, prefetch[1]);
        let program = Program::new(Vec::new(), Vec::new(), &SymbolTable::new(), pc);
        let options = InterpreterOptions {
            keep_history: true,
            history_size: 1,
        };
        let mut interpreter = Self::with_memory(program, memory, Some(options));
        interpreter.cpu = cpu;
        interpreter.memory.set_supervisor(cpu.is_supervisor());
        interpreter.stack_top = cpu.a_reg[7].get_long() as usize;
        interpreter.takes_trace = false;
        interpreter
    }

    /// An Interpreter about to run a raw image, a [`Program::from_image`],
    /// from its Entry point with A7 at `stack_pointer`: memory is zeros, as in
    /// Musashi's test driver, with the image written over them. The decode
    /// cache starts empty, since an image holds no assembled instruction, and
    /// the program has not ended before it starts, whatever it holds.
    pub fn from_image(
        program: Program,
        stack_pointer: u32,
        options: Option<InterpreterOptions>,
    ) -> Interpreter {
        let mut memory = Memory::zeroed();
        if let Err(e) = prepare_memory(&mut memory, &program) {
            panic!("the image does not fit in memory: {:?}", e);
        }
        memory.cache.take_cleared();
        let mut interpreter = Self::with_memory(program, memory, options);
        interpreter.cpu.a_reg[7].store_long(stack_pointer);
        interpreter.stack_top = stack_pointer as usize;
        interpreter
    }
}
