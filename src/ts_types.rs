//! The TypeScript declarations of the shapes that cross into JavaScript as
//! plain objects.
//!
//! `wasm-bindgen` writes a type for everything it exports itself; these are the
//! things it cannot see, because they cross as serialised values rather than as
//! handles. They are hand-written and hand-checked against the Rust types they
//! describe: a `Serialize` derive that changes shape has to change one of these
//! too. Every declaration below names the Rust type it mirrors.
//!
//! # Two conventions, and where the line is
//!
//! The Assembler's own shapes — [`Diagnostic`](crate::assembler::diagnostics::Diagnostic),
//! [`Location`](crate::assembler::source::Location), the parsed line and the
//! program information — are **camelCase**, which is what the design record's
//! "Public API" specifies and what the asm-editor reads. The Interpreter's
//! shapes are 1.4.2's and keep their **snake_case** field names
//! (`old_ccr`, `source_address`, `keep_history`), because 2.0 changes the
//! Interpreter's surface only where a line index became a Location: renaming
//! the rest would be churn in the editor for no gain. A Location nested inside
//! one of them is still written the one way Locations are written.
//!
//! # Optional fields
//!
//! `serde_wasm_bindgen` writes a Rust `None` as `undefined`, not as `null`, so
//! a field that may be missing is declared with `?` and a method that answers
//! "there is none" with an explicit `JsValue::NULL` is declared `| null`.

use wasm_bindgen::prelude::wasm_bindgen;

#[wasm_bindgen(typescript_custom_section)]
pub const IKeyState: &'static str = r#"
export type KeyStateRequest = { type: "Keys", value: [number, number, number, number] } |
{ type: "LastKeys" }

export type KeyStateResult = { type: "Keys", value: [boolean, boolean, boolean, boolean] } |
{ type: "LastKeys", value: { up: number, down: number } }
"#;

/// [`InputSettings`](crate::instructions::InputSettings),
/// [`FileDialogMode`](crate::instructions::FileDialogMode),
/// [`OpenedFile`](crate::instructions::OpenedFile) and
/// [`FileExistence`](crate::instructions::FileExistence): the shapes the input
/// settings and the file tasks cross with.
#[wasm_bindgen(typescript_custom_section)]
pub const IFileTypes: &'static str = r#"
/**
 * How the read tasks show what is typed: the echo of task 12, the input prompt
 * and the line feed after Enter of task 16. All three are on when a program
 * starts, and undo puts back what a task changed.
 */
export type InputSettings = {
    /** Whether what is typed is echoed. Enter still ends a line read on a new line with it off. */
    echo: boolean,
    /** Whether the input prompt, EASy68K's flashing cursor, shows while a read waits. */
    prompt: boolean,
    /** Whether a key read (task 5) of Enter echoes a line feed after the carriage return. */
    line_feed: boolean
}

/** Task 58's dialog: D1.L = 0 opens a file, 1 saves one. */
export type FileDialogMode = "Open" | "Save"

/** A file the host opened for task 51 or 52. */
export type OpenedFile = {
    /** The file number, 0 to 7, the lowest free one as EASy68K numbers its eight files. */
    handle: number,
    /** Whether it could only be opened for reading: task 51 then reports 3. */
    read_only: boolean
}

/** What task 59 found: D0.W is 0, 3 or 2. */
export type FileExistence = "Writable" | "ReadOnly" | "Missing"
"#;

/// [`Interrupt`](crate::instructions::Interrupt): what a `trap #15` asks the
/// host for. Every text a display task carries is ready to display, decoded
/// from Windows-1252 and, for a number, formatted with EASy68K's rules.
#[wasm_bindgen(typescript_custom_section)]
pub const IInterrupt: &'static str = r#"
/**
 * What a `trap #15` asks the host for. Each is answered by the `InterruptResult`
 * of the same `type`, or by `{type: "Terminate"}`.
 *
 * A path is the NUL terminated string at the address, at most 255 characters,
 * decoded from Windows-1252, with `\` written as `/`.
 */
