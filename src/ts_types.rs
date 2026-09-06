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

#[wasm_bindgen(typescript_custom_section)]
pub const IInstructionLine: &'static str = r#"
export type InstructionLine = {
    instruction: any //TODO add instruction types
    address: number
    parsed_line: ParsedLine
}
"#;
#[wasm_bindgen(typescript_custom_section)]
pub const IStep: &'static str = r#"
export type Step = [instruction: InstructionLine, status: InterpreterStatus]
"#;
#[wasm_bindgen(typescript_custom_section)]
pub const ILabel: &'static str = r#"
export type Label = {
    name: string,
    address: number,
    line: number
}
"#;

#[wasm_bindgen(typescript_custom_section)]
pub const IParsedLine: &'static str = r#"
export type ParsedLine = {
    line: string,
    line_index: number,
    parsed: LexedLine
}"#;

#[wasm_bindgen(typescript_custom_section)]
pub const ILexedLine: &'static str = r#"
export type LexedLine = {
    type: "Instruction"
    value: {
        name: string,
        operands: LexedOperand[],
        size: "Byte" | "Word" | "Long"
    }
} | {
    type: "Label",
    value: {
        name: string
    }
} | {
    type: "Directive",
    value: {
        args: string[]
    }
} | {
    type: "Empty"
} | {
    type: "Comment",
    value: {
        content: string
    }
} | {
    type: "Unknown",
    value: {
        content: string
    }
}
"#;

#[wasm_bindgen(typescript_custom_section)]
pub const IInterpreterOptions: &'static str = r#"
export type InterpreterOptions = {
    keep_history: boolean
    history_size: number    
}
"#;
#[wasm_bindgen(typescript_custom_section)]
pub const IExecutionStep: &'static str = r#"
export type ExecutionStep = {
    mutations: MutationOperation[],
    pc: number,
    old_ccr: {
        bits: number,
    },
    new_ccr: {
        bits: number,
    },
    line: number
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
#[wasm_bindgen(typescript_custom_section)]
pub const ILexedOperand: &'static str = r#"
export type LexedOperand = {
    type: "Register",
    value: [type: LexedRegisterType, name: string]
} | {
    type: "PreIndirect",
    value: LexedOperand
} | {
    type: "Immediate"
    value: string
} | {
    type: "PostIndirect",
    value: LexedOperand
} | {
    type: "Absolute",
    value: string
} | {
    type: "Label",
    value: string
} | {
    type: "Other",
    value: string
} | {
    type: "IndirectOrDisplacement",
    value: {
        offset: String,
        operand: LexedOperand
    }
} | {
    type: "IndirectBaseDisplacement",
    value: {
        offset: String,
        operands: LexedOperand[]
    }
}
"#;
#[wasm_bindgen(typescript_custom_section)]
pub const ILexedRegisterType: &'static str = r#"
export enum LexedRegisterType {
    LexedData = "Data",
    LexedAddress = "Address",
}
"#;
