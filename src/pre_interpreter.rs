use std::collections::HashMap;

use crate::lexer::{ParsedLine, RegisterType, Size, LexedLine};

struct FinalLabel {
    label: String,
    address: usize,
}

struct PreInterpreter {
    labels: HashMap<String, FinalLabel>,
}

pub enum Operand {
    Register(RegisterType, u8),
    Immediate(i64),
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
    Label(i32),
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
        };
        pre_interpreter.load(lines);
        pre_interpreter
    }
    pub fn load(&mut self, lines: &Vec<ParsedLine>) {
        let mut last_address = 1000;
        for (i, line) in lines.iter().enumerate() {
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
                        FinalLabel {
                            label: name,
                            address: last_address,
                        },
                    );
                }
                LexedLine::LabelDirective { name, directive } => {
                    self.labels.insert(
                        name.clone(),
                        FinalLabel {
                            label: name.clone(),
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
                _ => { }
            }
            last_address += 4;
        }
    }
}
