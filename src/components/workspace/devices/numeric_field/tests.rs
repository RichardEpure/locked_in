use super::*;

const HEX_16: NumericFormat = NumericFormat::Hexadecimal { width: 4 };
const HEX_8: NumericFormat = NumericFormat::Hexadecimal { width: 2 };

#[test]
fn numeric_values_are_formatted_for_their_display_base() {
    assert_eq!(format_numeric_value(32, NumericFormat::Decimal), "32");
    assert_eq!(format_numeric_value(0, HEX_16), "0000");
    assert_eq!(format_numeric_value(0x4359, HEX_16), "4359");
    assert_eq!(format_numeric_value(0xff60, HEX_16), "FF60");
    assert_eq!(format_numeric_value(0x0a, HEX_8), "0A");
}

#[test]
fn numeric_values_are_parsed_with_their_display_base_and_limit() {
    assert_eq!(
        parse_numeric_value("32", NumericFormat::Decimal, 100),
        Some(32)
    );
    assert_eq!(parse_numeric_value("ff60", HEX_16, u16::MAX), Some(0xff60));
    assert_eq!(
        parse_numeric_value("FFFF", HEX_16, u16::MAX),
        Some(u16::MAX)
    );
    assert_eq!(parse_numeric_value("FF", HEX_8, u8::MAX.into()), Some(255));
    assert_eq!(parse_numeric_value("100", HEX_8, u8::MAX.into()), None);
    assert_eq!(parse_numeric_value("", HEX_16, u16::MAX), None);
    assert_eq!(parse_numeric_value("G", HEX_16, u16::MAX), None);
}

#[test]
fn hexadecimal_input_is_uppercased_and_restricted_to_the_field_width() {
    assert_eq!(sanitize_hex_input("aBcD", 4), "ABCD");
    assert_eq!(sanitize_hex_input("f-g 6!", 4), "F6");
    assert_eq!(sanitize_hex_input("12345", 4), "1234");
}
