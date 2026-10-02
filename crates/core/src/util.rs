/// Milliseconds since the Unix epoch.
pub fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

/// Local month and day (1-based), for folder names like `tmp-1002-…`.
pub fn local_month_day() -> (u32, u32) {
    let t = (now_ms() / 1000) as libc::time_t;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    // SAFETY: localtime_r writes into `tm` and is thread-safe.
    unsafe { libc::localtime_r(&t, &mut tm) };
    (tm.tm_mon as u32 + 1, tm.tm_mday as u32)
}
