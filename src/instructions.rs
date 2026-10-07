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
//!
//! The Interpreter owns what a task means and the host only carries text,
//! keys, files and sounds: a display task hands over the text to display,
//! formatted and decoded the way EASy68K does it, a read task takes back what
//! was typed, as it was typed, and reads it the way EASy68K does
//! ([`charset`](crate::charset) for the characters, `c_runtime` for the
//! numbers), and a file task hands over the operation with its arguments
//! decoded and takes back what the host's file system did, which the
//! Interpreter turns into EASy68K's result codes.

use serde::{de, Deserialize, Deserializer, Serialize, Serializer};

pub use crate::assembler::instructions::encoded::{
    Condition, IndexRegister, Instruction, Operand, RegisterOperand, ShiftDirection, Sign, Size,
    TargetDirection, EXTENSION_WORD_OFFSET,
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

/// A run of bytes that crosses into JavaScript as a `Uint8Array`, and is read
/// back from one.
///
/// `serde` writes a `Vec<u8>` as a sequence, which `serde_wasm_bindgen` turns
/// into an array of numbers, one JavaScript value per byte. This writes it as
/// serde's bytes instead, which `serde_wasm_bindgen` copies into a
/// `Uint8Array` in one go, and reads it back from a `Uint8Array` (or an
/// `ArrayBuffer`, or an array of numbers) the same way: what `serde_bytes`
/// does, in a dozen lines rather than a dependency. JSON, which has no bytes,
/// writes and reads an array of numbers.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Bytes(pub Vec<u8>);

impl Bytes {
    /// The bytes themselves.
    pub fn as_slice(&self) -> &[u8] {
        &self.0
    }
}

impl From<Vec<u8>> for Bytes {
    fn from(bytes: Vec<u8>) -> Self {
        Bytes(bytes)
    }
}

impl Serialize for Bytes {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_bytes(&self.0)
    }
}

impl<'de> Deserialize<'de> for Bytes {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct BytesVisitor;
        impl<'de> de::Visitor<'de> for BytesVisitor {
            type Value = Bytes;
            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("bytes: a Uint8Array or an array of numbers 0 to 255")
            }
            fn visit_bytes<E: de::Error>(self, bytes: &[u8]) -> Result<Bytes, E> {
                Ok(Bytes(bytes.to_vec()))
            }
            fn visit_byte_buf<E: de::Error>(self, bytes: Vec<u8>) -> Result<Bytes, E> {
                Ok(Bytes(bytes))
            }
            fn visit_seq<A: de::SeqAccess<'de>>(self, mut sequence: A) -> Result<Bytes, A::Error> {
                let mut bytes = Vec::with_capacity(sequence.size_hint().unwrap_or(0));
                while let Some(byte) = sequence.next_element::<u8>()? {
                    bytes.push(byte);
                }
                Ok(Bytes(bytes))
            }
        }
        deserializer.deserialize_byte_buf(BytesVisitor)
    }
}

/// How EASy68K shows the input of the read tasks (2, 4, 5 and 18): what tasks
/// 12 and 16 set, and what the host's terminal honours while it waits for a
/// line or a key.
///
/// It is the Interpreter's state, journaled like a register so that undo puts
/// it back, and every run starts with all three on, as EASy68K's `initSim`
/// leaves them (`STARTSIM.CPP`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct InputSettings {
    /// Task 12: whether what is typed is echoed. EASy68K still moves to a new
    /// line when Enter ends a line read with the echo off.
    pub echo: bool,
    /// Task 16 with D1.B 0 or 1: whether the input prompt, EASy68K's flashing
    /// cursor, shows while a read waits.
    pub prompt: bool,
    /// Task 16 with D1.B 2 or 3: whether a key read (task 5) of Enter echoes a
    /// line feed after the carriage return. It changes nothing with the echo
    /// off.
    pub line_feed: bool,
}

