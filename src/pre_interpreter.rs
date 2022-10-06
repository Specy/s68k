use crate::{
    instructions::{
        Condition, Instruction, Operand, RegisterOperand, RegisterType, ShiftDirection, Size,
    },
    interpreter::Register,
    lexer::{LabelDirective, LexedLine, LexedOperand, LexedRegisterType, LexedSize, ParsedLine},
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

pub struct PreInterpreter {
    pub labels: HashMap<String, Label>,
    line_addresses: Vec<usize>,
    pub instructions: Vec<InstructionLine>,
    start_address: usize,
    final_instrucion_address: usize,
}
#[derive(Clone)]
pub struct InstructionLine {
    pub instruction: Instruction,
    pub address: usize,
    pub parsed_line: ParsedLine,
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
    pub fn get_instructions_map(&self) -> HashMap<usize, InstructionLine> {
        self.instructions
            .iter()
            .map(|x| (x.address, x.clone()))
            .collect()
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
                    let instuction_line = InstructionLine {
                        instruction: self.parse_instruction(name, parsed_operands, size),
                        address: self.line_addresses[i],
                        parsed_line: line.clone(),
                    };
                    self.instructions.push(instuction_line);
                }
                _ => {}
            }
        }
    }

    fn parse_instruction(
        &self,
        name: &String,
        mut operands: Vec<Operand>,
        size: &LexedSize,
    ) -> Instruction {
        //TODO add better error logging
        if operands.len() == 2 {
            let (op1, op2) = (operands.remove(0), operands.remove(0));
            match name.as_str() {
                "move" => Instruction::MOVE(op1, op2, self.get_size(size, Size::Word)),
                "add" => Instruction::ADD(op1, op2, self.get_size(size, Size::Word)),
                "sub" => Instruction::SUB(op1, op2, self.get_size(size, Size::Word)),
                "adda" => Instruction::ADDA(
                    op1,
                    self.extract_register(op2).unwrap(),
                    self.get_size(size, Size::Word),
                ),
                "divs" => Instruction::DIVS(op1, self.extract_register(op2).unwrap()),
                "divu" => Instruction::DIVU(op1, self.extract_register(op2).unwrap()),
                "muls" => Instruction::MULS(op1, self.extract_register(op2).unwrap()),
                "exg" => Instruction::EXG(self.extract_register(op1).unwrap(), op2),
                "cmp" => Instruction::CMP(op1, op2, self.get_size(size, Size::Word)),
                "or" => Instruction::OR(op1, op2),
                "and" => Instruction::AND(op1, op2),
                "eor" => Instruction::EOR(op1, op2),
                "lsl" => Instruction::LSd(
                    op1,
                    op2,
                    ShiftDirection::Left,
                    self.get_size(size, Size::Word),
                ),
                "lsr" => Instruction::LSd(
                    op1,
                    op2,
                    ShiftDirection::Right,
                    self.get_size(size, Size::Word),
                ),
                "asl" => Instruction::LSd(
                    op1,
                    op2,
                    ShiftDirection::Left,
                    self.get_size(size, Size::Word),
                ),
                "asr" => Instruction::LSd(
                    op1,
                    op2,
                    ShiftDirection::Right,
                    self.get_size(size, Size::Word),
                ),
                "rol" => Instruction::ROd(
                    op1,
                    op2,
                    ShiftDirection::Left,
                    self.get_size(size, Size::Word),
                ),
                "ror" => Instruction::ROd(
                    op1,
                    op2,
                    ShiftDirection::Right,
                    self.get_size(size, Size::Word),
                ),
                "btst" => Instruction::BTST(op1, op2),
                "bset" => Instruction::BSET(op1, op2),
                "bclr" => Instruction::BCLR(op1, op2),
                "bchg" => Instruction::BCHG(op1, op2),
                _ => panic!("Invalid instruction"),
            }
        } else if operands.len() == 1 {
            let op = operands[0].clone();
            match name.as_str() {
                "clr" => Instruction::CLR(op, self.get_size(size, Size::Word)),
                "neg" => Instruction::NEG(op, self.get_size(size, Size::Word)),
                "ext" => Instruction::EXT(
                    self.extract_register(op).unwrap(),
                    self.get_size(size, Size::Word),
                ),
                "tst" => Instruction::TST(op, self.get_size(size, Size::Word)),
                //bcc
                "beq" | "bne" | "blt" | "ble" | "bgt" | "bge" | "blo" | "bls" | "bhi" | "bhs"
                | "bsr" | "bra" => {
                    let condition = Condition::from_string(name);
                    match condition {
                        Ok(c) => Instruction::Bcc(op, c),
                        Err(e) => panic!("{}", e),
                    }
                }
                //scc
                "scc" | "scs" | "seq" | "sne" | "sge" | "sgt" | "sle" | "sls" | "slt" | "shi"
                | "smi" | "spl" | "svc" | "svs" | "sf" | "st" => {
                    let condition = Condition::from_string(name);
                    match condition {
                        Ok(c) => Instruction::Scc(op, c),
                        Err(e) => panic!("{}", e),
                    }
                }

                "not" => Instruction::NOT(op),
                "jsr" => Instruction::JSR(op),
                _ => panic!("Invalid instruction"),
            }
        } else if operands.len() == 0 {
            match name.as_str() {
                "rts" => Instruction::RTS,
                _ => panic!("Invalid instruction"),
            }
        } else {
            panic!("Invalid instruction");
        }
    }

    fn get_size(&self, size: &LexedSize, default: Size) -> Size {
        match size {
            LexedSize::Byte => Size::Byte,
            LexedSize::Word => Size::Word,
            LexedSize::Long => Size::Long,
            LexedSize::Unspecified => default,
            _ => panic!("Invalid size"),
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
        match directive.name.as_str() {
            "dc" => Directive::DC {
                data: match &directive.size {
                    LexedSize::Byte => parsed_args.iter().map(|x| *x as u8).collect(),
                    LexedSize::Word => parsed_args
                        .iter()
                        .flat_map(|x| (*x as u16).to_be_bytes())
                        .collect(),
                    LexedSize::Long => parsed_args
                        .iter()
                        .flat_map(|x| (*x as u32).to_be_bytes())
                        .collect(),
                    _ => panic!("Invalid or missing size for DC directive"),
                },
            },
            "ds" => {
                if directive.size == LexedSize::Unknown || directive.size == LexedSize::Unspecified
                {
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
                    LexedSize::Long => match parsed_args[..] {
                        [size, default] => vec![default as u32; size as usize]
                            .iter()
                            .flat_map(|x| (*x as u32).to_be_bytes())
                            .collect(),
                        _ => panic!("Invalid number of arguments for DCB directive"),
                    },
                    LexedSize::Word => match parsed_args[..] {
                        [size, default] => vec![default as u16; size as usize]
                            .iter()
                            .flat_map(|x| (*x as u16).to_be_bytes())
                            .collect(),
                        _ => panic!("Invalid number of arguments for DCB directive"),
                    },
                    LexedSize::Byte => match parsed_args[..] {
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

    pub fn extract_register(&self, operand: Operand) -> Result<RegisterOperand, &str> {
        match operand {
            Operand::Register(reg) => Ok(reg),
            _ => Err("Operand is not a register"),
        }
    }
    fn parse_operand(&mut self, operand: &LexedOperand, line: &ParsedLine) -> Operand {
        match operand {
            LexedOperand::Register(register_type, register_name) => match register_type {
                LexedRegisterType::Address => {
                    let num: u8 = register_name[1..]
                        .parse()
                        .expect("Failed to parse register");
                    Operand::Register(RegisterOperand::Address(num))
                }
                LexedRegisterType::Data => {
                    let num: u8 = register_name[1..]
                        .parse()
                        .expect("Failed to parse register");
                    Operand::Register(RegisterOperand::Data(num))
                }
                LexedRegisterType::SP => Operand::Register(RegisterOperand::Address(7)),
            },
            LexedOperand::Address(address) => match usize::from_str_radix(&address[1..], 16) {
                Ok(address) => Operand::Address(address),
                Err(_) => panic!("Invalid address"),
            },
            LexedOperand::Label(label) => {
                let label = self.labels.get(label).expect("label not found");
                Operand::Address(label.address)
            }
            LexedOperand::Indirect { offset, operand } => {
                let parsed_operand = self.parse_operand(operand, line);
                let parsed_operand = self.extract_register(parsed_operand).unwrap();
                let offset = if offset == "" {
                    0
                } else {
                    offset.parse().expect(
                        format!("Invalid numerical offset at line {}", line.line_index).as_str(),
                    )
                };
                Operand::Indirect {
                    offset,
                    operand: parsed_operand,
                }
            }
            LexedOperand::IndirectWithDisplacement { offset, operands } => {
                //TODO not sure how the indirect with displacement works
                let parsed_operands: Vec<RegisterOperand> = operands
                    .iter()
                    .map(|x| {
                        let parsed = self.parse_operand(x, line);
                        self.extract_register(parsed).unwrap()
                    })
                    .collect();
                let offset = if offset == "" {
                    0
                } else {
                    offset.parse().expect(
                        format!("Invalid numerical offset at line {}", line.line_index).as_str(),
                    )
                };
                Operand::IndirectWithDisplacement {
                    offset,
                    operands: parsed_operands,
                }
            }
            LexedOperand::PostIndirect(operand) => {
                let parsed_operand = self.parse_operand(operand, line);
                let parsed_operand = self.extract_register(parsed_operand).unwrap();
                Operand::PostIndirect(parsed_operand)
            }
            LexedOperand::PreIndirect(operand) => {
                let parsed_operand = self.parse_operand(operand, line);
                let parsed_operand = self.extract_register(parsed_operand).unwrap();
                Operand::PreIndirect(parsed_operand)
            }
            LexedOperand::Immediate(num) => match num.chars().collect::<Vec<char>>()[..] {
                ['#', '0', 'b'] => {
                    let value = i32::from_str_radix(&num[3..], 2).expect("Invalid binary number");
                    Operand::Immediate(value as u32)
                }
                ['#', '0', 'o'] => {
                    let value = i32::from_str_radix(&num[3..], 8).expect("Invalid octal number");
                    Operand::Immediate(value as u32)
                }
                ['#', '$', ..] => {
                    let value = i32::from_str_radix(&num[2..], 16).expect("Invalid hex number");
                    Operand::Immediate(value as u32)
                }
                ['#', ..] => {
                    let value = match self.labels.get(&num[1..]) {
                        Some(label) => label.address as i32,
                        None => num[1..].parse().expect("Invalid number"),
                    };
                    Operand::Immediate(value as u32)
                }
                _ => panic!("Invalid immediate value"),
            },
            _ => panic!("Invalid operand"),
        }
    }
}
