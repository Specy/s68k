use crate::{
    constants::EQU,
    lexer::{LexedLine, LexedOperand, ParsedLine, RegisterType, Size},
    utils::{num_to_signed_base, parse_char_or_num},
};
use bitflags::bitflags;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use wasm_bindgen::prelude::*;
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
    pub fn get_message(&self) -> String {
        format!("Error on line {}: {}", self.line.line_index + 1, self.error)
    }
}
#[wasm_bindgen]
impl SemanticError {
    pub fn wasm_get_message(&self) -> String {
        format!("Error on line {}: {}", self.line.line_index + 1, self.error)
    }
    pub fn wasm_get_line(&self) -> JsValue {
        serde_wasm_bindgen::to_value(&self.line).unwrap()
    }
}

bitflags! {
    struct AddressingMode: usize {
        const D_REG = 1<<0;
        const A_REG = 1<<1;
        const INDIRECT = 1<<9;
        const INDIRECT_POST_INCREMENT = 1<<3;
        const INDIRECT_PRE_DECREMENT = 1<<4;
        const INDIRECT_DISPLACEMENT = 1<<5;
        const IMMEDIATE = 1<<6;
        const LABEL = 1<<7;
        const ADDRESS = 1<<8;
    }
    struct Rules: usize {
        const NONE = 0;
        const NO_D_REG = AddressingMode::D_REG.bits;
        const NO_A_REG = AddressingMode::A_REG.bits;
        const NO_IMMEDIATE = AddressingMode::IMMEDIATE.bits;
        const NO_LABEL = AddressingMode::LABEL.bits;
        const NO_ADDRESS = AddressingMode::ADDRESS.bits;
        const NO_INDIRECT = AddressingMode::INDIRECT.bits;

        const ONLY_REG = !(AddressingMode::D_REG.bits | AddressingMode::A_REG.bits);
        const ONLY_A_REG = !AddressingMode::A_REG.bits;
        const ONLY_D_REG = !AddressingMode::D_REG.bits;
        const ONLY_INDIRECT = !AddressingMode::INDIRECT.bits;
        const ONLY_D_REG_OR_INDIRECT = !(AddressingMode::D_REG.bits | AddressingMode::INDIRECT.bits);
        const ONLY_D_REG_OR_INDIRECT_OR_ADDRESS = !(AddressingMode::D_REG.bits | AddressingMode::INDIRECT.bits | AddressingMode::ADDRESS.bits);
        const ONLY_ADDRESS_OR_LABEL = !(AddressingMode::ADDRESS.bits | AddressingMode::LABEL.bits);
    }
}
//TODO refactor this
impl AddressingMode {
    pub fn get_name(&self) -> String {
        match *self {
            AddressingMode::D_REG => "Dn",
            AddressingMode::A_REG => "An",
            AddressingMode::INDIRECT => "(An)",
            AddressingMode::INDIRECT_POST_INCREMENT => "(An)+",
            AddressingMode::INDIRECT_PRE_DECREMENT => "-(An)",
            AddressingMode::INDIRECT_DISPLACEMENT => "(Dn, An)",
            AddressingMode::IMMEDIATE => "Im",
            AddressingMode::LABEL => "<LABEL>",
            AddressingMode::ADDRESS => "Ea",
            _ => "UNKNOWN",
        }
        .to_string()
    }
}
impl Rules {
    pub fn get_valid_addressing_modes(&self) -> String {
        match *self {
            Rules::NONE => "Im/Dn/An/(An)/Ea/<LABEL>",
            Rules::NO_D_REG => "Im/An/(An)/Ea/<LABEL>",
            Rules::NO_A_REG => "Im/Dn/(An)/Ea/<LABEL>",
            Rules::NO_IMMEDIATE => "Dn/An/(An)/Ea/<LABEL>",
            Rules::NO_LABEL => "Im/Dn/An/(An)",
            Rules::NO_ADDRESS => "Im/Dn/An/(An)",
            Rules::NO_INDIRECT => "Im/Dn/An/Ea/<LABEL>",
            Rules::ONLY_REG => "Dn/An",
            Rules::ONLY_A_REG => "An",
            Rules::ONLY_D_REG => "Dn",
            Rules::ONLY_INDIRECT => "(An)",
            Rules::ONLY_D_REG_OR_INDIRECT => "Dn/(An)",
            Rules::ONLY_D_REG_OR_INDIRECT_OR_ADDRESS => "Dn/(An)/Ea",
            Rules::ONLY_ADDRESS_OR_LABEL => "Ea/<LABEL>",
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
pub struct SemanticChecker {
    labels: HashMap<String, String>,
    errors: Vec<SemanticError>,
    lines: Vec<ParsedLine>,
}
impl SemanticChecker {
    pub fn new(lines: &Vec<ParsedLine>) -> SemanticChecker {
        let mut syntax_checker = SemanticChecker {
            errors: Vec::new(),
            lines: Vec::new(),
            labels: HashMap::new(),
        };
        syntax_checker.check(lines);
        syntax_checker
    }

    pub fn check(&mut self, lines: &Vec<ParsedLine>) {
        self.lines = lines.iter().map(|x| x.clone()).collect();
        for line in lines.iter() {
            match &line.parsed {
                LexedLine::Label { name, .. } | LexedLine::LabelDirective { name, .. } => {
                    if self.labels.contains_key(name) {
                        self.errors.push(SemanticError::new(
                            line.clone(),
                            format!("Label {} already exists", name),
                        ));
                    } else {
                        self.labels.insert(name.to_string(), name.to_string());
                    }
                }
                _ => {}
            }
        }
        for line in lines.iter() {
            match &line.parsed {
                LexedLine::Empty | LexedLine::Comment { .. } => {}

                LexedLine::LabelDirective { .. } => {
                    self.verify_label(line);
                }
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
                    "move" | "add" | "sub" => {
                        self.verify_two_args(operands, Rules::NONE, Rules::NO_IMMEDIATE, line);
                        self.verify_instruction_size(SizeRules::AnySize, line);
                        self.verify_size_if_immediate(operands, line, size, Size::Word);
                    }
                    "adda" => {
                        self.verify_two_args(operands, Rules::NONE, Rules::ONLY_A_REG, line);
                        self.verify_instruction_size(SizeRules::AnySize, line);
                        self.verify_size_if_immediate(operands, line, size, Size::Word);
                    }

                    "divs" | "divu" | "muls" | "mulu" => {
                        self.verify_two_args(operands, Rules::NO_A_REG, Rules::ONLY_D_REG, line);
                        self.verify_instruction_size(SizeRules::NoSize, line);
                    }
                    "swap" => {
                        self.verify_one_arg(operands, Rules::ONLY_D_REG, line);
                        self.verify_instruction_size(SizeRules::NoSize, line);
                    }
                    "clr" => {
                        self.verify_one_arg(operands, Rules::ONLY_D_REG_OR_INDIRECT, line);
                        self.verify_instruction_size(SizeRules::AnySize, line);
                    }
                    "exg" => {
                        self.verify_two_args(operands, Rules::ONLY_REG, Rules::ONLY_REG, line);
                        self.verify_instruction_size(SizeRules::NoSize, line);
                    }
                    "neg" => {
                        self.verify_one_arg(
                            operands,
                            Rules::ONLY_D_REG_OR_INDIRECT_OR_ADDRESS,
                            line,
                        );
                        self.verify_instruction_size(SizeRules::AnySize, line);
                    }
                    "ext" => {
                        self.verify_one_arg(operands, Rules::ONLY_D_REG, line);
                        self.verify_instruction_size(SizeRules::OnlyLongOrWord, line);
                    }
                    "tst" => {
                        self.verify_one_arg(operands, Rules::NO_IMMEDIATE, line);
                        self.verify_instruction_size(SizeRules::AnySize, line);
                        self.verify_size_if_immediate(operands, line, size, Size::Word);
                    }
                    "cmp" => {
                        self.verify_two_args(operands, Rules::NONE, Rules::NO_IMMEDIATE, line);
                        self.verify_instruction_size(SizeRules::AnySize, line);
                        self.verify_size_if_immediate(operands, line, size, Size::Word);
                    }
                    "beq" | "bne" | "blt" | "ble" | "bgt" | "bge" | "blo" | "bls" | "bhi"
                    | "bhs" => {
                        self.verify_one_arg(operands, Rules::ONLY_ADDRESS_OR_LABEL, line);
                        self.verify_instruction_size(SizeRules::NoSize, line);
                    }
                    "scc" | "scs" | "seq" | "sne" | "sge" | "sgt" | "sle" | "sls" | "slt"
                    | "shi" | "smi" | "spl" | "svc" | "svs" | "sf" | "st" => {
                        self.verify_one_arg(operands, Rules::ONLY_ADDRESS_OR_LABEL, line);
                        self.verify_instruction_size(SizeRules::NoSize, line);
                    }
                    "not" => {
                        self.verify_one_arg(operands, Rules::NO_A_REG | Rules::NO_IMMEDIATE, line);
                    }
                    "or" | "and" | "eor" => {
                        self.verify_two_args(
                            operands,
                            Rules::NO_A_REG,
                            Rules::NO_IMMEDIATE | Rules::NO_A_REG,
                            line,
                        );
                        self.verify_instruction_size(SizeRules::NoSize, line);
                    }
                    "lsl" | "lsr" | "asr" | "asl" | "rol" | "ror" => {
                        //TODO i think i need to check fo the size of the immediate value
                        self.verify_two_args(
                            operands,
                            Rules::NO_A_REG,
                            Rules::NO_IMMEDIATE | Rules::NO_A_REG,
                            line,
                        );
                        self.verify_value_bounds_if_immediate(operands, 0, line, 0, 8);
                        self.verify_instruction_size(SizeRules::AnySize, line);
                    }
                    "btst" | "bclr" | "bchg" | "bset" => {
                        self.verify_two_args(
                            operands,
                            Rules::NO_A_REG,
                            Rules::NO_IMMEDIATE | Rules::NO_A_REG,
                            line,
                        );
                        self.verify_instruction_size(SizeRules::NoSize, line);
                    }
                    "bsr" => {
                        self.verify_one_arg(operands, Rules::ONLY_ADDRESS_OR_LABEL, line);
                        self.verify_instruction_size(SizeRules::NoSize, line);
                    }
                    _ => self.errors.push(SemanticError {
                        line: line.clone(),
                        error: format!("Unknown instruction: \"{}\"", name),
                    }),
                }
            }
            _ => self.errors.push(SemanticError {
                line: line.clone(),
                error: format!("Invalid line: \"{}\"", line.line),
            }),
        }
    }

    fn verify_directive(&mut self, line: &ParsedLine) {
        match &line.parsed {
            LexedLine::Directive { args } => match &args[..] {
                [_, _, ..] if args[1] == EQU => {
                    if args.len() != 3 {
                        self.errors.push(SemanticError::new(
                            line.clone(),
                            format!("Invalid number of arguments for directive equ"),
                        ));
                    }
                }
                [_, ..] if args[0] == "org" => {
                    if args.len() != 2 {
                        self.errors.push(SemanticError::new(
                            line.clone(),
                            format!("Invalid number of arguments for directive org"),
                        ));
                    }
                }
                _ => {
                    self.errors.push(SemanticError::new(
                        line.clone(),
                        format!("Unknown directive"),
                    ));
                }
            },
            _ => panic!("Line is not a directive"),
        }
    }
    fn verify_label(&mut self, line: &ParsedLine) {
        match &line.parsed {
            LexedLine::LabelDirective { directive, name } => {
                if directive.size == Size::Unknown || directive.size == Size::Unspecified {
                    self.errors.push(SemanticError::new(
                        line.clone(),
                        format!(
                            "Unknown or unspecified size for label directive: \"{}\"",
                            name
                        ),
                    ));
                }
                match directive.name.as_str() {
                    "dc" => match &directive.args[..] {
                        [] => self.errors.push(SemanticError::new(
                            line.clone(),
                            format!("No arguments for directive dc"),
                        )),
                        [..] => {
                            for (i, arg) in directive.args.iter().enumerate() {
                                match parse_char_or_num(&arg.value) {
                                    Ok(_) => {}
                                    Err(_) => self.errors.push(SemanticError::new(
                                        line.clone(),
                                        format!("Invalid argument \"{}\"for directive dc at position: {}",arg.value, i + 1),
                                    )),
                                }
                            }
                        }
                    },
                    "ds" => match &directive.args[..] {
                        [] => self.errors.push(SemanticError::new(
                            line.clone(),
                            format!("Missing arguments for label directive: \"{}\", expected 1, got {}", name, directive.args.len()),
                        )),
                        [arg] => match arg.value.parse::<u32>() {
                            Ok(_) => {}
                            Err(_) => self.errors.push(SemanticError::new(
                                line.clone(),
                                format!("Invalid argument for label directive: \"{}\"", name),
                            )),
                        },
                        _ => self.errors.push(SemanticError::new(
                            line.clone(),
                            format!("Too many arguments for label directive: \"{}\", expected 1, got {}", name, directive.args.len()),
                        )),
                    },
                    "dcb" => match &directive.args[..] {
                        [] | [_] => {
                            self.errors.push(SemanticError::new(
                                line.clone(),
                                format!("Too few arguments for label directive: \"{}\", expected 2, got {}", name, directive.args.len()),
                            ));
                        }
                        [first, second] => {
                            match first.value.parse::<u32>() {
                                Ok(_) => {}
                                Err(_) => {
                                    self.errors.push(SemanticError::new(
                                        line.clone(),
                                        format!("Invalid length argument for dcb label directive: \"{}\"", name),
                                    ));
                                }
                            }
                            match parse_char_or_num(&second.value) {
                                Ok(_) => {}
                                Err(_) => {
                                    self.errors.push(SemanticError::new(
                                        line.clone(),
                                        format!("Invalid default value argument for dcb label directive: \"{}\"", name),
                                    ));
                                }
                            }
                        }
                        _ => {
                            self.errors.push(SemanticError::new(
                                line.clone(),
                                format!("Too many arguments for label directive: \"{}\", expected 2, got {}", name, directive.args.len()),
                            ));
                        }
                    },
                    _ => self.errors.push(SemanticError::new(
                        line.clone(),
                        format!(
                            "Unknown label directive: \"{}\" at label: \"{}\" ",
                            directive.name, name
                        ),
                    )),
                }
            }
            LexedLine::Label { .. } => {}
            _ => panic!("Line is not a label"),
        }
    }
    fn verify_two_args(
        &mut self,
        args: &Vec<LexedOperand>,
        rule1: Rules,
        rule2: Rules,
        line: &ParsedLine,
    ) {
        match &args[..] {
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

    fn verify_one_arg(&mut self, args: &Vec<LexedOperand>, rule: Rules, line: &ParsedLine) {
        match &args[..] {
            [first] => {
                self.verify_arg_rule(first, rule, line, 1);
            }
            _ => self.errors.push(SemanticError::new(
                line.clone(),
                format!("Expected one operand, received \"{}\"", args.len()),
            )),
        }
    }
    fn verify_size_if_immediate(
        &mut self,
        args: &Vec<LexedOperand>,
        line: &ParsedLine,
        size: &Size,
        default: Size,
    ) {
        let size_value = match size {
            Size::Byte | Size::Word | Size::Long => size.clone() as i64,
            Size::Unspecified => match default {
                Size::Byte | Size::Word | Size::Long => default as i64,
                _ => panic!("Invalid default size"),
            },
            _ => 0,
        };
        match &args[..] {
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
        args: &Vec<LexedOperand>,
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
                if (mode.bits & rule.bits) != 0 {
                    self.errors.push(SemanticError::new(
                        line.clone(),
                        format!(
                            "Incorrect {} operand addressing mode at: \"{}\", received \"{}\", expected \"{}\"",
                            arg_position_name, line.line, mode.get_name(), rule.get_valid_addressing_modes()
                        ),
                    ));
                }
            }
            Err(e) => {
                let error = e.to_string();
                self.errors.push(SemanticError::new(
                    line.clone(),
                    format!("{} at line: \"{}\"", error, line.line),
                ));
            }
        }
    }
    fn verify_instruction_size(&mut self, rule: SizeRules, line: &ParsedLine) {
        match &line.parsed {
            LexedLine::Instruction { size, .. } => match rule {
                SizeRules::NoSize => {
                    if *size != Size::Unspecified || *size == Size::Unknown {
                        self.errors.push(SemanticError::new(
                            line.clone(),
                            format!(
                                "Invalid size at: \"{}\", instruction is not sized",
                                line.line
                            ),
                        ))
                    }
                }
                SizeRules::OnlyLongOrWord => {
                    if *size != Size::Long && *size != Size::Word {
                        self.errors.push(SemanticError::new(
                            line.clone(),
                            format!(
                                "Invalid size at: \"{}\", instruction must be long or word",
                                line.line
                            ),
                        ));
                    }
                }
                SizeRules::AnySize => {
                    if *size == Size::Unknown {
                        self.errors.push(SemanticError::new(
                            line.clone(),
                            format!("Unknown size at: \"{}\"", line.line),
                        ))
                    }
                }
            },
            _ => panic!("Line is not an instruction"),
        }
    }
    fn get_addressing_mode(&mut self, operand: &LexedOperand) -> Result<AddressingMode, &str> {
        match operand {
            LexedOperand::Register(reg_type, reg_name) => match reg_type {
                RegisterType::Data => match reg_name[1..].parse::<i8>() {
                    Ok(reg) if reg >= 0 && reg < 8 => Ok(AddressingMode::D_REG),
                    _ => Err("Invalid data register"),
                },
                RegisterType::Address => match reg_name[1..].parse::<i8>() {
                    Ok(reg) if reg >= 0 && reg < 8 => Ok(AddressingMode::A_REG),
                    _ => Err("Invalid address register"),
                },
                RegisterType::SP => Ok(AddressingMode::A_REG),
            },

            LexedOperand::Immediate(num) => match self.get_immediate_value(num) {
                Ok(_) => Ok(AddressingMode::IMMEDIATE),
                Err(e) => Err(e),
            },
            LexedOperand::PostIndirect(boxed_arg) => match boxed_arg.as_ref() {
                LexedOperand::Register(RegisterType::Address | RegisterType::SP, _) => {
                    Ok(AddressingMode::INDIRECT_POST_INCREMENT)
                }
                _ => Err("Invalid post indirect value, only address or SP registers allowed"),
            },
            LexedOperand::PreIndirect(boxed_arg) => match boxed_arg.as_ref() {
                LexedOperand::Register(RegisterType::Address | RegisterType::SP, _) => {
                    Ok(AddressingMode::INDIRECT_PRE_DECREMENT)
                }
                _ => Err("Invalid pre indirect value, only An or SP registers allowed"),
            },
            LexedOperand::Indirect {
                operand, offset, ..
            } => {
                if offset != "" {
                    match offset.parse::<i64>() {
                        Ok(num) => {
                            if num > 1 << 15 || num < -(1 << 15) {
                                return Err("Invalid offset, must be between -32768 and 32768");
                            }
                        }
                        Err(_) => return Err("Offset is not a valid decimal number"),
                    }
                }
                match operand.as_ref() {
                    LexedOperand::Register(RegisterType::Address, _) => {
                        Ok(AddressingMode::INDIRECT)
                    }
                    LexedOperand::Register(RegisterType::SP, _) => Ok(AddressingMode::INDIRECT),
                    _ => Err("Invalid indirect value, only address registers allowed"),
                }
            }
            LexedOperand::IndirectWithDisplacement {
                operands, offset, ..
            } => {
                match offset.parse::<i64>() {
                    Ok(num) => {
                        if num > 1 << 7 || num < -(1 << 7) {
                            return Err("Invalid offset, must be between -128 and 128");
                        }
                    }
                    Err(_) => return Err("Offset is not a valid decimal number"),
                }
                match operands[..] {
                    [LexedOperand::Register(RegisterType::Address, _), LexedOperand::Register(_, _)] => {
                        Ok(AddressingMode::INDIRECT_DISPLACEMENT)
                    }
                    _ => Err(
                        "Invalid indirect with displacement value, only \"(An, Dn/An)\" allowed",
                    ),
                }
            }
            LexedOperand::Label(name) => {
                if self.labels.contains_key(name) {
                    Ok(AddressingMode::LABEL)
                } else {
                    Err("Label does not exist")
                }
            }
            LexedOperand::Address(data) => match i64::from_str_radix(&data[1..], 16) {
                Ok(_) => Ok(AddressingMode::ADDRESS),
                Err(_) => Err("Invalid hex address"),
            },
            LexedOperand::Other(_) => Err("Unknown operand"),
        }
    }
    fn get_immediate_value(&self, num: &str) -> Result<i64, &str> {
        let chars = num.chars().collect::<Vec<char>>();
        //TODO could probabl get the radix from the number, then do a single check
        match chars[..] {
            ['#', '0', 'b'] => match i64::from_str_radix(&num[3..], 2) {
                Ok(n) => Ok(n),
                Err(_) => Err("Invalid binary number"),
            },
            ['#', '0', 'o'] => match i64::from_str_radix(&num[3..], 8) {
                Ok(n) => Ok(n),
                Err(_) => Err("Invalid octal number"),
            },
            ['#', '$', ..] => match i64::from_str_radix(&num[2..], 16) {
                Ok(n) => Ok(n),
                Err(_) => Err("Invalid hex number"),
            },
            ['#', ..] => {
                //TODO not sure if this should be checked here
                let label = &num[1..];
                if self.labels.contains_key(label) {
                    Ok(1i64 << 31)
                } else {
                    match i64::from_str_radix(&num[1..], 10) {
                        Ok(n) => Ok(n),
                        Err(_) => Err("Invalid decimal number"),
                    }
                }
            }
            _ => Err("Invalid immediate value"),
        }
    }
}
