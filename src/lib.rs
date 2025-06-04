use compiler::Compiler;
use interpreter::{Interpreter, InterpreterOptions};
use wasm_bindgen::prelude::*;
pub mod compiler;
mod constants;
pub mod instructions;
pub mod interpreter;
pub mod lexer;
mod semantic_checker;
mod utils;

mod debugger;
mod math;
mod test;
mod ts_types;
use crate::{
    lexer::{Lexer, ParsedLine},
    semantic_checker::{SemanticChecker, SemanticError},
};

#[wasm_bindgen]
pub struct S68k {
    code: String,
    lines: Vec<ParsedLine>,
}
impl S68k {
    pub fn new(code: String) -> S68k {
        let mut lexer = Lexer::new();
        lexer.lex(&code);
        S68k {
            code,
            lines: lexer.get_lines().clone(),
        }
    }
    pub fn semantic_check(&self) -> Vec<SemanticError> {
        let semantic_checker = SemanticChecker::new(&self.lines);
        semantic_checker.get_errors()
    }
    pub fn compile(&self) -> Result<Compiler, String> {
        Compiler::new(&self.lines)
    }
    pub fn get_lexed_lines(&self) -> &Vec<ParsedLine> {
        &self.lines
    }
    pub fn get_code(&self) -> &String {
        &self.code
    }
    pub fn create_interpreter(
        &self,
        pre_processed_program: Compiler,
        options: Option<InterpreterOptions>,
    ) -> Interpreter {
        Interpreter::new(pre_processed_program, options)
    }
}

#[wasm_bindgen]
impl S68k {
    #[wasm_bindgen(constructor)]
    pub fn wasm_new(code: String) -> S68k {
        console_error_panic_hook::set_once();
        let mut lexer = Lexer::new();
        lexer.lex(&code);
        S68k {
            code,
            lines: lexer.get_lines().clone(),
        }
    }
    pub fn wasm_get_lexed_lines(&self) -> Result<JsValue, JsValue> {
        console_error_panic_hook::set_once();
        match serde_wasm_bindgen::to_value(&self.get_lexed_lines()) {
            Ok(v) => Ok(v),
            Err(e) => Err(JsValue::from_str(&e.to_string())),
        }
    }
    pub fn wasm_compile(&self) -> Result<Compiler, String> {
        console_error_panic_hook::set_once();
        self.compile()
    }
    pub fn wasm_get_code(&self) -> String {
        console_error_panic_hook::set_once();
        self.get_code().clone()
    }
    pub fn wasm_semantic_check(&self) -> WasmSemanticErrors {
        console_error_panic_hook::set_once();
        WasmSemanticErrors::new(self.semantic_check())
    }
    pub fn wasm_create_interpreter(
        &self,
        pre_processed_program: Compiler,
        interpreter_options: JsValue,
    ) -> Interpreter {
        console_error_panic_hook::set_once();
        let interpreter_options: InterpreterOptions =
            serde_wasm_bindgen::from_value(interpreter_options).unwrap();
        self.create_interpreter(pre_processed_program, Some(interpreter_options))
    }
}

#[wasm_bindgen]
pub struct WasmSemanticErrors {
    errors: Vec<SemanticError>,
}
impl WasmSemanticErrors {
    pub fn new(errors: Vec<SemanticError>) -> Self {
        Self { errors }
    }
}

#[wasm_bindgen]
impl WasmSemanticErrors {
    pub fn get_length(&self) -> usize {
        self.errors.len()
    }
    pub fn get_errors(&self) -> Vec<JsValue> {
        self.errors
            .iter()
            .map(|e| serde_wasm_bindgen::to_value(e).unwrap())
            .collect()
    }
    pub fn get_error_at_index(&self, index: usize) -> SemanticError {
        self.errors[index].clone()
    }
}
