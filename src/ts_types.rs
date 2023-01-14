use wasm_bindgen::prelude::wasm_bindgen;


#[wasm_bindgen(typescript_custom_section)]
pub const IInterrupt: &'static str = r#"
export type Interrupt = { type: "DisplayStringWithCRLF", value: string } |
{ type: "DisplayStringWithoutCRLF", value: string } |
{ type: "ReadKeyboardString" } |
{ type: "DisplayNumber", value: number } |
{ type: "ReadNumber" } |
{ type: "ReadChar" } |
{ type: "GetTime" } |
{ type: "Terminate" } | 
{ type: "DisplayChar", value: string }
"#;

#[wasm_bindgen(typescript_custom_section)]
pub const IInterruptResult: &'static str = r#"
export type InterruptResult = { type: "DisplayStringWithCRLF" } |
{ type: "DisplayStringWithoutCRLF" } |
{ type: "ReadKeyboardString", value: string } |
{ type: "DisplayNumber" } |
{ type: "ReadNumber", value: number } |
{ type: "ReadChar", value: string } |
{ type: "GetTime", value: number } |
{ type: "DisplayChar" } | 
{ type: "Terminate" }
"#;
#[wasm_bindgen(typescript_custom_section)]
pub const IRuntimeError: &'static str = r#"
export type RuntimeError = { type: "Raw", value: string } |
{ type: "ExecutionLimit", value: number } |
{ type: "OutOfBounds", value: string } |
{ type: "DivisionByZero" } |
{ type: "IncorrectAddressingMode", value: string } |
{ type: "Unimplemented" }
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
    ccr: Flags
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
