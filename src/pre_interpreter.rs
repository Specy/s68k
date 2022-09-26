use std::collections::HashMap;

use crate::{
    lexer::{LexedLine, LexedOperand, ParsedLine, RegisterType, Size},
    utils::parse_char_or_num,
};

struct Directive {
    pub args: Vec<i32>,
    pub name: String,
    pub size: Size,
}
struct Label {
    directive: Option<Directive>,
    name: String,
    address: usize,
}
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
    Address(i32),
}

struct PreInterpreter {
    labels: HashMap<String, Label>,
    instructions: Vec<InstructionLine>,
}
struct InstructionLine {
    instruction: Instruction,
}

struct Instruction {
    opcode: String,
    operands: Vec<Operand>,
    size: Size,
}
impl PreInterpreter {
    pub fn new(lines: &Vec<ParsedLine>) -> PreInterpreter {
        let mut pre_interpreter = PreInterpreter {
            labels: HashMap::new(),
            instructions: Vec::new(),
        };
        pre_interpreter.load(lines);
        pre_interpreter
    }
    pub fn load(&mut self, lines: &Vec<ParsedLine>) {
        self.populate_label_map(lines);
    }
    fn populate_label_map(&mut self, lines: &Vec<ParsedLine>) {
        let mut last_address = 1000;
        for line in lines {
            match &line.parsed {
                LexedLine::Directive { args, .. } => {
                    if args[0] == "org" {
                        last_address = args[1].parse::<usize>().unwrap();
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
        for line in lines {
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
                    self.instructions.push(InstructionLine { instruction });
                }
                _ => {}
            }
        }
        todo!("think through what to provide in the InstructionLine and Instruction");
    }

    fn parse_operand(&mut self, operand: &LexedOperand) -> Operand {
        match operand {
            _ => {
                todo!("implement operand parsing")
            }
        }
    }
}