impl Default for InputSettings {
    fn default() -> Self {
        Self {
            echo: true,
            prompt: true,
            line_feed: true,
        }
    }
}

/// Which dialog task 58 shows, from D1.L.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FileDialogMode {
    /// D1.L = 0: choose a file to open.
    Open,
    /// D1.L = 1: choose a file to save to.
    Save,
}

/// A file the host opened for task 51 or 52.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenedFile {
    /// The file number the program gets in D1.L: 0 to 7, as EASy68K numbers
    /// its eight files.
    pub handle: u8,
    /// Whether the file could only be opened for reading. Only task 51 opens
    /// a file that way, and it reports 3 for it.
    pub read_only: bool,
}

/// What task 59 found at a path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FileExistence {
    /// A file that can be read and written: D0.W = 0.
    Writable,
    /// A file that can only be read: D0.W = 3.
    ReadOnly,
    /// No file, or nothing that can be opened as one: D0.W = 2.
    Missing,
}

/// What a `trap #15` asks the host for.
///
/// Every text a display task carries is ready to display: decoded from
/// Windows-1252 and, for a number, formatted with EASy68K's rules. A read task
/// shows what is typed as the [`InputSettings`] say, which the host reads from
/// the Interpreter while it waits. A file or sound task carries its arguments
/// decoded: a path is the NUL terminated string at the address, at most 255
/// characters, decoded from Windows-1252 and with `\` written as `/`, since
/// Windows takes both as a separator.
///
/// Every variant has an [`InterruptResult`] of the same name, which is the only
/// answer it takes apart from [`InterruptResult::Terminate`].
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "value")]
pub enum Interrupt {
    /// Tasks 0 and 13: display the text, then a new line. Task 0's is the D1.W
    /// characters at (A1), at most 255 and stopping at a NUL; task 13's is the
    /// NUL terminated string at (A1).
    DisplayStringWithCRLF(String),
    /// Tasks 1 and 14: the same without the new line.
    DisplayStringWithoutCRLF(String),
    /// Task 2: read a line into the buffer at (A1). Answered by
    /// [`InterruptResult::ReadKeyboardString`].
    ReadKeyboardString,
    /// Task 3: D1.L as a signed decimal number.
    DisplayNumber(String),
    /// Task 15: D1.L as an unsigned number in the base in D2.B, upper case.
    DisplayNumberInBase(String),
    /// Task 4: read a number into D1.L. Answered by
    /// [`InterruptResult::ReadNumber`].
    ReadNumber,
    /// Task 5: read one character into D1.B. Answered by
    /// [`InterruptResult::ReadChar`].
    ReadChar,
    /// Task 6: the character in D1.B.
    DisplayChar(char),
    GetTime,
    Terminate,
    Delay(u32),
    /// Task 20: D1.L as a signed decimal number in a field of D2.B columns,
    /// spaces on the left, or on the right when D2.B is negative.
    DisplaySignedNumberInField(String),
    /// Task 17: the NUL terminated string at (A1) and then D1.L as a signed
    /// decimal number, tasks 14 and 3 in one trap, as one text.
    DisplayStringAndNumber(String),
    /// Task 18: display the NUL terminated string at (A1), then read a number
    /// into D1.L, tasks 14 and 4 in one trap. Answered by
    /// [`InterruptResult::DisplayStringAndReadNumber`].
    DisplayStringAndReadNumber(String),

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
    DrawText(i32, i32, String), //95, the NUL terminated string at (A1), decoded from Windows-1252
    GetPenPosition,     //96
    SetScreenSize(u32, u32), //33
    GetScreenSize,      //33 with D1.L = 0
    SetScreenMode(u8),  //33 with D1.L = 1 windowed or 2 full screen, no-op here
    ClearScreen,        //11 with D1.W = $FF00
    SetTextCursorPosition(u32, u32), //11, column and row
    GetTextCursorPosition, //11 with D1.W = $00FF

