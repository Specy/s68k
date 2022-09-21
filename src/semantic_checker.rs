use crate::{
    constants::EQU,
    lexer::{Line, Operand, ParsedLine, RegisterType, Size},
};
use bitflags::bitflags;
use std::collections::HashMap;
#[derive(Debug, Clone)]

pub struct SyntaxError {
    line: ParsedLine,
    error: String,
}
impl SyntaxError {
    pub fn new(line: ParsedLine, error: String) -> Self {
        Self { line, error }
    }
    pub fn get_message(&self) -> String {
        format!("Error on line {}: {}", self.line.line_index + 1, self.error)
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
    errors: Vec<SyntaxError>,
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
                Line::Label { name, .. } | Line::LabelDirective { name,.. } => {
                    if self.labels.contains_key(name) {
                        self.errors.push(SyntaxError::new(
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
                Line::Empty | Line::Comment { .. } => {}

                Line::Label { .. } => {
                    self.verify_label(line);
                }

                //TODO make sure directives work as expected
                Line::LabelDirective {..}=> {
                    self.verify_directive(line);
                }

                Line::Instruction { .. } => {
                    self.check_instruction(line);
                }
                _ => self.errors.push(SyntaxError::new(
                    line.clone(),
                    format!("Unknown line: \"{}\"", line.line),
                )),
            }
        }
    }

    pub fn get_errors(&self) -> Vec<SyntaxError> {
        self.errors.clone()
    }

    fn check_instruction(&mut self, line: &ParsedLine) {
        match &line.parsed {
            Line::Instruction { name, operands, .. } => {
                let name = name.as_str();
                match name {
                    "move" | "add" | "sub" => {
                        self.verify_two_args(operands, Rules::NONE, Rules::NO_IMMEDIATE, line);
                        self.verify_instruction_size(SizeRules::AnySize, line);
                    }
                    "adda" => {
                        self.verify_two_args(operands, Rules::NONE, Rules::ONLY_A_REG, line);
                        self.verify_instruction_size(SizeRules::AnySize, line);
                    }

                    "divs" | "divu" | "muls" | "mulu" => {
                        //TODO not sure about the rule1
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
                    }
                    "cmp" => {
                        //TODO check rule1
                        self.verify_two_args(operands, Rules::NO_A_REG, Rules::NO_IMMEDIATE, line);
                        self.verify_instruction_size(SizeRules::AnySize, line);
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
                        //TODO make sure it's only dreg
                        self.verify_one_arg(operands, Rules::ONLY_REG, line);
                    }
                    "or" | "and" | "eor" => {
                        //TODO verify both rules
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
                    _ => self.errors.push(SyntaxError {
                        line: line.clone(),
                        error: format!("Unknown instruction: \"{}\"", name),
                    }),
                }
            }
            _ => self.errors.push(SyntaxError {
                line: line.clone(),
                error: format!("Invalid line: \"{}\"", line.line),
            }),
        }
    }

    fn verify_directive(&mut self, line: &ParsedLine) {
        match &line.parsed {
            Line::Directive { args } => match &args[..] {
                [_, _, ..] if args[1].eq(EQU) => {
                    if args.len() != 3 {
                        self.errors.push(SyntaxError::new(
                            line.clone(),
                            format!("Invalid number of arguments for directive equ"),
                        ));
                    }
                }
                _ => {
                    self.errors
                        .push(SyntaxError::new(line.clone(), format!("Unknown directive")));
                }
            },
            _ => panic!("Line is not a directive"),
        }
    }
    fn verify_label(&mut self, line: &ParsedLine) {
        match &line.parsed {
            Line::LabelDirective {
                directive,
                name,
            } => {
                if directive.size == Size::Unknown || directive.size == Size::Unspecified {
                    self.errors.push(SyntaxError::new(
                        line.clone(),
                        format!(
                            "Unknown or unspecified size for label directive: \"{}\"",
                            name
                        ),
                    ));
                }
                if directive.args.len() == 0 {
                    self.errors.push(SyntaxError::new(
                        line.clone(),
                        format!("No arguments for label directive: \"{}\"", name),
                    ));
                }
                match directive.name.as_str() {
                    "dc" => {}
                    "ds" => {
                        if directive.args.len() > 1 {
                            self.errors.push(SyntaxError::new(
                                line.clone(),
                                format!("Too many arguments for label directive: \"{}\"", name),
                            ));
                        }
                    }
                    "dcb" => match directive.args.len() {
                        1 => {
                            self.errors.push(SyntaxError::new(
                                line.clone(),
                                format!("Too few arguments for label directive: \"{}\"", name),
                            ));
                        }
                        2 => {}
                        _ => {
                            self.errors.push(SyntaxError::new(
                                line.clone(),
                                format!("Too many arguments for label directive: \"{}\"", name),
                            ));
                        }
                    },
                    _ => self.errors.push(SyntaxError::new(
                        line.clone(),
                        format!(
                            "Unknown label directive: \"{}\" at label: \"{}\" ",
                            directive.name, name
                        ),
                    )),
                }
            }
            Line::Label { .. } => {}
            _ => panic!("Line is not a label"),
        }
    }
    fn verify_two_args(
        &mut self,
        args: &Vec<Operand>,
        rule1: Rules,
        rule2: Rules,
        line: &ParsedLine,
    ) {
        match &args[..] {
            [first, second] => {
                self.verify_arg_rule(first, rule1, line, 1);
                self.verify_arg_rule(second, rule2, line, 2);
            }
            _ => self.errors.push(SyntaxError::new(
                line.clone(),
                format!("Expected two operands, received \"{}\"", args.len()),
            )),
        }
    }

    fn verify_one_arg(&mut self, args: &Vec<Operand>, rule: Rules, line: &ParsedLine) {
        match &args[..] {
            [first] => {
                self.verify_arg_rule(first, rule, line, 1);
            }
            _ => self.errors.push(SyntaxError::new(
                line.clone(),
                format!("Expected one operand, received \"{}\"", args.len()),
            )),
        }
    }
    fn verify_arg_rule(
        &mut self,
        arg: &Operand,
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
                    self.errors.push(SyntaxError::new(
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
                self.errors.push(SyntaxError::new(
                    line.clone(),
                    format!("{} at line: \"{}\"", error, line.line),
                ));
            }
        }
    }
    fn verify_instruction_size(&mut self, rule: SizeRules, line: &ParsedLine) {
        match &line.parsed {
            Line::Instruction { size, .. } => match rule {
                SizeRules::NoSize => {
                    if *size != Size::Unspecified || *size == Size::Unknown {
                        self.errors.push(SyntaxError::new(
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
                        self.errors.push(SyntaxError::new(
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
                        self.errors.push(SyntaxError::new(
                            line.clone(),
                            format!("Unknown size at: \"{}\"", line.line),
                        ))
                    }
                }
            },
            _ => panic!("Line is not an instruction"),
        }
    }
    fn get_addressing_mode(&mut self, operand: &Operand) -> Result<AddressingMode, &str> {
        //TODO check if registers are between 0 and 7
        match operand {
            Operand::Register(RegisterType::Data, _) => Ok(AddressingMode::D_REG),
            Operand::Register(RegisterType::Address, _) => Ok(AddressingMode::A_REG),
            Operand::Register(RegisterType::SP, _) => Ok(AddressingMode::A_REG),

            Operand::Immediate(num) => {
                if self.is_valid_number(num) {
                    Ok(AddressingMode::IMMEDIATE)
                } else {
                    Err("Invalid immediate value")
                }
            },
            Operand::PostIndirect(boxed_arg) => {
                let operand = boxed_arg.as_ref();
                match &operand {
                    Operand::Register(RegisterType::Address, _) => Ok(AddressingMode::INDIRECT_POST_INCREMENT),
                    Operand::Register(RegisterType::SP, _) => Ok(AddressingMode::INDIRECT_POST_INCREMENT),
                    _ => Err("Invalid post indirect value, only address or SP registers allowed")
                }
            }
            Operand::PreIndirect(boxed_arg) => {
                let operand = boxed_arg.as_ref();
                match &operand {
                    Operand::Register(RegisterType::Address, _) => Ok(AddressingMode::INDIRECT_PRE_DECREMENT),
                    Operand::Register(RegisterType::SP, _) => Ok(AddressingMode::INDIRECT_PRE_DECREMENT),
                    _ => Err("Invalid pre indirect value, only address or SP registers allowed")
                }
            }
            Operand::Indirect { operand , offset, ..} => {
                let operand = operand.as_ref();
                match offset.parse::<i32>() {
                    Ok(num) => {
                        if num > 1<<15 || num < -(1<<15) {
                            return Err("Invalid offset, must be between -32768 and 32767");
                        }
                    },
                    Err(_) => return Err("Offset is not a number")
                }
                match &operand {
                    Operand::Register(RegisterType::Address, _) => Ok(AddressingMode::INDIRECT),
                    Operand::Register(RegisterType::SP, _) => Ok(AddressingMode::INDIRECT),
                    _ => Err("Invalid indirect value, only address registers allowed")

                }
            }
            Operand::IndirectWithDisplacement { operands, offset,.. } => {
                match offset.parse::<i32>() {
                    Ok(num) => {
                        if num > 1<<7 || num < -(1<<7) {
                            return Err("Invalid offset, must be between -128 and 128");
                        }
                    },
                    Err(_) => return Err("Offset is not a number")
                }
                match operands[..]{
                    [Operand::Register(RegisterType::Data, _), Operand::Register(RegisterType::Address, _)] => Ok(AddressingMode::INDIRECT_DISPLACEMENT),
                    [Operand::Register(RegisterType::Data, _), Operand::Register(RegisterType::SP, _)] => Ok(AddressingMode::INDIRECT_DISPLACEMENT),
                    _ => Err("Invalid indirect with displacement value, only data and address registers allowed")
                }
            }
            Operand::Other(_) => Err("Unknown operand"),
            Operand::Label(name) => {
                if self.labels.contains_key(name) {
                    Ok(AddressingMode::LABEL)
                } else {
                    Err("Label does not exist")
                }
            },
            Operand::Address {..} => Ok(AddressingMode::ADDRESS),
        }
    }
    fn is_valid_number(&self, num: &str) -> bool {
        let chars = num.chars().collect::<Vec<char>>();
        match chars[..] {
            ['#', '0', 'b'] => {
                let num = &num[3..];
                num.chars().all(|c| c == '0' || c == '1')
            }
            ['#', '0', 'o'] => {
                let num = &num[3..];
                num.chars().all(|c| c >= '0' && c <= '7')
            }
            ['#', '$', ..] => {
                let num = &num[2..];
                let num = num.chars().collect::<Vec<char>>();
                num.iter().all(|c| c.is_ascii_hexdigit())
            }
            ['#', ..] => {
                let num = &num[1..].to_string();
                let num = num.chars().collect::<Vec<char>>();
                num.iter().all(|c| c.is_ascii_digit())
            }
            _ => false,
        }
    }
}
