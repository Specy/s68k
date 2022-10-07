use std::{
    fmt::{self, Debug},
    str::FromStr,
};

use serde::Serialize;
#[derive(Debug, Clone, Serialize)]
pub enum Size {
    Byte,
    Word,
    Long,
}
impl Size {
    pub fn to_bytes(&self) -> usize {
        match self {
            Size::Byte => 1,
            Size::Word => 2,
            Size::Long => 4,
        }
    }
    pub fn to_bits(&self) -> usize {
        self.to_bytes() * 8
    }
}

#[derive(Debug, Clone, Serialize)]
pub enum RegisterOperand {
    Address(u8),
    Data(u8),
}

#[derive(Debug, Clone, Serialize)]
pub enum Operand {
    Register(RegisterOperand),
    Immediate(u32),
    Indirect {
        offset: i32,
        operand: RegisterOperand,
    },
    IndirectWithDisplacement {
        offset: i32,
        operands: Vec<RegisterOperand>,
    },
    PostIndirect(RegisterOperand),
    PreIndirect(RegisterOperand),
    Address(usize),
}
/*
Thanks to:  https://github.com/transistorfet/moa/blob/main/emulator/cpus/m68k/src/instructions.rs
for the Conditions and inspiration
 */
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
            "cc" => Condition::CarryClear,
            "cs" => Condition::CarrySet,
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

#[derive(Clone, Debug, Serialize)]
pub enum Instruction {
    MOVE(Operand, Operand, Size),
    ADD(Operand, Operand, Size),
    SUB(Operand, Operand, Size),
    ADDA(Operand, RegisterOperand, Size),
    SUBA(Operand, RegisterOperand, Size),
    DIVx(Operand, RegisterOperand, Sign),
    MULx(Operand, RegisterOperand, Sign),
    SWAP(RegisterOperand),
    CLR(Operand, Size),
    EXG(RegisterOperand, RegisterOperand),
    NEG(Operand, Size),
    EXT(RegisterOperand, Size, Size),
    TST(Operand, Size),
    CMP(Operand, Operand, Size),
    Bcc(u32, Condition),
    BRA(u32), //could use offset instead of address
    Scc(Operand, Condition),
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
    RTS,
}

impl Instruction {
    pub fn get_instruction_name(&self) -> String {
        let string = format!("{:?}", self);
        let mut string = string.split("(");
        string.next().unwrap().to_string()
    }
}
