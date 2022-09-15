use std::collections::HashMap;

use regex::Regex;
enum RegisterType {
    Address,
    Data,
    PC,
}
macro_rules! string_vec {
    ($($x:expr),*) => (vec![$($x.to_string()),*]);
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
struct AsmRegex {
    instructions: Vec<String>,
    instructions_hash_map: HashMap<String, bool>,
    directives: Vec<String>,
    directives_hash_map: HashMap<String, bool>,
    register: Regex,
    immediate: Regex,
    indirect: Regex,
    indirect_displacement: Regex,
    post_indirect: Regex,
    pre_indirect: Regex,
    label_line: Regex,
    directive_line: Regex,
    comment_line: Regex,
    arg_separator: Regex,
}
#[derive(Debug)]
enum OperandKind {
    Register,
    Immediate,
    Indirect,
    IndirectDisplacement,
    PostIndirect,
    PreIndirect,
    Label,
}
#[derive(Debug)]
enum LineKind {
    Label,
    Directive,
    Instruction,
    Comment,
    Unknown,
}
impl AsmRegex {
    pub fn new() -> Self {
        let instructions = string_vec![
            "add", "addi", "adda", "sub", "subi", "suba", "muls", "mulu", "divs", "divu", "and",
            "andi", "or", "ori", "eor", "eori", "not", "neg", "clr", "cmp", "cmpi", "cmpa", "tst",
            "asl", "asr", "lsr", "lsl", "ror", "rol", "jmp", "bra", "jsr", "rts", "bsr", "beq",
            "bne", "bge", "bgt", "ble", "blt"
        ];
        let directives = string_vec!["equ", "org"];
        let directives_hash_map = directives
            .iter()
            .map(|x| (x.to_string(), true))
            .collect::<HashMap<String, bool>>();
        let instructions_hash_map = instructions
            .iter()
            .map(|x| (x.to_string(), true))
            .collect::<HashMap<String, bool>>();
        AsmRegex {
            arg_separator: Regex::new(r"(\s*,\s*)|\s").unwrap(),
            instructions,
            instructions_hash_map,
            directives,
            directives_hash_map,
            register: Regex::new(r"^((d|a)\d|pc)$").unwrap(),
            immediate: Regex::new(r"^\#\S+$").unwrap(),
            indirect: Regex::new(r"^\(((d|a)\d|pc)\)$").unwrap(),
            indirect_displacement: Regex::new(r"^((.+,)+.+)$").unwrap(),
            post_indirect: Regex::new(r"^\(\S+\)\+$").unwrap(),
            pre_indirect: Regex::new(r"^-\(\S+\)$").unwrap(),
            directive_line: Regex::new(r"^(\S+ \S+)").unwrap(),
            label_line: Regex::new(r"^\S+:.+").unwrap(),

            comment_line: Regex::new(r"^;.*").unwrap(),
        }
    }
    pub fn testOperand(&self, operand: &String) -> OperandKind {
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
    pub fn testLine(&self, line: &String) -> (LineKind, Vec<&str>) {
        let line = line.trim();
        let args = self.arg_separator.split(line).collect::<Vec<&str>>();
        let kind = match args[..] {
            [first, ..] if self.comment_line.is_match(first) => LineKind::Comment,
            [first, ..] if self.instructions_hash_map.contains_key(first) => LineKind::Instruction,
            [first, ..] if self.comment_line.is_match(first) => LineKind::Comment,
            [first, ..] if self.label_line.is_match(first) => LineKind::Label,
            _ => {
                let mut kind = LineKind::Unknown;
                for arg in args {
                    if self.directives_hash_map.contains_key(arg) {
                       kind = LineKind::Directive;
                       break;
                    }
                }
                kind
            }
        };
        (kind, args)
    }
}

pub struct Lexer {
    lines: Vec<Line>,
    regex: AsmRegex,
}
impl Lexer {
    pub fn new() -> Self {
        Lexer {
            lines: Vec::new(),
            regex: AsmRegex::new(),
        }
    }
    pub fn parse(&self, code: String) {
        let lines = code.lines();
        for line in lines {
            let kind = self.regex.testLine(&line.to_string());
            println!("{:?} ({})", kind, line);
        }
    }
    pub fn parseOperand(&self, operand: String) -> Operand {
        let operand = operand.trim().to_lowercase();
        match self.regex.testOperand(&operand) {
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
            OperandKind::Indirect => Operand::Indirect(IndirectType::Address, operand),
            OperandKind::IndirectDisplacement => {
                Operand::Indirect(IndirectType::Displacement, operand)
            }
            OperandKind::PostIndirect => Operand::PostIndirect(operand),
            OperandKind::PreIndirect => Operand::PreIndirect(operand),
            OperandKind::Label => Operand::Label(operand),
        }
    }
}
