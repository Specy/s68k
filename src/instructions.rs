use std::fmt::{self, Debug};
#[derive(Debug, Clone)]
pub enum RegisterType {
    Address,
    Data,
}
#[derive(Debug, Clone)]
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

#[derive(Debug, Clone)]
pub enum RegisterOperand {
    Address(u8),
    Data(u8),
}

#[derive(Debug, Clone)]
pub enum Operand {
    Register(RegisterOperand), //maybe use usize?
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
#[derive(Copy, Clone, Debug, PartialEq)]
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
impl Condition {
    pub fn from_string(s: &str) -> Result<Condition, String> {
        let s = s.to_lowercase();
        match s.as_str() {
            "t" => Ok(Condition::True),
            "f" => Ok(Condition::False),
            "hi" => Ok(Condition::High),
            "ls" => Ok(Condition::LowOrSame),
            "cc" => Ok(Condition::CarryClear),
            "cs" => Ok(Condition::CarrySet),
            "ne" => Ok(Condition::NotEqual),
            "eq" => Ok(Condition::Equal),
            "vc" => Ok(Condition::OverflowClear),
            "vs" => Ok(Condition::OverflowSet),
            "pl" => Ok(Condition::Plus),
            "mi" => Ok(Condition::Minus),
            "ge" => Ok(Condition::GreaterThanOrEqual),
            "lt" => Ok(Condition::LessThan),
            "gt" => Ok(Condition::GreaterThan),
            "le" => Ok(Condition::LessThanOrEqual),
            _ => Err(format!("Invalid condition: {}", s)),
        }
    }
}
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum ShiftDirection {
    Right,
    Left,
}
#[derive(Clone, Debug)]
pub enum Instruction {
    MOVE(Operand, Operand, Size),
    ADD(Operand, Operand, Size),
    SUB(Operand, Operand, Size),
    ADDA(Operand, RegisterOperand, Size),
    DIVS(Operand, RegisterOperand),
    DIVU(Operand, RegisterOperand),
    MULS(Operand, RegisterOperand),
    MULU(Operand, RegisterOperand),
    SWAP(RegisterOperand),
    CLR(Operand, Size),
    EXG(RegisterOperand, Operand),
    NEG(Operand, Size),
    EXT(RegisterOperand, Size),
    TST(Operand, Size),
    CMP(Operand, Operand, Size),
    Bcc(Operand, Condition),
    Scc(Operand, Condition),
    NOT(Operand),
    OR(Operand, Operand),
    AND(Operand, Operand),
    EOR(Operand, Operand),
    JSR(Operand),
    ASd(Operand, Operand, ShiftDirection, Size),
    ROd(Operand, Operand, ShiftDirection, Size),
    LSd(Operand, Operand, ShiftDirection, Size),
    BTST(Operand, Operand),
    BCLR(Operand, Operand),
    BSET(Operand, Operand),
    BCHG(Operand, Operand),
    RTS,
}

impl Instruction {
    pub fn get_instruction_name(&self) -> String {
        let string = format!("{:?}", self);
        let mut string = string.split("(");
        string.next().unwrap().to_string()
    }
}