export type Interrupt =
/** Tasks 0 and 13: display the text, then a new line. Task 0's text is at most 255 characters and stops at a NUL. */
{ type: "DisplayStringWithCRLF", value: string } |
/** Tasks 1 and 14: display the text. */
{ type: "DisplayStringWithoutCRLF", value: string } |
/** Task 2: read a line, answered with `{ type: "ReadKeyboardString", value: line }`, shown as the `InputSettings` say. */
{ type: "ReadKeyboardString" } |
/** Task 3: D1.L as a signed decimal number, `-5`. */
{ type: "DisplayNumber", value: string } |
/** Task 15: D1.L as an unsigned number in the base in D2.B, upper case, `FF`. */
{ type: "DisplayNumberInBase", value: string } |
/** Task 4: read a number, answered with the line typed, shown as the `InputSettings` say. */
{ type: "ReadNumber" } |
/** Task 5: read one key, answered with the key typed, echoed as the `InputSettings` say. */
{ type: "ReadChar" } |
{ type: "GetTime" } |
{ type: "Terminate" } |
/** Task 6: the character in D1.B. */
{ type: "DisplayChar", value: string } |
{ type: "Delay", value: number } |
/** Task 20: D1.L in a field of D2.B columns, right justified, or left justified when D2.B is negative. */
{ type: "DisplaySignedNumberInField", value: string } |
/** Task 17: the string at (A1) and then D1.L as a signed decimal number, as one text. */
{ type: "DisplayStringAndNumber", value: string } |
/** Task 18: display the string at (A1), then read a number, answered with the line typed, shown as the `InputSettings` say. */
{ type: "DisplayStringAndReadNumber", value: string } |
{ type: "CheckKeyboardInput" } |
{ type: "GetKeyState", value: KeyStateRequest } |
{ type: "ReadMouse", value: number } |
{ type: "SetSimulatorShortcuts", value: number } |
{ type: "SetPenColor", value: number } |
{ type: "SetFillColor", value: number } |
{ type: "DrawPixel", value: [number, number] } |
{ type: "GetPixelColor", value: [number, number] } |
{ type: "DrawLine", value: [number, number, number, number] } |
{ type: "DrawLineTo", value: [number, number] } |
{ type: "MoveTo", value: [number, number] } |
{ type: "DrawRectangle", value: [number, number, number, number] } |
{ type: "DrawEllipse", value: [number, number, number, number] } |
{ type: "FloodFill", value: [number, number] } |
{ type: "DrawUnfilledRectangle", value: [number, number, number, number] } |
{ type: "DrawUnfilledEllipse", value: [number, number, number, number] } |
{ type: "SetDrawingMode", value: number } |
{ type: "SetPenWidth", value: number } |
{ type: "Repaint" } |
/** Task 95: the text, decoded from Windows-1252, at x, y. */
{ type: "DrawText", value: [number, number, string] } |
{ type: "GetPenPosition" } |
{ type: "SetScreenSize", value: [number, number] } |
{ type: "GetScreenSize" } |
{ type: "SetScreenMode", value: number } |
{ type: "ClearScreen" } |
{ type: "SetTextCursorPosition", value: [number, number] } |
{ type: "GetTextCursorPosition" } |
/** Task 50: close every open file. */
{ type: "CloseAllFiles" } |
/** Task 51: open the existing file at the path for reading and writing, or for reading only when it cannot be written. */
{ type: "OpenFile", value: string } |
/** Task 52: open the file at the path for reading and writing, creating it, or emptying it when it exists. */
{ type: "NewFile", value: string } |
/** Task 53: read at most `count` bytes from the file's position. */
{ type: "ReadFile", value: { handle: number, count: number } } |
/** Task 54: write the bytes at the file's position. */
{ type: "WriteFile", value: { handle: number, bytes: Uint8Array } } |
/** Task 55: move the file's position to `offset` bytes from its start. */
{ type: "PositionFile", value: { handle: number, offset: number } } |
/** Task 56: close the file. */
{ type: "CloseFile", value: number } |
/** Task 57: delete the file at the path. */
{ type: "DeleteFile", value: string } |
/** Task 58: let the user choose a file; `title` and `filter` (such as `*.txt`) may be empty, `path` is where the dialog starts. */
{ type: "FileDialog", value: { mode: FileDialogMode, title: string, filter: string, path: string } } |
/** Task 59: whether there is a file at the path, and whether it can be written. */
{ type: "FileExists", value: string } |
/** Task 70: play the WAV file at the path. */
{ type: "PlaySound", value: string } |
/** Task 71: load the WAV file at the path into sound memory `index`, 0 to 255. */
{ type: "LoadSound", value: { path: string, index: number } } |
/** Task 72: play the sound loaded into the index. */
{ type: "PlayLoadedSound", value: number } |
/** Task 73: play the WAV file at the path with EASy68K's DirectX player. */
{ type: "PlaySoundDirectX", value: string } |
/** Task 74: load the WAV file at the path into DirectX sound memory `index`. */
{ type: "LoadSoundDirectX", value: { path: string, index: number } } |
/** Task 75: play the DirectX sound loaded into the index. */
{ type: "PlayLoadedSoundDirectX", value: number } |
/** Task 76: control the standard player: 0 plays sound `index` once, 1 loops it, 2 stops it, 3 stops every sound. */
{ type: "ControlSound", value: { index: number, control: number } } |
/** Task 77: the same for the DirectX player. */
{ type: "ControlSoundDirectX", value: { index: number, control: number } }
"#;

