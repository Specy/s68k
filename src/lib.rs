
use wasm_bindgen::prelude::*;
mod lexer;
mod constants;
mod semantic_checker;
mod pre_interpreter;
mod utils;
mod wasm;
use crate::{lexer::{ParsedLine, Lexer}, semantic_checker::{SemanticError, SemanticChecker}};
#[wasm_bindgen]
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
    pub fn semantic_check(&self) -> Vec<SemanticError>{
        let semantic_checker = SemanticChecker::new(&self.lines);
        semantic_checker.get_errors()
    }
    pub fn get_lexed_lines(&self) -> Vec<ParsedLine>{
        self.lines.clone()
    }
}

#[wasm_bindgen]
impl S68k{
    #[wasm_bindgen(constructor)]
    pub fn wasm_new(code: String) -> S68k{
        console_error_panic_hook::set_once();
        let mut lexer = Lexer::new();
        lexer.lex(&code);
        S68k{
            code,
            lines: lexer.get_lines()
        }
    }
    pub fn wasm_get_lexed_lines(&self) -> Result<JsValue, JsValue>{
        match serde_wasm_bindgen::to_value(&self.get_lexed_lines()) {
            Ok(v) => Ok(v),
            Err(e) => Err(JsValue::from_str(&e.to_string()))
        }
    } 
    pub fn wasm_semantic_check(&self) -> Vec<JsValue> {
        self.semantic_check().into_iter().map(JsValue::from).collect()
    }
}

