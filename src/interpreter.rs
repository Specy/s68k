/*
    Some of the implementations were inspired/taken from here, especially the complex flag handling and some mathematical operations
    https://github.com/transistorfet/moa/blob/main/emulator/cpus/m68k/src/execute.rs
*/

use crate::{
    instructions::{
        Condition, Instruction, Interrupt, InterruptResult, Operand, RegisterOperand,
        ShiftDirection, Sign, Size,
    },
    math::*,
    pre_interpreter::{Directive, InstructionLine, Label, PreInterpreter},
};
use bitflags::bitflags;
use core::panic;
use serde::Serialize;
use std::{collections::HashMap, hash::Hash};
use wasm_bindgen::{prelude::wasm_bindgen, JsValue};

#[derive(Debug)]
#[wasm_bindgen]
pub struct Memory {
    data: Vec<u8>,
    pub sp: usize,
}

bitflags! {
    #[wasm_bindgen]
    pub struct Flags: u16 {
        const Carry    = 1<<1;
        const Overflow = 1<<2;
        const Zero     = 1<<3;
        const Negative = 1<<4;
        const Extend   = 1<<5;
    }
}
#[wasm_bindgen]
impl Flags {
    pub fn new() -> Self {
        Flags::empty()
    }
    pub fn clear(&mut self) {
        self.bits = 0;
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
impl Memory {
    pub fn new(size: usize) -> Self {
        Self {
            data: vec![0; size],
            sp: size,
        }
    }

    pub fn push(&mut self, data: &MemoryCell) {
        match data {
            MemoryCell::Byte(byte) => {
                self.sp -= 1;
                self.write_byte(self.sp, *byte)
            }
            MemoryCell::Word(word) => {
                self.sp -= 2;
                self.write_word(self.sp, *word)
            }
            MemoryCell::Long(long) => {
                self.sp -= 4;
                self.write_long(self.sp, *long)
            }
        }
    }
    pub fn pop_empty_long(&mut self) {
        self.sp += 4;
    }
    pub fn pop(&mut self, size: Size) -> MemoryCell {
        match size {
            Size::Byte => {
                let byte = self.read_byte(self.sp);
                self.sp += 1;
                MemoryCell::Byte(byte)
            }
            Size::Word => {
                let word = self.read_word(self.sp);
                self.sp += 2;
                MemoryCell::Word(word)
            }
            Size::Long => {
                let long = self.read_long(self.sp);
                self.sp += 4;
                MemoryCell::Long(long)
            }
        }
    }
    pub fn read_long(&self, address: usize) -> u32 {
        u32::from_be_bytes(self.data[address..address + 4].try_into().unwrap())
    }
    pub fn read_word(&self, address: usize) -> u16 {
        u16::from_be_bytes(self.data[address..address + 2].try_into().unwrap())
    }
    pub fn read_byte(&self, address: usize) -> u8 {
        self.data[address]
    }
    pub fn read_size(&self, address: usize, size: &Size) -> u32 {
        match size {
            Size::Byte => self.read_byte(address) as u32,
            Size::Word => self.read_word(address) as u32,
            Size::Long => self.read_long(address),
        }
    }
    pub fn write_size(&mut self, address: usize, size: &Size, data: u32) {
        match size {
            Size::Byte => self.write_byte(address, data as u8),
            Size::Word => self.write_word(address, data as u16),
            Size::Long => self.write_long(address, data),
        }
    }
    pub fn write_long(&mut self, address: usize, value: u32) {
        self.data[address..address + 4].copy_from_slice(&value.to_be_bytes());
    }
    pub fn write_word(&mut self, address: usize, value: u16) {
        self.data[address..address + 2].copy_from_slice(&value.to_be_bytes());
    }
    pub fn write_byte(&mut self, address: usize, value: u8) {
        self.data[address] = value;
    }
    pub fn write_bytes(&mut self, address: usize, bytes: &[u8]) -> RuntimeResult<()> {
        match (address + bytes.len()) > self.data.len() {
            true => Err(RuntimeError::OutOfBounds(
                format!(
                    "Memory out of bounds, address: {}, size: {}",
                    address,
                    bytes.len()
                )
                .to_string(),
            )),
            false => {
                self.data[address..address + bytes.len()].copy_from_slice(bytes);
                Ok(())
            }
        }
    }
    pub fn read_bytes(&self, address: usize, size: usize) -> RuntimeResult<&[u8]> {
        match (address + size) > self.data.len() {
            true => Err(RuntimeError::OutOfBounds(
                format!("Memory out of bounds, address: {}, size: {}", address, size).to_string(),
            )),
            false => Ok(&self.data[address..address + size]),
        }
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
    pub fn get_size(&self, size: &Size) -> u32 {
        match size {
            Size::Byte => self.get_byte() as u32,
            Size::Word => self.get_word() as u32,
            Size::Long => self.get_long(),
        }
    }
    pub fn store_size(&mut self, size: &Size, data: u32) {
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
impl Cpu {
    pub fn new() -> Self {
        Self {
            d_reg: [Register::new(); 8],
            a_reg: [Register::new(); 8],
            ccr: Flags::new(),
        }
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
pub enum RuntimeError {
    Raw(String),
    OutOfBounds(String),
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

#[wasm_bindgen]
pub struct Interpreter {
    memory: Memory,
    cpu: Cpu,
    pc: usize,
    program: HashMap<usize, InstructionLine>,
    final_instruction_address: usize,
    current_interrupt: Option<Interrupt>,
    status: InterpreterStatus,
}

impl Interpreter {
    pub fn new(pre_interpreted_program: PreInterpreter, memory_size: usize) -> Self {
        let mut interpreter = Self {
            memory: Memory::new(memory_size),
            cpu: Cpu::new(),
            pc: pre_interpreted_program.get_start_address(),
            final_instruction_address: pre_interpreted_program.get_final_instruction_address(),
            program: pre_interpreted_program.get_instructions_map(),
            current_interrupt: None,
            status: InterpreterStatus::Running,
        };
        interpreter.cpu.a_reg[7].store_long((memory_size >> 1) as u32);
        match interpreter.prepare_memory(&pre_interpreted_program.get_labels_map()) {
            Ok(_) => interpreter,
            Err(e) => panic!("Error preparing memory: {:?}", e),
        }
    }

    //TODO could make this an external function and pass the memory in
    pub fn prepare_memory(&mut self, labels: &HashMap<String, Label>) -> RuntimeResult<()> {
        for (_, label) in labels {
            match &label.directive {
                Some(directive) => match directive {
                    Directive::DC { data } | Directive::DS { data } | Directive::DCB { data } => {
                        self.memory.write_bytes(label.address, &data)?;
                    }
                },
                _ => {}
            };
        }
        Ok(())
    }

    pub fn get_cpu(&self) -> &Cpu {
        &self.cpu
    }
    pub fn get_memory(&self) -> &Memory {
        &self.memory
    }
    pub fn get_pc(&self) -> usize {
        self.pc
    }

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
    pub fn has_terminated(&self) -> bool {
        return self.status == InterpreterStatus::Terminated
            || self.status == InterpreterStatus::TerminatedWithException;
    }
    pub fn has_reached_botton(&self) -> bool {
        self.pc > self.final_instruction_address
    }
    pub fn step(&mut self) -> RuntimeResult<(InstructionLine, InterpreterStatus)> {
        match self.get_instruction_at(self.pc) {
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

            Some(ins) => {
                let clone = ins.clone();
                //need to find a way to remove this clone
                self.execute_instruction(&clone)?;
                self.increment_pc(4);
                let status = self.get_status();
                //TODO not sure if doing this before or after running the instruction
                if self.has_reached_botton() && *status != InterpreterStatus::Interrupt {
                    self.set_status(InterpreterStatus::Terminated);
                }
                Ok((clone, self.status))
            }
            None if self.pc < self.final_instruction_address => {
                self.set_status(InterpreterStatus::TerminatedWithException);
                return Err(RuntimeError::OutOfBounds(format!(
                    "Invalid instruction address: {}",
                    self.pc,
                )));
            }
            None => {
                self.set_status(InterpreterStatus::TerminatedWithException);
                return Err(RuntimeError::Raw("Program has terminated".to_string()));
            }
        }
    }
    pub fn answer_interrupt(&mut self, interrupt_result: InterruptResult) -> RuntimeResult<()> {
        match interrupt_result {
            InterruptResult::DisplayNumber
            | InterruptResult::DisplayStringWithCRLF
            | InterruptResult::DisplayStringWithoutCRLF => {}
            InterruptResult::ReadKeyboardString(str) => {
                if str.len() > 80 {
                    //TODO should i error or truncate?
                    return Err(RuntimeError::Raw(
                        "String is longer than 80 chars".to_string(),
                    ));
                }
                let address = self.cpu.a_reg[0].get_long() as usize;
                self.memory.write_bytes(address, str.as_bytes())?;
                self.cpu.d_reg[1].store_word(str.len() as u16);
            }
            InterruptResult::ReadNumber(num) => {
                self.cpu.d_reg[1].store_long(num as u32);
            }
            InterruptResult::ReadChar(char) => {
                self.cpu.d_reg[1].store_byte(char as u8);
            }
            InterruptResult::GetTime(time) => {
                self.cpu.d_reg[1].store_long(time);
            }
            InterruptResult::Terminate => {
                self.set_status(InterpreterStatus::Terminated);
            }
        };
        self.current_interrupt = None;
        //edge case if the last instruction is an interrupt
        self.status = if self.has_reached_botton() {
            InterpreterStatus::Terminated
        } else {
            InterpreterStatus::Running
        };
        Ok(())
    }
    fn increment_pc(&mut self, amount: usize) {
        self.pc += amount;
    }
    pub fn get_instruction_at(&self, address: usize) -> Option<&InstructionLine> {
        self.program.get(&address)
    }
    pub fn get_current_interrupt(&self) -> RuntimeResult<Interrupt> {
        match &self.current_interrupt {
            Some(interrupt) => Ok(interrupt.clone()),
            None => Err(RuntimeError::Raw("No interrupt pending".to_string())),
        }
    }
    fn execute_instruction(&mut self, instruction_line: &InstructionLine) -> RuntimeResult<()> {
        let ins = &instruction_line.instruction;
        //println!("PC {:#X} - {:?}", self.pc, instruction_line);
        match ins {
            Instruction::MOVE(source, dest, size) => {
                let source_value = self.get_operand_value(source, size)?;
                self.set_logic_flags(source_value, size);
                self.store_operand_value(dest, source_value, size)?;
            }
            Instruction::ADD(source, dest, size) => {
                let source_value = self.get_operand_value(source, size)?;
                let dest_value = self.get_operand_value(dest, size)?;
                let (result, carry) = overflowing_add_sized(dest_value, source_value, size);
                let overflow = has_add_overflowed(dest_value, source_value, result, size);
                self.set_compare_flags(result, size, carry, overflow);
                self.set_flag(Flags::Extend, carry);
                self.store_operand_value(dest, result, size)?;
            }
            Instruction::SUB(source, dest, size) => {
                let source_value = self.get_operand_value(source, size)?;
                let dest_value = self.get_operand_value(dest, size)?;
                let (result, carry) = overflowing_sub_sized(dest_value, source_value, size);
                let overflow = has_sub_overflowed(dest_value, source_value, result, size);
                self.set_compare_flags(result, size, carry, overflow);
                self.set_flag(Flags::Extend, carry);
                self.store_operand_value(dest, result, size)?;
            }
            Instruction::ADDA(source, dest, size) => {
                let source_value =
                    sign_extend_to_long(self.get_operand_value(source, size)?, size) as u32;
                let dest_value = self.get_register_value(dest, size);
                let (result, _) = overflowing_add_sized(dest_value, source_value, &Size::Long);
                self.set_register_value(dest, result, &Size::Long);
            }
            Instruction::MULx(source, dest, sign) => {
                let src_val = self.get_operand_value(source, &Size::Word)?;
                let dest_val =
                    get_value_sized(self.get_register_value(dest, &Size::Long), &Size::Word);
                let result = match sign {
                    Sign::Signed => {
                        ((((dest_val as u16) as i16) as i64) * (((src_val as u16) as i16) as i64))
                            as u64
                    }
                    Sign::Unsigned => dest_val as u64 * src_val as u64,
                };
                self.set_compare_flags(result as u32, &Size::Long, false, false);
                self.set_register_value(dest, result as u32, &Size::Long);
            }
            Instruction::LSd(amount_source, dest, direction, size) => {
                let amount = self.get_operand_value(amount_source, size)? % 64;
                let mut pair = (self.get_operand_value(dest, size)?, false);
                for _ in 0..amount {
                    pair = shift(direction, pair.0, size, false);
                }
                self.store_operand_value(dest, pair.0, size)?;
                self.set_logic_flags(pair.0, size);
                self.set_flag(Flags::Overflow, false);
                if amount != 0 {
                    self.set_flag(Flags::Extend, pair.1);
                    self.set_flag(Flags::Carry, pair.1);
                } else {
                    self.set_flag(Flags::Carry, false);
                }
            }
            Instruction::BRA(address) => {
                //instead of using the absolute address, the original language uses pc + 2 + offset
                self.pc = *address as usize;
            }
            Instruction::BSR(address) => {
                self.memory.push(&MemoryCell::Long(self.pc as u32));
                self.pc = *address as usize;
            }
            Instruction::JMP(op) => {
                let addr = self.get_operand_value(op, &Size::Long)?;
                self.pc = addr as usize;
            }
            Instruction::JSR(source) => {
                let addr = self.get_operand_value(source, &Size::Long)?;
                self.memory.push(&MemoryCell::Long(self.pc as u32));
                self.pc = addr as usize;
            }
            Instruction::BCHG(bit_source, dest) => {
                let bit = self.get_operand_value(bit_source, &Size::Byte)?;
                let mut source_value = self.get_operand_value(dest, &Size::Long)?;
                let mask = self.set_bit_test_flags(source_value, bit, &Size::Long);
                source_value = (source_value & !mask) | (!(source_value & mask) & mask);
                self.store_operand_value(dest, source_value, &Size::Long)?;
            }
            Instruction::BCLR(bit_source, dest) => {
                let bit = self.get_operand_value(bit_source, &Size::Byte)?;
                let mut src_val = self.get_operand_value(dest, &Size::Long)?;
                let mask = self.set_bit_test_flags(src_val, bit, &Size::Long);
                src_val = src_val & !mask;
                self.store_operand_value(dest, src_val, &Size::Long)?;
            }
            Instruction::BSET(bit_source, dest) => {
                let bit = self.get_operand_value(bit_source, &Size::Byte)?;
                let mut value = self.get_operand_value(dest, &Size::Long)?;
                let mask = self.set_bit_test_flags(value, bit, &Size::Long);
                value = value | mask;
                self.store_operand_value(dest, value, &Size::Long)?;
            }

            Instruction::BTST(bit, op2) => {
                let bit = self.get_operand_value(bit, &Size::Byte)?;
                let value = self.get_operand_value(op2, &Size::Long)?;
                self.set_bit_test_flags(value, bit, &Size::Long);
            }
            Instruction::ASd(amount, dest, direction, size) => {
                let amount_value = self.get_operand_value(amount, size)? % 64;
                let dest_value = self.get_operand_value(dest, size)?;
                let mut has_overflowed = false;
                let (mut value, mut has_carry) = (dest_value, false);
                let mut previous_msb = get_sign(value, size);
                for _ in 0..amount_value {
                    (value, has_carry) = shift(direction, value, size, true);
                    if get_sign(value, size) != previous_msb {
                        has_overflowed = true;
                    }
                    previous_msb = get_sign(value, size);
                }
                self.store_operand_value(dest, value, size)?;

                let carry = match direction {
                    ShiftDirection::Left => has_carry,
                    ShiftDirection::Right => {
                        if amount_value < size.to_bits() as u32 {
                            has_carry
                        } else {
                            false
                        }
                    }
                };
                self.set_logic_flags(value, size);
                self.set_flag(Flags::Overflow, has_overflowed);
                if amount_value != 0 {
                    self.set_flag(Flags::Extend, carry);
                    self.set_flag(Flags::Carry, carry);
                } else {
                    self.set_flag(Flags::Carry, false);
                }
            }
            Instruction::ROd(amount, dest, direction, size) => {
                let count = self.get_operand_value(amount, size)? % 64;
                let (mut value, mut carry) = (self.get_operand_value(dest, size)?, false);
                for _ in 0..count {
                    (value, carry) = rotate(direction, value, size);
                }
                self.store_operand_value(dest, value, size)?;
                self.set_logic_flags(value, size);
                if carry {
                    self.set_flag(Flags::Carry, true);
                }
            }
            Instruction::SUBA(source, dest, size) => {
                let source_value =
                    sign_extend_to_long(self.get_operand_value(source, size)?, size) as u32;
                let dest_value = self.get_register_value(dest, size);
                let (result, _) = overflowing_sub_sized(dest_value, source_value, &Size::Long);
                self.set_register_value(dest, result, &Size::Long);
            }
            Instruction::AND(source, dest, size) => {
                let source_value = self.get_operand_value(source, size)?;
                let dest_value = self.get_operand_value(dest, size)?;
                let result = get_value_sized(dest_value & source_value, size);
                self.store_operand_value(dest, result, size)?;
                self.set_logic_flags(result, size);
            }
            Instruction::OR(source, dest, size) => {
                let source_value = self.get_operand_value(source, size)?;
                let dest_value = self.get_operand_value(dest, size)?;
                let result = get_value_sized(dest_value | source_value, size);
                self.store_operand_value(dest, result, size)?;
                self.set_logic_flags(result, size);
            }
            Instruction::EOR(source, dest, size) => {
                let source_value = self.get_operand_value(source, size)?;
                let dest_value = self.get_operand_value(dest, size)?;
                let result = get_value_sized(dest_value ^ source_value, size);
                self.store_operand_value(dest, result, size)?;
                self.set_logic_flags(result, size);
            }
            Instruction::NOT(op, size) => {
                //watchout for the "!"
                let value = !self.get_operand_value(op, size)?;
                let value = get_value_sized(value, size);
                self.store_operand_value(op, value, size)?;
                self.set_logic_flags(value, size);
            }
            Instruction::NEG(source, size) => {
                let original = self.get_operand_value(source, size)?;
                let (result, overflow) = overflowing_sub_signed_sized(0, original, size);
                let carry = result != 0;
                self.store_operand_value(source, result, size)?;
                self.set_compare_flags(result, size, carry, overflow);
                self.set_flag(Flags::Extend, carry);
            }
            Instruction::DIVx(source, dest, sign) => {
                let source_value = self.get_operand_value(source, &Size::Word)?;
                if source_value == 0 {
                    return Err(RuntimeError::DivisionByZero);
                }
                let dest_value = self.get_register_value(dest, &Size::Long);
                let dest_value = get_value_sized(dest_value, &Size::Long);
                let (remainder, quotient, has_overflowed) = match sign {
                    Sign::Signed => {
                        let dest_value = dest_value as i32;
                        let source_value = sign_extend_to_long(source_value, &Size::Word) as i32;
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
                    self.set_compare_flags(quotient as u32, &Size::Word, false, false);
                    self.set_register_value(
                        dest,
                        (remainder << 16) | (0xFFFF & quotient),
                        &Size::Long,
                    );
                } else {
                    self.set_flag(Flags::Carry, false);
                    self.set_flag(Flags::Overflow, true);
                }
            }
            Instruction::EXG(reg1, reg2) => {
                let reg1_value = self.get_register_value(reg1, &Size::Long);
                let reg2_value = self.get_register_value(reg2, &Size::Long);
                self.set_register_value(reg1, reg2_value, &Size::Long);
                self.set_register_value(reg2, reg1_value, &Size::Long);
            }
            Instruction::EXT(reg, from, to) => {
                let input = get_value_sized(self.get_register_value(reg, &Size::Long), from);
                let result = match (from, to) {
                    (Size::Byte, Size::Word) => ((((input as u8) as i8) as i16) as u16) as u32,
                    (Size::Word, Size::Long) => (((input as u16) as i16) as i32) as u32,
                    (Size::Byte, Size::Long) => (((input as u8) as i8) as i32) as u32,
                    _ => panic!("Invalid size for EXT instruction"),
                };
                self.set_register_value(reg, result, &Size::Long);
                self.set_logic_flags(result, to);
            }
            Instruction::SWAP(reg) => {
                let value = self.get_register_value(reg, &Size::Long);
                let new_value = ((value & 0x0000FFFF) << 16) | ((value & 0xFFFF0000) >> 16);
                self.set_register_value(reg, new_value, &Size::Long);
                self.set_logic_flags(new_value, &Size::Long);
            }
            Instruction::TST(source, size) => {
                let value = self.get_operand_value(source, size)?;
                self.set_logic_flags(value, size);
            }
            Instruction::CMP(source, dest, size) => {
                let source_value = self.get_operand_value(source, size)?;
                let dest_value = self.get_operand_value(dest, size)?;
                let (result, carry) = overflowing_sub_sized(dest_value, source_value, size);
                let overflow = has_sub_overflowed(dest_value, source_value, result, size);
                self.set_compare_flags(result, size, carry, overflow);
            }
            Instruction::Bcc(address, condition) => {
                if self.get_condition_value(condition) {
                    self.pc = *address as usize;
                }
            }
            Instruction::CLR(dest, size) => {
                self.get_operand_value(dest, size)?; //apply side effects
                self.store_operand_value(dest, 0, size)?;
                self.cpu.ccr.clear();
                self.set_flag(Flags::Zero, true);
            }
            Instruction::Scc(op, condition) => {
                if self.get_condition_value(condition) {
                    self.store_operand_value(op, 0xFF, &Size::Byte)?;
                } else {
                    self.store_operand_value(op, 0x00, &Size::Byte)?;
                }
            }
            Instruction::RTS => {
                let return_address = self.memory.pop(Size::Long).get_long();
                self.pc = return_address as usize;
            }
            Instruction::TRAP(value) => match value {
                15 => {
                    let task = self.cpu.d_reg[0].get_byte();
                    let interrupt = self.get_interrupt(task)?;
                    self.current_interrupt = Some(interrupt);
                    self.set_status(InterpreterStatus::Interrupt);
                }
                _ => {
                    return Err(RuntimeError::Raw(format!(
                        "Unknown trap: {}, only IO with #15 allowed",
                        value
                    )))
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
    pub fn get_register_value(&self, register: &RegisterOperand, size: &Size) -> u32 {
        match register {
            RegisterOperand::Address(num) => self.cpu.a_reg[*num as usize].get_size(size),
            RegisterOperand::Data(num) => self.cpu.d_reg[*num as usize].get_size(size),
        }
    }
    pub fn set_register_value(&mut self, register: &RegisterOperand, value: u32, size: &Size) {
        match register {
            RegisterOperand::Address(num) => self.cpu.a_reg[*num as usize].store_size(size, value),
            RegisterOperand::Data(num) => self.cpu.d_reg[*num as usize].store_size(size, value),
        }
    }

    fn get_interrupt(&mut self, value: u8) -> RuntimeResult<Interrupt> {
        match value {
            0 | 1 => {
                let address = self.cpu.a_reg[1].get_long();
                let length = self.cpu.d_reg[1].get_word() as i32;
                if length > 255 || length < 0 {
                    return Err(RuntimeError::Raw(format!("Invalid String read, length of string in d1 register is: {}, expected between 0 and 255", length)));
                } else {
                    let bytes = self.memory.read_bytes(address as usize, length as usize)?;
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
                Ok(Interrupt::DisplayNumber(value))
            }
            4 => Ok(Interrupt::ReadNumber),
            5 => Ok(Interrupt::ReadChar),
            6 => Ok(Interrupt::GetTime),
            7 => Ok(Interrupt::Terminate),
            _ => Err(RuntimeError::Raw(format!("Unknown interrupt: {}", value))),
        }
    }
    fn get_operand_value(&mut self, op: &Operand, size: &Size) -> RuntimeResult<u32> {
        match op {
            Operand::Immediate(v) => Ok(*v),
            Operand::Register(op) => Ok(self.get_register_value(&op, size)),
            Operand::Address(address) => Ok(self.memory.read_size(*address, size)),
            Operand::Indirect { offset, operand } => {
                //TODO not sure if this works fine with full 32bits
                let address = self.get_register_value(&operand, &Size::Long) as i32 + offset;
                Ok(self.memory.read_size(address as usize, size))
            }
            Operand::PreIndirect(op) => {
                let address = self.get_register_value(&op, &Size::Long) as usize - size.to_bytes();
                self.set_register_value(&op, address as u32, &Size::Long);
                Ok(self.memory.read_size(address, size))
            }
            Operand::PostIndirect(op) => {
                let address = self.get_register_value(&op, &Size::Long) as usize;
                self.set_register_value(&op, address as u32 + size.to_bytes() as u32, &Size::Long);
                Ok(self.memory.read_size(address, size))
            }
            Operand::IndirectWithDisplacement { offset, operands } => {
                Err(RuntimeError::Unimplemented)
            }
        }
    }
    fn store_operand_value(&mut self, op: &Operand, value: u32, size: &Size) -> RuntimeResult<()> {
        match op {
            Operand::Immediate(_) => Err(RuntimeError::IncorrectAddressingMode(
                "Attempted to store to immediate value".to_string(),
            )),
            Operand::Register(op) => Ok(self.set_register_value(&op, value, size)),
            Operand::Address(address) => Ok(self.memory.write_size(*address, size, value)),
            Operand::Indirect { offset, operand } => {
                //TODO not sure if this works fine with full 32bits
                let address = self.get_register_value(&operand, &Size::Long) as i32 + offset;
                Ok(self.memory.write_size(address as usize, size, value))
            }
            Operand::PreIndirect(op) => {
                //TODO not sure if this works fine with full 32bits
                let address = self.get_register_value(&op, &Size::Long) as usize - size.to_bytes();
                self.set_register_value(&op, address as u32, &Size::Long);
                Ok(self.memory.write_size(address, size, value))
            }
            Operand::PostIndirect(op) => {
                //TODO not sure if this works fine with full 32bits
                let address = self.get_register_value(&op, &Size::Long) as usize;
                self.set_register_value(&op, address as u32 + size.to_bytes() as u32, &Size::Long);
                Ok(self.memory.write_size(address, size, value))
            }
            Operand::IndirectWithDisplacement { offset, operands } => {
                Err(RuntimeError::Unimplemented)
            }
        }
    }
    pub fn run(&mut self) -> RuntimeResult<InterpreterStatus> {
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
        while self.status == InterpreterStatus::Running {
            match self.step() {
                Ok(_) => {}
                Err(e) => match self.get_instruction_at(self.pc) {
                    Some(ins) => {
                        return Err(RuntimeError::Raw(format!(
                            "Runtime error at line:{} {:?}",
                            ins.parsed_line.line_index, e
                        )))
                    }
                    None => {
                        return Err(RuntimeError::Raw(format!(
                            "Unknown Runtime error at PC:{} {:?}",
                            self.pc, e
                        )));
                    }
                },
            }
        }
        Ok(self.status)
    }

    pub fn get_flag(&self, flag: Flags) -> bool {
        self.cpu.ccr.contains(flag)
    }
    fn set_flag(&mut self, flag: Flags, value: bool) {
        self.cpu.ccr.set(flag, value)
    }
    fn set_logic_flags(&mut self, value: u32, size: &Size) {
        let mut flags = Flags::new();
        if get_sign(value, size) {
            flags |= Flags::Negative;
        }
        if value == 0 {
            flags |= Flags::Zero;
        }
        self.cpu.ccr.set(Flags::Carry, false);
        self.cpu.ccr |= flags;
    }
    fn set_bit_test_flags(&mut self, value: u32, bitnum: u32, size: &Size) -> u32 {
        let mask = 0x1 << (bitnum % size.to_bits() as u32);
        self.set_flag(Flags::Zero, (value & mask) == 0);
        mask
    }
    fn set_compare_flags(&mut self, value: u32, size: &Size, carry: bool, overflow: bool) {
        let value = sign_extend_to_long(value, &size);
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
        self.cpu.ccr.set(Flags::Carry, false);
        self.cpu.ccr |= flags;
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
    pub fn wasm_get_cpu_snapshot(&self) -> Cpu {
        self.cpu
    }
    pub fn wasm_get_pc(&self) -> usize {
        self.pc
    }
    pub fn wasm_get_instruction_at(&self, address: usize) -> JsValue {
        match self.get_instruction_at(address) {
            Some(ins) => serde_wasm_bindgen::to_value(ins).unwrap(),
            None => JsValue::NULL,
        }
    }
    pub fn wasm_step(&mut self) -> JsValue {
        match self.step() {
            Ok(line) => serde_wasm_bindgen::to_value(&line).unwrap(),
            Err(e) => serde_wasm_bindgen::to_value(&e).unwrap(),
        }
    }
    pub fn wasm_run(&mut self) -> InterpreterStatus {
        match self.run() {
            Ok(status) => status,
            Err(e) => panic!("Runtime error {:?}", e),
        }
    }
}
