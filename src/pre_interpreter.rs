use core::panic;
use std::collections::HashMap;
use std::fmt;
use crate::{
    lexer::{LexedLine, LexedOperand, ParsedLine, RegisterType, Size},
    utils::parse_char_or_num,
};
#[derive(Debug)]
struct Directive {
    pub args: Vec<i32>,
    pub name: String,
    pub size: Size,
}
#[derive(Debug)]
struct Label {
    directive: Option<Directive>,
    name: String,
    address: usize,
}
#[derive(Debug)]
pub enum Operand {
    Register(RegisterType, u8),
    Immediate(i32),
    Indirect {
        offset: String,
        operand: Box<Operand>,
    },
    IndirectWithDisplacement {
        offset: i32,
        operands: Vec<Operand>,
    },
    PostIndirect(Box<Operand>),
    PreIndirect(Box<Operand>),
    Address(u32),
}

pub struct PreInterpreter {
    labels: HashMap<String, Label>,
    line_addresses: Vec<usize>,
    instructions: Vec<InstructionLine>,
}

pub struct InstructionLine {
    instruction: Instruction,
    address: usize,
    parsed_line: ParsedLine,
}
#[derive(Debug)]
pub struct Instruction {
    opcode: String,
    operands: Vec<Operand>,
    size: Size,
}
impl fmt::Debug for InstructionLine {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("InstructionLine")
         .field("instruction", &self.instruction)
         .field("address", &self.address)
         //.field("parsed_line", &self.parsed_line)
         .finish()
    }
}
impl PreInterpreter {
    pub fn new(lines: &Vec<ParsedLine>) -> PreInterpreter {
        let mut pre_interpreter = PreInterpreter {
            labels: HashMap::new(),
            line_addresses: Vec::new(),
            instructions: Vec::new(),
        };
        pre_interpreter.load(lines);
        pre_interpreter
    }

    pub fn debug_print(&self) {
        if self.labels.len() == 0 {
            println!("\n[NO LABELS]\n");
        } else {
            println!("\n[LABELS]\n");
            for (key, value) in &self.labels {
                println!("{}: {:?}", key, value);
            }
        }
        println!("\n[INSTRUCTIONS]\n");
        for (i, value) in self.instructions.iter().enumerate() {
            println!("{}) {:#?}", i, value);
        }
    }

