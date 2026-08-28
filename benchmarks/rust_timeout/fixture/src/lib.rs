use std::time::Duration;

pub fn parse_timeout(value: &str) -> Result<Duration, &'static str> {
    if let Some(seconds) = value.strip_suffix('s') {
        return seconds
            .parse::<u64>()
            .map(Duration::from_secs)
            .map_err(|_| "invalid timeout value");
    }
    if let Some(milliseconds) = value.strip_suffix("ms") {
        return milliseconds
            .parse::<u64>()
            .map(Duration::from_millis)
            .map_err(|_| "invalid timeout value");
    }
    Err("unsupported timeout unit")
}
