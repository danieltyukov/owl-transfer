//! Wall clock helpers. Everything the engine stores is in milliseconds since
//! the Unix epoch so the index, the wire format and the interface agree.

use std::time::{SystemTime, UNIX_EPOCH};

/// The current wall clock in milliseconds since the Unix epoch.
pub fn now_ms() -> i64 {
    system_time_ms(SystemTime::now())
}

/// A `SystemTime` as milliseconds since the Unix epoch, floored.
pub fn system_time_ms(t: SystemTime) -> i64 {
    match t.duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_millis() as i64,
        Err(e) => -(e.duration().as_millis() as i64),
    }
}

/// A file's modification time in milliseconds, or zero if the platform
/// cannot say.
pub fn mtime_ms(md: &std::fs::Metadata) -> i64 {
    md.modified().map(system_time_ms).unwrap_or(0)
}

/// Sets a file's modification time from milliseconds since the epoch.
pub fn set_mtime_ms(path: &std::path::Path, ms: i64) -> std::io::Result<()> {
    let secs = ms.div_euclid(1000);
    let nanos = (ms.rem_euclid(1000) * 1_000_000) as u32;
    filetime::set_file_mtime(path, filetime::FileTime::from_unix_time(secs, nanos))
}
