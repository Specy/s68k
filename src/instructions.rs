use std::{
    fmt::Debug,
    str::FromStr,
};

use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::wasm_bindgen;

#[wasm_bindgen]
#[derive(Debug, Clone, Copy, Serialize, Eq, PartialEq)]
pub enum Size {
    Byte = 1,
    Word = 2,
    Long = 4,
}

#[wasm_bindgen]
#[derive(Debug, Clone, Copy, Serialize, Eq, PartialEq)]
pub enum TargetDirection {
    ToMemory,
    FromMemory,
}

impl Size {
    #[inline(always)]
    pub fn to_bytes(&self) -> usize {
        *self as usize
    }
    #[inline(always)]
    pub fn to_bits(&self) -> usize {
        *self as usize * 8
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Copy)]
#[serde(tag = "type", content = "value")]
pub enum RegisterOperand {
    Address(u8),
    Data(u8),
}

#[derive(Debug, Clone, Serialize, Copy)]
pub struct IndexRegister {
    pub register: RegisterOperand,
    //pub scale: u8,
    pub size: Size,
}

#[derive(Debug, Clone, Serialize, Copy)]
pub enum Operand {
    Immediate(u32),
    Register(RegisterOperand),
    Indirect(RegisterOperand),
    IndirectDisplacement {
        offset: i32,
        base: RegisterOperand,
    },
    IndirectIndex {
        base: RegisterOperand,
        offset: i32,
        index: IndexRegister,
    },
    PostIndirect(RegisterOperand),
    PreIndirect(RegisterOperand),
    Absolute(usize),
}
/*
Thanks to:  https://github.com/transistorfet/moa/blob/main/emulator/cpus/m68k/src/instructions.rs
for the Conditions and inspiration
 */


#[derive(Debug, Clone, Serialize)]
pub struct Label {
    pub name: String,
    pub address: usize,
    pub line: usize,
}

#[wasm_bindgen]
#[derive(Copy, Clone, Debug, Serialize)]
pub enum Condition {
    True,
    False,
    High,
    LowOrSame,
    CarryClear,
    CarrySet,
    NotEqual,
    Equal,
    OverflowClear,
    OverflowSet,
    Plus,
    Minus,
    GreaterThanOrEqual,
    LessThan,
    GreaterThan,
    LessThanOrEqual,
}

impl FromStr for Condition {
    type Err = String;
    fn from_str(s: &str) -> Result<Condition, Self::Err> {
        let s = s.to_lowercase();
        Ok(match s.as_str() {
            "t" => Condition::True,
            "f" => Condition::False,
            "hi" => Condition::High,
            "ls" => Condition::LowOrSame,
            "cc" | "hs" => Condition::CarryClear,
            "cs" | "lo" => Condition::CarrySet,
            "ne" => Condition::NotEqual,
            "eq" => Condition::Equal,
            "vc" => Condition::OverflowClear,
            "vs" => Condition::OverflowSet,
            "pl" => Condition::Plus,
            "mi" => Condition::Minus,
            "ge" => Condition::GreaterThanOrEqual,
            "lt" => Condition::LessThan,
            "gt" => Condition::GreaterThan,
            "le" => Condition::LessThanOrEqual,
            _ => return Err(format!("Invalid condition: {}", s)),
        })
    }
}

#[derive(Copy, Clone, Debug, Serialize)]
pub enum ShiftDirection {
    Right,
    Left,
}

#[derive(Copy, Clone, Debug, Serialize)]
pub enum Sign {
    Signed,
    Unsigned,
}

#[derive(Clone, Debug, Serialize, Copy)]
pub enum Instruction {
    ADDA(Operand, RegisterOperand, Size),
    SUBA(Operand, RegisterOperand, Size),
    CMPA(Operand, RegisterOperand, Size),
    MOVEA(Operand, RegisterOperand, Size), //add TAS()
    MOVEM{
        direction: TargetDirection,
        size: Size,
        registers_mask: u16,
        target: Operand,
    },
    MOVE(Operand, Operand, Size),
    ADD(Operand, Operand, Size),
    SUB(Operand, Operand, Size),
    ADDQ(u8, Operand, Size),
    MOVEQ(u8, RegisterOperand),
    SUBQ(u8, Operand, Size),
    ADDI(u32, Operand, Size),
    SUBI(u32, Operand, Size),
    ANDI(u32, Operand, Size),
    ORI(u32, Operand, Size),
    EORI(u32, Operand, Size),
    CMPI(u32, Operand, Size),
    CMPM(Operand, Operand, Size),
    DIVx(Operand, RegisterOperand, Sign),
    MULx(Operand, RegisterOperand, Sign),
    SWAP(RegisterOperand),
    CLR(Operand, Size),
    EXG(RegisterOperand, RegisterOperand),
    LEA(Operand, RegisterOperand),
    PEA(Operand),
    NEG(Operand, Size),
    EXT(RegisterOperand, Size, Size),
    TST(Operand, Size),
    CMP(Operand, RegisterOperand, Size),
    Bcc(u32, Condition),
    Scc(Operand, Condition),
    DBcc(RegisterOperand, u32, Condition),
    BRA(u32), //could use offset instead of address
    LINK(RegisterOperand, u32),
    UNLK(RegisterOperand),
    NOT(Operand, Size),
    OR(Operand, Operand, Size),
    AND(Operand, Operand, Size),
    EOR(Operand, Operand, Size),
    JSR(Operand),
    ASd(Operand, Operand, ShiftDirection, Size),
    ROd(Operand, Operand, ShiftDirection, Size),
    LSd(Operand, Operand, ShiftDirection, Size),
    BTST(Operand, Operand),
    BCLR(Operand, Operand),
    BSET(Operand, Operand),
    BCHG(Operand, Operand),
    JMP(Operand),
    BSR(u32),
    TRAP(u8),
    RTS,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "value")]
pub enum Interrupt {
    DisplayStringWithCRLF(String),
    DisplayStringWithoutCRLF(String),
    ReadKeyboardString,
    DisplayNumber(u32),
    ReadNumber,
    ReadChar,
    DisplayChar(char),
    GetTime,
    Terminate,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "value")]
pub enum InterruptResult {
    DisplayStringWithCRLF,
    DisplayStringWithoutCRLF,
    ReadKeyboardString(String),
    DisplayNumber,
    ReadNumber(i32),
    ReadChar(char),
    DisplayChar,
    GetTime(u32),
    Terminate,
}

impl Instruction {
    pub fn get_instruction_name(&self) -> String {
        let string = format!("{:?}", self);
        let mut string = string.split("(");
        string.next().unwrap().to_string()
    }
}
