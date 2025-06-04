use std::{collections::HashMap, str::FromStr};

use lazy_static::lazy_static;
use regex::Regex;

use crate::instructions::Label;

pub fn num_to_signed_base(num: i64, base: i64) -> Result<i64, &'static str> {
    let bound = 1i64 << (base - 1);
    if num >= bound * 2 || num < -bound {
        return Err("Number out of bounds");
    }
    if num >= bound {
        Ok(-bound * 2 + num)
    } else {
        Ok(num)
    }
}

pub const VALID_ARITHMETICAL_REGEX: &str =
    r"((?:[%@$]*\w+)|(?:'\S*'))((?:\*\*)|[\+\-\*/\^%\|\&\^])?(\S+)?";
pub const VALID_ARITHMETICAL_TOKENS: &str = r"(('.+')|(\*\*|[+\-*\&/^()|])|([%@$]?\w*)|)";
lazy_static! {
    static ref ARITHMETICAL_REGEX: Regex = Regex::new(VALID_ARITHMETICAL_REGEX).unwrap();
    static ref ARITHMETICAL_TOKEN_REGEX: Regex = Regex::new(VALID_ARITHMETICAL_TOKENS).unwrap();
}
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum ArithmeticalOperandToken {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Pow,
    BitAnd,
    BitOr,
    BitXor,
    OpenBracket,
    CloseBracket,
}

impl FromStr for ArithmeticalOperandToken {
    type Err = String;
    fn from_str(s: &str) -> Result<ArithmeticalOperandToken, String> {
        Ok(match s {
            "+" => Self::Add,
            "-" => Self::Sub,
            "*" => Self::Mul,
            "/" => Self::Div,
            "\\" => Self::Mod,
            "**" => Self::Pow,
            "&" => Self::BitAnd,
            "|" => Self::BitOr,
            "^" => Self::BitXor,
            "(" => Self::OpenBracket,
            ")" => Self::CloseBracket,
            _ => return Err(format!("Invalid Operand {}", s)),
        })
    }
}

