#![allow(dead_code)]
#![allow(unused_variables)]
#![allow(unused_macros)]

macro_rules! string_vec {
    ($($x:expr),*) => (vec![$($x.to_string()),*]);
}

pub const DIRECTIVES: &[&str] = &["equ", "org"];
pub const COMMENT_1: char = ';';
pub const COMMENT_2: char  = '*';
pub const OPERAND_SEPARATOR: char = ',';
pub const EQU: &'static str = "equ";
