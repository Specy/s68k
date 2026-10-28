use crate::instructions::{ShiftDirection, Size};

fn get_sign_mask(value: u32, size: Size) -> u32 {
    match size {
        Size::Byte => value & 0x00000080,
        Size::Word => value & 0x00008000,
        Size::Long => value & 0x80000000,
    }
}

/// Every bit of a value of that width, as the widest of the three.
fn size_mask(size: Size) -> u64 {
    match size {
        Size::Byte => 0xff,
        Size::Word => 0xffff,
        Size::Long => 0xffff_ffff,
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

/// `op1 + op2 + extend` at `size`, with the carry out of the most significant
/// bit: the arithmetic of `addx` (`Reference/68ks5e.htm`).
///
/// The sum is worked out in 64 bits and cut down afterwards, so the carry is
/// the one carry the 68000 reports and never two of them.
pub fn add_with_extend(op1: u32, op2: u32, extend: bool, size: Size) -> (u32, bool) {
    let mask = size_mask(size);
    let sum = (op1 as u64 & mask) + (op2 as u64 & mask) + u64::from(extend);
    ((sum & mask) as u32, sum > mask)
}

/// `op1 - op2 - extend` at `size`, with the borrow out of the most significant
/// bit: the arithmetic of `subx` and, from a destination of zero, of `negx`
/// (`Reference/68ks5v.htm`, `Reference/68ks5q.htm`).
pub fn sub_with_extend(op1: u32, op2: u32, extend: bool, size: Size) -> (u32, bool) {
    let mask = size_mask(size);
    let difference = (op1 as i64 & mask as i64) - (op2 as i64 & mask as i64) - i64::from(extend);
    ((difference as u64 & mask) as u32, difference < 0)
}

/// One byte of binary coded decimal plus another and the extend flag, with the
/// decimal carry out: the arithmetic of `abcd` (`Reference/68ks8e.htm`).
///
/// This is the 68000's own correction, which differs from adding digit by
/// digit once a digit is above 9. The bytes are added in binary first. Each
/// nibble that carried out in binary, or that came to 10 or more, is corrected
/// by 6, and the carry out is the binary carry of the high nibble or a carry
/// out of the correction. Two well formed BCD bytes give the decimal answer;
/// any other byte gives what the processor gives, which SingleStepTests/m68000
/// records and these lines match on every test of its `ABCD` file.
pub fn add_decimal(destination: u32, source: u32, extend: bool) -> (u32, bool) {
    let (destination, source) = ((destination & 0xff) as i32, (source & 0xff) as i32);
    let sum = destination + source + i32::from(extend);
    //the carry out of each nibble's binary addition, at bits 3 and 7
    let carries = ((destination & source) | (!sum & destination) | (!sum & source)) & 0x88;
    //each nibble that came to 10 or more, at bits 3 and 7
    let tens = (((sum + 0x66) ^ sum) & 0x110) >> 1;
    let corrections = carries | tens;
    let result = sum + (corrections - (corrections >> 2));
    let carry = ((carries | (sum & !result)) >> 7) & 1 != 0;
    ((result & 0xff) as u32, carry)
}

/// One byte of binary coded decimal less another and the extend flag, with the
/// decimal borrow out: the arithmetic of `sbcd`, and of `nbcd` from a
/// destination of zero (`Reference/68ks8g.htm`, `Reference/68ks8f.htm`).
///
/// The mirror of [`add_decimal`]: the bytes are subtracted in binary, each
/// nibble that borrowed is corrected by 6, and the borrow out is the binary
/// borrow of the high nibble or one out of the correction. It matches every
/// test of the suite's `SBCD` and `NBCD` files.
pub fn subtract_decimal(destination: u32, source: u32, extend: bool) -> (u32, bool) {
    let (destination, source) = ((destination & 0xff) as i32, (source & 0xff) as i32);
    let difference = destination - source - i32::from(extend);
    //the borrow out of each nibble's binary subtraction, at bits 3 and 7
    let borrows =
        ((!destination & source) | (difference & !destination) | (difference & source)) & 0x88;
    let result = difference - (borrows - (borrows >> 2));
    let borrow = ((borrows | (!difference & result)) >> 7) & 1 != 0;
    ((result & 0xff) as u32, borrow)
}

/// One place of a rotation through the extend flag, which answers the rotated
/// value and the bit that came out of it — the new extend and carry
/// (`Reference/68ks7g.htm`, `Reference/68ks7h.htm`).
///
/// The rotation is 9, 17 or 33 bits wide: the bit that leaves the operand goes
/// to the extend flag, and the bit the extend flag held comes in at the other
/// end.
pub fn rotate_with_extend(
    dir: &ShiftDirection,
    value: u32,
    size: Size,
    extend: bool,
) -> (u32, bool) {
    let value = get_value_sized(value, size);
    match dir {
        ShiftDirection::Left => {
            let out = get_sign(value, size);
            let rotated = get_value_sized((value << 1) | u32::from(extend), size);
            (rotated, out)
        }
        ShiftDirection::Right => {
            let out = (value & 0x1) != 0;
            let top = u32::from(extend) << (size.to_bits() - 1);
            ((value >> 1) | top, out)
        }
    }
}
