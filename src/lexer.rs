use regex::Regex;
enum RegisterType {
    Address,
    Data,
    PC,
}
enum IndirectType {
    Address,
    Displacement,
}
enum Operand {
    Register(RegisterType, String),
    Immediate(String),
    Indirect(IndirectType, String),
    PostIndirect(String),
    PreIndirect(String),
    Label(String),
    None,
}
enum Line {
    Label {
        name: String,
        operands: Vec<Operand>,
        line: usize,
        comment: Option<String>,
    },
    Directive {
        name: String,
        operands: Vec<Operand>,
        line: usize,
        comment: Option<String>,
    },
    Instruction {
        name: String,

        operands: Vec<Operand>,
        line: usize,
        comment: Option<String>,
    },
    Comment {
        comment: String,
        line: usize,
    },
}
struct OperandRegex {
    register: Regex,
    immediate: Regex,
    indirect: Regex,
    indirect_displacement: Regex,
    post_indirect: Regex,
    pre_indirect: Regex,
}
enum OperandKind {
    Register,
    Immediate,
    Indirect,
    IndirectDisplacement,
    PostIndirect,
    PreIndirect,
    Label,
}
impl OperandRegex {
    pub fn new() -> Self {
        OperandRegex {
            register: Regex::new(r"^((d|a)\d*|pc)$").unwrap(),
            immediate: Regex::new(r"^\#\S+$").unwrap(),
            indirect: Regex::new(r"^(.+)$").unwrap(),
            indirect_displacement: Regex::new(r"^((.+,)+.+)$").unwrap(),
            post_indirect: Regex::new(r"^\(\S+\)\+$").unwrap(),
            pre_indirect: Regex::new(r"^-\(\S+\)$").unwrap(),
        }
    }
    pub fn test(&self, operand: &String) -> OperandKind {
        match operand {
            _ if self.register.is_match(operand) => OperandKind::Register,
            _ if self.immediate.is_match(operand) => OperandKind::Immediate,
            _ if self.indirect.is_match(operand) => OperandKind::Indirect,
            _ if self.indirect_displacement.is_match(operand) => OperandKind::IndirectDisplacement,
            _ if self.post_indirect.is_match(operand) => OperandKind::PostIndirect,
            _ if self.pre_indirect.is_match(operand) => OperandKind::PreIndirect,
            _ => OperandKind::Label,
        }
    }
}

struct Lexer {
    lines: Vec<Line>,
    operand_regex: OperandRegex,
}
impl Lexer {
    pub fn new() -> Self {
        Lexer {
            lines: Vec::new(),
            operand_regex: OperandRegex::new(),
        }
    }
    pub fn parse(code: String) {}
    pub fn parseOperand(&self, operand: String) -> Operand {
        let operand = operand.trim().to_lowercase();
        match self.operand_regex.test(&operand) {
            OperandKind::Immediate => Operand::Immediate(operand),
            OperandKind::Register => {
                let register_type = match operand.chars().nth(0).unwrap() {
                    'd' => RegisterType::Data,
                    'a' => RegisterType::Address,
                    'p' => RegisterType::PC,
                    _ => panic!("Invalid register type"),
                };
                Operand::Register(register_type, operand)
            }
            OperandKind::Indirect => {
                Operand::Indirect(IndirectType::Address, operand)
            }
            OperandKind::IndirectDisplacement => {
                Operand::Indirect(IndirectType::Displacement, operand)
            }
            OperandKind::PostIndirect => {
                Operand::PostIndirect(operand)
            }
            OperandKind::PreIndirect => {
                Operand::PreIndirect(operand)
            }
            OperandKind::Label => {
                Operand::Label(operand)
            }   
        }
    }
}
