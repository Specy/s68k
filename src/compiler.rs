use serde::Serialize;
use wasm_bindgen::prelude::wasm_bindgen;

use crate::{
    instructions::{
        DisplacementOperands, Instruction, Operand, RegisterOperand, ShiftDirection, Sign, Size,
    },
    lexer::{LabelDirective, LexedLine, LexedOperand, LexedRegisterType, LexedSize, ParsedLine},
    utils::parse_char_or_num, math::sign_extend_to_long,
};
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
#[wasm_bindgen]
pub struct Compiler {
    labels: HashMap<String, Label>,
    line_addresses: Vec<usize>,
    instructions: Vec<InstructionLine>,
    start_address: usize,
    final_instrucion_address: usize,
}
#[derive(Clone, Serialize)]
pub struct InstructionLine {
    pub instruction: Instruction,
    pub address: usize,
    pub parsed_line: ParsedLine,
}
pub enum CompilationError {
    Raw(String),
    InvalidTrap(String),
    InvalidAddressingMode(String),
    ParseError(String),
}
impl CompilationError {
    pub fn get_message(&self) -> String {
        match self {
            CompilationError::Raw(message)
            | CompilationError::InvalidTrap(message)
            | CompilationError::InvalidAddressingMode(message)
            | CompilationError::ParseError(message) => message.clone(),
        }
    }
}

