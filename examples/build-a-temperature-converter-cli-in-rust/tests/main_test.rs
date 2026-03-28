

#[test]
fn converts_0c_to_32f() {
    assert_eq!(convert_temperature(0.0, "c", "f"), 32.0);
}

#[test]
fn converts_32f_to_0c() {
    assert_eq!(convert_temperature(32.0, "f", "c"), 0.0);
}

#[test]
fn returns_input_for_invalid_units() {
    assert_eq!(convert_temperature(100.0, "invalid", "units"), 100.0);
}

#[test]
fn handles_empty_string_input() {
    assert_eq!(convert_temperature(0.0, "", ""), 0.0);
}

#[test]
fn handles_non_numeric_input() {
    assert_eq!(convert_temperature(0.0, "c", "x"), 0.0);
}

#[test]
fn handles_boundary_value_very_high_temp() {
    assert_eq!(convert_temperature(1000000.0, "c", "f"), 1800032.0);
}

#[test]
fn handles_boundary_value_very_low_temp() {
    assert_eq!(convert_temperature(-1000000.0, "c", "f"), -1799968.0);
}

#[test]
fn handles_zero_input() {
    assert_eq!(convert_temperature(0.0, "c", "f"), 32.0);
}

#[test]
fn handles_negative_input() {
    assert_eq!(convert_temperature(-40.0, "c", "f"), -40.0);
}

#[test]
fn handles_unicode_edge_case() {
    assert_eq!(convert_temperature(0.0, "c", "f"), 32.0);
}