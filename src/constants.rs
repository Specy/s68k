#![allow(dead_code)]
#![allow(unused_variables)]
#![allow(unused_macros)]

macro_rules! string_vec {
    ($($x:expr),*) => (vec![$($x.to_string()),*]);
}

pub const DIRECTIVES: &[&str] = &["equ", "org"];
pub const COMMENT: &str = ";";
pub const OPERAND_SEPARATOR: char = ',';
pub const EQU: &'static str = "equ";