/// [`InterruptResult`](crate::instructions::InterruptResult): the host's
/// answer. A read task is answered with what was typed, as it was typed, and the
/// Interpreter reads it with EASy68K's rules.
#[wasm_bindgen(typescript_custom_section)]
pub const IInterruptResult: &'static str = r#"
export type InterruptResult = { type: "DisplayStringWithCRLF" } |
{ type: "DisplayStringWithoutCRLF" } |
/**
 * Task 2: the line typed, without the Enter that ended it. Its first 79
 * characters are stored at (A1) in Windows-1252, `?` for a character that has
 * no byte, then a NUL, and their count goes in D1.L. A line ends at its first
 * line terminator.
 */
{ type: "ReadKeyboardString", value: string } |
{ type: "DisplayNumber" } |
{ type: "DisplayNumberInBase" } |
/**
 * Task 4: the line typed, which `atoi` reads into D1.L: `"12abc"` is 12, and a
 * line with no number is 0, never an error.
 */
{ type: "ReadNumber", value: string } |
/**
 * Task 5: the one key typed, stored in D1.B in Windows-1252, `?` when it has
 * no byte. Enter is `$0D`, whether it is answered as `"\r"` or `"\n"`.
 */
{ type: "ReadChar", value: string } |
{ type: "GetTime", value: number } |
{ type: "DisplayChar" } |
/** Ends the program, whatever task is pending. */
{ type: "Terminate" } |
{ type: "Delay" } |
{ type: "DisplaySignedNumberInField" } |
{ type: "DisplayStringAndNumber" } |
/** Task 18: the line typed, read as `ReadNumber` reads it. */
{ type: "DisplayStringAndReadNumber", value: string } |
{ type: "CheckKeyboardInput", value: boolean } |
{ type: "GetKeyState", value: KeyStateResult } |
{ type: "ReadMouse", value: { flags: number, x: number, y: number } } |
{ type: "SetSimulatorShortcuts" } |
{ type: "SetPenColor" } |
{ type: "SetFillColor" } |
{ type: "DrawPixel" } |
{ type: "GetPixelColor", value: number } |
{ type: "DrawLine" } |
{ type: "DrawLineTo" } |
{ type: "MoveTo" } |
{ type: "DrawRectangle" } |
{ type: "DrawEllipse" } |
{ type: "FloodFill" } |
{ type: "DrawUnfilledRectangle" } |
{ type: "DrawUnfilledEllipse" } |
{ type: "SetDrawingMode" } |
{ type: "SetPenWidth" } |
{ type: "Repaint" } |
{ type: "DrawText" } |
{ type: "GetPenPosition", value: [number, number] } |
{ type: "SetScreenSize" } |
{ type: "GetScreenSize", value: [number, number] } |
{ type: "SetScreenMode" } |
{ type: "ClearScreen" } |
{ type: "SetTextCursorPosition" } |
{ type: "GetTextCursorPosition", value: [number, number] } |
/** Task 50: whether every file closed. D0.W is 0, or 2. */
{ type: "CloseAllFiles", value: boolean } |
/**
 * Task 51: the file opened, or null when it could not be. D1.L is its number
 * and D0.W 0, or 3 when it opened for reading only; D1.L is -1 and D0.W 2 for
 * null.
 */
{ type: "OpenFile", value: OpenedFile | null } |
/** Task 52: the number of the file opened, or null: D1.L and D0.W as for task 51. */
{ type: "NewFile", value: number | null } |
/**
 * Task 53: the bytes read, at most the count asked for, or null when the read
 * failed. Some bytes go to (A1), their count to D2.L and 0 to D0.W, a short
 * read included; no bytes at all is the end of the file, 1 in D0.W with D2.L
 * left as it was; null is 2. An array of numbers is taken too.
 */
{ type: "ReadFile", value: Uint8Array | number[] | null } |
/** Task 54: whether every byte was written. D0.W is 0, or 2. */
{ type: "WriteFile", value: boolean } |
/** Task 55: whether the position moved. D0.W is 0, or 2. */
{ type: "PositionFile", value: boolean } |
/** Task 56: whether the file closed. D0.W is 0, or 2. */
{ type: "CloseFile", value: boolean } |
/** Task 57: whether the file was deleted. D0.W is 0, or 2. */
{ type: "DeleteFile", value: boolean } |
/**
 * Task 58: the path chosen, or null for a cancel. A path goes to (A3), at most
 * 255 characters in Windows-1252 and NULs to 256 bytes, with 1 in D1.L; a
 * cancel puts 0 in D1.L. D0.W is 0, or 2 when the 256 bytes do not fit.
 */
{ type: "FileDialog", value: string | null } |
/** Task 59: what is at the path. D0.W is 0, 3 or 2. */
{ type: "FileExists", value: FileExistence } |
/** Tasks 70, 72 to 77: whether the sound task happened, 1 or 0 in D0.W. */
{ type: "PlaySound", value: boolean } |
/** Task 71 writes no result. */
{ type: "LoadSound" } |
{ type: "PlayLoadedSound", value: boolean } |
{ type: "PlaySoundDirectX", value: boolean } |
{ type: "LoadSoundDirectX", value: boolean } |
{ type: "PlayLoadedSoundDirectX", value: boolean } |
{ type: "ControlSound", value: boolean } |
{ type: "ControlSoundDirectX", value: boolean }
"#;

