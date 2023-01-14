use serde::Serialize;
use wasm_bindgen::prelude::wasm_bindgen;

use crate::{
    instructions::{
        Condition, DisplacementOperands, Instruction, Operand, RegisterOperand, ShiftDirection,
        Sign, Size, Label,
    },
    lexer::{LexedLine, LexedOperand, LexedRegisterType, LexedSize, ParsedLine},
    math::sign_extend_to_long,
    utils::{parse_absolute_expression, parse_string_into_padded_bytes},
};
use std::fmt;
use std::{collections::HashMap, vec};
#[derive(Debug)]
pub enum Directive {
    DC { data: Vec<u8>, address: usize },
    DS { data: Vec<u8>, address: usize },
    DCB { data: Vec<u8>, address: usize },
    Other,
}
#[wasm_bindgen]
pub struct Compiler {
    labels: HashMap<String, Label>,
    line_addresses: Vec<usize>,
    directives: Vec<Directive>,
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
        let mut pre_interpreter = Compiler {
            labels: HashMap::new(),
            line_addresses: Vec::new(),
            directives: Vec::new(),
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
    pub fn get_directives(&self) -> &Vec<Directive> {
        &self.directives
    }
    fn load(&mut self, lines: &[ParsedLine]) -> Result<(), String> {
        self.parse_labels_and_addresses(lines)?; //has side effect, place before the parsing
        self.parse_instruction_lines(lines)?;
        self.start_address = match self.labels.get("start") {
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
                "subq" => Instruction::SUBQ(
                    self.extract_immediate(&op1)? as u8,
                    op2,
                    self.get_size(size, Size::Word)?,
                ),
                "addq" => Instruction::ADDQ(
                    self.extract_immediate(&op1)? as u8,
                    op2,
                    self.get_size(size, Size::Word)?,
                ),
                "moveq" => Instruction::MOVEQ(
                    self.extract_immediate(&op1)? as u8,
                    self.extract_register(op2)?,
                ),
                "divs" => Instruction::DIVx(op1, self.extract_register(op2)?, Sign::Signed),
                "divu" => Instruction::DIVx(op1, self.extract_register(op2)?, Sign::Unsigned),
                "muls" => Instruction::MULx(op1, self.extract_register(op2)?, Sign::Signed),
                "mulu" => Instruction::MULx(op1, self.extract_register(op2)?, Sign::Unsigned),
                "exg" => Instruction::EXG(self.extract_register(op1)?, self.extract_register(op2)?),
                "cmp" => Instruction::CMP(
                    op1,
                    self.extract_register(op2)?,
                    self.get_size(size, Size::Word)?,
                ),
                "or" => Instruction::OR(op1, op2, self.get_size(size, Size::Word)?),
                "and" => Instruction::AND(op1, op2, self.get_size(size, Size::Word)?),
                "eor" => Instruction::EOR(op1, op2, self.get_size(size, Size::Word)?),
                "addi" => Instruction::ADDI(
                    self.extract_immediate(&op1)?,
                    op2,
                    self.get_size(size, Size::Word)?,
                ),
                "subi" => Instruction::SUBI(
                    self.extract_immediate(&op1)?,
                    op2,
                    self.get_size(size, Size::Word)?,
                ),
                "andi" => Instruction::ANDI(
                    self.extract_immediate(&op1)?,
                    op2,
                    self.get_size(size, Size::Word)?,
                ),
                "cmpi" => Instruction::CMPI(
                    self.extract_immediate(&op1)?,
                    op2,
                    self.get_size(size, Size::Word)?,
                ),
                "ori" => Instruction::ORI(
                    self.extract_immediate(&op1)?,
                    op2,
                    self.get_size(size, Size::Word)?,
                ),
                "eori" => Instruction::EORI(
                    self.extract_immediate(&op1)?,
                    op2,
                    self.get_size(size, Size::Word)?,
                ),
                "cmpa" => Instruction::CMPA(
                    op1,
                    self.extract_register(op2)?,
                    self.get_size(size, Size::Word)?,
                ),
                "cmpm" => Instruction::CMPM(op1, op2, self.get_size(size, Size::Word)?),
                "movea" => Instruction::MOVEA(
                    op1,
                    self.extract_register(op2)?,
                    self.get_size(size, Size::Word)?,
                ),
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
                "lea" => Instruction::LEA(op1, self.extract_register(op2)?),
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
                "dbcc" | "dbcs" | "dbeq" | "dbne" | "dbge" | "dbgt" | "dble" | "dbls" | "dblt"
                | "dbhi" | "dbmi" | "dbpl" | "dbvc" | "dbvs" | "dbf" | "dbt" | "dbhs" | "dblo" => {
                    match name[2..].parse() {
                        Ok(condition) => Instruction::DBcc(
                            self.extract_register(op1)?,
                            self.extract_address(&op2)?,
                            condition,
                        ),
                        Err(_) => {
                            return Err(CompilationError::ParseError(format!(
                                "Invalid condition code: {}",
                                name
                            )))
                        }
                    }
                }
                "dbra" => Instruction::DBcc(
                    self.extract_register(op1)?,
                    self.extract_address(&op2)?,
                    Condition::False,
                ),
                "link" => {
                    Instruction::LINK(self.extract_register(op1)?, self.extract_immediate(&op2)?)
                }
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
                "pea" => Instruction::PEA(op),
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
                "unlk" => Instruction::UNLK(self.extract_register(op)?),
                "extb" => Instruction::EXT(self.extract_register(op)?, Size::Byte, Size::Long),
                "tst" => Instruction::TST(op, self.get_size(size, Size::Word)?),
                "bcc" | "bcs" | "beq" | "bne" | "blt" | "ble" | "bgt" | "bge" | "blo" | "bls" | "bhi" | "bhs"
                | "bpl" | "bmi" | "bvc" | "bvs" => {
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
                | "smi" | "spl" | "svc" | "svs" | "sf" | "st" | "shs" | "slo" => match name[1..].parse() {
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
            Operand::Absolute(addr) => Ok(*addr as u32),
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
            LexedOperand::Absolute(value) => match self.parse_absolute(value) {
                Ok(absolute) => Ok(Operand::Absolute(absolute as usize)),
                Err(_) => Err(CompilationError::ParseError(format!(
                    "Invalid absolute: {}",
                    value
                ))),
            },
            LexedOperand::Label(label) => match self.labels.get(label) {
                Some(label) => Ok(Operand::Absolute(label.address)),
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
                    match parse_absolute_expression(offset, &self.labels) {
                        Ok(offset) => sign_extend_to_long(offset as u32, &Size::Word),
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
            LexedOperand::Immediate(num) => match self.parse_immediate(num) {
                Ok(absolute) => Ok(Operand::Immediate(absolute)),
                Err(e) => Err(CompilationError::ParseError(format!(
                    "Invalid immediate: {}",
                    e.get_message()
                ))),
            },
            _ => Err(CompilationError::ParseError(format!(
                "Invalid operand: {:?}",
                operand
            ))),
        }
    }

    fn parse_immediate(&self, num: &str) -> CompilationResult<u32> {
        self.parse_absolute(&num[1..])
    }

    fn parse_absolute(&self, num: &str) -> CompilationResult<u32> {
        match parse_absolute_expression(num, &self.labels) {
            Ok(absolute) => Ok(absolute as u32),
            Err(e) => Err(CompilationError::ParseError(e)),
        }
    }
    fn parse_absolutes(&self, nums: &[String]) -> CompilationResult<Vec<u32>> {
        nums.iter()
            .map(|x| self.parse_absolute(x))
            .collect::<CompilationResult<Vec<u32>>>()
    }
    fn parse_directive(
        &self,
        name: &String,
        size: &LexedSize,
        args: &Vec<String>,
        address: usize,
    ) -> CompilationResult<Directive> {
        match name.as_str() {
            "dc" => {
                let mut data: Vec<u8> = vec![];

                for arg in args[1..].iter() {
                    match arg {
                        _ if arg.starts_with('\'') && arg.ends_with('\'') => {
                            let string_bytes = parse_string_into_padded_bytes(
                                &arg[1..arg.len() - 1],
                                size.to_bytes_word_default() as usize,
                            );
                            data.extend_from_slice(&string_bytes);
                        }
                        _ => {
                            let num = self.parse_absolute(arg)?;
                            match size {
                                LexedSize::Byte => data.push(num as u8),
                                //TODO is word default?
                                LexedSize::Word | LexedSize::Unspecified => {
                                    data.extend_from_slice(&(num as u16).to_be_bytes())
                                }
                                LexedSize::Long => {
                                    data.extend_from_slice(&(num as u32).to_be_bytes())
                                }
                                _ => {
                                    return Err(CompilationError::Raw(
                                        "Invalid size for DC directive".to_string(),
                                    ))
                                }
                            };
                        }
                    }
                }
                Ok(Directive::DC { data, address })
            }
            "ds" => {
                if *size == LexedSize::Unknown {
                    return Err(CompilationError::Raw(
                        "Invalid size for DS directive".to_string(),
                    ));
                }
                if args.len() != 2 {
                    return Err(CompilationError::Raw(
                        "Invalid number of arguments for DS directive".to_string(),
                    ));
                }
                let amount = self.parse_absolute(&args[1])?;
                let data = vec![0; amount as usize * size.to_bytes_word_default() as usize / 8];
                Ok(Directive::DS { data, address })
            }
            "dcb" => {
                let parsed_args = self.parse_absolutes(&args[1..])?;
                let data = match size {
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
                    //TODO is word default?
                    LexedSize::Word | LexedSize::Unspecified => match parsed_args[..] {
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
                            "Invalid size for DCB directive".to_string(),
                        ))
                    }
                };
                Ok(Directive::DCB { data, address })
            }
            _ => Ok(Directive::Other),
        }
    }
    fn get_next_address(&self, line: &ParsedLine, last_address: usize) -> Result<usize, String> {
        let mut next_address = last_address;
        match &line.parsed {
            LexedLine::Directive { args, name, size } => {
                match name.as_str() {
                    "org" => {
                        let parsed = match self.parse_absolute(&args[1]) {
                            Ok(value) => value as usize,
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
                        next_address = parsed;
                        //align at 2 bytes intervals
                        //TODO should i align here?
                        if next_address % 2 != 0 {
                            next_address = next_address + 1;
                        }
                    }
                    "ds" => match self.parse_absolute(&args[1]) {
                        Ok(bytes) => {
                            next_address = last_address
                                + (bytes * size.to_bytes_word_default() as u32) as usize;
                        }
                        Err(_) => {
                            return Err(format!(
                                "Invalid number of bytes for DS directive at line {}",
                                line.line_index
                            ));
                        }
                    },
                    "dcb" => match self.parse_absolute(&args[1]) {
                        Ok(bytes) => {
                            next_address = last_address
                                + (bytes * size.to_bytes_word_default() as u32) as usize;
                        }
                        Err(_) => {
                            return Err(format!(
                                "Invalid number of bytes for dcb directive at line {}",
                                line.line_index
                            ));
                        }
                    },

                    "dc" => {
                        next_address = last_address;
                        for arg in args[1..].iter() {
                            match arg {
                                _ if arg.starts_with('\'') && arg.ends_with('\'') => {
                                    next_address += parse_string_into_padded_bytes(
                                        &arg[1..arg.len() - 1],
                                        size.to_bytes_word_default() as usize,
                                    )
                                    .len();
                                }
                                _ => {
                                    next_address =
                                        next_address + size.to_bytes_word_default() as usize;
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
            LexedLine::Instruction { .. } => {
                //align at 2 bytes intervals
                if next_address % 2 != 0 {
                    next_address += 1;
                }
                next_address += 4;
            }

            _ => {}
        }
        Ok(next_address)
    }
    fn parse_labels_and_addresses(&mut self, lines: &[ParsedLine]) -> Result<(), String> {
        let mut last_address = 4096; //same as ORG $1000
        let mut labels: HashMap<String, Label> = HashMap::new();
        let mut directives: Vec<Directive> = Vec::new();
        let mut line_addresses: Vec<usize> = Vec::new();
        for line in lines.iter() {
            line_addresses.push(last_address);
            match &line.parsed {
                LexedLine::Label { name } => {
                    if labels.contains_key(name) {
                        return Err(format!(
                            "Label {} already defined at line {}",
                            name, line.line_index
                        ));
                    }
                    labels.insert(
                        name.clone(),
                        Label {
                            address: last_address,
                            name: name.clone(),
                            line: line.line_index
                        },
                    );
                }
                _ => {}
            }
            match self.get_next_address(line, last_address) {
                Ok(address) => {
                    last_address = address;
                }
                Err(e) => {
                    return Err(e);
                }
            }
        }
        self.labels = labels;
        self.line_addresses = line_addresses;
        //TODO i could merge this inthe previous loop but it would now allow for labels to be defined after the directive
        for (i, line) in lines.iter().enumerate() {
            match &line.parsed {
                LexedLine::Directive { name, size, args } => {
                    match self.parse_directive(name, size, args, self.line_addresses[i]) {
                        Ok(directive) => {
                            directives.push(directive);
                        }
                        Err(e) => {
                            return Err(format!(
                                "Error parsing directive at line {}: {}",
                                line.line_index,
                                e.get_message()
                            ));
                        }
                    }
                }
                _ => {}
            }
        }
        self.directives = directives;
        Ok(())
    }
}