pub type CompilationResult<T> = Result<T, CompilationError>;
impl fmt::Debug for InstructionLine {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("InstructionLine")
            .field("instruction", &self.instruction)
            .field("address", &self.address)
            //.field("parsed_line", &self.parsed_line)
            .finish()
    }
}
impl Compiler {
    pub fn new(lines: &[ParsedLine]) -> Result<Compiler, String> {
        let (labels, line_addresses) = parse_labels_and_addresses(lines)?;
        let mut pre_interpreter = Compiler {
            labels,
            line_addresses,
            instructions: Vec::new(),
            start_address: 0,
            final_instrucion_address: 0,
        };
        pre_interpreter.load(lines)?;
        Ok(pre_interpreter)
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

    pub fn get_labels_map(&self) -> &HashMap<String, Label> {
        &self.labels
    }
    fn load(&mut self, lines: &[ParsedLine]) -> Result<(), String> {
        self.parse_instruction_lines(lines)?;
        let start = self.labels.get("start");
        self.start_address = match start {
            Some(label) => label.address,
            None => match self.instructions.first() {
                Some(instruction) => instruction.address,
                None => 0,
            },
        };
        self.final_instrucion_address = match self.instructions.last() {
            Some(instruction) => instruction.address,
            None => 0,
        };
        Ok(())
    }

    fn parse_instruction_lines(&mut self, lines: &[ParsedLine]) -> Result<(), String> {
        for (i, line) in lines.iter().enumerate() {
            match &line.parsed {
                LexedLine::Instruction {
                    name,
                    operands,
                    size,
                } => {
                    let parsed_operands = operands
                        .iter()
                        .map(|x| self.parse_operand(x, line))
                        .collect::<CompilationResult<Vec<Operand>>>();
                    match parsed_operands {
                        Ok(ops) => {
                            let instruction = self.parse_instruction(name, ops, size);
                            match instruction {
                                Ok(ins) => {
                                    let instuction_line = InstructionLine {
                                        instruction: ins,
                                        address: self.line_addresses[i],
                                        parsed_line: line.clone(),
                                    };
                                    self.instructions.push(instuction_line);
                                }
                                Err(e) => {
                                    return Err(format!(
                                        "{}; at line {}",
                                        e.get_message(),
                                        line.line_index
                                    )
                                    .to_string())
                                }
                            }
                        }
                        Err(e) => {
                            return Err(format!("{}; at line {}", e.get_message(), line.line_index)
                                .to_string())
                        }
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }
    fn parse_instruction(
        &self,
        name: &String,
        mut operands: Vec<Operand>,
        size: &LexedSize,
    ) -> CompilationResult<Instruction> {
        //TODO add better error logging
        if operands.len() == 2 {
            let (op1, op2) = (operands.remove(0), operands.remove(0));
            let parsed = match name.as_str() {
                "move" => Instruction::MOVE(op1, op2, self.get_size(size, Size::Word)?),
                "add" => Instruction::ADD(op1, op2, self.get_size(size, Size::Word)?),
                "sub" => Instruction::SUB(op1, op2, self.get_size(size, Size::Word)?),
                "adda" => Instruction::ADDA(
                    op1,
                    self.extract_register(op2)?,
                    self.get_size(size, Size::Word)?,
                ),
                "suba" => Instruction::SUBA(
                    op1,
                    self.extract_register(op2)?,
                    self.get_size(size, Size::Word)?,
                ),
                "divs" => Instruction::DIVx(op1, self.extract_register(op2)?, Sign::Signed),
                "divu" => Instruction::DIVx(op1, self.extract_register(op2)?, Sign::Unsigned),
                "muls" => Instruction::MULx(op1, self.extract_register(op2)?, Sign::Signed),
                "mulu" => Instruction::MULx(op1, self.extract_register(op2)?, Sign::Unsigned),
                "exg" => Instruction::EXG(self.extract_register(op1)?, self.extract_register(op2)?),
                "cmp" => Instruction::CMP(op1, op2, self.get_size(size, Size::Word)?),
                "or" => Instruction::OR(op1, op2, self.get_size(size, Size::Word)?),
                "and" => Instruction::AND(op1, op2, self.get_size(size, Size::Word)?),
                "eor" => Instruction::EOR(op1, op2, self.get_size(size, Size::Word)?),
                "lsl" => Instruction::LSd(
                    op1,
                    op2,
                    ShiftDirection::Left,
                    self.get_size(size, Size::Word)?,
                ),
                "lsr" => Instruction::LSd(
                    op1,
                    op2,
                    ShiftDirection::Right,
                    self.get_size(size, Size::Word)?,
                ),
                "asl" => Instruction::LSd(
                    op1,
                    op2,
                    ShiftDirection::Left,
                    self.get_size(size, Size::Word)?,
                ),
                "asr" => Instruction::LSd(
                    op1,
                    op2,
                    ShiftDirection::Right,
                    self.get_size(size, Size::Word)?,
                ),
                "rol" => Instruction::ROd(
                    op1,
                    op2,
                    ShiftDirection::Left,
                    self.get_size(size, Size::Word)?,
                ),
                "ror" => Instruction::ROd(
                    op1,
                    op2,
                    ShiftDirection::Right,
                    self.get_size(size, Size::Word)?,
                ),
                "btst" => Instruction::BTST(op1, op2),
                "bset" => Instruction::BSET(op1, op2),
                "bclr" => Instruction::BCLR(op1, op2),
                "bchg" => Instruction::BCHG(op1, op2),
                _ => {
                    return Err(CompilationError::Raw(format!(
                        "Unknown instruction {}",
                        name
                    )))
                }
            };
            Ok(parsed)
        } else if operands.len() == 1 {
            let op = operands[0].clone();
            let result = match name.as_str() {
                "clr" => Instruction::CLR(op, self.get_size(size, Size::Word)?),
                "neg" => Instruction::NEG(op, self.get_size(size, Size::Word)?),
                "ext" => Instruction::EXT(
                    self.extract_register(op)?,
                    //from
                    match self.get_size(size, Size::Word)? {
                        Size::Word => Size::Byte,
                        Size::Long => Size::Word,
                        s => {
                            return Err(CompilationError::Raw(
                                format!("Invalid size {:?}", s).to_string(),
                            ))
                        }
                    },
                    //to
                    match self.get_size(size, Size::Word)? {
                        Size::Word => Size::Word,
                        Size::Long => Size::Long,
                        s => {
                            return Err(CompilationError::Raw(
                                format!("Invalid size {:?}", s).to_string(),
                            ))
                        }
                    },
                ),
                "extb" => Instruction::EXT(self.extract_register(op)?, Size::Byte, Size::Long),
                "tst" => Instruction::TST(op, self.get_size(size, Size::Word)?),
                "beq" | "bne" | "blt" | "ble" | "bgt" | "bge" | "blo" | "bls" | "bhi" | "bhs" => {
                    let address = self.extract_address(&op)?;
                    match name[1..].parse() {
                        Ok(condition) => Instruction::Bcc(address, condition),
                        Err(_) => {
                            return Err(CompilationError::ParseError(format!(
                                "Invalid condition code: {}",
                                name
                            )))
                        }
                    }
                }
                "bra" => {
                    let address = self.extract_address(&op)?;
                    Instruction::BRA(address)
                }
                "bsr" => {
                    let address = self.extract_address(&op)?;
                    Instruction::BSR(address)
                }
                "jmp" => Instruction::JMP(op),
                //scc
                "scc" | "scs" | "seq" | "sne" | "sge" | "sgt" | "sle" | "sls" | "slt" | "shi"
                | "smi" | "spl" | "svc" | "svs" | "sf" | "st" => match name[1..].parse() {
                    Ok(condition) => Instruction::Scc(op, condition),
                    Err(_) => {
                        return Err(CompilationError::ParseError(format!(
                            "Invalid condition code: {}",
                            name
                        )))
                    }
                },
                "swap" => Instruction::SWAP(self.extract_register(op)?),
                //not sure if the default is word
                "not" => Instruction::NOT(op, self.get_size(size, Size::Word)?),
                "jsr" => Instruction::JSR(op),

                "trap" => {
                    let value = self.extract_immediate(&op)? as i32;
                    if value > 15 || value < 0 {
                        return Err(CompilationError::InvalidTrap(format!(
                            "Invalid trap value: {}, must be between 0 and 15",
                            value
                        )));
                    }
                    Instruction::TRAP(value as u8)
                }
                _ => {
                    return Err(CompilationError::Raw(format!(
                        "Unknown instruction {}",
                        name
                    )))
                }
            };
            Ok(result)
        } else if operands.len() == 0 {
            let result = match name.as_str() {
                "rts" => Instruction::RTS,
                _ => {
                    return Err(CompilationError::Raw(format!(
                        "Unknown instruction {}",
                        name
                    )))
                }
            };
            Ok(result)
        } else {
            Err(CompilationError::ParseError(format!(
                "Invalid instruction: {}",
                name
            )))
        }
    }

    fn get_size(&self, size: &LexedSize, default: Size) -> CompilationResult<Size> {
        match size {
            LexedSize::Byte => Ok(Size::Byte),
            LexedSize::Word => Ok(Size::Word),
            LexedSize::Long => Ok(Size::Long),
            LexedSize::Unspecified => Ok(default),
            _ => Err(CompilationError::ParseError(format!(
                "Invalid size: {:?}",
                size
            ))),
        }
    }
    fn extract_immediate(&self, operand: &Operand) -> CompilationResult<u32> {
        match operand {
            Operand::Immediate(imm) => Ok(*imm),
            _ => Err(CompilationError::InvalidAddressingMode(
                "Expected Immediate".to_string(),
            )),
        }
    }
    pub fn extract_register(&self, operand: Operand) -> CompilationResult<RegisterOperand> {
        match operand {
            Operand::Register(reg) => Ok(reg),
            _ => Err(CompilationError::InvalidAddressingMode(
                "Operand is not a register".to_string(),
            )),
        }
    }
    fn extract_address(&self, operand: &Operand) -> CompilationResult<u32> {
        match operand {
            Operand::Address(addr) => Ok(*addr as u32),
            _ => Err(CompilationError::InvalidAddressingMode(
                "Operand is not an address".to_string(),
            )),
        }
    }
    fn parse_operand(
        &mut self,
        operand: &LexedOperand,
        line: &ParsedLine,
    ) -> CompilationResult<Operand> {
        match operand {
            LexedOperand::Register(register_type, register_name) => match register_type {
                LexedRegisterType::Address => match register_name[1..].parse() {
                    Ok(reg) => Ok(Operand::Register(RegisterOperand::Address(reg))),
                    Err(_) => Err(CompilationError::ParseError(format!(
                        "Invalid a register name: {}",
                        register_name
                    ))),
                },
                LexedRegisterType::Data => match register_name[1..].parse() {
                    Ok(reg) => Ok(Operand::Register(RegisterOperand::Data(reg))),
                    Err(_) => Err(CompilationError::ParseError(format!(
                        "Invalid d register name: {}",
                        register_name
                    ))),
                },
                LexedRegisterType::SP => Ok(Operand::Register(RegisterOperand::Address(7))),
            },
            LexedOperand::Address(address) => match usize::from_str_radix(&address[1..], 16) {
                Ok(address) => Ok(Operand::Address(address)),
                Err(_) => Err(CompilationError::ParseError(format!(
                    "Invalid address: {}",
                    address
                ))),
            },
            LexedOperand::Label(label) => match self.labels.get(label) {
                Some(label) => Ok(Operand::Address(label.address)),
                None => Err(CompilationError::ParseError(format!(
                    "Label \"{}\" not found",
                    label
                ))),
            },
            LexedOperand::IndirectOrDisplacement { offset, operand } => {
                let parsed_operand = self.parse_operand(operand, line)?;
                let parsed_operand = self.extract_register(parsed_operand)?;
                let offset = if offset.trim() == "" {
                    0
                } else {
                    match offset.parse() {
                        Ok(offset) => sign_extend_to_long(offset, &Size::Word),
                        Err(_) => {
                            return Err(CompilationError::ParseError(format!(
                                "Invalid offset: {}",
                                offset
                            )))
                        }
                    }
                };
                Ok(Operand::IndirectOrDisplacement {
                    offset,
                    operand: parsed_operand,
                })
            }
            LexedOperand::IndirectBaseDisplacement { offset, operands } => {
                let parsed_operands = operands
                    .iter()
                    .map(|x| {
                        let parsed = self.parse_operand(x, line)?;
                        self.extract_register(parsed)
                    })
                    .collect::<CompilationResult<Vec<RegisterOperand>>>();
                let offset = if offset == "" {
                    0
                } else {
                    match offset.parse() {
                        Ok(offset) => sign_extend_to_long(offset, &Size::Byte),
                        Err(_) => {
                            return Err(CompilationError::ParseError(format!(
                                "Invalid offset: {}",
                                offset
                            )))
                        }
                    }
                };
                match parsed_operands {
                    Ok(registers_operands) => match &registers_operands[..] {
                        [RegisterOperand::Address(_), RegisterOperand::Address(_) | RegisterOperand::Data(_)] => {
                            Ok(Operand::IndirectBaseDisplacement {
                                offset,
                                operands: DisplacementOperands {
                                    base: registers_operands[0].clone(),
                                    index: registers_operands[1].clone(),
                                },
                            })
                        }
                        _ => Err(CompilationError::InvalidAddressingMode(
                            "Invalid displacement addressing mode".to_string(),
                        )),
                    },
                    Err(e) => Err(e),
                }
            }
            LexedOperand::PostIndirect(operand) => {
                let parsed_operand = self.parse_operand(operand, line)?;
                let parsed_operand = self.extract_register(parsed_operand)?;
                Ok(Operand::PostIndirect(parsed_operand))
            }
            LexedOperand::PreIndirect(operand) => {
                let parsed_operand = self.parse_operand(operand, line)?;
                let parsed_operand = self.extract_register(parsed_operand)?;
                Ok(Operand::PreIndirect(parsed_operand))
            }
            LexedOperand::Immediate(num) => match num.chars().collect::<Vec<char>>()[..] {
                ['#', '%'] => match i32::from_str_radix(&num[2..], 2) {
                    Ok(value) => Ok(Operand::Immediate(value as u32)),
                    Err(_) => Err(CompilationError::ParseError(format!(
                        "Invalid binary number: {}",
                        &num
                    ))),
                },
                ['#', '@'] => match i32::from_str_radix(&num[2..], 8) {
                    Ok(value) => Ok(Operand::Immediate(value as u32)),
                    Err(_) => Err(CompilationError::ParseError(format!(
                        "Invalid octal number: {}",
                        &num
                    ))),
                },
                ['#', '$', ..] => match i32::from_str_radix(&num[2..], 16) {
                    Ok(value) => Ok(Operand::Immediate(value as u32)),
                    Err(_) => Err(CompilationError::ParseError(format!(
                        "Invalid hex number: {}",
                        &num
                    ))),
                },
                ['#', '\'', c, '\''] => {
                    let value = c as u32;
                    Ok(Operand::Immediate(value))
                }
                ['#', ..] => match self.labels.get(&num[1..]) {
                    Some(label) => Ok(Operand::Immediate((label.address as i32) as u32)),
                    None => match i32::from_str_radix(&num[1..], 10) {
                        Ok(value) => Ok(Operand::Immediate(value as u32)),
                        Err(_) => Err(CompilationError::ParseError(format!(
                            "Invalid immediate number: {}",
                            &num
                        ))),
                    },
                },
                _ => Err(CompilationError::ParseError(format!(
                    "Invalid immediate number: {}",
                    &num
                ))),
            },
            _ => Err(CompilationError::ParseError(format!(
                "Invalid operand: {:?}",
                operand
            ))),
        }
    }
}

fn parse_label_directive(directive: &LabelDirective) -> CompilationResult<Directive> {
    let parsed_args = directive
        .args
        .iter()
        .map(|x| match parse_char_or_num(&x.value) {
            Ok(value) => Ok(value as u32),
            Err(_) => Err(CompilationError::ParseError(format!(
                "Invalid numerical directive argument"
            ))),
        })
        .collect::<CompilationResult<Vec<u32>>>()?;
    match directive.name.as_str() {
        "dc" => Ok(Directive::DC {
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
                _ => {
                    return Err(CompilationError::Raw(
                        "Invalid or missing size for DC directive".to_string(),
                    ))
                }
            },
        }),
        "ds" => {
            if directive.size == LexedSize::Unknown || directive.size == LexedSize::Unspecified {
                return Err(CompilationError::Raw(
                    "Invalid or missing size for DS directive".to_string(),
                ));
            }
            match parsed_args[..] {
                [amount] => {
                    let data = vec![0; amount as usize * directive.size.clone() as usize / 8];
                    Ok(Directive::DS { data })
                }
                _ => Err(CompilationError::Raw(
                    "Invalid number of arguments for DS directive".to_string(),
                )),
            }
        }
        "dcb" => {
            let data = match directive.size {
                LexedSize::Long => match parsed_args[..] {
                    [size, default] => vec![default as u32; size as usize]
                        .iter()
                        .flat_map(|x| (*x as u32).to_be_bytes())
                        .collect(),
                    _ => {
                        return Err(CompilationError::Raw(
                            "Invalid number of arguments for DCB directive".to_string(),
                        ))
                    }
                },
                LexedSize::Word => match parsed_args[..] {
                    [size, default] => vec![default as u16; size as usize]
                        .iter()
                        .flat_map(|x| (*x as u16).to_be_bytes())
                        .collect(),
                    _ => {
                        return Err(CompilationError::Raw(
                            "Invalid number of arguments for DCB directive".to_string(),
                        ))
                    }
                },
                LexedSize::Byte => match parsed_args[..] {
                    [size, default] => vec![default as u8; size as usize],
                    _ => {
                        return Err(CompilationError::Raw(
                            "Invalid number of arguments for DCB directive".to_string(),
                        ))
                    }
                },
                _ => {
                    return Err(CompilationError::Raw(
                        "Invalid or missing size for DCB directive".to_string(),
                    ))
                }
            };
            Ok(Directive::DCB { data })
        }
        _ => Err(CompilationError::Raw(format!(
            "Invalid directive: {}",
            directive.name
        ))),
    }
}
fn parse_labels_and_addresses(
    lines: &[ParsedLine],
) -> Result<(HashMap<String, Label>, Vec<usize>), String> {
    let mut last_address = 4096; //same as ORG $1000
    let mut labels = HashMap::new();
    let mut line_addresses: Vec<usize> = Vec::new();
    for line in lines.iter() {
        line_addresses.push(last_address);
        match &line.parsed {
            LexedLine::Directive { args, .. } => {
                if args[0] == "org" {
                    let hex = args[1].trim_start_matches("$");
                    let parsed = match usize::from_str_radix(hex, 16) {
                        Ok(value) => value,
                        Err(_) => {
                            return Err(format!(
                                "Invalid hex ORG address: {}; at line {}",
                                args[1], line.line_index
                            ))
                        }
                    };
                    if parsed < last_address {
                        return Err(format!("The address of the ORG directive ({}) must be greater than the previous address ({}); at line {}",parsed, last_address, line.line_index));
                    }
                    last_address = match usize::from_str_radix(hex, 16) {
                        Ok(value) => value,
                        Err(_) => {
                            return Err(format!(
                                "Invalid hex value: {}; at line {}",
                                hex, line.line_index
                            ))
                        }
                    };
                    //align at 2 bytes intervals
                    //TODO should i align here?
                    if last_address % 2 != 0 {
                        last_address += 1;
                    }
                }
            }
            LexedLine::Label { name, .. } => {
                let name = name.to_string();
                labels.insert(
                    name.clone(),
                    Label {
                        name,
                        directive: None,
                        address: last_address,
                    },
                );
            }
            LexedLine::LabelDirective { name, directive } => {
                match parse_label_directive(directive) {
                    Ok(parsed_directive) => {
                        labels.insert(
                            name.clone(),
                            Label {
                                name: name.clone(),
                                directive: Some(parsed_directive),
                                address: last_address,
                            },
                        );
                    }
                    Err(e) => {
                        return Err(format!(
                            "Error parsing directive: \"{}\"; at line:{}",
                            e.get_message(),
                            line.line_index
                        ))
                    }
                };

                match directive.name.as_str() {
                    "dcb" | "ds" => {
                        let bytes = directive.args[0].value.parse::<usize>();
                        match bytes {
                            Ok(bytes) => {
                                last_address += bytes * directive.size.to_bytes() as usize;
                            }
                            Err(_) => {
                                return Err(format!(
                                    "Invalid number of bytes for DS directive at line {}",
                                    line.line_index
                                ));
                            }
                        }
                    }
                    "dc" => {
                        let args = directive.args.len();
                        last_address += args * directive.size.to_bytes() as usize;
                    }
                    _ => {}
                }
            }
            LexedLine::Instruction { .. } => {
                //align at 2 bytes intervals
                if last_address % 2 != 0 {
                    last_address += 1;
                }
                last_address += 4;
            }

            _ => {}
        }
    }
    Ok((labels, line_addresses))
}