/// [`RuntimeError`](crate::interpreter::RuntimeError) and
/// [`Termination`](crate::interpreter::Termination): what every method throws,
/// and why a program ended.
#[wasm_bindgen(typescript_custom_section)]
pub const IRuntimeError: &'static str = r#"
/**
 * What every method throws, as a plain object. An error an instruction raised
 * ends the program with an exception and is its `Termination`; the others are
 * the host's (an answer refused, a value that is not what a call takes, a call
 * at the wrong time, the run's own limit) and change nothing.
 */
export type RuntimeError = { type: "Raw", value: string } |
{ type: "ExecutionLimit", value: number } |
{ type: "OutOfBounds", value: string } |
{ type: "DivisionByZero" } |
{ type: "IncorrectAddressingMode", value: string } |
{ type: "Unimplemented" } |
{ type: "AddressError", value : { address: number, size: Size } } |
/** `chk` found the register outside 0 to the bound it was given. */
{ type: "ChkOutOfBounds", value: { value: number, bound: number } } |
/** `trapv` with the overflow flag set. */
{ type: "OverflowException" } |
/** The `illegal` instruction, which always ends the run. */
{ type: "IllegalInstruction" } |
/**
 * A `trap #15` task s68k does not carry out, by its number in D0.B: one that
 * is not EASy68K's, or the printer (10), the text window's font and contents
 * (21, 22, 25), the cycle counter (30, 31), the hardware window (32), the
 * serial ports (40 to 43), the interrupt requests (60, 62) and the network
 * (100 to 107). The host says why from the number.
 */
{ type: "UnsupportedTrapTask", value: { task: number } } |
/** A `trap #15` task given a value it cannot take; `reason` names the register. */
{ type: "InvalidTrapArgument", value: { task: number, reason: string } } |
/** An answer when no interrupt waits for one. */
{ type: "NoPendingInterrupt" } |
/** An answer the pending interrupt cannot take; it still waits. */
{ type: "InvalidAnswer", value: { interrupt: string, reason: string } } |
/** A value that is not what the call takes: a register that does not exist, a breakpoint that is not `{file, line}`. */
{ type: "InvalidArgument", value: string }

