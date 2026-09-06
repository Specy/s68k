use std::{fmt::Debug, str::FromStr};

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
impl RegisterOperand {
    pub fn to_index(&self) -> u16 {
        match self {
            RegisterOperand::Address(index) => *index as u16 + 8,
            RegisterOperand::Data(index) => *index as u16,
        }
    }
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
    Indirect(u8),
    PostIndirect(u8),
    PreIndirect(u8),
    IndirectDisplacement {
        offset: i32,
        base: RegisterOperand,
    },
    IndirectIndex {
        base: RegisterOperand,
        offset: i32,
        index: IndexRegister,
    },

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
    MOVEM {
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
    NOP,
}

/// Which form of task 19 the program asked for, decided by D1.L: EASy68K reads
/// four key codes packed one per byte, or the codes of the last keys when D1.L is zero.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "value")]
pub enum KeyStateRequest {
    Keys([u8; 4]),
    LastKeys,
}

/// The two answers task 19 accepts, one per request form of [`KeyStateRequest`].
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "value")]
pub enum KeyStateResult {
    /// Whether each requested key is down, in the order the codes were given
    Keys([bool; 4]),
    /// Codes of the last key released and of the last key pressed
    LastKeys {
        up: u8,
        down: u8,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "value")]
pub enum Interrupt {
    DisplayStringWithCRLF(String),
    DisplayStringWithoutCRLF(String),
    ReadKeyboardString,
    DisplayNumber(i32),
    DisplayNumberInBase {
        value: u32,
        base: u8,
    },
    ReadNumber,
    ReadChar,
    DisplayChar(char),
    GetTime,
    Terminate,
    Delay(u32),
    DisplaySignedNumberInField {
        //20
        value: i32,
        width: u8,
    },
    DisplayStringAndNumber {
        //17, tasks 14 and 3 in one trap
        string: String,
        number: i32,
    },
    DisplayStringAndReadNumber(String), //18, tasks 14 and 4 in one trap

    // keyboard and mouse
    CheckKeyboardInput,           //7
    GetKeyState(KeyStateRequest), //19
    ReadMouse(u8),                //61, 0 current state, 1 last button up, 2 last button down
    SetSimulatorShortcuts(u32),   //24, no-op: the screen already receives every key

    // graphics
    SetPenColor(u32),                          //80
    SetFillColor(u32),                         //81
    DrawPixel(u32, u32),                       //82
    GetPixelColor(u32, u32),                   //83
    DrawLine(u32, u32, u32, u32),              //84
    DrawLineTo(u32, u32),                      //85
    MoveTo(u32, u32),                          //86
    DrawRectangle(u32, u32, u32, u32),         //87
    DrawEllipse(u32, u32, u32, u32),           //88
    FloodFill(u32, u32),                       //89
    DrawUnfilledRectangle(u32, u32, u32, u32), //90
    DrawUnfilledEllipse(u32, u32, u32, u32),   //91
    SetDrawingMode(u8),                        //92, 2 move only, 4 draw, 16 and 17 double buffering off and on
    SetPenWidth(u32),                          //93
    Repaint,                                   //94, shows the off screen buffer of drawing mode 17
    DrawText(u32, u32, String),                //95
    GetPenPosition,                            //96
    SetScreenSize(u32, u32),                   //33
    GetScreenSize,                             //33 with D1.L = 0
    SetScreenMode(u8),                         //33 with D1.L = 1 windowed or 2 full screen, no-op here
    ClearScreen,                               //11 with D1.W = $FF00
    SetTextCursorPosition(u32, u32),           //11, column and row
    GetTextCursorPosition,                     //11 with D1.W = $00FF
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "value")]
pub enum InterruptResult {
    DisplayStringWithCRLF,
    DisplayStringWithoutCRLF,
    ReadKeyboardString(String),
    DisplayNumber,
    DisplayNumberInBase,
    ReadNumber(i32),
    ReadChar(char),
    DisplayChar,
    GetTime(u32),
    Terminate,
    Delay,
    DisplaySignedNumberInField,
    DisplayStringAndNumber,
    DisplayStringAndReadNumber(i32),

    // keyboard and mouse
    CheckKeyboardInput(bool),
    GetKeyState(KeyStateResult),
    /// Button and modifier flags, and the position in screen pixels
    ReadMouse {
        flags: u8,
        x: u16,
        y: u16,
    },
    SetSimulatorShortcuts,

    // graphics
    SetPenColor,
    SetFillColor,
    DrawPixel,
    GetPixelColor(u32),
    DrawLine,
    DrawLineTo,
    MoveTo,
    DrawRectangle,
    DrawEllipse,
    FloodFill,
    DrawUnfilledRectangle,
    DrawUnfilledEllipse,
    SetDrawingMode,
    SetPenWidth,
    Repaint,
    DrawText,
    GetPenPosition(u32, u32),
    SetScreenSize,
    GetScreenSize(u32, u32),
    SetScreenMode,
    ClearScreen,
    SetTextCursorPosition,
    GetTextCursorPosition(u32, u32),
}

impl Instruction {
    pub fn get_instruction_name(&self) -> String {
        let string = format!("{:?}", self);
        let mut string = string.split('(');
        string.next().unwrap().to_string()
    }
}
