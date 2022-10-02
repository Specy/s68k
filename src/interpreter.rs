use crate::pre_interpreter::InstructionLine;

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
    pub fn read_long(&self,  address: usize) -> u32 {
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
}

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
}
pub struct Ccr {
    data: u8,
}
pub struct Cpu {
    d_reg: [Register; 8],
    a_reg: [Register; 8],
    ccr: Ccr,
}
pub struct Interpreter {
    memory: Memory,
    cpu: Cpu,
    pc: usize,
    program: Vec<InstructionLine>,
}


/*
Detecting overflows can be done with checked_add (returns None on overflow) or overflowing_add (returns a tuple of (wrapped_result, did_it_overflow)). 
Also be aware of saturating_add (stops "just short" of overflowing, e.g. 250u8.saturating_add(10) == 255u8) and wrapping_add (explicitly wraps). 
These operations all exist for sub and mul as well, and div has a checked variant (catches x / 0 and iX::MIN / -1)
*/