    // files, tasks 50 to 59: the host does them on its own file system, and the
    // Interpreter writes EASy68K's result, 0 success, 1 end of file, 2 error and
    // 3 read only, to D0.W. A file number outside 0 to 7, a count of zero, a
    // buffer past the end of memory and a negative position are answered 2 by
    // the Interpreter itself, without asking the host.
    /// Task 50: close every open file.
    CloseAllFiles,
    /// Task 51: open the existing file at the path for reading and writing, or
    /// for reading only when it cannot be written.
    OpenFile(String),
    /// Task 52: open the file at the path for reading and writing, creating it
    /// when there is none and emptying it when there is one.
    NewFile(String),
    /// Task 53: read at most `count` bytes, D2.L, from the file's position.
    ReadFile {
        handle: u8,
        count: u32,
    },
    /// Task 54: write the D2.L bytes at (A1) at the file's position.
    WriteFile {
        handle: u8,
        bytes: Bytes,
    },
    /// Task 55: move the file's position to `offset` bytes from its start,
    /// D2.L.
    PositionFile {
        handle: u8,
        offset: u32,
    },
    /// Task 56: close the file.
    CloseFile(u8),
    /// Task 57: delete the file at the path.
    DeleteFile(String),
    /// Task 58: let the user choose a file. The title at (A1) and the filter
    /// at (A2), such as `*.txt`, are empty when their register is 0; the path
    /// at (A3) is the file the dialog starts on.
    FileDialog {
        mode: FileDialogMode,
        title: String,
        filter: String,
        path: String,
    },
    /// Task 59: whether there is a file at the path, and whether it can be
    /// written.
    FileExists(String),

    // sound, tasks 70 to 77: the arguments only, until the host has a sound
    // device to play them on. A sound in memory is numbered 0 to 255, D1.B.
    /// Task 70: play the WAV file at the path.
    PlaySound(String),
    /// Task 71: load the WAV file at the path into sound memory `index`.
    LoadSound {
        path: String,
        index: u8,
    },
    /// Task 72: play the sound loaded into `index`.
    PlayLoadedSound(u8),
    /// Task 73: play the WAV file at the path with EASy68K's DirectX player.
    PlaySoundDirectX(String),
    /// Task 74: load the WAV file at the path into DirectX sound memory `index`.
    LoadSoundDirectX {
        path: String,
        index: u8,
    },
    /// Task 75: play the DirectX sound loaded into `index`.
    PlayLoadedSoundDirectX(u8),
    /// Task 76: control the standard player, D2.L: 0 plays sound `index` once,
    /// 1 loops it, 2 stops it and 3 stops every sound.
    ControlSound {
        index: u8,
        control: u32,
    },
    /// Task 77: the same for the DirectX player.
    ControlSoundDirectX {
        index: u8,
        control: u32,
    },
}