impl ArithmeticalOperandToken {
    pub fn to_precedence(&self) -> u8 {
        match self {
            Self::OpenBracket | Self::CloseBracket => 0,
            Self::BitAnd => 1,
            Self::BitXor => 2,
            Self::BitOr => 3,
            Self::Add | Self::Sub => 4,
            Self::Mul | Self::Div | Self::Mod => 5,
            Self::Pow => 6,
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq)]
pub enum ArithmeticalToken {
    Number(i64),
    Operand(ArithmeticalOperandToken),
}

pub fn to_reverse_polish_notation(
    tokens: &[ArithmeticalToken],
) -> Result<Vec<ArithmeticalToken>, String> {
    let mut operators: Vec<ArithmeticalOperandToken> = Vec::new();
    let mut result: Vec<ArithmeticalToken> = Vec::new();
    for token in tokens.iter() {
        match token {
            ArithmeticalToken::Number(_) => {
                result.push(*token);
            }
            ArithmeticalToken::Operand(op) => {
                match op {
                    ArithmeticalOperandToken::OpenBracket => operators.push(*op),
                    ArithmeticalOperandToken::CloseBracket => {
                        //adds all operands untill it finds a open bracket
                        while operators.last() != Some(&ArithmeticalOperandToken::OpenBracket) {
                            match operators.pop() {
                                Some(op) => result.push(ArithmeticalToken::Operand(op)),
                                None => {
                                    return Err(
                                        "Invalid expression, could not pop arguments".to_string()
                                    )
                                }
                            }
                        }
                        match operators.pop() {
                            None => {
                                return Err(
                                    "Invalid expression, could not find open bracket".to_string()
                                )
                            }
                            _ => {}
                        }
                    }
                    _ => {
                        while should_unwind(&operators, op) {
                            match operators.pop() {
                                None => {
                                    return Err("Invalid expression, failed to unwind".to_string())
                                }
                                Some(op) => result.push(ArithmeticalToken::Operand(op)),
                            }
                        }
                        operators.push(*op);
                    }
                }
            }
        }
    }
    for op in operators.iter().rev() {
        result.push(ArithmeticalToken::Operand(*op));
    }
    Ok(result)
}

fn should_unwind(operators: &[ArithmeticalOperandToken], next: &ArithmeticalOperandToken) -> bool {
    match operators.last() {
        None => false,
        Some(last) => last.to_precedence() >= next.to_precedence(),
    }
}

fn calculate_rpn(tokens: &[ArithmeticalToken]) -> Result<i64, String> {
    if tokens.is_empty() {
        return Err("Invalid number of arguments for expression, it must not be empty".to_string());
    }
    let mut stack = Vec::new();
    for token in tokens.iter() {
        match token {
            ArithmeticalToken::Number(num) => stack.push(*num),
            ArithmeticalToken::Operand(op) => {
                match (stack.pop(), stack.pop()) {
                    (Some(second), Some(first)) => {
                        let result = match op {
                            ArithmeticalOperandToken::Add => first + second,
                            ArithmeticalOperandToken::Sub => first - second,
                            ArithmeticalOperandToken::Mul => first * second,
                            ArithmeticalOperandToken::Div => first / second,
                            ArithmeticalOperandToken::Mod => first % second,
                            ArithmeticalOperandToken::Pow => first.pow(second as u32),
                            ArithmeticalOperandToken::BitAnd => first & second,
                            ArithmeticalOperandToken::BitOr => first | second,
                            ArithmeticalOperandToken::BitXor => first ^ second,

                            _ => return Err(format!("Invalid operand \"{:?}\"", op)),
                        };
                        stack.push(result)
                    }
                    //to handle the cases like #-1, does not work with 10 - -1
                    (Some(unary), None) => {
                        let result = match op {
                            ArithmeticalOperandToken::Add => unary,
                            ArithmeticalOperandToken::Sub => -unary,
                            _ => return Err(format!("Invalid operand \"{:?}\"", op)),
                        };
                        stack.push(result)
                    }
                    _ => {
                        return Err(format!(
                            "Invalid number of arguments for expression \"{:?}\"",
                            op
                        ))
                    }
                }
            }
        }
    }
    match stack.pop() {
        Some(num) => Ok(num),
        None => Err("Invalid number of arguments for expression".to_string()),
    }
}

pub fn parse_absolute_expression(
    str: &str,
    labels: &HashMap<String, Label>,
) -> Result<i64, String> {
    let tokens: Vec<&str> = ARITHMETICAL_TOKEN_REGEX
        .find_iter(str)
        .map(|m| m.as_str())
        .collect();
    let parsed_tokens = tokens
        .iter()
        .map(|t| match t.parse::<ArithmeticalOperandToken>() {
            Ok(parsed) => Ok(ArithmeticalToken::Operand(parsed)),
            Err(_) => Ok(ArithmeticalToken::Number(parse_absolute(t, labels)? as i64)),
        })
        .collect::<Result<Vec<ArithmeticalToken>, String>>()?;

    let rpn_tokens = to_reverse_polish_notation(&parsed_tokens)?;
    let result = calculate_rpn(&rpn_tokens)?;
    Ok(result)
}

pub fn parse_string_into_padded_bytes(str: &str, chunk_size: usize) -> Vec<u8> {
    //TODO to decide if i should use utf-8 or ascii
    let mut bytes = str.as_bytes().to_vec(); //full utf-8 bytes
                                             //let mut bytes = str.chars().map(|c| c as u8).collect::<Vec<u8>>(); //ascii bytes
                                             //fill space if not a modulo of chunk_size
    let padding = chunk_size - bytes.len() % chunk_size;
    if padding > 0 && padding < chunk_size {
        bytes.resize(bytes.len() + padding, 0);
    }

    bytes
}

pub fn parse_string_into_u32_chunks(str: &str, align_left: bool) -> Vec<u32> {
    let mut chunks = str.as_bytes().chunks_exact(4);
    let mut result: Vec<u32> = chunks
        .by_ref()
        .map(|c| u32::from_be_bytes(c.try_into().unwrap()))
        .collect();
    let rem = chunks.remainder();
    if !rem.is_empty() {
        let mut buf = [0; 4];
        if align_left {
            buf[..rem.len()].copy_from_slice(rem);
        } else {
            buf[4 - rem.len()..].copy_from_slice(rem);
        }
        result.push(u32::from_be_bytes(buf));
    }
    result
}

pub fn parse_absolute(str: &str, labels: &HashMap<String, Label>) -> Result<u32, String> {
    match str.chars().collect::<Vec<char>>()[..] {
        ['%', ..] => match i64::from_str_radix(&str[1..], 2) {
            Ok(n) => Ok(n as u32),
            Err(e) => Err(format!("Invalid binary number: {}, {}", str, e)),
        },
        ['@', ..] => match i64::from_str_radix(&str[1..], 8) {
            Ok(n) => Ok(n as u32),
            Err(e) => Err(format!("Invalid octal number: {}, {}", str, e)),
        },
        ['$', ..] => match i64::from_str_radix(&str[1..], 16) {
            Ok(n) => Ok(n as u32),
            Err(e) => Err(format!("Invalid hexadecimal number: {}, {}", str, e)),
        },
        ['\'', .., '\''] => {
            //parse characters into list of bytes
            let chunks = parse_string_into_u32_chunks(&str[1..str.len() - 1], false);
            if chunks.len() > 1 {
                return Err(format!("String exceedes 32bits: {}", str));
            }
            if chunks.is_empty() {
                return Err(format!("Empty string: {}", str));
            }
            Ok(chunks[0])
        }
        [..] => match labels.get(str) {
            Some(label) => Ok((label.address as i64) as u32),
            None => match str.parse::<i64>() {
                Ok(value) => Ok(value as u32),
                Err(e) => Err(format!(
                    "Invalid decimal number or non existing label: {}, {}",
                    str, e
                )),
            },
        },
    }
}
