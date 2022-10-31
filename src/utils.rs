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
lazy_static!{
    static ref ARITHMETICAL_REGEX: Regex = Regex::new(r"([%@$]*\w+)([\+\-\*/])*(\S+)*").unwrap();
}
pub fn parse_absolute_expression(str: &str, labels: &HashMap<String, Label>) -> Result<u32, String> {
    match ARITHMETICAL_REGEX.captures(str){
        Some(groups) => {
            let l = groups.get(1);
            let op = groups.get(2);
            let r = groups.get(3);
            if l.is_some() && op.is_some() && r.is_some(){
                let l = parse_absolute_expression(l.unwrap().as_str(), labels)?;
                let r = parse_absolute_expression(r.unwrap().as_str(), labels)?;
                let op = op.unwrap().as_str();
                return Ok(match op {
                    "+" => l + r,
                    "-" => l - r,
                    "*" => l * r,
                    "/" => l / r,
                    _ => return Err(format!("Invalid operator: {}", op))
                } as u32)
            }
            if l.is_some() && op.is_none() && r.is_none(){
                return Ok(parse_absolute(str, labels)?)
            }
            Err(format!("Invalid expression: {}", str))
        }
        None => parse_absolute(str, labels)
    }
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
        ['\'', c, '\''] => Ok(c as u32),
        [..] => match labels.get(str) {
            Some(label) => Ok((label.address as i32) as u32),
            None => match i32::from_str_radix(str, 10) {
                Ok(value) => Ok(value as u32),
                Err(_) => Err(format!("Invalid decimal number or non existing label: {}", str)),
            },
        },
    }
}
