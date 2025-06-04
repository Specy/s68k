//TODO some instructions might accept indirect and also displacement, check that

use std::collections::HashMap;

use bitflags::bitflags;
use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;

use crate::{
    instructions::Label,
    lexer::{LexedLine, LexedOperand, LexedRegisterType, LexedSize, ParsedLine},
    utils::{num_to_signed_base, parse_absolute_expression},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[wasm_bindgen]
pub struct SemanticError {
    line: ParsedLine,
    error: String,
}

impl SemanticError {
    pub fn new(line: ParsedLine, error: String) -> Self {
        Self { line, error }
    }
    pub fn get_line(&self) -> &ParsedLine {
        &self.line
    }
    pub fn get_line_index(&self) -> usize {
        self.line.line_index
    }
    pub fn get_message(&self) -> String {
        format!("Error on line {}: {}", self.line.line_index + 1, self.error)
    }
    pub fn get_message_with_line(&self) -> String {
        format!(
            "Error on line {}, \"{}\": {}",
            self.line.line_index + 1,
            self.line.line,
            self.error
        )
    }
}

#[wasm_bindgen]
impl SemanticError {
    pub fn wasm_get_message(&self) -> String {
        self.get_message()
    }
    pub fn wasm_get_line(&self) -> JsValue {
        serde_wasm_bindgen::to_value(&self.line).unwrap()
    }
    pub fn wasm_get_message_with_line(&self) -> String {
        self.get_message_with_line()
    }
    pub fn wasm_get_line_index(&self) -> usize {
        self.get_line_index()
    }
    pub fn wasm_get_error(&self) -> String {
        self.error.clone()
    }
}

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    struct AdrMode: usize {
        const IMMEDIATE = 1<<0;
        const D_REG = 1<<1;
        const A_REG = 1<<2;
        const INDIRECT = 1<<3;
        const INDIRECT_DISPLACEMENT = 1<<4;
        const INDIRECT_INDEX = 1<<5;
        const INDIRECT_POST_INCREMENT = 1<<6;
        const INDIRECT_PRE_DECREMENT = 1<<7;
        const ADDRESS = 1<<8;
        const REG_LIST = 1<<9;
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    struct Rules: usize {
        //TODO refactor this
        const NONE = 0;
        const NO_D_REG = AdrMode::D_REG.bits();
        const NO_A_REG = AdrMode::A_REG.bits();
        const NO_IMMEDIATE = AdrMode::IMMEDIATE.bits();
        const NO_ADDRESS = AdrMode::ADDRESS.bits();
        const NO_INDIRECT = AdrMode::INDIRECT.bits()
            | AdrMode::INDIRECT_DISPLACEMENT.bits()
            | AdrMode::INDIRECT_INDEX.bits()
            | AdrMode::INDIRECT_POST_INCREMENT.bits()
            | AdrMode::INDIRECT_PRE_DECREMENT.bits();

        const ONLY_POST_INCREMENT = !AdrMode::INDIRECT_POST_INCREMENT.bits();
        const ONLY_REG = !(AdrMode::D_REG.bits() | AdrMode::A_REG.bits());
        const ONLY_A_REG = !AdrMode::A_REG.bits();
        const ONLY_D_REG = !AdrMode::D_REG.bits();
        const ONLY_INDIRECT = !(AdrMode::INDIRECT.bits()
            | AdrMode::INDIRECT_DISPLACEMENT.bits()
            | AdrMode::INDIRECT_INDEX.bits()
            | AdrMode::INDIRECT_POST_INCREMENT.bits()
            | AdrMode::INDIRECT_PRE_DECREMENT.bits()
        );
        const ONLY_D_REG_OR_INDIRECT = !(AdrMode::D_REG.bits()
            | AdrMode::INDIRECT.bits()
            | AdrMode::INDIRECT_DISPLACEMENT.bits()
            | AdrMode::INDIRECT_INDEX.bits()
            | AdrMode::INDIRECT_POST_INCREMENT.bits()
            | AdrMode::INDIRECT_PRE_DECREMENT.bits()
        );
        const ONLY_D_REG_OR_INDIRECT_OR_ADDRESS = !(AdrMode::D_REG.bits()
            | AdrMode::ADDRESS.bits()
            | AdrMode::INDIRECT.bits()
            | AdrMode::INDIRECT_DISPLACEMENT.bits()
            | AdrMode::INDIRECT_INDEX.bits()
            | AdrMode::INDIRECT_POST_INCREMENT.bits()
            | AdrMode::INDIRECT_PRE_DECREMENT.bits()
        );
        const ONLY_ADDRESS = !(AdrMode::ADDRESS.bits());
        const ONLY_IMMEDIATE = !AdrMode::IMMEDIATE.bits();
        const ONLY_INDIRECT_OR_ABSOLUTE = !(AdrMode::INDIRECT.bits()
            | AdrMode::INDIRECT_DISPLACEMENT.bits()
            | AdrMode::INDIRECT_INDEX.bits()
            | AdrMode::INDIRECT_POST_INCREMENT.bits()
            | AdrMode::INDIRECT_PRE_DECREMENT.bits()
            | AdrMode::ADDRESS.bits()
        );
        const ONLY_REG_LIST = !AdrMode::REG_LIST.bits();
        const NO_A_REG_OR_IMMEDIATE = AdrMode::A_REG.bits() | AdrMode::IMMEDIATE.bits();
    }
}
//TODO refactor this
impl AdrMode {
    pub fn get_name(&self) -> String {
        match *self {
            AdrMode::D_REG => "Dn",
            AdrMode::A_REG => "An",
            AdrMode::INDIRECT | AdrMode::INDIRECT_DISPLACEMENT => "(An)",
            AdrMode::INDIRECT_POST_INCREMENT => "(An)+",
            AdrMode::INDIRECT_PRE_DECREMENT => "-(An)",
            AdrMode::INDIRECT_INDEX => "(An, Dn)",
            AdrMode::IMMEDIATE => "Im",
            AdrMode::ADDRESS => "Ea/<LABEL>",

            _ => "UNKNOWN",
        }
        .to_string()
    }
}

impl Rules {
    pub fn get_valid_addressing_modes(&self) -> String {
        match *self {
            Rules::NONE => "Im/Dn/An/(An)/Ea/<label>",
            Rules::NO_D_REG => "Im/An/(An)/Ea/<label>",
            Rules::NO_A_REG => "Im/Dn/(An)/Ea/<label>",
            Rules::NO_IMMEDIATE => "Dn/An/(An)/Ea/<label>",
            Rules::NO_ADDRESS => "Im/Dn/An/(An)",
            Rules::NO_INDIRECT => "Im/Dn/An/Ea/<label>",
            Rules::ONLY_REG => "Dn/An",
            Rules::ONLY_A_REG => "An",
            Rules::ONLY_D_REG => "Dn",
            Rules::ONLY_INDIRECT => "(An)",
            Rules::ONLY_D_REG_OR_INDIRECT => "Dn/(An)",
            Rules::ONLY_D_REG_OR_INDIRECT_OR_ADDRESS => "Dn/(An)/Ea",
            Rules::ONLY_ADDRESS => "Ea/<label>",
            Rules::ONLY_IMMEDIATE => "Im",
            Rules::ONLY_INDIRECT_OR_ABSOLUTE => "(An)/Ea/<label>",
            Rules::ONLY_POST_INCREMENT => "(An)",
            Rules::ONLY_REG_LIST => "<reg list>",
            Rules::NO_A_REG_OR_IMMEDIATE => "Dn/(An)/(An)/Ea/<label>",
            _ => "UNKNOWN",
        }
        .to_string()
    }
}

pub enum SizeRules {
    NoSize,
    AnySize,
    OnlyLongOrWord,
}

impl SizeRules {
    pub fn get_valid_sizes(&self) -> String {
        match *self {
            SizeRules::NoSize => "",
            SizeRules::AnySize => "b, w, l",
            SizeRules::OnlyLongOrWord => "w, l",
        }
        .to_string()
    }
}

pub struct SemanticChecker {
    labels: HashMap<String, Label>,
    errors: Vec<SemanticError>,
    lines: Vec<ParsedLine>,
}

impl SemanticChecker {
    pub fn new(lines: &[ParsedLine]) -> SemanticChecker {
        let mut syntax_checker = SemanticChecker {
            errors: Vec::new(),
            lines: Vec::new(),
            labels: HashMap::new(),
        };
        syntax_checker.check(lines);
        syntax_checker
    }

    pub fn check(&mut self, lines: &[ParsedLine]) {
        self.lines = lines.to_vec();
        for line in lines.iter() {
            match &line.parsed {
                LexedLine::Label { name } => {
                    if self.labels.contains_key(name) {
                        self.errors.push(SemanticError::new(
                            line.clone(),
                            format!("Label \"{}\" already exists", name),
                        ));
                    } else {
                        self.labels.insert(
                            name.to_string(),
                            Label {
                                name: name.to_string(),
                                address: 1 << 31usize, //placeholder value,
                                line: line.line_index,
                            },
                        );
                    }
                }
                _ => {}
            }
        }
        for line in lines.iter() {
            self.check_one(line);
        }
    }
    pub fn check_one(&mut self, line: &ParsedLine) {
        match &line.parsed {
            LexedLine::Empty | LexedLine::Comment { .. } => {}

            LexedLine::Label { .. } => {}
            LexedLine::Directive { .. } => {
                self.verify_directive(line);
            }
            LexedLine::Instruction { .. } => {
                self.check_instruction(line);
            }
            _ => self.errors.push(SemanticError::new(
                line.clone(),
                format!("Unknown line: \"{}\"", line.line),
            )),
        }
    }
    pub fn get_errors(&self) -> Vec<SemanticError> {
        self.errors.clone()
    }

    fn check_instruction(&mut self, line: &ParsedLine) {
        match &line.parsed {
            LexedLine::Instruction {
                name,
                operands,
                size,
            } => {
                let name = name.as_str();
                match name {
                    "add" | "sub" => {
                        self.verify_two_args(operands, Rules::NONE, Rules::NO_IMMEDIATE, line);
                        match &operands[..] {
                            [op1, op2, ] if op1.affects_memory() && op2.affects_memory()
                            => {
                                self.errors.push(SemanticError::new(
                                    line.clone(),
                                    "Invalid operands addressing mode, at most one of the two operands can read/write to memory.".to_string(),
                                ));
                            }
                            _ => {}
                        };
                        match (self.get_size_of_instruction(line), operands.first(), operands.get(1)) {
                            (Some(LexedSize::Byte), _, Some(LexedOperand::Register(LexedRegisterType::Address | LexedRegisterType::SP, _))) => {
                                self.errors.push(SemanticError::new(
                                    line.clone(),
                                    "Byte size not allowed when destination operand is address".to_string(),
                                ));
                            }
                            _ => {} //cannot reach this
                        };
                        self.verify_size(SizeRules::AnySize, line);
                        self.verify_size_if_immediate(operands, line, size, LexedSize::Word);
                    }
                    "move" => {
                        self.verify_two_args(operands, Rules::NONE, Rules::NO_IMMEDIATE, line);
                        self.verify_size(SizeRules::AnySize, line);
                        self.verify_size_if_immediate(operands, line, size, LexedSize::Word);
                    }
                    "adda" | "suba" => {
                        self.verify_two_args(operands, Rules::NONE, Rules::ONLY_A_REG, line);
                        self.verify_size(SizeRules::OnlyLongOrWord, line);
                        self.verify_size_if_immediate(operands, line, size, LexedSize::Word);
                    }

                    "divs" | "divu" | "muls" | "mulu" => {
                        self.verify_two_args(operands, Rules::NO_A_REG, Rules::ONLY_D_REG, line);
                        self.verify_size(SizeRules::NoSize, line);
                        self.verify_size_if_immediate(operands, line, size, LexedSize::Word);
                    }
                    "swap" => {
                        self.verify_one_arg(operands, Rules::ONLY_D_REG, line);
                        self.verify_size(SizeRules::NoSize, line);
                    }
                    "clr" => {
                        self.verify_one_arg(operands, Rules::NO_A_REG_OR_IMMEDIATE, line);
                        self.verify_size(SizeRules::AnySize, line);
                    }
                    "exg" => {
                        self.verify_two_args(operands, Rules::ONLY_REG, Rules::ONLY_REG, line);
                        self.verify_size(SizeRules::NoSize, line);
                    }
                    "neg" => {
                        self.verify_one_arg(
                            operands,
                            Rules::ONLY_D_REG_OR_INDIRECT_OR_ADDRESS,
                            line,
                        );
                        self.verify_size(SizeRules::AnySize, line);
                    }
                    "ext" => {
                        self.verify_one_arg(operands, Rules::ONLY_D_REG, line);
                        self.verify_size(SizeRules::OnlyLongOrWord, line);
                    }
                    "tst" => {
                        self.verify_one_arg(operands, Rules::NO_IMMEDIATE, line);
                        self.verify_size(SizeRules::AnySize, line);
                        self.verify_size_if_immediate(operands, line, size, LexedSize::Word);
                    }
                    "cmp" => {
                        self.verify_two_args(operands, Rules::NONE, Rules::ONLY_REG, line);
                        self.verify_size(SizeRules::AnySize, line);
                        self.verify_size_if_immediate(operands, line, size, LexedSize::Word);
                    }
                    "bcc" | "bcs" | "beq" | "bne" | "blt" | "ble" | "bgt" | "bge" | "bls" | "bhi"
                    | "bpl" | "bmi" | "blo" | "bhs" | "bvc" | "bvs"
                    //other
                    | "bsr" | "bra" => {
                        self.verify_one_arg(operands, Rules::ONLY_ADDRESS, line);
                        self.verify_size(SizeRules::NoSize, line);
                    }
                    "scc" | "scs" | "seq" | "sne" | "sge" | "sgt" | "sle" | "sls" | "slt"
                    | "shi" | "smi" | "spl" | "svc" | "svs" | "slo" | "shs" | "sf" | "st" => {
                        self.verify_one_arg(operands, Rules::NO_A_REG_OR_IMMEDIATE, line);
                        self.verify_size(SizeRules::NoSize, line);
                    }
                    "dbcc" | "dbcs" | "dbeq" | "dbne" | "dbge" | "dbgt" | "dble" | "dbls" | "dblt"
                    | "dbhi" | "dbmi" | "dbpl" | "dbvc" | "dbvs" | "dbf" | "dbt" | "dbhs" | "dblo"
                    | "dbra" => {
                        self.verify_two_args(
                            operands,
                            Rules::ONLY_D_REG,
                            Rules::ONLY_ADDRESS,
                            line,
                        );
                        self.verify_size(SizeRules::NoSize, line);
                    }
                    "link" => {
                        self.verify_two_args(operands, Rules::ONLY_A_REG, Rules::ONLY_IMMEDIATE, line);
                        self.verify_size(SizeRules::NoSize, line);
                    }
                    "unlk" => {
                        self.verify_one_arg(operands, Rules::ONLY_A_REG, line);
                        self.verify_size(SizeRules::NoSize, line);
                    }
                    "not" => {
                        self.verify_one_arg(operands, Rules::NO_A_REG_OR_IMMEDIATE, line);
                        self.verify_size(SizeRules::AnySize, line);
                    }
                    "addi" | "andi" | "ori" | "eori" | "subi" | "cmpi" => {
                        self.verify_two_args(operands, Rules::ONLY_IMMEDIATE, Rules::NO_A_REG_OR_IMMEDIATE, line);
                        self.verify_size(SizeRules::AnySize, line);
                    }
                    "movea" => {
                        self.verify_two_args(operands, Rules::NONE, Rules::ONLY_A_REG, line);
                        self.verify_size(SizeRules::OnlyLongOrWord, line);
                    }
                    "cmpa" => {
                        self.verify_two_args(operands, Rules::NONE, Rules::ONLY_A_REG, line);
                        self.verify_size(SizeRules::OnlyLongOrWord, line);
                    }
                    "cmpm" => {
                        self.verify_two_args(operands, Rules::ONLY_POST_INCREMENT, Rules::ONLY_POST_INCREMENT, line);
                    }
                    "or" | "and" | "eor" => {
                        self.verify_two_args(
                            operands,
                            Rules::NO_A_REG,
                            Rules::NO_A_REG_OR_IMMEDIATE,
                            line,
                        );
                        self.verify_size(SizeRules::AnySize, line);
                    }
                    "lea" => {
                        self.verify_two_args(operands, Rules::ONLY_INDIRECT_OR_ABSOLUTE, Rules::ONLY_A_REG, line);
                        self.verify_size(SizeRules::NoSize, line);
                    }
                    "pea" => {
                        self.verify_one_arg(operands, Rules::ONLY_INDIRECT_OR_ABSOLUTE, line);
                        self.verify_size(SizeRules::NoSize, line);
                    }
                    "addq" | "subq" => {
                        //.b not allowed for address registers
                        self.verify_two_args(operands, Rules::ONLY_IMMEDIATE, Rules::NO_IMMEDIATE, line);
                        self.verify_value_bounds_if_immediate(operands, 0, line, 1, 8);
                        self.verify_size(SizeRules::AnySize, line);
                        match operands.get(1) {
                            Some(LexedOperand::Register(LexedRegisterType::Address, _)) => {
                                if *size == LexedSize::Byte {
                                    self.errors.push(SemanticError::new(
                                        line.clone(),
                                        "Byte size not allowed for address register".to_string(),
                                    ))
                                }
                            }
                            _ => {}
                        }
                    }
                    "moveq" => {
                        self.verify_two_args(operands, Rules::ONLY_IMMEDIATE, Rules::ONLY_D_REG, line);
                        self.verify_value_bounds_if_immediate(operands, 0, line, -127, 127);
                        self.verify_size(SizeRules::NoSize, line);
                    }
                    "movem" => {
                        self.verify_size(SizeRules::OnlyLongOrWord, line);
                        match &operands[..] {
                            [LexedOperand::RegisterRange { .. } | LexedOperand::Register(_, _), op2] => {
                                self.verify_arg_rule(op2, Rules::ONLY_INDIRECT_OR_ABSOLUTE, line, 2);
                            }
                            [op1, LexedOperand::RegisterRange { .. } | LexedOperand::Register(_, _)] => {
                                self.verify_arg_rule(op1, Rules::ONLY_INDIRECT_OR_ABSOLUTE, line, 1);
                            }
                            _ => {
                                self.errors.push(SemanticError::new(
                                    line.clone(),
                                    format!("Invalid operands for movem instruction, register list and memory, or memory and register list, received \"{}\" operands", operands.len()),
                                ));
                            }
                        }
                    }
                    "jmp" => {
                        self.verify_one_arg(
                            operands,
                            Rules::ONLY_INDIRECT_OR_ABSOLUTE,
                            line,
                        );
                        self.verify_size(SizeRules::NoSize, line);
                    }
                    "jsr" => {
                        self.verify_one_arg(
                            operands,
                            Rules::ONLY_INDIRECT_OR_ABSOLUTE,
                            line,
                        );
                        self.verify_size(SizeRules::NoSize, line);
                    }
                    "trap" => {
                        self.verify_one_arg(operands, Rules::ONLY_IMMEDIATE, line);
                        self.verify_size(SizeRules::NoSize, line);
                        match &operands[..] {
                            [LexedOperand::Immediate(value)] => {
                                match self.get_immediate_value(value) {
                                    Ok(value) => {
                                        if value != 15 {
                                            self.errors.push(SemanticError::new(
                                                line.clone(),
                                                format!(
                                                    "Only implemented TRAP is 15 for IO, received \"{}\"",
                                                    value
                                                ),
                                            ));
                                        }
                                    }
                                    Err(e) => self.errors.push(SemanticError::new(line.clone(), e)),
                                }
                            }
                            _ => {}
                        }
                    }
                    "rts" => {
                        self.verify_size(SizeRules::NoSize, line);
                        if !operands.is_empty() {
                            self.errors.push(SemanticError::new(
                                line.clone(),
                                "RTS instruction does not accept operands".to_string(),
                            ));
                        }
                    }
                    "lsl" | "lsr" | "asr" | "asl" | "rol" | "ror" => {
                        self.verify_two_args(
                            operands,
                            Rules::NO_A_REG,
                            Rules::NO_A_REG_OR_IMMEDIATE,
                            line,
                        );
                        self.verify_value_bounds_if_immediate(operands, 0, line, 0, 8);
                        self.verify_size(SizeRules::AnySize, line);
                    }
                    "btst" | "bclr" | "bchg" | "bset" => {
                        self.verify_two_args(
                            operands,
                            Rules::NO_A_REG,
                            Rules::NO_A_REG_OR_IMMEDIATE,
                            line,
                        );
                        self.verify_size(SizeRules::NoSize, line);
                        self.verify_value_bounds_if_immediate(operands, 0, line, 0, 0xFF);
                    }

                    _ => self.errors.push(SemanticError::new(
                        line.clone(),
                        format!("Unknown instruction: \"{}\"", name),
                    )),
                }
            }
            _ => self.errors.push(SemanticError::new(
                line.clone(),
                format!("Invalid line: \"{}\"", line.line),
            )),
        }
    }

    fn verify_directive(&mut self, line: &ParsedLine) {
        match &line.parsed {
            LexedLine::Directive { args, name, size } => match name.as_str() {
                "equ" => {
                    if args.len() != 3 {
                        self.errors.push(SemanticError::new(
                            line.clone(),
                            "Invalid number of arguments for directive equ".to_string(),
                        ));
                    }
                }
                "org" => {
                    if args.len() != 2 {
                        self.errors.push(SemanticError::new(
                            line.clone(),
                            "Invalid number of arguments for directive org".to_string(),
                        ));
                    }
                }
                "dc" => {
                    self.verify_size(SizeRules::AnySize, line);
                    match &args[..] {
                        [_, ..] => {
                            for (i, arg) in args[1..].iter().enumerate() {
                                match arg {
                                    _ if arg.starts_with('\'') && arg.ends_with('\'') => {}
                                    _ => {
                                        match self.get_absolute_value(arg) {
                                            Ok(_) => {}
                                            Err(_) => self.errors.push(SemanticError::new(
                                                line.clone(),
                                                format!("Invalid argument \"{}\" for directive dc at position {}", arg, i + 1),
                                            )),
                                        }
                                    }
                                }
                            }
                        }
                        _ => self.errors.push(SemanticError::new(
                            line.clone(),
                            "No arguments for directive dc".to_string(),
                        )),
                    }
                }
                "ds" => {
                    self.verify_size(SizeRules::AnySize, line);
                    match &args[..] {
                        [_] => self.errors.push(SemanticError::new(
                            line.clone(),
                            format!(
                                "Missing arguments for directive: \"{}\", expected 1, got {}",
                                "ds",
                                args.len()
                            ),
                        )),
                        [_, arg] => match self.get_absolute_value(arg) {
                            Ok(_) => {}
                            Err(_) => self.errors.push(SemanticError::new(
                                line.clone(),
                                format!("Invalid argument for directive: \"{}\"", "ds"),
                            )),
                        },
                        _ => self.errors.push(SemanticError::new(
                            line.clone(),
                            format!(
                                "Too many arguments for label directive: \"{}\", expected 1, got {}",
                                "ds",
                                args.len()
                            ),
                        )),
                    }
                }
                "dcb" => {
                    self.verify_size(SizeRules::AnySize, line);
                    match &args[..] {
                        [_] | [_, _] => {
                            self.errors.push(SemanticError::new(
                                line.clone(),
                                format!("Too few arguments for label directive: \"{}\", expected 2, got {}", "ds", args.len()),
                            ));
                        }
                        [_, first, second] => {
                            match self.get_absolute_value(first) {
                                Ok(_) => {}
                                Err(_) => {
                                    self.errors.push(SemanticError::new(
                                        line.clone(),
                                        "Invalid length argument for dcb directive".to_string(),
                                    ));
                                }
                            }
                            let el = match self.get_absolute_value(second) {
                                Ok(v) => v,
                                Err(_) => {
                                    self.errors.push(SemanticError::new(
                                        line.clone(),
                                        "Invalid default value argument for dcb directive"
                                            .to_string(),
                                    ));
                                    return;
                                }
                            };
                            let max = 1 << size.to_bits_word_default();
                            if el > max {
                                self.errors.push(SemanticError::new(
                                    line.clone(),
                                    format!("Value exceeds the limit of the specified size{}", max),
                                ));
                            }
                        }
                        _ => {
                            self.errors.push(SemanticError::new(
                                line.clone(),
                                format!(
                                    "Too many arguments for directive: \"{}\", expected 2, got {}",
                                    "dcb",
                                    args.len()
                                ),
                            ));
                        }
                    }
                }
                _ => {
                    self.errors.push(SemanticError::new(
                        line.clone(),
                        format!("Unknown directive {}", name),
                    ));
                }
            },
            _ => panic!("Line is not a directive"),
        }
    }
    fn verify_two_args(
        &mut self,
        args: &[LexedOperand],
        rule1: Rules,
        rule2: Rules,
        line: &ParsedLine,
    ) {
        match args {
            [first, second] => {
                self.verify_arg_rule(first, rule1, line, 1);
                self.verify_arg_rule(second, rule2, line, 2);
            }
            _ => self.errors.push(SemanticError::new(
                line.clone(),
                format!("Expected two operands, received \"{}\"", args.len()),
            )),
        }
    }

    fn verify_one_arg(&mut self, args: &[LexedOperand], rule: Rules, line: &ParsedLine) {
        match args {
            [first] => {
                self.verify_arg_rule(first, rule, line, 1);
            }
            _ => self.errors.push(SemanticError::new(
                line.clone(),
                format!("Expected one operand, received {}", args.len()),
            )),
        }
    }
    fn verify_size_if_immediate(
        &mut self,
        args: &[LexedOperand],
        line: &ParsedLine,
        size: &LexedSize,
        default: LexedSize,
    ) {
        let size_value = match size {
            //TODO do i really have to use i64?
            LexedSize::Byte | LexedSize::Word | LexedSize::Long => {
                size.to_bits_word_default() as i64
            }
            LexedSize::Unspecified => match default {
                LexedSize::Byte | LexedSize::Word | LexedSize::Long => {
                    default.to_bits_word_default() as i64
                }
                _ => panic!("Invalid default size"),
            },
            _ => 0,
        };
        match args {
            [LexedOperand::Immediate(value), ..] => match self.get_immediate_value(value) {
                Ok(parsed) => match num_to_signed_base(parsed, size_value) {
                    Ok(_) => {}
                    Err(_) => self.errors.push(SemanticError::new(
                        line.clone(),
                        format!(
                            "Immediate value \"{}\" is not a valid {} bits number, received \"{}\"",
                            value, size_value, parsed
                        ),
                    )),
                },
                Err(_) => {}
            },
            _ => {}
        }
    }

    fn verify_value_bounds_if_immediate(
        &mut self,
        args: &[LexedOperand],
        arg_position: usize,
        line: &ParsedLine,
        min: i64,
        max: i64,
    ) {
        match args.get(arg_position) {
            Some(LexedOperand::Immediate(value)) => {
                match self.get_immediate_value(value.as_str()) {
                    Ok(n) => {
                        if n < min || n > max {
                            self.errors.push(SemanticError::new(
                                line.clone(),
                                format!("Immediate value \"{}\" out of range, must be between \"{}\" and \"{}\" ", value, min, max),
                            ));
                        }
                    }
                    Err(_) => {}
                }
            }
            _ => {}
        }
    }
    fn verify_arg_rule(
        &mut self,
        arg: &LexedOperand,
        rule: Rules,
        line: &ParsedLine,
        arg_position: usize,
    ) {
        let arg_position_name = match arg_position {
            1 => "first",
            2 => "second",
            _ => "unknown",
        };
        let addressing_mode = self.get_addressing_mode(arg);
        match addressing_mode {
            Ok(mode) => {
                if (mode.bits() & rule.bits()) != 0 {
                    self.errors.push(SemanticError::new(
                        line.clone(),
                        format!(
                            "Incorrect {} operand addressing mode, received \"{}\", expected \"{}\"",
                            arg_position_name, mode.get_name(), rule.get_valid_addressing_modes()
                        ),
                    ));
                }
            }
            Err(e) => {
                let error = e.to_string();
                self.errors.push(SemanticError::new(line.clone(), error));
            }
        }
    }

    fn get_size_of_instruction(&self, line: &ParsedLine) -> Option<LexedSize> {
        match &line.parsed {
            LexedLine::Instruction { size, .. } => Some(size.clone()),
            _ => None,
        }
    }
    fn verify_size(&mut self, rule: SizeRules, line: &ParsedLine) {
        match &line.parsed {
            LexedLine::Instruction { size, .. } | LexedLine::Directive { size, .. } => match rule {
                _ if *size == LexedSize::Unknown => {
                    self.errors.push(SemanticError::new(
                        line.clone(),
                        format!(
                            "Unknown size, expected any of \"{}\"",
                            rule.get_valid_sizes()
                        ),
                    ));
                }

                SizeRules::NoSize => {
                    if *size != LexedSize::Unspecified || *size == LexedSize::Unknown {
                        self.errors.push(SemanticError::new(
                            line.clone(),
                            "Invalid size, instruction is not sized".to_string(),
                        ))
                    }
                }
                SizeRules::OnlyLongOrWord => {
                    if *size != LexedSize::Long
                        && *size != LexedSize::Word
                        && *size != LexedSize::Unspecified
                    {
                        self.errors.push(SemanticError::new(
                            line.clone(),
                            "Invalid size, instruction must be long or word".to_string(),
                        ));
                    }
                }
                SizeRules::AnySize => {
                    match *size {
                        LexedSize::Byte => {
                            match &line.parsed {
                                LexedLine::Instruction { operands, .. } => {
                                    //check if it has any address register
                                    let has_address_reg = operands.iter().find(|op| {
                                        matches!(
                                            op,
                                            LexedOperand::Register(LexedRegisterType::Address, _)
                                        )
                                    });
                                    if let Some(_op) = has_address_reg {
                                        self.errors.push(SemanticError::new(
                                            line.clone(),
                                            "Invalid size, address register cannot be used with byte size".to_string(),
                                        ));
                                    }
                                }
                                _ => {}
                            }
                        }
                        _ => {}
                    }
                }
            },
            _ => panic!("Line is not an instruction or directive"),
        }
    }
    fn get_addressing_mode(&mut self, operand: &LexedOperand) -> Result<AdrMode, String> {
        match operand {
            LexedOperand::Register(reg_type, reg_name)
            | LexedOperand::RegisterWithSize(reg_type, reg_name, _) => match reg_type {
                LexedRegisterType::Data => match reg_name[1..].parse::<i8>() {
                    Ok(reg) if (0..8).contains(&reg) => Ok(AdrMode::D_REG),
                    _ => Err("Invalid data register".to_string()),
                },
                LexedRegisterType::Address => match reg_name[1..].parse::<i8>() {
                    Ok(reg) if (0..8).contains(&reg) => Ok(AdrMode::A_REG),
                    _ => Err("Invalid address register".to_string()),
                },
                LexedRegisterType::SP => Ok(AdrMode::A_REG),
            },
            LexedOperand::RegisterRange { .. } => Ok(AdrMode::REG_LIST),
            LexedOperand::Immediate(num) => match self.get_immediate_value(num) {
                Ok(_) => Ok(AdrMode::IMMEDIATE),
                Err(e) => Err(format!("Invalid immediate: {}", e)),
            },
            LexedOperand::PostIndirect(boxed_arg) => match boxed_arg.as_ref() {
                LexedOperand::Register(LexedRegisterType::Address | LexedRegisterType::SP, _) => {
                    Ok(AdrMode::INDIRECT_POST_INCREMENT)
                }
                _ => Err(
                    "Invalid post indirect value, only address or SP registers allowed".to_string(),
                ),
            },
            LexedOperand::PreIndirect(boxed_arg) => match boxed_arg.as_ref() {
                LexedOperand::Register(LexedRegisterType::Address | LexedRegisterType::SP, _) => {
                    Ok(AdrMode::INDIRECT_PRE_DECREMENT)
                }
                _ => Err("Invalid pre indirect value, only An or SP registers allowed".to_string()),
            },

            LexedOperand::Indirect(boxed_arg) => match boxed_arg.as_ref() {
                LexedOperand::Register(LexedRegisterType::Address | LexedRegisterType::SP, _) => {
                    Ok(AdrMode::INDIRECT)
                }
                _ => Err("Invalid indirect value, only address registers allowed".to_string()),
            },

            LexedOperand::IndirectDisplacement {
                operand, offset, ..
            } => {
                match parse_absolute_expression(offset, &self.labels) {
                    Ok(num) => {
                        if !(-(1 << 15)..=1 << 15).contains(&num) {
                            return Err(format!(
                                "Invalid offset \"{}\", must be between -32768 and 32768",
                                num
                            ));
                        }
                    }
                    Err(_) => {
                        return Err(format!(
                            "Offset \"{}\" is not a valid decimal number",
                            offset
                        ))
                    }
                }
                match operand.as_ref() {
                    LexedOperand::Register(LexedRegisterType::Address, _)
                    | LexedOperand::Register(LexedRegisterType::SP, _) => {
                        Ok(AdrMode::INDIRECT_DISPLACEMENT)
                    }
                    _ => Err("Invalid indirect value, only address registers allowed".to_string()),
                }
            }
            LexedOperand::IndirectIndex {
                operands, offset, ..
            } => {
                if !offset.is_empty() {
                    match offset.parse::<i64>() {
                        Ok(num) => {
                            if !(-(1 << 7)..=1 << 7).contains(&num) {
                                return Err(
                                    "Invalid offset, must be between -128 and 128".to_string()
                                );
                            }
                        }
                        Err(_) => return Err("Offset is not a valid decimal number".to_string()),
                    }
                }
                match operands[..] {
                    [LexedOperand::Register(LexedRegisterType::Address, _), LexedOperand::Register(_, _) | LexedOperand::RegisterWithSize(_, _, _)] =>
                    {
                        let op2 = &operands[1];
                        if let LexedOperand::RegisterWithSize(_, _, size) = op2 {
                            if *size == LexedSize::Byte {
                                return Err(
                                    "Byte size in register is not allowed for index indirect"
                                        .to_string(),
                                );
                            }
                        }
                        Ok(AdrMode::INDIRECT_INDEX)
                    }
                    _ => Err(
                        "Invalid operands for index indirect, only \"(An, Dn/An)\" allowed"
                            .to_string(),
                    ),
                }
            }
            LexedOperand::Label(name) => {
                if self.labels.contains_key(name) {
                    Ok(AdrMode::ADDRESS)
                } else {
                    Err("Label does not exist".to_string())
                }
            }

            LexedOperand::Absolute(data) => match self.get_absolute_value(data) {
                Ok(_) => Ok(AdrMode::ADDRESS),
                Err(e) => Err(format!("Invalid absolute: {}", e)),
            },
            LexedOperand::Other(_) => Err("Unknown operand".to_string()),
        }
    }
    fn get_immediate_value(&self, num: &str) -> Result<i64, String> {
        self.get_absolute_value(&num[1..])
    }
    fn get_absolute_value(&self, num: &str) -> Result<i64, String> {
        match parse_absolute_expression(num, &self.labels) {
            Ok(num) => Ok(num),
            Err(e) => Err(e),
        }
    }
}
