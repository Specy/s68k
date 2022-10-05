use std::fmt::{self, Debug};
#[derive(Debug, Clone)]
pub enum RegisterType {
    Address,
    Data,
}
#[derive(Debug, Clone)]
pub enum Size{
    Byte,
    Word,
    Long,
}
impl Size{
    pub fn to_bits(&self) -> usize{
        match self{
            Size::Byte => 8,
            Size::Word => 16,
            Size::Long => 32,
        }
    }
    pub fn to_bytes(&self) -> usize{
        match self{
            Size::Byte => 1,
            Size::Word => 2,
            Size::Long => 4,
        }
    }
}



#[derive(Debug, Clone)]
pub enum Operand {
    Register(RegisterType, u8), //maybe use usize?
    Immediate(i32),
    Indirect {
        offset: String,
        operand: Box<Operand>,
    },
    IndirectWithDisplacement {
        offset: i32,
        operands: Vec<Operand>,
    },
    PostIndirect(Box<Operand>),
    PreIndirect(Box<Operand>),
    Address(u32), //maybe use usize?
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
impl Condition{
    pub fn from_string(s: &str) -> Result<Condition, String>{
        let s = s.to_lowercase();
        match s.as_str(){
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
pub enum Instruction{
    MOVE(Operand, Operand, Size),
    ADD(Operand, Operand, Size),
    SUB(Operand, Operand, Size),
    ADDA(Operand, Operand, Size),
    DIVS(Operand, Operand),
    DIVU(Operand, Operand),
    MULS(Operand, Operand),
    MULU(Operand, Operand),
    SWAP(Operand),
    CLR(Operand, Size),
    EXG(Operand, Operand),
    NEG(Operand, Size),
    EXT(Operand, Size),
    TST(Operand, Size),
    CMP(Operand, Operand, Size),
    Bcc(Operand, Condition),
    Scc(Operand, Condition),
    NOT(Operand),
    OR(Operand, Operand),
    AND(Operand, Operand),
    EOR(Operand, Operand),
    JSR(Operand),
    ASd(Operand, Operand,ShiftDirection, Size),
    ROd(Operand, Operand,ShiftDirection, Size),
    LSd(Operand, Operand,ShiftDirection, Size),
    BTST(Operand, Operand),
    BCLR(Operand, Operand),
    BSET(Operand, Operand),
    BCHG(Operand, Operand),
    RTS,
}

impl Instruction {
    pub fn get_instruction_name(&self) -> String{
        let string = format!("{:?}", self);
        let string = string.split("(").collect::<Vec<&str>>();
        string[0].to_string()
    }
}