/** Why a program ended. */
export type Termination =
/** Task 9. */
{ type: "TerminateTask" } |
/** The program counter left the last instruction, or there was none to run. */
{ type: "EndOfProgram" } |
/** The host answered an interrupt with `Terminate`. */
{ type: "TerminatedByHost" } |
/** A runtime error, and the program ended with an exception. */
{ type: "Exception", value: RuntimeError }
"#;

#[wasm_bindgen(typescript_custom_section)]
pub const IRegisterOperand: &'static str = r#"
export type RegisterOperand = { type: "Address", value: number } |
{type: "Data", value: number}
"#;

/// [`Files`](crate::assembler::source::Files): the Project
/// [`wasm_assemble`](crate::wasm_assemble) is handed.
///
/// It is the type of that function's first argument
/// ([`SourceFiles`](crate::SourceFiles)), so the declarations say what a
/// project is and not `any`.
#[wasm_bindgen(typescript_custom_section)]
pub const ISourceFiles: &'static str = r#"
/**
 * The files of a project: a root-relative path with `/` separators, to the
 * text of a source file or to the bytes of a binary one. `include` reads a
 * source file, `incbin` either kind.
 */
export type SourceFiles = Record<string, string | Uint8Array>
"#;

/// [`Location`](crate::assembler::source::Location): where in the source
/// something is.
#[wasm_bindgen(typescript_custom_section)]
pub const ILocation: &'static str = r#"
export type Location = {
    /** Root-relative path of the file, with `/` separators. */
    file: string,
    /** 0-based index of the source line. */
    line: number,
    /** 0-based column of the first character. */
    column: number,
    /** 0-based column one past the last character. */
    endColumn: number
}
"#;

/// [`Diagnostic`](crate::assembler::diagnostics::Diagnostic): one finding about
/// the source, with its message and hint already rendered.
#[wasm_bindgen(typescript_custom_section)]
pub const IDiagnostic: &'static str = r#"
export type Severity = "error" | "warning" | "suggestion"

export type RelatedLocation = {
    location: Location,
    /** Why this other place is worth looking at. */
    message: string
}

export type Diagnostic = {
    /** Only an `error` stops the program from being built. */
    severity: Severity,
    /** The stable snake_case name of the kind, which is what to match on. */
    code: string,
    /** What is wrong, in one sentence, for a person to read. */
    message: string,
    /** What to do about it, when there is something short to say. */
    hint?: string,
    location: Location,
    /** The first definition of a name defined twice, and the like. */
    related: RelatedLocation[]
}
"#;

