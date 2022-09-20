#![allow(dead_code)]
#![allow(unused_variables)]
#![allow(unused_macros)]

macro_rules! string_vec {
    ($($x:expr),*) => (vec![$($x.to_string()),*]);
}

pub const DIRECTIVES: &[&str] = &["equ", "org"];
pub const INSTRUCTIONS: &[&str] = &["add", "addi", "adda", "sub", "subi", "suba", "muls", "mulu", "divs", "divu", "and",
"andi", "or", "ori", "eor", "eori", "not", "neg", "clr", "cmp", "cmpi", "cmpa", "tst",
"asl", "asr", "lsr", "lsl", "ror", "rol", "jmp", "bra", "jsr", "rts", "bsr", "beq",
"bne", "bge", "bgt", "ble", "blt"];
pub const COMMENT: &str = ";";
pub const OPERAND_SEPARATOR: char = ',';
pub const EQU: &'static str = "equ";