    fn load(&mut self, lines: &Vec<ParsedLine>) {
        self.populate_label_map(lines);
        self.parse_instruction_lines(lines);
    }
    fn populate_label_map(&mut self, lines: &Vec<ParsedLine>) {
        let mut last_address = 4096; //same as ORG $1000
        for line in lines.iter(){
            self.line_addresses.push(last_address);
            match &line.parsed {
                LexedLine::Directive { args, .. } => {
                    if args[0] == "org" {
                        let hex = args[1].trim_start_matches("$");
                        last_address = usize::from_str_radix(hex, 16).expect("Invalid hex value");
                    }
                }
                LexedLine::Label { name, .. } => {
                    let name = name.to_string();
                    self.labels.insert(
                        name.clone(),
                        Label {
                            name,
                            directive: None,
                            address: last_address,
                        },
                    );
                }
                LexedLine::LabelDirective { name, directive } => {
                    let parsed_directive_args = Directive {
                        name: directive.name.clone(),
                        args: directive
                            .args
                            .iter()
                            .map(|x| parse_char_or_num(&x.value).unwrap() as i32)
                            .collect(),
                        size: directive.size.clone(),
                    };
                    self.labels.insert(
                        name.clone(),
                        Label {
                            name: name.clone(),
                            directive: Some(parsed_directive_args),
                            address: last_address,
                        },
                    );
                    match name.as_str() {
                        "dcb" | "ds" => {
                            let bytes = directive.args[0].value.parse::<usize>().unwrap();
                            last_address += bytes * directive.size.clone() as usize;
                        }
                        "dc" => {
                            let args = directive.args.len();
                            last_address += args * directive.size.clone() as usize;
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
            last_address += 4;
        }
    }

    fn parse_instruction_lines(&mut self, lines: &Vec<ParsedLine>) {
        for (i, line) in lines.iter().enumerate() {
            match &line.parsed {
                LexedLine::Instruction {
                    name,
                    operands,
                    size,
                } => {
                    let parsed_operands: Vec<Operand> =
                        operands.iter().map(|x| self.parse_operand(x)).collect();
                    let instruction = Instruction {
                        opcode: name.clone(),
                        operands: parsed_operands,
                        size: size.clone(),
                    };
                    let instuction_line = InstructionLine {
                        instruction,
                        address: self.line_addresses[i],
                        parsed_line: line.clone(),
                    };
                    self.instructions.push(instuction_line);
                }
                _ => {}
            }
        }
    }

    fn parse_operand(&mut self, operand: &LexedOperand) -> Operand {
        match operand {
            LexedOperand::Register(register_type, register_name) => match register_type {
                RegisterType::Address => {
                    let num: u8 = register_name[1..]
                        .parse()
                        .expect("failed to parse register");
                    Operand::Register(RegisterType::Address, num)
                }
                RegisterType::Data => {
                    let num: u8 = register_name[1..]
                        .parse()
                        .expect("failed to parse register");
                    Operand::Register(RegisterType::Data, num)
                }
                RegisterType::SP => Operand::Register(RegisterType::SP, 0),
            },
            LexedOperand::Address(address) => match u32::from_str_radix(&address[1..], 16) {
                Ok(address) => Operand::Address(address),
                Err(_) => panic!("Invalid address"),
            },
            LexedOperand::Label(label) => {
                let label = self.labels.get(label).expect("label not found");
                Operand::Address(label.address as u32)
            }
            LexedOperand::Indirect { offset, operand } => {
                let parsed_operand = self.parse_operand(operand);
                Operand::Indirect {
                    offset: offset.clone(),
                    operand: Box::new(parsed_operand),
                }
            }
            LexedOperand::IndirectWithDisplacement { offset, operands } => {
                let parsed_operands: Vec<Operand> =
                    operands.iter().map(|x| self.parse_operand(x)).collect();
                Operand::IndirectWithDisplacement {
                    offset: offset.clone().parse().unwrap(),
                    operands: parsed_operands,
                }
            }
            LexedOperand::PostIndirect(operand) => {
                let parsed_operand = self.parse_operand(operand);
                Operand::PostIndirect(Box::new(parsed_operand))
            }
            LexedOperand::PreIndirect(operand) => {
                let parsed_operand = self.parse_operand(operand);
                Operand::PreIndirect(Box::new(parsed_operand))
            }
            LexedOperand::Immediate(num) => {
                match num.chars().collect::<Vec<char>>()[..] {
                    ['#', '0', 'b'] => {
                        let value =
                            i32::from_str_radix(&num[3..], 2).expect("Invalid binary number");
                        Operand::Immediate(value)
                    }
                    ['#', '0', 'o'] => {
                        let value =
                            i32::from_str_radix(&num[3..], 8).expect("Invalid octal number");
                        Operand::Immediate(value)
                    }
                    ['#', '$', ..] => {
                        let value = i32::from_str_radix(&num[2..], 16).expect("Invalid hex number");
                        Operand::Immediate(value)
                    }
                    ['#', ..] => {
                        //TODO not sure if this is correct, the label is 64bits while the immediate is 32bits
                        //it's unlikely for the program to exceed 16bits so it SHOULD be fine
                        let value = self.labels.get(&num[1..]).expect("Invalid label");
                        Operand::Immediate(value.address as i32)
                    }
                    _ => panic!("Invalid immediate value"),
                }
            }
            _ => panic!("Invalid operand"),
        }
    }
}
