/// Milliseconds since the Unix epoch.
pub fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

/// Local month and day (1-based), for folder names like `tmp-1002-…`.
pub fn local_month_day() -> (u32, u32) {
    use chrono::Datelike;
    let now = chrono::Local::now();
    (now.month(), now.day())
}

/// A Unix time in seconds as local "Oct 21 14:05", for messages.
pub fn local_time(secs: i64) -> String {
    use chrono::TimeZone;
    chrono::Local.timestamp_opt(secs, 0).single().map(|t| t.format("%b %-d %H:%M").to_string()).unwrap_or_default()
}
