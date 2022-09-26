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

pub fn parse_char_or_num(str: &str) -> Result<i64, &'static str> {
    match str.as_bytes()[..] {
        [b'\'', c, b'\''] => Ok(c as i64),
        _ => match str.parse::<i64>() {
            Ok(num) => Ok(num),
            Err(_) => Err("Invalid number"),
        },
    }
}
