
use wasm_bindgen::prelude::*;
mod lexer;
mod constants;
mod semantic_checker;
mod utils;
mod pre_interpreter;
use crate::{lexer::{ParsedLine, Lexer}, semantic_checker::{SyntaxError, SemanticChecker}};
pub struct S68k{
    code: String,
    lines: Vec<ParsedLine>
}
impl S68k{

    pub fn new(code: String) -> S68k{
        let mut lexer = Lexer::new();
        lexer.lex(&code);
        S68k{
            code,
            lines: lexer.get_lines()
        }
    }
    pub fn semantic_check(&self) -> Vec<SyntaxError>{
        let semantic_checker = SemanticChecker::new(&self.lines);
        semantic_checker.get_errors()
    }

    pub fn get_lexed_lines(&self) -> Vec<ParsedLine>{
        self.lines.clone()
    }
}

#[wasm_bindgen]
pub struct WASM_S68k{
    s86k: S68k
}
#[wasm_bindgen]
impl WASM_S68k{
    #[wasm_bindgen(constructor)]
    pub fn new(code: String) -> WASM_S68k{
        WASM_S68k{
            s86k: S68k::new(code)
        }
    }
    pub fn wasm_get_lexed_lines(&self) -> Result<JsValue, JsValue>{
        match serde_wasm_bindgen::to_value(&self.s86k.get_lexed_lines()) {
            Ok(v) => Ok(v),
            Err(e) => Err(JsValue::from_str(&e.to_string()))
        }
       
    }
    pub fn wasm_semantic_check(&self) -> Result<JsValue, JsValue>{
        let errors = self.s86k.semantic_check();
        match serde_wasm_bindgen::to_value(&errors) {
            Ok(v) => Ok(v),
            Err(e) => Err(JsValue::from_str(&e.to_string()))
        }
    }
}

