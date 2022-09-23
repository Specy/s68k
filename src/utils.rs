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
