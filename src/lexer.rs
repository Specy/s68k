use std::collections::HashSet;

use bitflags::bitflags;
use regex::Regex;
use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::wasm_bindgen;

//TODO remake everything with an actual lexer
use crate::constants::{COMMENT_1, COMMENT_2, EQU};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[wasm_bindgen]
pub enum LexedRegisterType {
    Address,
    Data,
    SP,
}

impl LexedRegisterType {
    pub fn from_string(string: &str) -> Result<LexedRegisterType, String> {
        if string.len() < 2 {
            return Err(format!("Invalid register type '{}'", string));
        }
        match string.chars().collect::<Vec<char>>().as_slice() {
            ['d', num] if num.is_digit(10) => Ok(LexedRegisterType::Data),
            ['a', num] if num.is_digit(10) => Ok(LexedRegisterType::Address),
            ['s', 'p'] => Ok(LexedRegisterType::SP),
            _ => Err(format!("Invalid register type '{}'", string)),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[wasm_bindgen]
pub enum LexedSize {
    Byte,
    Word,
    Long,
    Unspecified,
    Unknown,
}

impl LexedSize {
    pub fn to_bytes(&self, default: LexedSize) -> u8 {
        match self {
            LexedSize::Byte => 1,
            LexedSize::Word => 2,
            LexedSize::Long => 4,
            LexedSize::Unspecified => default.to_bytes(LexedSize::Unknown),
            _ => 0,
        }
    }
    pub fn to_bytes_word_default(&self) -> u8 {
        self.to_bytes(LexedSize::Word)
    }
    pub fn to_bits(&self, default: LexedSize) -> u8 {
        self.to_bytes(default) * 8
    }
    pub fn to_bits_word_default(&self) -> u8 {
        self.to_bits(LexedSize::Word)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "value")]
pub enum LexedOperand {
    Immediate(String),
    RegisterRange{
        mask: u16
    },
    Register(LexedRegisterType, String),
    RegisterWithSize(LexedRegisterType, String, LexedSize),
    Indirect(Box<LexedOperand>),
    IndirectDisplacement {
        offset: String,
        operand: Box<LexedOperand>,
    },
    IndirectIndex {
        offset: String,
        operands: Vec<LexedOperand>,
    },
    PostIndirect(Box<LexedOperand>),
    PreIndirect(Box<LexedOperand>),
    Absolute(String),
    Label(String),
    Other(String),
}

impl LexedOperand {
    pub fn affects_memory(&self) -> bool {
        match self {
            LexedOperand::Indirect(_) => true,
            LexedOperand::IndirectDisplacement { .. } => true,
            LexedOperand::IndirectIndex { .. } => true,
            LexedOperand::PostIndirect(_) => true,
            LexedOperand::PreIndirect(_) => true,
            LexedOperand::Absolute(_) => true,

            _ => false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "value")]
pub enum LexedLine {
    Label {
        name: String,
    },
    Directive {
        name: String,
        size: LexedSize,
        args: Vec<String>,
    },
    Instruction {
        name: String,
        operands: Vec<LexedOperand>,
        size: LexedSize,
    },
    Comment {
        content: String,
    },
    Empty,
    Unknown {
        content: String,
    },
}

#[derive(Debug)]
#[wasm_bindgen]
pub enum OperandKind {
    Register,
    RegisterList,
    RegisterWithSize,
    Immediate,
    Indirect,
    IndirectDisplacement,
    IndirectIndex,
    PostIndirect,
    PreIndirect,
    Absolute,
}

#[derive(Debug, Clone)]
pub enum LineKind {
    Label { name: String, inner: Option<String> },
    Directive,
    Instruction { size: LexedSize, name: String },
    Comment,
    Empty,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SeparatorKind {
    Comma,
    Space,
}
/*
TODO maybe instead of making this LabelDirective, make it as a simple directive, so that the label actually refers to
the next instruction, and not the directive itself
for example

data: dc.b 1, 2, 3
--------------------
data:
dc.b 1,2,3
--------------------
should be the same thing.

At the same time, the directive should have dc/ds/dcb/etc and not just org
*/

enum Grammar {
    Directive,
    Register,
    RegisterWithSize,
    Indirect,
    RegisterRange,
    IndirectDisplacement,
    IndirectIndex,
    PostIndirect,
    PreIndirect,
    Immediate,
    Comment,
    CommentLine,
    Label,
    LabelInclusive,
    Operand,
    OperandArg,
    Absolute,
}

bitflags! {
    struct GrammarOptions:u32{
        const NONE = 0;
        const IS_LINE = 1;
        const IGNORE_CASE = 2;
    }
}

#[derive(Debug)]
enum LexLineResult {
    Line(LexedLine),
    Multiple(Vec<LexedLine>),
}

impl Grammar {
    fn get_regex(&self) -> String {
        match &self {
            Grammar::Directive => r"(.+\s+equ\s+.+)|((org|dc|dcb|ds)\s*.*)".to_string(),
            Grammar::Register => r"(d\d|a\d|sp)".to_string(),
            Grammar::RegisterRange => {
                let r = Grammar::Register.get_regex();
                //this accepts strings like: "d0-d5/a0-a6/a0/a4"
                format!("(((({})-({}))|({}))\\/)*((({})-({}))|({}))", r, r, r, r, r, r)
            }
            Grammar::RegisterWithSize => format!(
                r"({})\.(b|w|l)",
                Grammar::Register.get_regex()
            ),
            Grammar::Indirect => format!(r"\({}\)", Grammar::Register.get_regex()),
            Grammar::IndirectDisplacement => format!(r"([^\r\n\t\f\v,])*\({}\)", Grammar::Register.get_regex()),
            Grammar::IndirectIndex => r"([^\r\n\t\f\v,])*\((.+,)+.+\)".to_string(),
            Grammar::PostIndirect => r"\(\w+\)\+".to_string(), //TODO should i include registers in here or leave it?
            Grammar::PreIndirect => r"-\(\w+\)".to_string(),
            Grammar::Immediate => r"#(('.+')|(\S+))".to_string(), //TODO could #add absolute here but it wouldn't change the end result
            Grammar::Comment => r"\s+((;|\*).*)|^(;.*|\*.*)".to_string(),
            Grammar::CommentLine => r"^((;|\*).*)$".to_string(),
            Grammar::Label => r"\w+:$".to_string(),
            Grammar::LabelInclusive => r"\w+:.*".to_string(),
            Grammar::Absolute => r"((%|@|$|)\w+)|('.')|(\d+)".to_string(), //TODO this does not include labels
            Grammar::OperandArg => r"(\w*\((?:.+,)+.+\)\w*)|(\w+)|(#\S+)".to_string(),
            Grammar::Operand => format!(
                r"(({})|({})|({})|({})|({})|({})|({})|({}))",
                Grammar::Register.get_regex(),
                Grammar::Indirect.get_regex(),
                Grammar::IndirectIndex.get_regex(),
                Grammar::IndirectDisplacement.get_regex(),
                Grammar::PostIndirect.get_regex(),
                Grammar::PreIndirect.get_regex(),
                Grammar::Immediate.get_regex(),
                Grammar::Absolute.get_regex()
            ),
        }
    }
    fn get_opt(&self, options: GrammarOptions) -> String {
        let mut regex = self.get_regex();
        if options.contains(GrammarOptions::IGNORE_CASE) {
            regex = format!(r"(?i){}", regex);
        }
        if options.contains(GrammarOptions::IS_LINE) {
            regex = format!(r"^{}$", regex);
        }

        regex
    }
}
/*

indirect_displacement
*/

struct AsmRegex {
    register_only: Regex,
    register_list_only: Regex,
    register_with_size_only: Regex,
    immediate_only: Regex,
    indirect_only: Regex,
    indirect_displacement_only: Regex,
    indirect_index_only: Regex,
    post_indirect_only: Regex,
    pre_indirect_only: Regex,
    label_line: Regex,
    directive: Regex,
    operand_arg: Regex,
    comment_line: Regex,
    comment: Regex,
}

impl AsmRegex {
    pub fn new() -> Self {
        AsmRegex {
            register_only: Regex::new(&Grammar::Register.get_opt(GrammarOptions::IGNORE_CASE | GrammarOptions::IS_LINE)).unwrap(),
            register_list_only: Regex::new(&Grammar::RegisterRange.get_opt(GrammarOptions::IGNORE_CASE | GrammarOptions::IS_LINE)).unwrap(),
            register_with_size_only: Regex::new(&Grammar::RegisterWithSize.get_opt(GrammarOptions::IGNORE_CASE | GrammarOptions::IS_LINE)).unwrap(),
            immediate_only: Regex::new(&Grammar::Immediate.get_opt(GrammarOptions::IS_LINE)).unwrap(),
            indirect_only: Regex::new(&Grammar::Indirect.get_opt(GrammarOptions::IGNORE_CASE | GrammarOptions::IS_LINE)).unwrap(),
            indirect_displacement_only: Regex::new(&Grammar::IndirectDisplacement.get_opt(GrammarOptions::IS_LINE)).unwrap(),
            indirect_index_only: Regex::new(&Grammar::IndirectIndex.get_opt(GrammarOptions::IS_LINE)).unwrap(),
            post_indirect_only: Regex::new(&Grammar::PostIndirect.get_opt(GrammarOptions::IS_LINE)).unwrap(),
            pre_indirect_only: Regex::new(&Grammar::PreIndirect.get_opt(GrammarOptions::IS_LINE)).unwrap(),
            label_line: Regex::new(r"^\S+:.*").unwrap(),
            directive: Regex::new(&format!(r"^\s*({})", Grammar::Directive.get_opt(GrammarOptions::IGNORE_CASE)))
                .unwrap(),
            operand_arg: Regex::new(&Grammar::OperandArg.get_regex()).unwrap(),
            comment_line: Regex::new(&Grammar::CommentLine.get_regex()).unwrap(),
            comment: Regex::new(&Grammar::Comment.get_regex()).unwrap(),
        }
    }
    pub fn get_operand_kind(&self, operand: &String) -> OperandKind {
        let kind = match operand {
            //TODO order is important
            _ if self.post_indirect_only.is_match(operand) => OperandKind::PostIndirect,
            _ if self.pre_indirect_only.is_match(operand) => OperandKind::PreIndirect,
            _ if self.indirect_only.is_match(operand) => OperandKind::Indirect,
            _ if self.indirect_index_only.is_match(operand) => OperandKind::IndirectIndex,
            _ if self.indirect_displacement_only.is_match(operand) => OperandKind::IndirectDisplacement,
            _ if self.register_with_size_only.is_match(operand) => OperandKind::RegisterWithSize,
            _ if self.register_only.is_match(operand) => OperandKind::Register,
            _ if self.register_list_only.is_match(operand) => OperandKind::RegisterList,
            _ if self.immediate_only.is_match(operand) => OperandKind::Immediate,
            //_ if self.absolute.is_match(operand) => OperandKind::Absolute,
            _ => OperandKind::Absolute,
        };
        kind
    }
    pub fn split_at_size(&self, data: &String) -> (String, LexedSize) {
        let data = data.to_string();
        let split = data.split('.').collect::<Vec<&str>>();
        match split[..] {
            [first] => (first.to_string(), LexedSize::Unspecified),
            [first, size] => {
                let size = match size {
                    "b" | "B" => LexedSize::Byte,
                    "w" | "W" => LexedSize::Word,
                    "l" | "L" => LexedSize::Long,
                    _ => LexedSize::Unknown,
                };
                (first.to_string(), size)
            }
            _ => (data, LexedSize::Unspecified),
        }
    }
    pub fn split_into_separated_args(&self, line: &str, ignore_space: bool) -> Vec<String> {
        let mut args = vec![];
        let mut current_arg = String::new();
        let mut in_parenthesis = false;
        let mut in_quotes = false;
        let mut last_char = ' ';
        let mut last_separator = ' ';
        if line.len() == 0 {
            return args;
        }
        for c in line.chars() {
            match c {
                '(' if !in_quotes => {
                    in_parenthesis = true;
                    current_arg.push(c);
                }
                ')' if !in_quotes => {
                    in_parenthesis = false;
                    current_arg.push(c);
                }
                '\'' if !in_parenthesis => {
                    in_quotes = !in_quotes;
                    current_arg.push(c);
                }
                ',' => {
                    if in_parenthesis || in_quotes {
                        //ignore if in parenthesis or in quotes
                        current_arg.push(c);
                    } else {
                        args.push(current_arg.trim().to_string());
                        current_arg = String::new();
                        last_separator = c;
                    }
                }
                COMMENT_1 | COMMENT_2 => {
                    if last_char == ' ' {
                        break;
                    }
                    current_arg.push(c);
                } //if it reaches the end where there is a comment
                ' ' => {
                    // last_char == ',' ||
                    if ignore_space && !in_quotes {
                        continue;
                    }
                    if in_parenthesis || in_quotes {
                        //ignore if in parenthesis or if it's a char
                        current_arg.push(c);
                    } else {
                        if current_arg == "" {
                            continue;
                        }
                        args.push(current_arg.trim().to_string());
                        current_arg = String::new();
                        last_separator = c;
                    }
                }
                _ => {
                    current_arg.push(c);
                }
            }
            last_char = c;
        }
        match current_arg.trim() {
            "" => args,
            _ => match last_separator {
                _ => {
                    args.push(current_arg.trim().to_string());
                    args
                }
            },
        }
    }
    pub fn split_at_whitespace(&self, line: &str) -> Vec<String> {
        line.replace("\t", " ")
            .trim()
            .split(' ')
            .map(|x| x.to_string())
            .filter(|x| !x.is_empty())
            .collect::<Vec<String>>()
    }
    pub fn split_at_comment<'a>(&self, string: &'a str) -> Vec<&'a str> {
        self.comment.split(&string).collect()
    }
    pub fn get_line_kind(&self, line: &String) -> LineKind {
        let line = line.trim();
        let args = line
            .split_whitespace()
            .map(|x| x.trim().to_string())
            .collect::<Vec<String>>();
        match args[..] {
            [] => LineKind::Empty,
            _ if self.comment_line.is_match(line) => LineKind::Comment,
            _ if self.label_line.is_match(line) => {
                let split = line.splitn(2, ':');
                match split.collect::<Vec<&str>>()[..] {
                    [name, inclusive] => LineKind::Label {
                        name: name.to_string(),
                        inner: Some(inclusive.to_string()),
                    },
                    [name] => LineKind::Label {
                        name: name.to_string(),
                        inner: None,
                    },
                    _ => LineKind::Unknown,
                }
            }
            _ if self.directive.is_match(line) => LineKind::Directive,
            //TODO why this distinction?
            [_, _, ..] | [_] => {
                let (instruction, size) = self.split_at_size(&args[0].to_lowercase());
                LineKind::Instruction {
                    size,
                    name: instruction,
                }
            }
        }
    }
}

#[derive(Debug)]
pub struct EquValue {
    pub name: String,
    pub replacement: String,
}

impl EquValue {
    pub fn new(name: String, replacement: String) -> EquValue {
        EquValue { name, replacement }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParsedLine {
    pub parsed: LexedLine,
    pub line: String,
    pub line_index: usize,
}

pub struct Lexer {
    lines: Vec<ParsedLine>,
    regex: AsmRegex,
}

impl Lexer {
    pub fn new() -> Self {
        Lexer {
            lines: Vec::new(),
            regex: AsmRegex::new(),
        }
    }
    pub fn parse_operands(&self, operands: Vec<String>) -> Vec<LexedOperand> {
        operands
            .iter()
            .map(|o| self.parse_operand(o))
            .collect()
    }
    pub fn parse_operand(&self, operand: &String) -> LexedOperand {
        let operand = operand.to_string();
        match self.regex.get_operand_kind(&operand) {
            OperandKind::Immediate => LexedOperand::Immediate(operand),
            OperandKind::RegisterWithSize => {
                let split = operand.split('.').collect::<Vec<&str>>();
                match split[..] {
                    [register, size] => {
                        let register = self.parse_operand(&register.to_string());
                        let size = match size {
                            "b" => LexedSize::Byte,
                            "w" => LexedSize::Word,
                            "l" => LexedSize::Long,
                            _ => return LexedOperand::Other(split.join(".")),
                        };
                        match register {
                            LexedOperand::Register(reg, name) => {
                                LexedOperand::RegisterWithSize(reg, name, size)
                            }
                            _ => LexedOperand::Other(operand),
                        }
                    }
                    _ => LexedOperand::Other(operand),
                }
            }
            OperandKind::Register => {
                let operand = operand.to_lowercase();
                let register_type = match operand.chars().nth(0).expect("Missing register") {
                    'd' => LexedRegisterType::Data,
                    'a' => LexedRegisterType::Address,
                    's' => LexedRegisterType::SP, //TODO this might fail
                    _ => panic!("Invalid register type '{}'", operand),
                };
                LexedOperand::Register(register_type, operand)
            }
            OperandKind::RegisterList => {
                let groups = operand.split('/').collect::<Vec<&str>>();
                let mut mask = 0u16;
                for group in groups {
                    let split = group.split('-').collect::<Vec<&str>>();
                    match split[..] {
                        [start, end] => {
                            let start = parse_register_range(start);
                            let end = parse_register_range(end);
                            if start.is_err() || end.is_err() {
                                return LexedOperand::Other(operand);
                            }
                            let (start_reg, start_num) = start.unwrap();
                            let (end_reg, end_num) = end.unwrap();
                            if start_reg != end_reg {
                                return LexedOperand::Other(operand);
                            }
                            let base = match start_reg {
                                LexedRegisterType::Data => 0,
                                LexedRegisterType::Address => 8,
                                LexedRegisterType::SP => 15,
                            };
                            for i in start_num..=end_num {
                                mask |= 1 << (base + i);
                            }
                        }
                        [single] => {
                            let single = parse_register_range(single);
                            if single.is_err() {
                                return LexedOperand::Other(operand);
                            }
                            let (reg, num) = single.unwrap();
                            let base = match reg {
                                LexedRegisterType::Data => 0,
                                LexedRegisterType::Address => 8,
                                LexedRegisterType::SP => 15,
                            };
                            mask |= 1 << (base + num);
                        }
                        _ => return LexedOperand::Other(operand),
                    }
                }
                LexedOperand::RegisterRange { mask }
            }
            OperandKind::Indirect => {
                let operand = operand.replace("(", "").replace(")", "");
                let operand = self.parse_operand(&operand);
                LexedOperand::Indirect(Box::new(operand))
            }
            OperandKind::IndirectIndex
            => {
                let split = operand.split('(').collect::<Vec<&str>>();
                if split.len() != 2 {
                    return LexedOperand::Other(operand);
                }
                let offset = split[0].trim().to_string();
                let args = split[1].replace(")", "");
                let args = self.regex.split_into_separated_args(args.trim(), true);
                let operands = self.parse_operands(args);
                LexedOperand::IndirectIndex {
                    offset,
                    operands,
                }
            }
            OperandKind::IndirectDisplacement => {
                let split = operand.split('(').collect::<Vec<&str>>();
                if split.len() != 2 {
                    return LexedOperand::Other(operand);
                }
                let offset = split[0].trim().to_string();
                let args = split[1].replace(")", "");
                let args = self.regex.split_into_separated_args(args.trim(), true);
                let operands = self.parse_operands(args);
                if operands.len() != 1 {
                    return LexedOperand::Other(operand);
                }
                LexedOperand::IndirectDisplacement {
                    offset,
                    operand: Box::new(operands[0].to_owned()),
                }
            }
            OperandKind::Absolute => LexedOperand::Absolute(operand),
            OperandKind::PostIndirect => {
                let parsed_operand = operand.replace("(", "").replace(")+", "");
                let arg = self.parse_operand(&parsed_operand);
                LexedOperand::PostIndirect(Box::new(arg))
            }
            OperandKind::PreIndirect => {
                let parsed_operand = operand.replace("-(", "").replace(")", "");
                let arg = self.parse_operand(&parsed_operand);
                LexedOperand::PreIndirect(Box::new(arg))
            }
        }
    }

    pub fn make_equ_map(&self, lines: &Vec<String>) -> Vec<(String, String)> {
        let mut equs: Vec<(String, String)> = vec![];
        lines
            .iter()
            .map(|line| {
                let split_at_comments = self.regex.split_at_comment(line);
                let code = split_at_comments[0].trim();
                self.regex.split_at_whitespace(code)
            })
            .for_each(|args| {
                if args.len() >= 3 && args[1] == EQU {
                    equs.push((args[0].to_string(), args[2..].join(" ")));
                }
            });
        //sort by length so that the longest ones are replaced first
        equs.sort_by(|a, b| b.0.len().cmp(&a.0.len()));
        equs
    }
    pub fn lex(&mut self, code: &String) -> &Vec<ParsedLine> {
        let lines = code.lines().map(String::from).collect::<Vec<String>>();
        let equ_map = self.make_equ_map(&lines);
        let mut parsed = vec![];
        for (i, line) in lines.iter().enumerate() {
            match self.lex_line(line) {
                LexLineResult::Line(parsed_line) => parsed.push(ParsedLine {
                    parsed: self.apply_equ_to_line(parsed_line, &equ_map),
                    line: line.to_string(),
                    line_index: i,
                }),
                LexLineResult::Multiple(parsed_lines) => {
                    for parsed_line in parsed_lines {
                        parsed.push(ParsedLine {
                            parsed: self.apply_equ_to_line(parsed_line, &equ_map),
                            line: line.to_string(),
                            line_index: i,
                        })
                    }
                }
            }
        }
        self.lines = parsed;
        &self.lines
    }
    fn apply_equ_to_line(&self, line: LexedLine, equ_map: &Vec<(String, String)>) -> LexedLine {
        match line {
            LexedLine::Instruction { name, operands, size } => LexedLine::Instruction {
                name,
                operands: operands
                    .into_iter()
                    .map(|op| self.apply_equ_to_operand(op, equ_map))
                    .collect(),
                size,
            },
            LexedLine::Directive { name, args, size } => LexedLine::Directive {
                name,
                args: args
                    .into_iter()
                    .map(|arg| {
                        self.apply_equ_to_expression_string(arg, equ_map)
                    })
                    .collect(),
                size,
            },
            _ => line,
        }
    }

    fn apply_equ_to_expression_string(&self, mut expression: String, equ_map: &Vec<(String, String)>) -> String {
        for (key, value) in equ_map.iter() {
            expression = expression.replace(key, value);
        }
        expression
    }
    fn apply_equ_to_operand(&self, op: LexedOperand, equ_map: &Vec<(String, String)>) -> LexedOperand {
        match op {
            LexedOperand::Register(_, _)
            | LexedOperand::RegisterRange { .. }
            | LexedOperand::Other(_)
            | LexedOperand::PostIndirect(_)
            | LexedOperand::PreIndirect(_) => op,
            | LexedOperand::Immediate(im) => {
                LexedOperand::Immediate(self.apply_equ_to_expression_string(im, equ_map))
            }
            LexedOperand::Absolute(abs) => {
                let string = self.apply_equ_to_expression_string(abs, equ_map);
                //TODO this is a bit of a hack, after applying the equ, it could change the operand type
                self.parse_operand(&string)
            }
            LexedOperand::Label(label) => {
                let string = self.apply_equ_to_expression_string(label, equ_map);
                self.parse_operand(&string)
            }
            LexedOperand::Indirect(operand) => {
                let operand = self.apply_equ_to_operand(*operand, equ_map);
                LexedOperand::Indirect(Box::new(operand))
            }

            LexedOperand::IndirectDisplacement { offset, operand } => {
                let operand = self.apply_equ_to_operand(*operand, equ_map);
                let offset = self.apply_equ_to_expression_string(offset, equ_map);
                LexedOperand::IndirectDisplacement { offset, operand: Box::new(operand) }
            }
            LexedOperand::IndirectIndex { offset, operands } => {
                let operands = operands
                    .into_iter()
                    .map(|op| self.apply_equ_to_operand(op, equ_map))
                    .collect();
                let offset = self.apply_equ_to_expression_string(offset, equ_map);
                LexedOperand::IndirectIndex { offset, operands }
            }
            LexedOperand::RegisterWithSize(reg, name, size) => {
                LexedOperand::RegisterWithSize(reg, name, size)
            }
        }
    }

    fn lex_line(&mut self, line: &String) -> LexLineResult {
        let line = line.trim();
        let split_at_comments = self.regex.split_at_comment(line);
        let code = split_at_comments[0].trim();
        /*
                let comment = match split_at_comments[..] {
            [_, ..] => split_at_comments[1..].join(&COMMENT_1.to_string()),
            _ => "".to_string(),
        };
         */
        let kind = self.regex.get_line_kind(&code.to_string());
        let args = self.regex.split_at_whitespace(code);
        match kind {
            LineKind::Instruction { size, name } => {
                let operands = self
                    .regex
                    .split_into_separated_args(args[1..].join(" ").as_str(), true);
                let operands = self.parse_operands(operands);
                LexLineResult::Line(LexedLine::Instruction {
                    name,
                    size,
                    operands,
                })
            }
            LineKind::Comment => LexLineResult::Line(LexedLine::Comment {
                content: line.to_string(),
            }),
            LineKind::Label { name, inner } => match &inner {
                Some(inn) => {
                    let mut parsed_inner = match self.lex_line(inn) {
                        LexLineResult::Line(l) => vec![l],
                        LexLineResult::Multiple(l) => l,
                    };
                    parsed_inner.insert(0, LexedLine::Label { name });
                    LexLineResult::Multiple(parsed_inner)
                }
                None => LexLineResult::Line(LexedLine::Label { name }),
            },
            LineKind::Directive => {
                let mut parsed_args: Vec<String> =
                    self.regex.split_into_separated_args(&code.replace("\t", " "), false);
                //lowercase the first arg
                let first = parsed_args.get(0).expect("Missing first argument").to_lowercase();
                parsed_args[0] = first;
                let line = match &parsed_args[..] {
                    [_, equ, ..] if equ.to_lowercase() == "equ" => LexedLine::Directive {
                        name: equ.to_lowercase(),
                        size: LexedSize::Unspecified,
                        args: parsed_args,
                    },
                    [first, ..] => {
                        let (name, size) = self.regex.split_at_size(&first.to_lowercase());
                        LexedLine::Directive {
                            name,
                            size,
                            args: parsed_args,
                        }
                    }
                    _ => LexedLine::Unknown {
                        content: line.to_string(),
                    },
                };
                LexLineResult::Line(line)
            }
            LineKind::Unknown => LexLineResult::Line(LexedLine::Unknown {
                content: line.to_string(),
            }),
            LineKind::Empty => LexLineResult::Line(LexedLine::Empty),
        }
    }
    pub fn get_lines(&self) -> &Vec<ParsedLine> {
        &self.lines
    }
}


fn parse_register_range(range: &str) -> Result<(LexedRegisterType, u32), String>{
    let reg_type = match LexedRegisterType::from_string(range) {
        Ok(reg) => reg,
        Err(e) => return Err(format!("Invalid register range '{}': {}", range, e))
    };
    if reg_type == LexedRegisterType::SP {
        return Ok((reg_type, 0));
    }
    let num = range.chars().nth(1).map(|x| x.to_digit(10)).flatten();
    match num {
        Some(num) => Ok((reg_type, num)),
        None => Err(format!("Invalid register range '{}'", range))
    }
}