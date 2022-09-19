use crate::{lexer::{Line, Operand, ParsedLine, RegisterType}};
use bitflags::bitflags;
use std::collections::HashMap;
#[derive(Debug, Clone)]

pub struct SyntaxError {
    line: ParsedLine,
    line_index: usize,
    error: String,
}
impl SyntaxError {
    pub fn new(line: ParsedLine, line_index: usize, error: String) -> Self {
        Self {
            line,
            line_index,
            error,
        }
    }
    pub fn get_message(&self) -> String {
        format!("Error on line {}: {}", self.line_index + 1, self.error)
    }
}

bitflags! {
    struct Rules: usize {
        const None = 0;
        const NoDataRegister = 1<<0;
        const NoAddressRegister = 1<<1;
        const NoAddressRegisterIndirect = 1<<2;
        const NoAddressRegisterIndirectWithPostIncrement = 1<<3;
        const NoAddressRegisterIndirectWithPreDecrement = 1<<4;
        const NoAddressRegisterIndirectWithDisplacement = 1<<5;
        const NoImmediate = 1<<6;
        const NoLabel = 1<<7;
        const NoAddress = 1<<8;
        const NoIndirect = 1<<9;
    }
    struct AddressingMode: usize {
        const DataRegister = 1<<0;
        const AddressRegister = 1<<1;
        const AddressRegisterIndirect = 1<<2;
        const AddressRegisterIndirectWithPostIncrement = 1<<3;
        const AddressRegisterIndirectWithPreDecrement = 1<<4;
        const AddressRegisterIndirectWithDisplacement = 1<<5;
        const Immediate = 1<<6;
        const Label = 1<<7;
        const Address = 1<<8;
        const Indirect = 1<<9;
    }
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
        for (index, line) in lines.iter().enumerate() {
            match &line.parsed {
                Line::Label { name, args } => {
                    if self.labels.contains_key(name) {
                        self.errors.push(SyntaxError {
                            line: line.clone(),
                            line_index: index,
                            error: format!("Label {} already exists", name),
                        })
                    } else {
                        self.labels.insert(name.to_string(), name.to_string());
                    }
                }
                _ => {}
            }
        }
        for (index, line) in lines.iter().enumerate() {
            match &line.parsed {
                Line::Empty
                | Line::Comment { .. }
                | Line::Label { .. }
                | Line::Directive { .. } => {}

                Line::Instruction { .. } => {
                    self.check_instruction(line, index);
                }
                _ => self.errors.push(SyntaxError {
                    line: line.clone(),
                    line_index: index,
                    error: format!("Unknown line: \"{}\"", line.line),
                }),
            }
        }
    }

    pub fn get_errors(&self) -> Vec<SyntaxError> {
        self.errors.clone()
    }

    fn check_instruction(&mut self, line: &ParsedLine, line_index: usize) {
        match &line.parsed {
            Line::Instruction {
                name,
                operands,
                size,
            } => {
                let name = name.as_str();

                match name {
                    "move" => {
                        self.verify_two_args(
                            operands,
                            Rules::None,
                            Rules::NoImmediate,
                            line,
                            line_index,
                        );
                    }
                    _ => self.errors.push(SyntaxError {
                        line: line.clone(),
                        line_index,
                        error: format!("Unknown instruction: \"{}\"", name),
                    }),
                }
            }
            _ => self.errors.push(SyntaxError {
                line: line.clone(),
                line_index,
                error: format!("Invalid line instruction: \"{}\"", line.line),
            }),
        }
    }

    fn verify_two_args(
        &mut self,
        args: &Vec<Operand>,
        disallow1: Rules,
        disallow2: Rules,
        line: &ParsedLine,
        line_index: usize,
    ) {
        match &args[..] {
            [first, second] => {
                self.disallow_arg(first, disallow1, line, line_index);
                self.disallow_arg(second, disallow2, line, line_index);
            }
            _ => self.errors.push(SyntaxError {
                line: line.clone(),
                line_index,
                error: format!("Expected two operands, received \"{}\"", args.len()),
            }),
        }
    }

    fn disallow_arg(
        &mut self,
        arg: &Operand,
        disallow: Rules,
        line: &ParsedLine,
        line_index: usize,
    ) {
        let addressing_mode = self.get_addressing_mode(arg);
        match addressing_mode {
            Ok(mode) => {
                if (mode.bits & disallow.bits) != 0 {
                    self.errors.push(SyntaxError {
                        line: line.clone(),
                        line_index,
                        error: format!("Invalid addressing mode at: \"{}\"", line.line),
                    })
                }
            }
            Err(e) => {
                let error = e.to_string();
                self.errors.push(SyntaxError {
                    line: line.clone(),
                    line_index,
                    error: format!("{} at line: \"{}\"", error, line.line),
                })
            }
        }
    }
    fn get_addressing_mode(&mut self, operand: &Operand) -> Result<AddressingMode, &str> {
        match operand {
            Operand::Register(RegisterType::Data, _) => Ok(AddressingMode::DataRegister),
            Operand::Register(RegisterType::Address, _) => Ok(AddressingMode::AddressRegister),
            Operand::Register(RegisterType::SP, _) => Ok(AddressingMode::AddressRegister),
            Operand::Immediate(num) => {
                //TODO not sure if this is correct
                if self.is_valid_number(num) {
                    Ok(AddressingMode::Immediate)
                } else {
                    Err("Invalid immediate value")
                }
            },
            Operand::PostIndirect(boxed_arg) => {
                let operand = boxed_arg.as_ref();
                match &operand {
                    Operand::Register(RegisterType::Address, _) => Ok(AddressingMode::AddressRegisterIndirectWithPostIncrement),
                    Operand::Register(RegisterType::SP, _) => Ok(AddressingMode::AddressRegisterIndirectWithPostIncrement),
                    _ => Err("Invalid post indirect value, only address registers allowed")
                }
            }
            Operand::PreIndirect(boxed_arg) => {
                let operand = boxed_arg.as_ref();
                match &operand {
                    Operand::Register(RegisterType::Address, _) => Ok(AddressingMode::AddressRegisterIndirectWithPreDecrement),
                    Operand::Register(RegisterType::SP, _) => Ok(AddressingMode::AddressRegisterIndirectWithPreDecrement),
                    _ => Err("Invalid pre indirect value, only address registers allowed")
                }
            }
            Operand::Indirect { operand , ..} => {
                let operand = operand.as_ref();
                match &operand {
                    Operand::Register(RegisterType::Address, _) => Ok(AddressingMode::Indirect),
                    Operand::Register(RegisterType::SP, _) => Ok(AddressingMode::Indirect),
                    _ => Err("Invalid indirect value, only address registers allowed")

                }
            }
            Operand::IndirectWithDisplacement { operands, .. } => {
                match operands[..]{
                    [Operand::Register(RegisterType::Data, _), Operand::Register(RegisterType::Address, _)] => Ok(AddressingMode::AddressRegisterIndirectWithDisplacement),
                    [Operand::Register(RegisterType::Data, _), Operand::Register(RegisterType::SP, _)] => Ok(AddressingMode::AddressRegisterIndirectWithDisplacement),
                    _ => Err("Invalid indirect with displacement value, only data and address registers allowed")
                }
            }
            Operand::Other(_) => Err("Unknown operand"),
            Operand::Label(name) => {
                if self.labels.contains_key(name) {
                    Ok(AddressingMode::Label)
                } else {
                    Err("Label does not exist")
                }
            },
            Operand::Address {..} => Ok(AddressingMode::Address),
        }
    }
    fn is_valid_number(&self, num: &str) -> bool {
        let chars = num.chars().collect::<Vec<char>>();
        match chars[..] {
            ['#','0','b'] => {
                let num = &num[3..];
                num.chars().all(|c| c == '0' || c == '1')
            },
            ['#','0','o'] => {
                let num = &num[3..];
                num.chars().all(|c| c >= '0' && c <= '7')
            },
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
