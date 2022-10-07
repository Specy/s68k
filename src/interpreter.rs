/*
    Some of the implementations were inspired/taken from here, especially the complex flag handling and some mathematical operations
    https://github.com/transistorfet/moa/blob/main/emulator/cpus/m68k/src/execute.rs
*/


use crate::{
    instructions::{
        Condition, Instruction, Operand, RegisterOperand, RegisterType, ShiftDirection, Sign, Size,
    },
    math::*,
    pre_interpreter::{Directive, InstructionLine, Label, PreInterpreter},
};
use bitflags::bitflags;
use core::panic;
use std::{collections::HashMap, hash::Hash};

#[derive(Debug)]
pub struct Memory {
    data: Vec<u8>,
    pub sp: usize,
}

bitflags! {
    struct Flags: u16 {
        const Carry    = 1<<1;
        const Overflow = 1<<2;
        const Zero     = 1<<3;
        const Negative = 1<<4;
        const Extend   = 1<<5;
    }
}
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
        interpreter.cpu.a_reg[7].store_long((memory_size >> 1) as u32);
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
        self.pc > self.final_instruction_address
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
        //println!("PC {:#X} - {:?}", self.pc, instruction_line);
        match ins {
            Instruction::MOVE(source, dest, size) => {
                let source_value = self.get_operand_value(source, size);
                self.set_logic_flags(source_value, size);
                self.store_operand_value(dest, source_value, size);
            }
            Instruction::ADD(source, dest, size) => {
                let source_value = self.get_operand_value(source, size);
                let dest_value = self.get_operand_value(dest, size);
                let (result, carry) = overflowing_add_sized(dest_value, source_value, size);
                let overflow = has_add_overflowed(dest_value, source_value, result, size);
                self.set_compare_flags(result, size, carry, overflow);
                self.set_flag(Flags::Extend, carry);
                self.store_operand_value(dest, result, size);
            }
            Instruction::SUB(source, dest, size) => {
                let source_value = self.get_operand_value(source, size);
                let dest_value = self.get_operand_value(dest, size);
                let (result, carry) = overflowing_sub_sized(dest_value, source_value, size);
                let overflow = has_sub_overflowed(dest_value, source_value, result, size);
                self.set_compare_flags(result, size, carry, overflow);
                self.set_flag(Flags::Extend, carry);
                self.store_operand_value(dest, result, size);
            }
            Instruction::ADDA(source, dest, size) => {
                let source_value =
                    sign_extend_to_long(self.get_operand_value(source, size), size) as u32;
                let dest_value = self.get_register_value(dest, size);
                let (result, _) = overflowing_add_sized(dest_value, source_value, &Size::Long);
                self.set_register_value(dest, result, &Size::Long);
            }
            Instruction::MULx(source, dest, sign) => {
                let src_val = self.get_operand_value(source, &Size::Word);
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
                let amount = self.get_operand_value(amount_source, size) % 64;
                let mut pair = (self.get_operand_value(dest, size), false);
                for _ in 0..amount {
                    pair = shift(direction, pair.0, size, false);
                }
                self.store_operand_value(dest, pair.0, size);
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
                let addr = self.get_operand_value(op, &Size::Long);
                self.pc = addr as usize;
            }
            Instruction::JSR(source) => {
                let addr = self.get_operand_value(source, &Size::Long);
                self.memory.push(&MemoryCell::Long(self.pc as u32));
                self.pc = addr as usize;
                
            }
            Instruction::BCHG(bit_source, dest) => {
                let bit = self.get_operand_value(bit_source, &Size::Byte);
                let mut source_value = self.get_operand_value(dest, &Size::Long);
                let mask = self.set_bit_test_flags(source_value, bit, &Size::Long);
                source_value = (source_value & !mask) | (!(source_value & mask) & mask);
                self.store_operand_value(dest, source_value, &Size::Long);
            }
            Instruction::BCLR(bit_source, dest) => {
                let bit = self.get_operand_value(bit_source, &Size::Byte);
                let mut src_val = self.get_operand_value(dest, &Size::Long);
                let mask = self.set_bit_test_flags(src_val, bit, &Size::Long);
                src_val = src_val & !mask;
                self.store_operand_value(dest, src_val, &Size::Long);
            }
            Instruction::BSET(bit_source, dest) => {
                let bit = self.get_operand_value(bit_source, &Size::Byte);
                let mut value = self.get_operand_value(dest, &Size::Long);
                let mask = self.set_bit_test_flags(value, bit, &Size::Long);
                value = value | mask;
                self.store_operand_value(dest, value, &Size::Long);
            }

            Instruction::BTST(bit, op2) => {
                let bit = self.get_operand_value(bit, &Size::Byte);
                let value = self.get_operand_value(op2, &Size::Long);
                self.set_bit_test_flags(value, bit, &Size::Long);
            }
            Instruction::ASd(amount, dest, direction, size) => {
                let amount_value = self.get_operand_value(amount, size) % 64;
                let dest_value = self.get_operand_value(dest, size);
                let mut has_overflowed = false;
                let (mut value,mut has_carry) = (dest_value, false);
                let mut previous_msb = get_sign(value, size);
                for _ in 0..amount_value {
                    (value, has_carry) = shift(direction, value, size, true);
                    if get_sign(value, size) != previous_msb {
                        has_overflowed = true;
                    }
                    previous_msb = get_sign(value, size);
                }
                self.store_operand_value(dest, value, size);

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
                let count = self.get_operand_value(amount, size) % 64;
                let (mut value,mut carry) = (self.get_operand_value(dest, size), false);
                for _ in 0..count {
                    (value, carry) = rotate(direction, value, size);
                }
                self.store_operand_value(dest, value, size);
                self.set_logic_flags(value, size);
                if carry {
                    self.set_flag(Flags::Carry, true);
                }
            }
            Instruction::SUBA(source, dest, size) => {
                let source_value =
                    sign_extend_to_long(self.get_operand_value(source, size), size) as u32;
                let dest_value = self.get_register_value(dest, size);
                let (result, _) = overflowing_sub_sized(dest_value, source_value, &Size::Long);
                self.set_register_value(dest, result, &Size::Long);
            }
            Instruction::AND(source, dest, size) => {
                let source_value = self.get_operand_value(source, size);
                let dest_value = self.get_operand_value(dest, size);
                let result = get_value_sized(dest_value & source_value, size);
                self.store_operand_value(dest, result, size);
                self.set_logic_flags(result, size);
            }
            Instruction::OR(source, dest, size) => {
                let source_value = self.get_operand_value(source, size);
                let dest_value = self.get_operand_value(dest, size);
                let result = get_value_sized(dest_value | source_value, size);
                self.store_operand_value(dest, result, size);
                self.set_logic_flags(result, size);
            }
            Instruction::EOR(source, dest, size) => {
                let source_value = self.get_operand_value(source, size);
                let dest_value = self.get_operand_value(dest, size);
                let result = get_value_sized(dest_value ^ source_value, size);
                self.store_operand_value(dest, result, size);
                self.set_logic_flags(result, size);
            }
            Instruction::NOT(op, size) => {
                //watchout for the "!"
                let value = !self.get_operand_value(op, size);
                let value = get_value_sized(value, size);
                self.store_operand_value(op, value, size);
                self.set_logic_flags(value, size);
            }
            Instruction::NEG(source, size) => {
                let original = self.get_operand_value(source, size);
                let (result, overflow) = overflowing_sub_signed_sized(0, original, size);
                let carry = result != 0;
                self.store_operand_value(source, result, size);
                self.set_compare_flags(result, size, carry, overflow);
                self.set_flag(Flags::Extend, carry);
            }
            Instruction::DIVx(source, dest, sign) => {
                let source_value = self.get_operand_value(source, &Size::Word);
                if source_value == 0 {
                    panic!("Division by zero");
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
                let value = self.get_operand_value(source, size);
                self.set_logic_flags(value, size);
            }
            Instruction::CMP(source, dest, size) => {
                let source_value = self.get_operand_value(source, size);
                let dest_value = self.get_operand_value(dest, size);
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
                self.get_operand_value(dest, size); //apply side effects
                self.store_operand_value(dest, 0, size);
                self.cpu.ccr.clear();
                self.set_flag(Flags::Zero, true);
            }
            Instruction::Scc(op, condition) => {
                if self.get_condition_value(condition) {
                    self.store_operand_value(op, 0xFF, &Size::Byte);
                } else {
                    self.store_operand_value(op, 0x00, &Size::Byte);
                }
            }

            Instruction::RTS => {
                let return_address = self.memory.pop(Size::Long).get_long();
                self.pc = return_address as usize;
            }
        }
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
    pub fn get_operand_value(&mut self, op: &Operand, size: &Size) -> u32 {
        match op {
            Operand::Immediate(v) => *v,
            Operand::Register(op) => self.get_register_value(&op, size),
            Operand::Address(address) => self.memory.read_size(*address, size),
            Operand::Indirect { offset, operand } => {
                //TODO not sure if this works fine with full 32bits
                let address = self.get_register_value(&operand, &Size::Long) as i32 + offset;
                self.memory.read_size(address as usize, size)
            }
            Operand::PreIndirect(op) => {
                let address = self.get_register_value(&op, &Size::Long) as usize - size.to_bytes();
                self.set_register_value(&op, address as u32, &Size::Long);
                self.memory.read_size(address, size)
            }
            Operand::PostIndirect(op) => {
                let address = self.get_register_value(&op, &Size::Long) as usize;
                self.set_register_value(&op, address as u32 + size.to_bytes() as u32, &Size::Long);
                self.memory.read_size(address, size)
            }
            Operand::IndirectWithDisplacement { offset, operands } => {
                unimplemented!("IndirectWithDisplacement");
            }
        }
    }
    pub fn store_operand_value(&mut self, op: &Operand, value: u32, size: &Size) {
        match op {
            Operand::Immediate(_) => panic!("Cannot store value to immediate operand"),
            Operand::Register(op) => self.set_register_value(&op, value, size),
            Operand::Address(address) => self.memory.write_size(*address, size, value),
            Operand::Indirect { offset, operand } => {
                //TODO not sure if this works fine with full 32bits
                let address = self.get_register_value(&operand, &Size::Long) as i32 + offset;
                self.memory.write_size(address as usize, size, value)
            }
            Operand::PreIndirect(op) => {
                //TODO not sure if this works fine with full 32bits
                let address = self.get_register_value(&op, &Size::Long) as usize - size.to_bytes();
                self.set_register_value(&op, address as u32, &Size::Long);
                self.memory.write_size(address, size, value)
            }
            Operand::PostIndirect(op) => {
                //TODO not sure if this works fine with full 32bits
                let address = self.get_register_value(&op, &Size::Long) as usize;
                self.set_register_value(&op, address as u32 + size.to_bytes() as u32, &Size::Long);
                self.memory.write_size(address, size, value)
            }
            Operand::IndirectWithDisplacement { offset, operands } => {
                unimplemented!("IndirectWithDisplacement");
            }
        }
    }

    pub fn run(&mut self) {
        while !self.has_finished() {
            self.step();
        }
    }

    fn get_flag(&self, flag: Flags) -> bool {
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

    fn get_condition_value(&self, cond: &Condition) -> bool {
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

/*
Detecting overflows can be done with checked_add (returns None on overflow) or overflowing_add (returns a tuple of (wrapped_result, did_it_overflow)).
Also be aware of saturating_add (stops "just short" of overflowing, e.g. 250u8.saturating_add(10) == 255u8) and wrapping_add (explicitly wraps).
These operations all exist for sub and mul as well, and div has a checked variant (catches x / 0 and iX::MIN / -1)
*/