impl Interrupt {
    /// The variant's name, which is the `type` it crosses into JavaScript with
    /// and the name of the one [`InterruptResult`] that answers it.
    pub fn name(&self) -> &'static str {
        match self {
            Interrupt::DisplayStringWithCRLF(_) => "DisplayStringWithCRLF",
            Interrupt::DisplayStringWithoutCRLF(_) => "DisplayStringWithoutCRLF",
            Interrupt::ReadKeyboardString => "ReadKeyboardString",
            Interrupt::DisplayNumber(_) => "DisplayNumber",
            Interrupt::DisplayNumberInBase(_) => "DisplayNumberInBase",
            Interrupt::ReadNumber => "ReadNumber",
            Interrupt::ReadChar => "ReadChar",
            Interrupt::DisplayChar(_) => "DisplayChar",
            Interrupt::GetTime => "GetTime",
            Interrupt::Terminate => "Terminate",
            Interrupt::Delay(_) => "Delay",
            Interrupt::DisplaySignedNumberInField(_) => "DisplaySignedNumberInField",
            Interrupt::DisplayStringAndNumber(_) => "DisplayStringAndNumber",
            Interrupt::DisplayStringAndReadNumber(_) => "DisplayStringAndReadNumber",
            Interrupt::CheckKeyboardInput => "CheckKeyboardInput",
            Interrupt::GetKeyState(_) => "GetKeyState",
            Interrupt::ReadMouse(_) => "ReadMouse",
            Interrupt::SetSimulatorShortcuts(_) => "SetSimulatorShortcuts",
            Interrupt::SetPenColor(_) => "SetPenColor",
            Interrupt::SetFillColor(_) => "SetFillColor",
            Interrupt::DrawPixel(_, _) => "DrawPixel",
            Interrupt::GetPixelColor(_, _) => "GetPixelColor",
            Interrupt::DrawLine(_, _, _, _) => "DrawLine",
            Interrupt::DrawLineTo(_, _) => "DrawLineTo",
            Interrupt::MoveTo(_, _) => "MoveTo",
            Interrupt::DrawRectangle(_, _, _, _) => "DrawRectangle",
            Interrupt::DrawEllipse(_, _, _, _) => "DrawEllipse",
            Interrupt::FloodFill(_, _) => "FloodFill",
            Interrupt::DrawUnfilledRectangle(_, _, _, _) => "DrawUnfilledRectangle",
            Interrupt::DrawUnfilledEllipse(_, _, _, _) => "DrawUnfilledEllipse",
            Interrupt::SetDrawingMode(_) => "SetDrawingMode",
            Interrupt::SetPenWidth(_) => "SetPenWidth",
            Interrupt::Repaint => "Repaint",
            Interrupt::DrawText(_, _, _) => "DrawText",
            Interrupt::GetPenPosition => "GetPenPosition",
            Interrupt::SetScreenSize(_, _) => "SetScreenSize",
            Interrupt::GetScreenSize => "GetScreenSize",
            Interrupt::SetScreenMode(_) => "SetScreenMode",
            Interrupt::ClearScreen => "ClearScreen",
            Interrupt::SetTextCursorPosition(_, _) => "SetTextCursorPosition",
            Interrupt::GetTextCursorPosition => "GetTextCursorPosition",
            Interrupt::CloseAllFiles => "CloseAllFiles",
            Interrupt::OpenFile(_) => "OpenFile",
            Interrupt::NewFile(_) => "NewFile",
            Interrupt::ReadFile { .. } => "ReadFile",
            Interrupt::WriteFile { .. } => "WriteFile",
            Interrupt::PositionFile { .. } => "PositionFile",
            Interrupt::CloseFile(_) => "CloseFile",
            Interrupt::DeleteFile(_) => "DeleteFile",
            Interrupt::FileDialog { .. } => "FileDialog",
            Interrupt::FileExists(_) => "FileExists",
            Interrupt::PlaySound(_) => "PlaySound",
            Interrupt::LoadSound { .. } => "LoadSound",
            Interrupt::PlayLoadedSound(_) => "PlayLoadedSound",
            Interrupt::PlaySoundDirectX(_) => "PlaySoundDirectX",
            Interrupt::LoadSoundDirectX { .. } => "LoadSoundDirectX",
            Interrupt::PlayLoadedSoundDirectX(_) => "PlayLoadedSoundDirectX",
            Interrupt::ControlSound { .. } => "ControlSound",
            Interrupt::ControlSoundDirectX { .. } => "ControlSoundDirectX",
        }
    }
}

