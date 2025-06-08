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
use std::{collections::HashMap, hash::Hash};

use bitflags::bitflags;
use serde::{Deserialize, Serialize};
use wasm_bindgen::{prelude::wasm_bindgen, JsValue};

use crate::debugger::PrettyStackFrame;
use crate::instructions::TargetDirection;
use crate::{
    compiler::{Compiler, Directive, InstructionLine},
    debugger::{Debugger, ExecutionStep, MutationOperation},
    instructions::{
        Condition, Instruction, Interrupt, InterruptResult, Operand, RegisterOperand,
        ShiftDirection, Sign, Size,
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

#[derive(Debug, Clone, Copy)]
#[wasm_bindgen]
pub struct Cpu {
    d_reg: [Register; 8],
    a_reg: [Register; 8],
    ccr: Flags,
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
            ccr: Flags::new(),
        }
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
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", content = "value")]
pub enum RuntimeError {
    Raw(String),
    ExecutionLimit(usize),
    OutOfBounds(String),
    AddressError {
        address: usize,
        size: Size
    },
    DivisionByZero,
    IncorrectAddressingMode(String),
    Unimplemented,
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

#[wasm_bindgen]
pub struct Interpreter {
    memory: Memory,
    cpu: Cpu,
    pc: usize,
    program: Vec<InstructionLine>,
    //i could store this in the memory instead of having a separate vector but it prevents accidental memory writes overriding the instruction map
    //even tho this is more similar to how a real cpu works
    instruction_map: Vec<usize>,
    debugger: Debugger,
    keep_history: bool,
    last_line_address: usize,
    final_instruction_address: usize,
    current_interrupt: Option<Interrupt>,
    status: InterpreterStatus,
}

impl Interpreter {
    pub fn new(compiled_program: Compiler, options: Option<InterpreterOptions>) -> Self {
        let sp = 0x01000000;
        let start = compiled_program.get_start_address();
        let end = compiled_program.get_final_instruction_address();
        let program = compiled_program.get_instructions().clone();
        let length = program.len();
        let options = options.unwrap_or(InterpreterOptions {
            keep_history: false,
            history_size: 100,
        });
        let max_address = program.iter().map(|i| i.address).max().unwrap_or(0);
        let mut instruction_map = vec![usize::MAX; max_address + 1];
        for (index, ins) in program.iter().enumerate() {
            //no need to check if the array is big enough because i already checked the max address
            instruction_map[ins.address] = index;
        }
        let mut interpreter = Self {
            memory: Memory::new(),
            instruction_map,
            cpu: Cpu::new(),
            pc: start,
            final_instruction_address: end,
            program,
            keep_history: options.keep_history,
            last_line_address: 0,
            debugger: Debugger::new(options.history_size, compiled_program.get_labels_map()),
            current_interrupt: None,
            status: if start <= end && length > 0 {
                InterpreterStatus::Running
            } else {
                InterpreterStatus::Terminated
            },
        };
        interpreter.cpu.a_reg[7].store_long(sp as u32);
        match interpreter.prepare_memory(compiled_program.get_directives()) {
            Ok(_) => interpreter,
            Err(e) => panic!("Error preparing memory: {:?}", e),
        }
    }

    //TODO could make this an external function and pass the memory in
    fn prepare_memory(&mut self, directives: &Vec<Directive>) -> RuntimeResult<()> {
        for directive in directives {
            match &directive {
                Directive::DC { data, address }
                | Directive::DS { data, address }
                | Directive::DCB { data, address } => {
                    self.memory.write_bytes(*address, data)?;
                }
                Directive::Other => {}
            };
        }
        Ok(())
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
        match self.status {
            InterpreterStatus::Terminated | InterpreterStatus::TerminatedWithException => {
                panic!("Cannot change status of terminated program")
            }
            _ => self.status = status,
        }
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

    #[inline(always)]
    pub fn has_reached_bottom(&self) -> bool {
        self.pc > self.final_instruction_address
    }

    pub fn step(&mut self) -> RuntimeResult<InterpreterStatus> {
        if self.keep_history {
            self.debugger
                .add_step(ExecutionStep::new(self.pc, self.cpu.ccr));
        }
        self.last_line_address = self.pc;
        let instruction = self
            .get_instruction_at(self.pc)
            .map(|i| (i.parsed_line.line_index, i.instruction));
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

            Some((index, ins)) => {
                if self.keep_history {
                    self.debugger.set_line(index);
                }
                self.increment_pc(4);
                self.execute_instruction(&ins)?;
                let status = self.get_status();
                //TODO not sure if doing this before or after running the instruction
                if self.has_reached_bottom() && *status != InterpreterStatus::Interrupt {
                    self.set_status(InterpreterStatus::Terminated);
                }
                if self.keep_history {
                    self.debugger.set_new_ccr(self.cpu.ccr);
                }
                Ok(self.status)
            }
            None if self.pc < self.final_instruction_address => {
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
                self.cpu.ccr = step.get_ccr();
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
                            //try to get the address of the function that popped the call
                            let ins = self.get_instruction_at(to.wrapping_sub(4));
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
            //| InterruptResult::SetDrawingMode
            | InterruptResult::SetPenWidth
            //| InterruptResult::Repaint
            | InterruptResult::DrawText
            //| InterruptResult::GetPenPosition(_)
            | InterruptResult::SetScreenSize
            | InterruptResult::ClearScreen

            => {}
            InterruptResult::ReadKeyboardString(str) => {
                if str.len() > 80 {
                    //TODO should i error or truncate?
                    return Err(RuntimeError::Raw(
                        "String is longer than 80 chars".to_string(),
                    ));
                }
                let address = self.cpu.a_reg[0].get_long() as usize;
                self.set_memory_bytes(address, str.as_bytes())?;
                self.set_register_value(RegisterOperand::Data(1), str.len() as u32, Size::Word);
                //self.cpu.d_reg[1].store_word(str.len() as u16);
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
    #[inline(always)]
    pub fn get_instruction_at(&self, address: usize) -> Option<&InstructionLine> {
        let index = self.instruction_map.get(address);
        match index {
            Some(index) => self.program.get(*index),
            None => None,
        }
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
                                let dest_value = self.get_register_value(
                                    RegisterOperand::Address(*reg),
                                    Size::Long,
                                );
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
                                let dest_value = self.get_register_value(
                                    RegisterOperand::Address(*reg),
                                    Size::Long,
                                );
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
                        from: self.get_pc().wrapping_sub(4), //the pc is incremented before the instruction is executed
                    });
                }
                let new_sp = self
                    .memory
                    .push(&MemoryCell::Long(self.pc as u32), self.get_sp())?;
                self.set_sp(new_sp);
                let caller_address = self.pc;
                self.pc = *address as usize;
                self.debugger.push_call(
                    self.pc,
                    caller_address,
                    self.cpu.get_register_values(),
                );
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
                        from: self.get_pc().wrapping_sub(4), //pc is incremented before the instruction is executed
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
            Instruction::RTS => {
                let (value, new_sp) = self.memory.pop(Size::Long, self.get_sp())?;
                if self.keep_history {
                    self.debugger.add_mutation(MutationOperation::PopCall {
                        to: value.get_long() as usize,
                        from: self.get_pc().wrapping_sub(4), //pc is incremented before execution
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
                    self.current_interrupt = Some(interrupt);
                    self.set_status(InterpreterStatus::Interrupt);
                }
                _ => {
                    return Err(RuntimeError::Raw(format!(
                        "Unknown trap: {}, only IO with #15 allowed",
                        value
                    )));
                }
            },
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
    pub fn get_next_instruction(&self) -> Option<&InstructionLine> {
        self.get_instruction_at(self.pc)
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
            8 => Ok(Interrupt::GetTime),
            9 => {
                self.status = InterpreterStatus::Terminated;
                Ok(Interrupt::Terminate)
            }
            13 | 14 => {
                //read until null char
                let max = 16384; //to prevent infinite loop
                let address = self.cpu.a_reg[1].get_long() as usize;
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
                match String::from_utf8(bytes.to_vec()) {
                    Ok(str) if value == 13 => Ok(Interrupt::DisplayStringWithCRLF(str)),
                    Ok(str) if value == 14 => Ok(Interrupt::DisplayStringWithoutCRLF(str)),
                    Err(_) | Ok(_) => Err(RuntimeError::Raw(format!(
                        "Invalid String read, received: {:?}, expected UTF-8",
                        bytes
                    ))),
                }
            }
            15 => {
               /*
               Display the unsigned number in D1.L converted to number base (2 through 36) contained in D2.B.
    For example, to display D1.L in base16 put 16 in D2.B
 Values of D2.B outside the range 2 to 36 inclusive are ignored.
                */
                let value = self.cpu.d_reg[1].get_long();
                let base = self.cpu.d_reg[2].get_byte() as u32;
                if !(2..=36).contains(&base) {
                    return Err(RuntimeError::Raw(format!(
                        "Invalid base for display number: {} in register D2.b, expected between 2 and 36",
                        base
                    )));
                };
                Ok(Interrupt::DisplayNumberInBase { value, base: base as u8 })
            }
            23 => {
                let time = self.cpu.d_reg[1].get_long();
                Ok(Interrupt::Delay(time))
            }
            // Graphics interrupts
            11 => Ok(Interrupt::ClearScreen),
            33 => {
                let width = self.cpu.d_reg[1].get_long();
                let height = self.cpu.d_reg[2].get_long();
                Ok(Interrupt::SetScreenSize(width, height))
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
                Ok(Interrupt::DrawPixel(x as u32, y as u32))
            }
            83 => {
                let x = self.cpu.d_reg[1].get_word();
                let y = self.cpu.d_reg[2].get_word();
                Ok(Interrupt::GetPixelColor(x as u32, y as u32))
            }
            84 => {
                let x1 = self.cpu.d_reg[1].get_word();
                let y1 = self.cpu.d_reg[2].get_word();
                let x2 = self.cpu.d_reg[3].get_word();
                let y2 = self.cpu.d_reg[4].get_word();
                Ok(Interrupt::DrawLine(x1 as u32, y1 as u32, x2 as u32, y2 as u32))
            }
            85 => {
                let x = self.cpu.d_reg[1].get_word();
                let y = self.cpu.d_reg[2].get_word();
                Ok(Interrupt::DrawLineTo(x as u32, y as u32))
            }
            86 => {
                let x = self.cpu.d_reg[1].get_word();
                let y = self.cpu.d_reg[2].get_word();
                Ok(Interrupt::MoveTo(x as u32, y as u32))
            }
            87 => {
                let left_x = self.cpu.d_reg[1].get_word();
                let upper_y = self.cpu.d_reg[2].get_word();
                let right_x = self.cpu.d_reg[3].get_word();
                let lower_y = self.cpu.d_reg[4].get_word();
                Ok(Interrupt::DrawRectangle(left_x as u32, upper_y as u32, right_x as u32, lower_y as u32))
            }
            88 => {
                let left_x = self.cpu.d_reg[1].get_word();
                let upper_y = self.cpu.d_reg[2].get_word();
                let right_x = self.cpu.d_reg[3].get_word();
                let lower_y = self.cpu.d_reg[4].get_word();
                Ok(Interrupt::DrawEllipse(left_x as u32, upper_y as u32, right_x as u32, lower_y as u32))
            }
            89 => {
                let x = self.cpu.d_reg[1].get_word();
                let y = self.cpu.d_reg[2].get_word();
                Ok(Interrupt::FloodFill(x as u32, y as u32))
            }
            90 => {
                let left_x = self.cpu.d_reg[1].get_word();
                let upper_y = self.cpu.d_reg[2].get_word();
                let right_x = self.cpu.d_reg[3].get_word();
                let lower_y = self.cpu.d_reg[4].get_word();
                Ok(Interrupt::DrawUnfilledRectangle(left_x as u32, upper_y as u32, right_x as u32, lower_y as u32))
            }
            91 => {
                let left_x = self.cpu.d_reg[1].get_word();
                let upper_y = self.cpu.d_reg[2].get_word();
                let right_x = self.cpu.d_reg[3].get_word();
                let lower_y = self.cpu.d_reg[4].get_word();
                Ok(Interrupt::DrawUnfilledEllipse(left_x as u32, upper_y as u32, right_x as u32, lower_y as u32))
            }
            93 => {
                let width = self.cpu.d_reg[1].get_byte();
                Ok(Interrupt::SetPenWidth(width as u32))
            }
            95 => {
                // Read null-terminated string from address in A1
                let max = 16384; // to prevent infinite loop
                let address = self.cpu.a_reg[1].get_long() as usize;
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
                match String::from_utf8(bytes.to_vec()) {
                    Ok(str) => {
                        let x = self.cpu.d_reg[1].get_word();
                        let y = self.cpu.d_reg[2].get_word();
                        Ok(Interrupt::DrawText(x as u32, y as u32, str))
                    }
                    Err(_) => Err(RuntimeError::Raw(format!(
                        "Invalid String read for DrawText, received: {:?}, expected UTF-8",
                        bytes
                    ))),
                }
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

    pub fn generate_breakpoints_map(&self, breakpoint_lines: &Vec<usize>) -> Vec<bool> {
        let breakpoints_lines_map = breakpoint_lines
            .iter()
            .map(|l| (l, true))
            .collect::<HashMap<&usize, bool>>();
        let breakpoints_addresses = self
            .program
            .iter()
            .filter(|l| breakpoints_lines_map.contains_key(&l.parsed_line.line_index))
            .map(|l| l.address)
            .collect::<Vec<usize>>();
        let max = *breakpoints_addresses.iter().max().unwrap_or(&0);
        let mut breakpoints_addresses_map = vec![false; max + 1];
        for line in breakpoints_addresses {
            breakpoints_addresses_map[line] = true
        }
        breakpoints_addresses_map
    }
    pub fn run_with_breakpoints(
        &mut self,
        breakpoint_lines: &Vec<usize>,
        limit: Option<usize>,
    ) -> RuntimeResult<InterpreterStatus> {
        self.verify_can_run()?;
        let breakpoints_map = self.generate_breakpoints_map(breakpoint_lines);
        let mut iterations = 0;
        let limit = limit.unwrap_or(usize::MAX);
        let mut limit_counter = limit;
        while self.status == InterpreterStatus::Running && limit_counter > 0 {
            match breakpoints_map.get(self.pc) {
                //skip the first iteration if the pc is in a breakpoint
                Some(true) if iterations > 0 => {
                    self.status = InterpreterStatus::Running;
                    break;
                }
                _ => {
                    self.step()?;
                }
            }
            limit_counter -= 1;
            iterations += 1;
        }
        if limit_counter == 0 {
            return Err(RuntimeError::ExecutionLimit(limit));
        }
        Ok(self.status)
        //convert the line numbers to their corresponding addresses, to then save it in a vector to check if the current pc is in it
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
    pub fn wasm_step(&mut self) -> Result<JsValue, JsValue> {
        match self.step() {
            Ok(step) => Ok(serde_wasm_bindgen::to_value(&step).unwrap()),
            Err(e) => Err(serde_wasm_bindgen::to_value(&e).unwrap()),
        }
    }
    pub fn wasm_step_only_status(&mut self) -> Result<InterpreterStatus, JsValue> {
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
    pub fn wasm_run_with_breakpoints(
        &mut self,
        breakpoint_lines: Vec<usize>,
        limit: Option<usize>,
    ) -> Result<InterpreterStatus, JsValue> {
        match self.run_with_breakpoints(&breakpoint_lines, limit) {
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
    pub fn wasm_get_status(&self) -> InterpreterStatus {
        *self.get_status()
    }
    pub fn wasm_get_flag(&self, flag: Flags) -> bool {
        self.get_flag(flag)
    }
    pub fn wasm_get_flags_as_number(&self) -> u16 {
        self.cpu.ccr.bits()
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
        self.last_line_address
    }
    pub fn wasm_get_last_instruction(&self) -> JsValue {
        self.wasm_get_instruction_at(self.last_line_address)
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

    pub fn wasm_get_current_line_index(&self) -> usize {
        match self.get_instruction_at(self.pc) {
            Some(ins) => ins.parsed_line.line_index,
            None => 0,
        }
    }
}
