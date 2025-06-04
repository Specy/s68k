use crate::instructions::{ShiftDirection, Size};

fn get_sign_mask(value: u32, size: Size) -> u32 {
    match size {
        Size::Byte => value & 0x00000080,
        Size::Word => value & 0x00008000,
        Size::Long => value & 0x80000000,
    }
}

pub fn get_sign(value: u32, size: Size) -> bool {
    let mask = get_sign_mask(value, size);
    (value & mask) != 0
}

pub fn overflowing_add_sized(op1: u32, op2: u32, size: Size) -> (u32, bool) {
    match size {
        Size::Byte => {
            let (result, carry) = (op1 as u8).overflowing_add(op2 as u8);
            (result as u32, carry)
        }
        Size::Word => {
            let (result, carry) = (op1 as u16).overflowing_add(op2 as u16);
            (result as u32, carry)
        }
        Size::Long => op1.overflowing_add(op2),
    }
}

pub fn overflowing_sub_sized(op1: u32, op2: u32, size: Size) -> (u32, bool) {
    match size {
        Size::Byte => {
            let (result, carry) = (op1 as u8).overflowing_sub(op2 as u8);
            (result as u32, carry)
        }
        Size::Word => {
            let (result, carry) = (op1 as u16).overflowing_sub(op2 as u16);
            (result as u32, carry)
        }
        Size::Long => op1.overflowing_sub(op2),
    }
}

pub fn overflowing_sub_signed_sized(op1: u32, op2: u32, size: Size) -> (u32, bool) {
    match size {
        Size::Byte => {
            let (result, overflow) = (op1 as i8).overflowing_sub(op2 as i8);
            (result as u32, overflow)
        }
        Size::Word => {
            let (result, overflow) = (op1 as i16).overflowing_sub(op2 as i16);
            (result as u32, overflow)
        }
        Size::Long => {
            let (result, overflow) = (op1 as i32).overflowing_sub(op2 as i32);
            (result as u32, overflow)
        }
    }
}

pub fn sign_extend_to_long(value: u32, from: Size) -> i32 {
    match from {
        Size::Byte => ((value as u8) as i8) as i32,
        Size::Word => ((value as u16) as i16) as i32,
        Size::Long => value as i32,
    }
}

pub fn get_value_sized(value: u32, size: Size) -> u32 {
    match size {
        Size::Byte => 0x000000FF & value,
        Size::Word => 0x0000FFFF & value,
        Size::Long => value,
    }
}

pub fn has_add_overflowed(op1: u32, op2: u32, result: u32, size: Size) -> bool {
    let s1 = get_sign(op1, size);
    let s2 = get_sign(op2, size);
    let result_sign = get_sign(result, size);
    (s1 && s2 && !result_sign) || (!s1 && !s2 && result_sign)
}

pub fn has_sub_overflowed(op1: u32, op2: u32, result: u32, size: Size) -> bool {
    let s1 = get_sign(op1, size);
    let s2 = !get_sign(op2, size);
    let result_sign = get_sign(result, size);

    (s1 && s2 && !result_sign) || (!s1 && !s2 && result_sign)
}

pub fn shift(dir: &ShiftDirection, value: u32, size: Size, is_arithmetic: bool) -> (u32, bool) {
    match dir {
        ShiftDirection::Left => {
            let bit = get_sign(value, size);
            let shift = match size {
                Size::Byte => ((value as u8) << 1) as u32,
                Size::Word => ((value as u16) << 1) as u32,
                Size::Long => value << 1,
            };
            (shift, bit)
        }
        ShiftDirection::Right => {
            let mask = if is_arithmetic {
                get_sign_mask(value, size)
            } else {
                0
            };
            ((value >> 1) | mask, (value & 0x1) != 0)
        }
    }
}

pub fn rotate(dir: &ShiftDirection, value: u32, size: Size) -> (u32, bool) {
    match dir {
        ShiftDirection::Left => {
            let bit = get_sign(value, size);
            let mask = bit as u32;
            let rotate = match size {
                Size::Byte => ((value as u8) << 1) as u32,
                Size::Word => ((value as u16) << 1) as u32,
                Size::Long => value << 1,
            };
            ((mask | rotate), bit)
        }
        ShiftDirection::Right => {
            let bit = (value & 0x01) != 0;
            let mask = if bit {
                get_sign_mask(0xffffffff, size)
            } else {
                0x0
            };
            ((value >> 1) | mask, bit)
        }
    }
}