/// The host's answer to an [`Interrupt`], one variant per variant.
///
/// A read task is answered with what was typed, and the Interpreter reads it
/// with EASy68K's rules: a line, without the Enter that ended it, or a key. A
/// file task is answered with what the host's file system did — `false` or
/// `None` when it could not do it — and the Interpreter writes EASy68K's result
/// code for it.
///
/// An answer is taken only by the interrupt of the same name, except
/// [`InterruptResult::Terminate`], which answers any of them.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "value")]
pub enum InterruptResult {
    DisplayStringWithCRLF,
    DisplayStringWithoutCRLF,
    /// Task 2: the line typed. Its first 79 characters are stored at (A1) in
    /// Windows-1252, `?` for a character that has no byte, then a NUL, and
    /// their count goes in D1.L. A line ends at its first line terminator.
    ReadKeyboardString(String),
    DisplayNumber,
    DisplayNumberInBase,
    /// Task 4: the line typed, which `atoi` reads into D1.L: `"12abc"` is 12,
    /// and a line with no number is 0, never an error.
    ReadNumber(String),
    /// Task 5: the key typed, stored in D1.B in Windows-1252, `?` when it has
    /// no byte. Enter is `$0D`, whether it is answered as `'\r'` or `'\n'`.
    ReadChar(char),
    DisplayChar,
    GetTime(u32),
    Terminate,
    Delay,
    DisplaySignedNumberInField,
    DisplayStringAndNumber,
    /// Task 18: the line typed, read as [`InterruptResult::ReadNumber`] reads it.
    DisplayStringAndReadNumber(String),

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

    // files
    /// Task 50: whether every file closed. D0.W is 0, or 2 when one did not.
    CloseAllFiles(bool),
    /// Task 51: the file opened, or `None` when it could not be: D1.L is its
    /// number and D0.W 0, or 3 when it is open for reading only; D1.L is -1
    /// and D0.W 2 when there is none.
    OpenFile(Option<OpenedFile>),
    /// Task 52: the number of the file opened, or `None` when it could not be
    /// created: D1.L and D0.W as for task 51.
    NewFile(Option<u8>),
    /// Task 53: the bytes read, at most the count asked for, or `None` when
    /// the read failed. Some bytes go to (A1), their count to D2.L and 0 to
    /// D0.W, a short read included; none at all is the end of the file, 1 in
    /// D0.W and D2.L left as it was; `None` is 2.
    ReadFile(Option<Bytes>),
    /// Task 54: whether all the bytes were written. D0.W is 0, or 2.
    WriteFile(bool),
    /// Task 55: whether the position moved. D0.W is 0, or 2.
    PositionFile(bool),
    /// Task 56: whether the file closed. D0.W is 0, or 2.
    CloseFile(bool),
    /// Task 57: whether the file was deleted. D0.W is 0, or 2.
    DeleteFile(bool),
    /// Task 58: the path chosen, or `None` when the dialog was cancelled. A
    /// path is written to (A3), at most 255 characters in Windows-1252 and NULs
    /// to 256 bytes, with 1 in D1.L; a cancel puts 0 in D1.L. D0.W is 0, or 2
    /// when the 256 bytes do not fit in memory.
    FileDialog(Option<String>),
    /// Task 59: what is at the path. D0.W is 0, 3 or 2.
    FileExists(FileExistence),

    // sound: whether it happened, which is 1 or 0 in D0.W
    PlaySound(bool),
    /// Task 71 writes no result.
    LoadSound,
    PlayLoadedSound(bool),
    PlaySoundDirectX(bool),
    LoadSoundDirectX(bool),
    PlayLoadedSoundDirectX(bool),
    ControlSound(bool),
    ControlSoundDirectX(bool),
}

