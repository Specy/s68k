use std::{collections::HashMap, vec};
use std::fmt;

use serde::Serialize;
use wasm_bindgen::prelude::wasm_bindgen;

use crate::{
    instructions::{
        Condition, Instruction, Label, Operand, RegisterOperand,
        ShiftDirection, Sign, Size,
    },
    lexer::{LexedLine, LexedOperand, LexedRegisterType, LexedSize, ParsedLine},
    math::sign_extend_to_long,
    utils::{parse_absolute_expression, parse_string_into_padded_bytes},
};
use crate::instructions::{IndexRegister, TargetDirection};

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

#[derive(Debug)]
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

    pub fn get_instructions(&self) -> &Vec<InstructionLine> {
        &self.instructions
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
        self.start_address = match self.labels.get("START") {
            Some(label) => {
                //find the closest instruction after the label
                self.instructions
                    .iter()
                    .map(|x| x.address)
                    .filter(|x| *x >= label.address)
                    .min()
                    .unwrap_or(0)
            }
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
                                        .to_string());
                                }
                            }
                        }
                        Err(e) => {
                            return Err(format!("{}; at line {}", e.get_message(), line.line_index)
                                .to_string());
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
                "move" => match op2 {
                    Operand::Register(RegisterOperand::Address(a)) => Instruction::MOVEA(
                        op1,
                        RegisterOperand::Address(a),
                        self.get_size(size, Size::Word)?,
                    ),
                    _ => Instruction::MOVE(op1, op2, self.get_size(size, Size::Word)?),
                },
                "movem" => {
                    let (register_mask, target, direction) = match (op1, op2) {
                        (Operand::Immediate(mask), op2) => (mask as u16, op2, TargetDirection::ToMemory),
                        (op1, Operand::Immediate(mask)) => (mask as u16, op1, TargetDirection::FromMemory),
                        _ => {
                            return Err(CompilationError::InvalidAddressingMode(
                                "Invalid operands for MOVEM".to_string(),
                            ));
                        }
                    };
                    Instruction::MOVEM {
                        registers_mask: register_mask,
                        target: target,
                        direction,
                        size: self.get_size(size, Size::Word)?,
                    }
                }
                "add" => match (op1, op2) {
                    (Operand::Immediate(num), _) => {
                        Instruction::ADDI(
                            num,
                            op2,
                            self.get_size(size, Size::Word)?,
                        )
                    }
                    (_, Operand::Register(RegisterOperand::Address(a))) => Instruction::ADDA(
                        op1,
                        RegisterOperand::Address(a),
                        self.get_size(size, Size::Word)?,
                    ),
                    _ => Instruction::ADD(op1, op2, self.get_size(size, Size::Word)?),
                },
                "sub" => match (op1, op2) {
                    (Operand::Immediate(num), _) => {
                        Instruction::SUBI(
                            num,
                            op2,
                            self.get_size(size, Size::Word)?,
                        )
                    }
                    (_, Operand::Register(RegisterOperand::Address(a))) => Instruction::SUBA(
                        op1,
                        RegisterOperand::Address(a),
                        self.get_size(size, Size::Word)?,
                    ),
                    _ => Instruction::SUB(op1, op2, self.get_size(size, Size::Word)?),
                },
                "cmp" => match (op1, op2) {
                    (_, Operand::Register(RegisterOperand::Address(a))) => Instruction::CMPA(
                        op1,
                        RegisterOperand::Address(a),
                        self.get_size(size, Size::Word)?,
                    ),
                    (Operand::Immediate(num), op2) => {
                        Instruction::CMPI(num, op2, self.get_size(size, Size::Word)?)
                    }
                    (Operand::PostIndirect(_), Operand::PostIndirect(_)) => Instruction::CMPM(
                        op1,
                        op2,
                        self.get_size(size, Size::Word)?,
                    ),
                    _ => Instruction::CMP(
                        op1,
                        self.extract_register(op2)?,
                        self.get_size(size, Size::Word)?,
                    ),
                },
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
                "asl" => Instruction::ASd(
                    op1,
                    op2,
                    ShiftDirection::Left,
                    self.get_size(size, Size::Word)?,
                ),
                "asr" => Instruction::ASd(
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
                            )));
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
                    )));
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
                            ));
                        }
                    },
                    //to
                    match self.get_size(size, Size::Word)? {
                        Size::Word => Size::Word,
                        Size::Long => Size::Long,
                        s => {
                            return Err(CompilationError::Raw(
                                format!("Invalid size {:?}", s).to_string(),
                            ));
                        }
                    },
                ),
                "unlk" => Instruction::UNLK(self.extract_register(op)?),
                "extb" => Instruction::EXT(self.extract_register(op)?, Size::Byte, Size::Long),
                "tst" => Instruction::TST(op, self.get_size(size, Size::Word)?),
                "bcc" | "bcs" | "beq" | "bne" | "blt" | "ble" | "bgt" | "bge" | "blo" | "bls"
                | "bhi" | "bhs" | "bpl" | "bmi" | "bvc" | "bvs" => {
                    let address = self.extract_address(&op)?;
                    match name[1..].parse() {
                        Ok(condition) => Instruction::Bcc(address, condition),
                        Err(_) => {
                            return Err(CompilationError::ParseError(format!(
                                "Invalid condition code: {}",
                                name
                            )));
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
                | "smi" | "spl" | "svc" | "svs" | "sf" | "st" | "shs" | "slo" => {
                    match name[1..].parse() {
                        Ok(condition) => Instruction::Scc(op, condition),
                        Err(_) => {
                            return Err(CompilationError::ParseError(format!(
                                "Invalid condition code: {}",
                                name
                            )));
                        }
                    }
                }
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
                    )));
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
                    )));
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

    fn parse_register_with_size(&mut self, operand: &LexedOperand, line: &ParsedLine) -> CompilationResult<(RegisterOperand, Size)> {
        match operand {
            LexedOperand::Register(register_type, register_name) => {
                let register = self.parse_register(register_type, register_name)?;
                Ok((register, Size::Word))
            }
            LexedOperand::RegisterWithSize(register_type, register_name, size) => {
                let register = self.parse_register(register_type, register_name)?;
                let size = self.get_size(size, Size::Word).unwrap();
                if size == Size::Byte {
                    Err(CompilationError::InvalidAddressingMode(
                        "Invalid size for register, byte is not allowed".to_string(),
                    ))
                } else {
                    Ok((register, size))
                }
            }
            _ => Err(CompilationError::ParseError(format!(
                "Invalid operand: {:?}",
                operand
            ))),
        }
    }
    fn parse_register(&mut self, register_type: &LexedRegisterType, register_name: &String) -> CompilationResult<RegisterOperand> {
        match register_type {
            LexedRegisterType::Address => match register_name[1..].parse() {
                Ok(reg) => Ok(RegisterOperand::Address(reg)),
                Err(_) => Err(CompilationError::ParseError(format!(
                    "Invalid a register name: {}",
                    register_name
                ))),
            },
            LexedRegisterType::Data => match register_name[1..].parse() {
                Ok(reg) => Ok(RegisterOperand::Data(reg)),
                Err(_) => Err(CompilationError::ParseError(format!(
                    "Invalid d register name: {}",
                    register_name
                ))),
            },
            LexedRegisterType::SP => Ok(RegisterOperand::Address(7)),
        }
    }
    fn parse_operand(
        &mut self,
        operand: &LexedOperand,
        line: &ParsedLine,
    ) -> CompilationResult<Operand> {
        match operand {
            LexedOperand::Register(register_type, register_name) => {
                let register = self.parse_register(register_type, register_name)?;
                Ok(Operand::Register(register))
            }
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

            LexedOperand::IndirectDisplacement { offset, operand } => {
                let parsed_operand = self.parse_operand(operand, line)?;
                let parsed_operand = self.extract_register(parsed_operand)?;
                let offset = if offset.trim() == "" {
                    0
                } else {
                    match parse_absolute_expression(offset, &self.labels) {
                        Ok(offset) => sign_extend_to_long(offset as u32, Size::Word),
                        Err(_) => {
                            return Err(CompilationError::ParseError(format!(
                                "Invalid offset: {}",
                                offset
                            )));
                        }
                    }
                };
                Ok(Operand::IndirectDisplacement {
                    offset,
                    base: parsed_operand,
                })
            }
            LexedOperand::IndirectIndex { offset, operands } => {
                let offset = if offset == "" {
                    0
                } else {
                    match parse_absolute_expression(offset, &self.labels) {
                        Ok(offset) => sign_extend_to_long(offset as u32, Size::Byte),
                        Err(_) => {
                            return Err(CompilationError::ParseError(format!(
                                "Invalid offset: {}",
                                offset
                            )));
                        }
                    }
                };
                if operands.len() != 2 {
                    return Err(CompilationError::ParseError(format!(
                        "Invalid number of operands for indirect index addressing mode: {:?}, expected 2 operands, found {}",
                        operands,
                        operands.len()
                    )));
                }
                let first = self.parse_operand(&operands[0], line)?;
                let first = self.extract_register(first)?;
                let second = self.parse_register_with_size(&operands[1], line)?;
                match first {
                    RegisterOperand::Data(_) => {
                        return Err(CompilationError::InvalidAddressingMode(
                            "First operand of indirect index addressing mode must be an address register".to_string(),
                        ));
                    }
                    RegisterOperand::Address(_) => {
                        Ok(Operand::IndirectIndex {
                            offset,
                            base: first,
                            index: IndexRegister {
                                register: second.0,
                                size: second.1,
                            },
                        })
                    }
                }
            }
            LexedOperand::Indirect(operand) => {
                let parsed_operand = self.parse_operand(operand, line)?;
                let parsed_operand = self.extract_register(parsed_operand)?;
                match parsed_operand {
                    RegisterOperand::Data(_) => {
                        Err(CompilationError::InvalidAddressingMode( 
                            "Operand of indirect addressing mode must be an address register".to_string(),
                        ))
                    }
                    RegisterOperand::Address(a) => Ok(Operand::Indirect(a)),
                }
            }
            LexedOperand::PostIndirect(operand) => {
                let parsed_operand = self.parse_operand(operand, line)?;
                let parsed_operand = self.extract_register(parsed_operand)?;
                match parsed_operand {
                    RegisterOperand::Data(_) => {
                        Err(CompilationError::InvalidAddressingMode(
                            "Operand of post indirect addressing mode must be an address register".to_string(),
                        ))
                    }
                    RegisterOperand::Address(a) => Ok(Operand::PostIndirect(a)),
                }
            }
            LexedOperand::PreIndirect(operand) => {
                let parsed_operand = self.parse_operand(operand, line)?;
                let parsed_operand = self.extract_register(parsed_operand)?;
                match parsed_operand {
                    RegisterOperand::Data(_) => {
                        Err(CompilationError::InvalidAddressingMode(
                            "Operand of pre indirect addressing mode must be an address register".to_string(),
                        ))
                    }
                    RegisterOperand::Address(a) => Ok(Operand::PreIndirect(a)),
                }
            }
            LexedOperand::Immediate(num) => match self.parse_immediate(num) {
                Ok(absolute) => Ok(Operand::Immediate(absolute)),
                Err(e) => Err(CompilationError::ParseError(format!(
                    "Invalid immediate: {}",
                    e.get_message()
                ))),
            },
            LexedOperand::RegisterRange { mask } => Ok(Operand::Immediate(*mask as u32)),
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
                                    ));
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
                            .flat_map(|x| x.to_be_bytes())
                            .collect(),
                        _ => {
                            return Err(CompilationError::Raw(
                                "Invalid number of arguments for DCB directive".to_string(),
                            ));
                        }
                    },
                    //TODO is word default?
                    LexedSize::Word | LexedSize::Unspecified => match parsed_args[..] {
                        [size, default] => vec![default as u16; size as usize]
                            .iter()
                            .flat_map(|x| x.to_be_bytes())
                            .collect(),
                        _ => {
                            return Err(CompilationError::Raw(
                                "Invalid number of arguments for DCB directive".to_string(),
                            ));
                        }
                    },
                    LexedSize::Byte => match parsed_args[..] {
                        [size, default] => vec![default as u8; size as usize],
                        _ => {
                            return Err(CompilationError::Raw(
                                "Invalid number of arguments for DCB directive".to_string(),
                            ));
                        }
                    },
                    _ => {
                        return Err(CompilationError::Raw(
                            "Invalid size for DCB directive".to_string(),
                        ));
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
                            Err(e) => {
                                return Err(format!(
                                    "Invalid hex ORG address: {}; at line {}, {:?}",
                                    args[1], line.line_index, e
                                ));
                            }
                        };
                        if parsed < last_address {
                            return Err(format!("The address of the ORG directive ({}) must be greater than the previous address ({}); at line {}", parsed, last_address, line.line_index));
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
                        Err(e) => {
                            return Err(format!(
                                "Invalid number of bytes for DS directive at line {}, {:?}",
                                line.line_index, e
                            ));
                        }
                    },
                    "dcb" => match self.parse_absolute(&args[1]) {
                        Ok(bytes) => {
                            next_address = last_address
                                + (bytes * size.to_bytes_word_default() as u32) as usize;
                        }
                        Err(e) => {
                            return Err(format!(
                                "Invalid number of bytes for dcb directive at line {}, {:?}",
                                line.line_index, e
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
                            line: line.line_index,
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
