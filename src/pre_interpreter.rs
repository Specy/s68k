use crate::{
    lexer::{LabelDirective, LexedLine, LexedOperand, LexedRegisterType, ParsedLine, Size},
    utils::parse_char_or_num,
};
use core::panic;
use std::collections::HashMap;
use std::fmt;
#[derive(Debug)]
pub enum Directive {
    DC { data: Vec<u8> },
    DS { data: Vec<u8> },
    DCB { data: Vec<u8> },
}
#[derive(Debug)]
pub struct Label {
    pub directive: Option<Directive>,
    pub name: String,
    pub address: usize,
}
#[derive(Debug, Clone)]
pub enum RegisterType {
    Address,
    Data,
}
#[derive(Debug)]
pub enum Operand {
    Register(RegisterType, u8), //maybe use usize?
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
    Address(u32), //maybe use usize?
}

pub struct PreInterpreter {
    pub labels: HashMap<String, Label>,
    line_addresses: Vec<usize>,
    pub instructions: Vec<InstructionLine>,
    start_address: usize,
    final_instrucion_address: usize,
}

pub struct InstructionLine {
    pub instruction: Instruction,
    pub address: usize,
    pub parsed_line: ParsedLine,
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
            start_address: 0,
            final_instrucion_address: 0,
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
    pub fn get_start_address(&self) -> usize {
        self.start_address
    }
    pub fn get_final_instruction_address(&self) -> usize {
        self.final_instrucion_address
    }
    fn load(&mut self, lines: &Vec<ParsedLine>) {
        self.populate_label_map(lines);
        self.parse_instruction_lines(lines);

        let start = self.labels.get("start");
        match start {
            Some(label) => {
                self.start_address = label.address;
            }
            None => {}
        }
        self.final_instrucion_address = match self.instructions.last() {
            Some(instruction) => instruction.address,
            None => 0,
        };
    }
    fn populate_label_map(&mut self, lines: &Vec<ParsedLine>) {
        let mut last_address = 4096; //same as ORG $1000
        for line in lines.iter() {
            self.line_addresses.push(last_address);
            match &line.parsed {
                LexedLine::Directive { args, .. } => {
                    if args[0] == "org" {
                        let hex = args[1].trim_start_matches("$");
                        let parsed = usize::from_str_radix(hex, 16).expect("Invalid hex value");
                        if parsed < last_address {
                            panic!("The address of the ORG directive ({}) must be greater than the previous address ({})",parsed, last_address);
                        }
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
                    let parsed_directive = self.parse_label_directive(directive, line);
                    println!("parsed directive: {:?}", parsed_directive);
                    self.labels.insert(
                        name.clone(),
                        Label {
                            name: name.clone(),
                            directive: Some(parsed_directive),
                            address: last_address,
                        },
                    );
                    match directive.name.as_str() {
                        "dcb" | "ds" => {
                            let bytes = directive.args[0].value.parse::<usize>().expect(
                                format!("Invalid number at line {}", line.line_index).as_str(),
                            );
                            last_address += bytes * directive.size.clone() as usize;
                        }
                        "dc" => {
                            let args = directive.args.len();
                            println!("args: {}", args);
                            last_address += args * directive.size.clone() as usize;
                        }
                        _ => {}
                    }
                }
                LexedLine::Instruction { .. } => {
                    last_address += 4;
                }

                _ => {}
            }
            //this aligns the address to the next 4 byte boundary, it works by incrementing the address by 3 then
            //masking the first 2 bits to 0
            last_address = (last_address + 3) & !3;
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
                    let parsed_operands: Vec<Operand> = operands
                        .iter()
                        .map(|x| self.parse_operand(x, line))
                        .collect();
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

    fn parse_label_directive(&self, directive: &LabelDirective, line: &ParsedLine) -> Directive {
        let parsed_args: Vec<u32> = directive
            .args
            .iter()
            .map(|x| {
                parse_char_or_num(&x.value).expect(
                    format!(
                        "Invalid numerical directive argument at line {}",
                        line.line_index
                    )
                    .as_str(),
                ) as u32
            })
            .collect();
        //TODO finish this
        match directive.name.as_str() {
            "dc" => Directive::DC {
                data: match &directive.size {
                    Size::Byte => parsed_args.iter().map(|x| *x as u8).collect(),
                    Size::Word => parsed_args
                        .iter()
                        .flat_map(|x| (*x as u16).to_be_bytes())
                        .collect(),
                    Size::Long => parsed_args
                        .iter()
                        .flat_map(|x| (*x as u32).to_be_bytes())
                        .collect(),
                    _ => panic!("Invalid or missing size for DC directive"),
                },
            },
            "ds" => {
                if directive.size == Size::Unknown || directive.size == Size::Unspecified {
                    panic!("Invalid or missing size for DS directive");
                }
                let data = match parsed_args[..] {
                    [amount] => vec![0; amount as usize * directive.size.clone() as usize / 8],
                    _ => panic!("Invalid number of arguments for DS directive"),
                };
                Directive::DS { data }
            }
            "dcb" => {
                let data = match directive.size {
                    Size::Long => match parsed_args[..] {
                        [size, default] => vec![default as u32; size as usize]
                            .iter()
                            .flat_map(|x| (*x as u32).to_be_bytes())
                            .collect(),
                        _ => panic!("Invalid number of arguments for DCB directive"),
                    },
                    Size::Word => match parsed_args[..] {
                        [size, default] => vec![default as u16; size as usize]
                            .iter()
                            .flat_map(|x| (*x as u16).to_be_bytes())
                            .collect(),
                        _ => panic!("Invalid number of arguments for DCB directive"),
                    },
                    Size::Byte => match parsed_args[..] {
                        [size, default] => vec![default as u8; size as usize],
                        _ => panic!("Invalid number of arguments for DCB directive"),
                    },
                    _ => panic!("Invalid or missing size for DCB directive"),
                };
                Directive::DCB { data }
            }
            _ => panic!("Invalid directive"),
        }
    }
    fn parse_operand(&mut self, operand: &LexedOperand, line: &ParsedLine) -> Operand {
        match operand {
            LexedOperand::Register(register_type, register_name) => match register_type {
                LexedRegisterType::Address => {
                    let num: u8 = register_name[1..]
                        .parse()
                        .expect("Failed to parse register");
                    Operand::Register(RegisterType::Address, num)
                }
                LexedRegisterType::Data => {
                    let num: u8 = register_name[1..]
                        .parse()
                        .expect("Failed to parse register");
                    Operand::Register(RegisterType::Data, num)
                }
                LexedRegisterType::SP => Operand::Register(RegisterType::Address, 7),
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
                let parsed_operand = self.parse_operand(operand, line);
                Operand::Indirect {
                    offset: offset.clone(),
                    operand: Box::new(parsed_operand),
                }
            }
            LexedOperand::IndirectWithDisplacement { offset, operands } => {
                let parsed_operands: Vec<Operand> = operands
                    .iter()
                    .map(|x| self.parse_operand(x, line))
                    .collect();
                Operand::IndirectWithDisplacement {
                    offset: offset.clone().parse().expect(
                        format!("Invalid numerical offset at line {}", line.line_index).as_str(),
                    ),
                    operands: parsed_operands,
                }
            }
            LexedOperand::PostIndirect(operand) => {
                let parsed_operand = self.parse_operand(operand, line);
                Operand::PostIndirect(Box::new(parsed_operand))
            }
            LexedOperand::PreIndirect(operand) => {
                let parsed_operand = self.parse_operand(operand, line);
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