/// [`AssembledInstruction`](crate::assembler::program::AssembledInstruction),
/// under the name the editor has always called it by.
#[wasm_bindgen(typescript_custom_section)]
pub const IInstructionLine: &'static str = r#"
export type InstructionLine = {
    instruction: any //TODO add instruction types
    /** The address it is laid out at, always even. */
    address: number
    /** How many bytes it takes up. */
    size: number
    /** The source line it was written on. */
    location: Location
    /**
     * The `include` lines it was reached through, innermost first; empty for an
     * instruction of the entry file. Two copies of a file included twice share
     * one location and differ only in this.
     */
    includeChain: Location[]
    /** That line, as it was written. */
    source: string
}
"#;

/// [`Breakpoint`](crate::interpreter::Breakpoint): a Location without its
/// columns.
#[wasm_bindgen(typescript_custom_section)]
pub const IBreakpoint: &'static str = r#"
export type Breakpoint = {
    file: string,
    line: number
}
"#;

/// [`ProgramSymbol`](crate::assembler::program::ProgramSymbol) and
/// [`ProgramInfo`](crate::ProgramInfo): what a built program says about itself.
#[wasm_bindgen(typescript_custom_section)]
pub const IProgramSymbol: &'static str = r#"
export type ProgramSymbol = {
    /**
     * The full name: a local label is written under the global label above it,
     * with the dot replaced by a colon, so `.loop` under `start` is
     * `start:loop`.
     */
    name: string,
    kind: "label" | "constant" | "variable" | "register_list",
    /** An address for a label, the mask for a register list. */
    value: number,
    location: Location
}

export type ProgramInfo = {
    /** The address the program starts running at. */
    entryPoint: number,
    /** One past the last byte of the last instruction. */
    endAddress: number,
    instructionCount: number,
    /** Every symbol, by full name. */
    symbols: Record<string, ProgramSymbol>
}
"#;

/// [`ParsedLine`](crate::ParsedLine): what `parseLine` answers, for the
/// editor's hover.
#[wasm_bindgen(typescript_custom_section)]
pub const IParsedLine: &'static str = r#"
/** A range of the line, in characters, `end` exclusive. */
export type LineSpan = {
    start: number,
    end: number
}

/** What the line turned out to be; the operation decides it. */
export type ParsedLineKind = "blank" | "comment" | "label" | "instruction" | "directive" | "unknown"

export type ParsedLabel = {
    /** The name, without the colon. */
    name: string,
    colon: boolean,
    span: LineSpan
}

export type ParsedOperand = {
    /** The addressing mode: `immediate`, `data_register_direct`, ... */
    mode: string,
    /** The same in English ("an immediate"). */
    description: string,
    /** The operand exactly as it was written. */
    text: string,
    span: LineSpan
}

/** The raw operand field of `include`, `incbin` or `fail`. */
export type ParsedText = {
    text: string,
    span: LineSpan
}

export type ParsedOperation = {
    /** The mnemonic or directive name, as it was written. */
    name: string,
    size?: "byte" | "word" | "long" | "short",
    nameSpan: LineSpan,
    sizeSpan?: LineSpan,
    /** The whole operation, operand field included. */
    span: LineSpan,
    operands: ParsedOperand[],
    text?: ParsedText
}

export type ParsedComment = {
    kind: "line" | "explicit" | "bare",
    /** The text, marker included. */
    text: string,
    span: LineSpan
}

export type ParsedLine = {
    kind: ParsedLineKind,
    label?: ParsedLabel,
    operation?: ParsedOperation,
    comment?: ParsedComment
}
"#;

