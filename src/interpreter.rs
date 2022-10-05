use core::panic;
use std::{collections::HashMap, hash::Hash};

use crate::{pre_interpreter::{Directive, InstructionLine, Label, PreInterpreter}, instructions::Instruction};

#[derive(Debug)]
pub struct Memory {
    data: Vec<u8>,
    sp: usize,
}
impl Memory {
    pub fn new(size: usize) -> Self {
        Self {
            data: vec![0; size],
            sp: size,
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
        match self.program.get(&self.pc) {
            Some(ins) => {
                self.execute_instruction(ins)
            }
            None if self.pc < self.final_instruction_address => {
                panic!("Invalid instruction address: {}", self.pc);
            }
            _ => {}
        }
        self.pc += 4;
    }


    fn execute_instruction(&self, instruction_line: &InstructionLine){
        let ins = &instruction_line.instruction;
        match ins{
            Instruction::RTS => {

            }
            //TODO add better string conversion
            _ => panic!("Invalid or unimplemented instruction: {:?}", ins.get_instruction_name()),
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
