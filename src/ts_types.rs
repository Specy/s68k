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

#[wasm_bindgen(typescript_custom_section)]
pub const IInterrupt: &'static str = r#"
export type Interrupt = { type: "DisplayStringWithCRLF", value: string } |
{ type: "DisplayStringWithoutCRLF", value: string } |
{ type: "ReadKeyboardString" } |
{ type: "DisplayNumber", value: number } |
{ type: "DisplayNumberInBase", value: { value: number, base: number } } |
{ type: "ReadNumber" } |
{ type: "ReadChar" } |
{ type: "GetTime" } |
{ type: "Terminate" } | 
{ type: "DisplayChar", value: string } | 
{ type: "Delay", value: number } |
{ type: "DisplaySignedNumberInField", value: { value: number, width: number } } |
{ type: "DisplayStringAndNumber", value: { string: string, number: number } } |
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
{ type: "DrawText", value: [number, number, string] } |
{ type: "GetPenPosition" } |
{ type: "SetScreenSize", value: [number, number] } |
{ type: "GetScreenSize" } |
{ type: "SetScreenMode", value: number } |
{ type: "ClearScreen" } |
{ type: "SetTextCursorPosition", value: [number, number] } |
{ type: "GetTextCursorPosition" }
"#;

#[wasm_bindgen(typescript_custom_section)]
pub const IInterruptResult: &'static str = r#"
export type InterruptResult = { type: "DisplayStringWithCRLF" } |
{ type: "DisplayStringWithoutCRLF" } |
{ type: "ReadKeyboardString", value: string } |
{ type: "DisplayNumber" } |
{ type: "DisplayNumberInBase" } |
{ type: "ReadNumber", value: number } |
{ type: "ReadChar", value: string } |
{ type: "GetTime", value: number } |
{ type: "DisplayChar" } | 
{ type: "Terminate" } |
{ type: "Delay" } |
{ type: "DisplaySignedNumberInField" } |
{ type: "DisplayStringAndNumber" } |
{ type: "DisplayStringAndReadNumber", value: number } |
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
{ type: "GetTextCursorPosition", value: [number, number] }
"#;

#[wasm_bindgen(typescript_custom_section)]
pub const IRuntimeError: &'static str = r#"
export type RuntimeError = { type: "Raw", value: string } |
{ type: "ExecutionLimit", value: number } |
{ type: "OutOfBounds", value: string } |
{ type: "DivisionByZero" } |
{ type: "IncorrectAddressingMode", value: string } |
{ type: "Unimplemented" } |
{ type: "AddressError", value : { address: number, size: Size } }


"#;

#[wasm_bindgen(typescript_custom_section)]
pub const IRegisterOperand: &'static str = r#"
export type RegisterOperand = { type: "Address", value: number } |
{type: "Data", value: number}
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
    /** The full name, so a local label reads as `start:.loop`. */
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
/// them as the string `bitflags` serialises.
#[wasm_bindgen(typescript_custom_section)]
pub const IExecutionStep: &'static str = r#"
export type ExecutionStep = {
    /** Identifies this execution, including repeated visits to the same PC. */
    id: number,
    mutations: MutationOperation[],
    pc: number,
    old_ccr: {
        bits: number,
    },
    new_ccr: {
        bits: number,
    },
    /** Where the instruction that ran was written; absent when it ran on none. */
    location?: Location
}
"#;
#[wasm_bindgen(typescript_custom_section)]
pub const IMutationOperation: &'static str = r#"
export type MutationOperation = {
    type: "WriteRegister",
    value: {
        register: RegisterOperand,
        old: number,
        size: Size
    }
} | {
    type: "WriteMemory",
    value: {
        address: number,
        old: number,
        size: Size
    }
} | {
    type: "WriteMemoryBytes",
    value: {
        address: number,
        old: number[]
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
}
"#;
