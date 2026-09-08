//! What the Interpreter runs and what it asks the outside world for: the
//! encoded instruction types, and the interrupt catalogue.
//!
//! The instruction types themselves live in
//! [`crate::assembler::instructions::encoded`], where the Assembler builds
//! them; they are re-exported here so that the Interpreter and the debugger go
//! on naming them where they always did.
//!
//! The [`Interrupt`]s are the Interpreter's own: EASy68K's `trap #15` tasks,
//! one variant per task, with an [`InterruptResult`] per variant for the answer
//! the host gives back.

use serde::{Deserialize, Serialize};

pub use crate::assembler::instructions::encoded::{
    Condition, IndexRegister, Instruction, Operand, RegisterOperand, ShiftDirection, Sign, Size,
    TargetDirection,
};

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
    LastKeys { up: u8, down: u8 },
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
    SetPenColor(u32),  //80
    SetFillColor(u32), //81
    //coordinates are signed: EASy68K casts each one to a short before it draws, so a program may
    //draw off the left or the top of the screen and have the part that is on it clipped
    DrawPixel(i32, i32),                       //82
    GetPixelColor(i32, i32),                   //83
    DrawLine(i32, i32, i32, i32),              //84
    DrawLineTo(i32, i32),                      //85
    MoveTo(i32, i32),                          //86
    DrawRectangle(i32, i32, i32, i32),         //87
    DrawEllipse(i32, i32, i32, i32),           //88
    FloodFill(i32, i32),                       //89
    DrawUnfilledRectangle(i32, i32, i32, i32), //90
    DrawUnfilledEllipse(i32, i32, i32, i32),   //91
    SetDrawingMode(u8), //92, 2 move only, 4 draw, 16 and 17 double buffering off and on
    SetPenWidth(u32),   //93
    Repaint,            //94, shows the off screen buffer of drawing mode 17
    DrawText(i32, i32, String), //95
    GetPenPosition,     //96
    SetScreenSize(u32, u32), //33
    GetScreenSize,      //33 with D1.L = 0
    SetScreenMode(u8),  //33 with D1.L = 1 windowed or 2 full screen, no-op here
    ClearScreen,        //11 with D1.W = $FF00
    SetTextCursorPosition(u32, u32), //11, column and row
    GetTextCursorPosition, //11 with D1.W = $00FF
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
    GetPenPosition(i32, i32),
    SetScreenSize,
    GetScreenSize(u32, u32),
    SetScreenMode,
    ClearScreen,
    SetTextCursorPosition,
    GetTextCursorPosition(u32, u32),
}