/// [`PrettyStackFrame`](crate::debugger::PrettyStackFrame): one frame of the
/// call stack, with the Label of the routine it is in already looked up.
#[wasm_bindgen(typescript_custom_section)]
pub const IStackFrame: &'static str = r#"
export type StackFrame = {
    /** The address the routine starts at. */
    address: number,
    /** The address the call returns to, not the address of the calling instruction. */
    source_address: number,
    /** The registers as they were when it was called. */
    registers: number[],
    /** The name of the label on `address`, or `Unknown` when it has none. */
    label_name: string,
    label_address: number,
    /** Where that label was written; absent when the address has none. */
    label_location?: Location
}
"#;

/// [`InterpreterOptions`](crate::interpreter::InterpreterOptions), 1.4.2's
/// shape unchanged.
#[wasm_bindgen(typescript_custom_section)]
pub const IInterpreterOptions: &'static str = r#"
export type InterpreterOptions = {
    keep_history: boolean
    history_size: number    
}
"#;

/// [`ExecutionStep`](crate::debugger::ExecutionStep) as `ts-lib` hands it on:
/// the condition codes are a bitfield there, where the wasm boundary writes
/// them as the string `bitflags` serialises. Its `kind` and its `writes` are
/// [`ExecutionStepKind`](crate::debugger::ExecutionStepKind) and
/// [`PokeWrite`](crate::debugger::PokeWrite).
#[wasm_bindgen(typescript_custom_section)]
pub const IExecutionStep: &'static str = r#"
/** Whether a step of the history is an instruction or a poke. */
export type ExecutionStepKind = "instruction" | "poke"

/**
 * One value a poke wrote: what was there before it and what is there when the
 * poke closed. A register is named as the editor spells it (`d0`, `a7`).
 */
export type PokeWrite = {
    type: "register",
    name: string,
    old: number,
    new: number
} | {
    type: "memory",
    address: number,
    old: number[],
    new: number[]
}

export type ExecutionStep = {
    /** Identifies this execution, including repeated visits to the same PC. */
    id: number,
    /** An instruction the program ran, or a poke the host made between two of them. */
    kind: ExecutionStepKind,
    mutations: MutationOperation[],
    /** What a poke wrote, old and new; empty on an instruction. */
    writes: PokeWrite[],
    pc: number,
    old_ccr: {
        bits: number,
    },
    new_ccr: {
        bits: number,
    },
    /**
     * The whole status register before the step, as the processor numbers it:
     * the system byte, then the condition codes (extend 16, negative 8, zero 4,
     * overflow 2, carry 1). It is what undo puts back, and it overlaps
     * `old_ccr`, which is the same flags in this crate's own bits.
     */
    old_sr: number,
    /** The whole status register after the step. */
    new_sr: number,
    /** Where the instruction that ran was written; absent when it ran on none. */
    location?: Location
}
"#;
#[wasm_bindgen(typescript_custom_section)]
pub const IMutationOperation: &'static str = r#"
/**
 * One thing a step changed, with the value it replaced beside the value it
 * wrote.
 *
 * Both sides are read where the write happened and never reconstructed
 * afterwards. `PushCall` and `PopCall` write no value and carry neither.
 */
export type MutationOperation = {
    type: "WriteRegister",
    value: {
        register: RegisterOperand,
        /** The whole register before the store. */
        old: number,
        /** The whole register after it, sized store or not. */
        new: number,
        size: Size
    }
} | {
    type: "WriteMemory",
    value: {
        address: number,
        /** The value the write replaced, at `size`. */
        old: number,
        /** The value it stored, at `size`. */
        new: number,
        size: Size
    }
} | {
    type: "WriteMemoryBytes",
    value: {
        address: number,
        /** The bytes the write replaced. */
        old: number[],
        /** The bytes it stored. */
        new: number[]
    }
} | {
    type: "PopCall",
    value: {
        to: number,
        from: number,
    }
} | {
    type: "PushCall",
    value: {
        to: number,
        from: number,
    }
} | {
    /** Task 12 or 16 changed the input settings. */
    type: "SetInputSettings",
    value: {
        old: InputSettings,
        new: InputSettings
    }
}
"#;