impl InterruptResult {
    /// The variant's name, which is the `type` it crosses from JavaScript with
    /// and the name of the [`Interrupt`] it answers.
    pub fn name(&self) -> &'static str {
        match self {
            InterruptResult::DisplayStringWithCRLF => "DisplayStringWithCRLF",
            InterruptResult::DisplayStringWithoutCRLF => "DisplayStringWithoutCRLF",
            InterruptResult::ReadKeyboardString(_) => "ReadKeyboardString",
            InterruptResult::DisplayNumber => "DisplayNumber",
            InterruptResult::DisplayNumberInBase => "DisplayNumberInBase",
            InterruptResult::ReadNumber(_) => "ReadNumber",
            InterruptResult::ReadChar(_) => "ReadChar",
            InterruptResult::DisplayChar => "DisplayChar",
            InterruptResult::GetTime(_) => "GetTime",
            InterruptResult::Terminate => "Terminate",
            InterruptResult::Delay => "Delay",
            InterruptResult::DisplaySignedNumberInField => "DisplaySignedNumberInField",
            InterruptResult::DisplayStringAndNumber => "DisplayStringAndNumber",
            InterruptResult::DisplayStringAndReadNumber(_) => "DisplayStringAndReadNumber",
            InterruptResult::CheckKeyboardInput(_) => "CheckKeyboardInput",
            InterruptResult::GetKeyState(_) => "GetKeyState",
            InterruptResult::ReadMouse { .. } => "ReadMouse",
            InterruptResult::SetSimulatorShortcuts => "SetSimulatorShortcuts",
            InterruptResult::SetPenColor => "SetPenColor",
            InterruptResult::SetFillColor => "SetFillColor",
            InterruptResult::DrawPixel => "DrawPixel",
            InterruptResult::GetPixelColor(_) => "GetPixelColor",
            InterruptResult::DrawLine => "DrawLine",
            InterruptResult::DrawLineTo => "DrawLineTo",
            InterruptResult::MoveTo => "MoveTo",
            InterruptResult::DrawRectangle => "DrawRectangle",
            InterruptResult::DrawEllipse => "DrawEllipse",
            InterruptResult::FloodFill => "FloodFill",
            InterruptResult::DrawUnfilledRectangle => "DrawUnfilledRectangle",
            InterruptResult::DrawUnfilledEllipse => "DrawUnfilledEllipse",
            InterruptResult::SetDrawingMode => "SetDrawingMode",
            InterruptResult::SetPenWidth => "SetPenWidth",
            InterruptResult::Repaint => "Repaint",
            InterruptResult::DrawText => "DrawText",
            InterruptResult::GetPenPosition(_, _) => "GetPenPosition",
            InterruptResult::SetScreenSize => "SetScreenSize",
            InterruptResult::GetScreenSize(_, _) => "GetScreenSize",
            InterruptResult::SetScreenMode => "SetScreenMode",
            InterruptResult::ClearScreen => "ClearScreen",
            InterruptResult::SetTextCursorPosition => "SetTextCursorPosition",
            InterruptResult::GetTextCursorPosition(_, _) => "GetTextCursorPosition",
            InterruptResult::CloseAllFiles(_) => "CloseAllFiles",
            InterruptResult::OpenFile(_) => "OpenFile",
            InterruptResult::NewFile(_) => "NewFile",
            InterruptResult::ReadFile(_) => "ReadFile",
            InterruptResult::WriteFile(_) => "WriteFile",
            InterruptResult::PositionFile(_) => "PositionFile",
            InterruptResult::CloseFile(_) => "CloseFile",
            InterruptResult::DeleteFile(_) => "DeleteFile",
            InterruptResult::FileDialog(_) => "FileDialog",
            InterruptResult::FileExists(_) => "FileExists",
            InterruptResult::PlaySound(_) => "PlaySound",
            InterruptResult::LoadSound => "LoadSound",
            InterruptResult::PlayLoadedSound(_) => "PlayLoadedSound",
            InterruptResult::PlaySoundDirectX(_) => "PlaySoundDirectX",
            InterruptResult::LoadSoundDirectX(_) => "LoadSoundDirectX",
            InterruptResult::PlayLoadedSoundDirectX(_) => "PlayLoadedSoundDirectX",
            InterruptResult::ControlSound(_) => "ControlSound",
            InterruptResult::ControlSoundDirectX(_) => "ControlSoundDirectX",
        }
    }
}
