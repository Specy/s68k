use std::collections::HashMap;

use crate::lexer::{ParsedLine, Line, Operand, RegisterType};
struct Error {
    line: ParsedLine,
    line_index: usize,
    error: String,
}



enum AddressingMode {
    DataRegister = 1<<0,
    AddressRegister = 1<<1,
    AddressRegisterIndirect = 1<<2,
    AddressRegisterIndirectWithPostIncrement = 1<<3,
    AddressRegisterIndirectWithPreDecrement = 1<<4,
    AddressRegisterIndirectWithDisplacement = 1<<5,
    Immediate = 1<<6,
    Label = 1<<7,
    Address = 1<<8,
    Indirect = 1<<9,
}
enum Rules {
    None = 0,
    NoDataRegister = 1<<0,
    NoAddressRegister = 1<<1,
    NoAddressRegisterIndirect = 1<<2,
    NoAddressRegisterIndirectWithPostIncrement = 1<<3,
    NoAddressRegisterIndirectWithPreDecrement = 1<<4,
    NoAddressRegisterIndirectWithDisplacement = 1<<5,
    NoImmediate = 1<<6,
    NoLabel = 1<<7,
    NoAddress = 1<<8,
    NoIndirect = 1<<9,
}
struct SyntaxChecker{
    labels: HashMap<String, String>,
    errors: Vec<Error>,
    lines: Vec<ParsedLine>
}
impl SyntaxChecker{
    pub fn new() -> SyntaxChecker{
        SyntaxChecker{
            errors: Vec::new(),
            lines: Vec::new(),
            labels: HashMap::new()
        }
    }

    pub fn check(&mut self, lines: Vec<ParsedLine>) {
        self.lines = lines.iter().map(|x| x.clone()).collect();
        for (index, line) in lines.iter().enumerate(){
            match &line.parsed {
                Line::Label {name, args} => {
                    if self.labels.contains_key(name) {
                        self.errors.push(Error{
                            line: line.clone(),
                            line_index: index,
                            error: format!("Label {} already exists", name)
                        })
                    } else {
                        self.labels.insert(name.to_string(), name.to_string());
                    }
                }
                _ => {}
            }
        }
        for (index, line) in lines.iter().enumerate(){
            match &line.parsed {
                Line::Empty | Line::Comment{..} | Line::Label {..} | Line:: Directive {..}=> {}

                Line::Instruction { .. } => {
                    self.check_instruction(line, index);
                }
                _ => {
                    self.errors.push(Error{
                        line: line.clone(),
                        line_index: index,
                        error: format!("Unknown line: \"{}\"", line.line)
                    })
                }
            }
        }
    }




    fn check_instruction(&mut self, line: &ParsedLine, line_index: usize){
        match &line.parsed {
            Line::Instruction { name, operands, size } => {
                let name = name.as_str();
                match name {
                    "move" => {
                        self.verify_two_args(operands, Rules::None, Rules::NoImmediate, line, line_index);
                    }
                    _ => {
                        self.errors.push(Error{
                            line: line.clone(),
                            line_index: line_index,
                            error: format!("Unknown instruction: \"{}\"", line.line)
                        })
                    }
                }
            }
            _ => {
                self.errors.push(Error{
                    line: line.clone(),
                    line_index: line_index,
                    error: format!("Invalid line instruction: \"{}\"", line.line)
                })
            }
        }
    }

    fn verify_two_args(&mut self, args: &Vec<Operand>, disallow1: usize, disallow2: usize, line: &ParsedLine, line_index: usize){
        match &args[..]{
            [first, second] => {
                self.disallow_arg(first, disallow1, line, line_index);
                self.disallow_arg(second, disallow2, line, line_index);
            }
            _ => {
                self.errors.push(Error{
                    line: line.clone(),
                    line_index: line_index,
                    error: format!("Expected two operands, received \"{}\" at line {}",args.len(), line.line)
                })
            }
        }
    }

    fn disallow_arg(&mut self, arg: &Operand, disallow: usize, line: &ParsedLine, line_index: usize){
        let addressing_mode = self.get_addressing_mode(arg);
        match addressing_mode{
            Ok(mode) => {
                if (disallow & mode as usize) != 0 {
                    self.errors.push(Error{
                        line: line.clone(),
                        line_index,
                        error: format!("Invalid addressing mode at line: \"{}\"", line.line)
                    })
                }
            }
            Err(e) => {
                let error = e.to_string();
                self.errors.push(Error{
                    line: line.clone(),
                    line_index,
                    error: format!("{} at line: \"{}\"", error, line.line)
                })
            }
        }
    }
    fn get_addressing_mode(&mut self, operand: &Operand) -> Result<AddressingMode, &str>{
        match operand {
            Operand::Register(RegisterType::Data, _) => Ok(AddressingMode::DataRegister),
            Operand::Register(RegisterType::Address, _) => Ok(AddressingMode::AddressRegister),
            Operand::Register(RegisterType::SP, _) => Ok(AddressingMode::AddressRegister),
            Operand::Immediate(num) => {
                let mut num = num.chars();
                num.next();
                let num = num.as_str();
                //TODO not sure if this is correct
                if num.starts_with("0x") || num.starts_with("0b") || num.starts_with("0o") || num.starts_with("$") {
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
}



