use std::time::Duration;

use timeout_parser_benchmark::parse_timeout;

#[test]
fn parses_seconds() {
    assert_eq!(parse_timeout("12s"), Ok(Duration::from_secs(12)));
}

#[test]
fn parses_milliseconds() {
    assert_eq!(parse_timeout("250ms"), Ok(Duration::from_millis(250)));
}

#[test]
fn preserves_large_millisecond_values() {
    assert_eq!(parse_timeout("1500ms"), Ok(Duration::from_millis(1500)));
}

#[test]
fn rejects_invalid_number() {
    assert_eq!(parse_timeout("xs"), Err("invalid timeout value"));
}

#[test]
fn rejects_unsupported_unit() {
    assert_eq!(parse_timeout("10m"), Err("unsupported timeout unit"));
}
