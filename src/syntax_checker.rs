use crate::lexer::{Line, Operand, ParsedLine, RegisterType, Size};
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
        const NO_A_REG_INDIRECT_POST_INCREMENT = AddressingMode::INDIRECT_POST_INCREMENT.bits;
        const NO_A_REG_INDIRECT_PRE_DECREMENT = AddressingMode::INDIRECT_PRE_DECREMENT.bits;
        const NO_A_REG_INDIRECT_DISPLACEMENT = AddressingMode::INDIRECT_DISPLACEMENT.bits;
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
pub enum SizeRules {
    NoSize,
    AnySize,
    OnlyLongOrWord,
}
pub struct SyntaxChecker {
    labels: HashMap<String, String>,
    errors: Vec<SyntaxError>,
    lines: Vec<ParsedLine>,
}
impl SyntaxChecker {
    pub fn new(lines: &Vec<ParsedLine>) -> SyntaxChecker {
        let mut syntax_checker = SyntaxChecker {
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
                Line::Label { name, .. } => {
                    if self.labels.contains_key(name) {
                        self.errors.push(SyntaxError {
                            line: line.clone(),
                            error: format!("Label {} already exists", name),
                        })
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

                //TODO check labels
                Line::Label { .. } => {
                    self.verify_label(line);
                }

                //TODO check directives
                Line::Directive { .. } => {}

                Line::Instruction { .. } => {
                    self.check_instruction(line);
                }
                _ => self.errors.push(SyntaxError {
                    line: line.clone(),
                    error: format!("Unknown line: \"{}\"", line.line),
                }),
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

    fn verify_label(&mut self, line: &ParsedLine) {
        match &line.parsed {
            Line::Label {
                directive: Some(data),
                name,
            } => {
                if data.size == Size::Unknown || data.size == Size::Unspecified {
                    self.errors.push(SyntaxError {
                        line: line.clone(),
                        error: format!(
                            "Unknown or unspecified size for label directive: \"{}\"",
                            name
                        ),
                    });
                }
                if data.args.len() == 0 {
                    self.errors.push(SyntaxError {
                        line: line.clone(),
                        error: format!("No arguments for label directive: \"{}\"", name),
                    });
                }
                match data.name.as_str() {
                    "dc" => {}
                    "ds" => {
                        if data.args.len() > 1 {
                            self.errors.push(SyntaxError {
                                line: line.clone(),
                                error: format!(
                                    "Too many arguments for label directive: \"{}\"",
                                    name
                                ),
                            });
                        }
                    }
                    "dcb" => match data.args.len() {
                        1 => {
                            self.errors.push(SyntaxError {
                                line: line.clone(),
                                error: format!(
                                    "Too few arguments for label directive: \"{}\"",
                                    name
                                ),
                            });
                        }
                        2 => {}
                        _ => {
                            self.errors.push(SyntaxError {
                                line: line.clone(),
                                error: format!(
                                    "Too many arguments for label directive: \"{}\"",
                                    name
                                ),
                            });
                        }
                    },
                    _ => self.errors.push(SyntaxError {
                        line: line.clone(),
                        error: format!(
                            "Unknown label directive: \"{}\" at label: \"{}\" ",
                            data.name, name
                        ),
                    }),
                }
            }
            Line::Label {
                directive: None, ..
            } => {}
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
                self.verify_arg_rule(first, rule1, line);
                self.verify_arg_rule(second, rule2, line);
            }
            _ => self.errors.push(SyntaxError {
                line: line.clone(),
                error: format!("Expected two operands, received \"{}\"", args.len()),
            }),
        }
    }

    fn verify_one_arg(&mut self, args: &Vec<Operand>, rule: Rules, line: &ParsedLine) {
        match &args[..] {
            [first] => {
                self.verify_arg_rule(first, rule, line);
            }
            _ => self.errors.push(SyntaxError {
                line: line.clone(),
                error: format!("Expected one operand, received \"{}\"", args.len()),
            }),
        }
    }
    fn verify_arg_rule(&mut self, arg: &Operand, rule: Rules, line: &ParsedLine) {
        let addressing_mode = self.get_addressing_mode(arg);
        match addressing_mode {
            Ok(mode) => {
                if (mode.bits & rule.bits) != 0 {
                    self.errors.push(SyntaxError {
                        line: line.clone(),
                        error: format!("Invalid addressing mode at: \"{}\"", line.line),
                    })
                }
            }
            Err(e) => {
                let error = e.to_string();
                self.errors.push(SyntaxError {
                    line: line.clone(),
                    error: format!("{} at line: \"{}\"", error, line.line),
                })
            }
        }
    }
    fn verify_instruction_size(&mut self, rule: SizeRules, line: &ParsedLine) {
        match &line.parsed {
            Line::Instruction { size, .. } => match rule {
                SizeRules::NoSize => {
                    if *size != Size::Unspecified || *size == Size::Unknown {
                        self.errors.push(SyntaxError {
                            line: line.clone(),
                            error: format!(
                                "Invalid size at: \"{}\", instruction is not sized",
                                line.line
                            ),
                        })
                    }
                }
                SizeRules::OnlyLongOrWord => {
                    if *size != Size::Long && *size != Size::Word {
                        self.errors.push(SyntaxError {
                            line: line.clone(),
                            error: format!(
                                "Invalid size at: \"{}\", instruction must be long or word",
                                line.line
                            ),
                        })
                    }
                }
                SizeRules::AnySize => {
                    if *size == Size::Unknown {
                        self.errors.push(SyntaxError {
                            line: line.clone(),
                            error: format!("Unknown size at: \"{}\"", line.line),
                        })
                    }
                }
            },
            _ => panic!("Line is not an instruction"),
        }
    }
    fn get_addressing_mode(&mut self, operand: &Operand) -> Result<AddressingMode, &str> {
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
            Operand::Indirect { operand , ..} => {
                let operand = operand.as_ref();
                match &operand {
                    Operand::Register(RegisterType::Address, _) => Ok(AddressingMode::INDIRECT),
                    Operand::Register(RegisterType::SP, _) => Ok(AddressingMode::INDIRECT),
                    _ => Err("Invalid indirect value, only address registers allowed")

                }
            }
            Operand::IndirectWithDisplacement { operands, .. } => {
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
