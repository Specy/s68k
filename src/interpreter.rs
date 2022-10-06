use core::panic;
use std::{collections::HashMap, hash::Hash};

use crate::{
    instructions::{Instruction, Size, Operand, RegisterType, RegisterOperand},
    pre_interpreter::{Directive, InstructionLine, Label, PreInterpreter},
};

#[derive(Debug)]
pub struct Memory {
    data: Vec<u8>,
    pub sp: usize,
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
    pub fn write_bytes(&mut self, address: usize, bytes: &[u8]) {
        if (address + bytes.len()) > self.data.len() {
            panic!(
                "Memory out of bounds, address: {}, size: {}",
                address,
                bytes.len()
            );
        }
        self.data[address..address + bytes.len()].copy_from_slice(bytes);
    }
    pub fn read_bytes(&self, address: usize, size: usize) -> &[u8] {
        if (address + size) > self.data.len() {
            panic!(
                "Memory out of bounds, address \"{}\" is not in range 0..{}",
                address,
                self.data.len()
            );
        }
        &self.data[address..address + size]
    }
}
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
pub struct Ccr {
    data: u8,
}
pub struct Cpu {
    d_reg: [Register; 8],
    a_reg: [Register; 8],
    ccr: Ccr,
}
impl Cpu {
    pub fn new() -> Self {
        Self {
            d_reg: [Register::new(); 8],
            a_reg: [Register::new(); 8],
            ccr: Ccr { data: 0 },
        }
    }
}
pub struct Interpreter {
    memory: Memory,
    cpu: Cpu,
    pc: usize,
    program: HashMap<usize, InstructionLine>,
    final_instruction_address: usize,
}

impl Interpreter {
    pub fn new(pre_interpreted_program: PreInterpreter, memory_size: usize) -> Self {
        let mut interpreter = Self {
            memory: Memory::new(0),
            cpu: Cpu::new(),
            pc: pre_interpreted_program.get_start_address(),
            final_instruction_address: pre_interpreted_program.get_final_instruction_address(),
            program: pre_interpreted_program.get_instructions_map(),
        };
        interpreter.prepare_memory(memory_size, Some(&pre_interpreted_program.labels));
        interpreter
    }
    pub fn prepare_memory(&mut self, size: usize, labels: Option<&HashMap<String, Label>>) {
        self.memory = Memory::new(size);
        match labels {
            Some(labels) => {
                for (_, label) in labels {
                    match &label.directive {
                        Some(directive) => match directive {
                            Directive::DC { data }
                            | Directive::DS { data }
                            | Directive::DCB { data } => {
                                self.memory.write_bytes(label.address, &data);
                            }
                        },
                        _ => {}
                    }
                }
            }
            None => {}
        }
    }

    pub fn has_finished(&self) -> bool {
        self.pc >= self.final_instruction_address
    }

    pub fn step(&mut self) {
        match self.get_instruction_at(self.pc) {
            Some(ins) => {
                self.execute_instruction(&ins.clone());
            }
            None if self.pc < self.final_instruction_address => {
                panic!("Invalid instruction address: {}", self.pc);
            }
            _ => {}
        }
        self.increment_pc(4);
    }
    pub fn increment_pc(&mut self, amount: usize) {
        self.pc += amount;
    }
    pub fn get_instruction_at(&self, address: usize) -> Option<&InstructionLine> {
        self.program.get(&address)
    }
    fn execute_instruction(&mut self, instruction_line: &InstructionLine) {
        let ins = &instruction_line.instruction;
        match ins {

            Instruction::RTS => {
                let return_address = self.memory.pop(Size::Long).get_long();
                self.pc = return_address as usize;
            }
            //TODO add better string conversion
            _ => panic!(
                "Invalid or unimplemented instruction: {:?}",
                ins.get_instruction_name()
            ),
        }
    }
    pub fn get_register_value(&self, register: &RegisterOperand, size: &Size) -> u32 {
        match register {
            RegisterOperand::Address(num) => self.cpu.a_reg[*num as usize].get_size(size),
            RegisterOperand::Data(num) => self.cpu.d_reg[*num as usize].get_size(size),
        }
    }
    pub fn set_register_value(&mut self, register: &RegisterOperand, size: &Size, value: u32) {
        match register {
            RegisterOperand::Address(num) => self.cpu.a_reg[*num as usize].store_size(size, value),
            
            RegisterOperand::Data(num) => self.cpu.d_reg[*num as usize].store_size(size, value)
        }
    }
    pub fn get_operand_value(&mut self, op: Operand, size: &Size) -> u32{
        match op{
            Operand::Immediate(v) => v,
            Operand::Register(op) => self.get_register_value(&op, size),
            Operand::Address(address) => {
                self.memory.read_size(address, size)
            }
            Operand::Indirect { offset, operand } => {
                let address = self.get_register_value(&operand, &Size::Long) as i32 + offset;
                self.memory.read_size(address as usize, size)
            }
            Operand::PreIndirect(op) => {
                let address = self.get_register_value(&op, &Size::Long) as usize - size.to_bytes();
                self.set_register_value(&op, &Size::Long, address as u32);
                self.memory.read_size(address, size)
            }
            Operand::PostIndirect(op) => {
                let address = self.get_register_value(&op, &Size::Long) as usize;
                self.set_register_value(&op, &Size::Long, address as u32 + size.to_bytes() as u32);
                self.memory.read_size(address, size)
            }
            Operand::IndirectWithDisplacement { offset, operands } => {
                unimplemented!("IndirectWithDisplacement not yet implemented");
            }
        }

    }


    pub fn run(&mut self) {
        while !self.has_finished() {
            self.step();
        }
    }
}

/*
Detecting overflows can be done with checked_add (returns None on overflow) or overflowing_add (returns a tuple of (wrapped_result, did_it_overflow)).
Also be aware of saturating_add (stops "just short" of overflowing, e.g. 250u8.saturating_add(10) == 255u8) and wrapping_add (explicitly wraps).
These operations all exist for sub and mul as well, and div has a checked variant (catches x / 0 and iX::MIN / -1)
*/
