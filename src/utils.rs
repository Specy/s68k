use std::collections::HashMap;

use lazy_static::lazy_static;
use regex::Regex;

use crate::semantic_checker::Label;

pub fn num_to_signed_base(num: i64, base: i64) -> Result<i64, &'static str> {
    let bound = 1i64 << base - 1;
    if num >= bound * 2 || num < -bound {
        return Err("Number out of bounds");
    }
    if num >= bound {
        Ok(-bound * 2 + num)
    } else {
        Ok(num)
    }
}
pub const VALID_ARITHMETICAL_REGEX: &str =  r"((?:[%@$]*\w+)|(?:'\S*'))((?:\*\*)|[\+\-\*/\^%\|\&\^])?(\S+)?";
lazy_static! {
    static ref ARITHMETICAL_REGEX: Regex =
        Regex::new(VALID_ARITHMETICAL_REGEX).unwrap();
}
pub fn parse_absolute_expression(
    str: &str,
    labels: &HashMap<String, Label>,
) -> Result<i64, String> {
    match ARITHMETICAL_REGEX.captures(str) {
        Some(groups) => {
            let l = groups.get(1);
            let op = groups.get(2);
            let r = groups.get(3);
            if l.is_some() && op.is_some() && r.is_some() {
                let l = parse_absolute_expression(l.unwrap().as_str(), labels)?;
                let r = parse_absolute_expression(r.unwrap().as_str(), labels)?;
                let op = op.unwrap().as_str();
                return Ok(match op {
                    "+" => l + r,
                    "-" => l - r,
                    "*" => l * r,
                    "/" => l / r,
                    "**" => l.pow(r as u32),
                    "%" => l % r,
                    "&" => l & r,
                    //🚗
                    "|" => l | r,
                    "^" => l ^ r,
                    _ => return Err(format!("Invalid operator: {}", op)),
                } as i64);
            }
            if l.is_some() && op.is_none() && r.is_none() {
                return Ok(parse_absolute(str, labels)? as i64);
            }
            Err(format!("Invalid expression: {}", str))
        }
        None => {
            let value = parse_absolute(str, labels)?;
            Ok(value as i64)
        },
    }
}

pub fn parse_string_into_padded_bytes(str: &str, chunk_size: usize) -> Vec<u8> {
    //TODO to decide if i should use utf-8 or ascii
    let mut bytes = str.as_bytes().to_vec();
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
        ['%', ..] => match i32::from_str_radix(&str[1..], 2) {
            Ok(n) => Ok(n as u32),
            Err(_) => Err(format!("Invalid binary number: {}", str)),
        },
        ['@', ..] => match i32::from_str_radix(&str[1..], 8) {
            Ok(n) => Ok(n as u32),
            Err(_) => Err(format!("Invalid octal number: {}", str)),
        },
        ['$', ..] => match i32::from_str_radix(&str[1..], 16) {
            Ok(n) => Ok(n as u32),
            Err(_) => Err(format!("Invalid hexadecimal number: {}", str)),
        },
        ['\'', .., '\''] => {
            //parse characters into list of bytes
            let chunks = parse_string_into_u32_chunks(&str[1..str.len() - 1], false);
            if chunks.len() > 1 {
                return Err(format!("String exceedes 32bits: {}", str));
            }
            if chunks.len() == 0 {
                return Err(format!("Empty string: {}", str));
            }
            Ok(chunks[0])
        }
        [..] => match labels.get(str) {
            Some(label) => Ok((label.address as i32) as u32),
            None => match i32::from_str_radix(str, 10) {
                Ok(value) => Ok(value as u32),
                Err(_) => Err(format!(
                    "Invalid decimal number or non existing label: {}",
                    str
                )),
            },
        },
    }
}
