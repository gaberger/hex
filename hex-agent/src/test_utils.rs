use std::str::FromStr;

pub fn is_valid_hex(input: &str) -> bool {
    input.chars().all(|c| c.is_digit(16))
}

pub fn parse_hex_to_u32(hex_str: &str) -> Result<u32, std::num::ParseIntError> {
    u32::from_str_radix(hex_str, 16)
}