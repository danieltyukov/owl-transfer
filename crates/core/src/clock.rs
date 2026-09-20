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
