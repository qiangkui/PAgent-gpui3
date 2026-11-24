//! A small rotating log and a panic report.
//!
//! The window is a GUI program: a panic normally leaves no trace once the
//! window disappears. This module writes startup errors, banner failures and
//! panics to `logs/paagent.log` beside the settings, so a bug report has
//! something concrete to attach. It writes only after [`init`], so unit tests
//! never touch the user's profile.

use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

/// Largest log kept before it is moved aside.
const MAX_LOG_BYTES: u64 = 2 * 1024 * 1024;
/// Lines the diagnostics page may paste from the end of the log.
const TAIL_LINES: usize = 80;

static LOG: OnceLock<Mutex<PathBuf>> = OnceLock::new();

/// Start the log and install a panic hook. Safe to call more than once.
pub fn init() {
    let dir = crate::config::AiSettings::home().join("logs");
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("paagent.log");
    rotate(&path);
    let _ = LOG.set(Mutex::new(path.clone()));

    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let backtrace = std::backtrace::Backtrace::force_capture();
        log_line("PANIC", &format!("{info}\n{backtrace}"));
        default_hook(info);
    }));
    log_line(
        "INFO",
        &format!("PAgent {} started", env!("CARGO_PKG_VERSION")),
    );
}

/// Append one line to the log. Does nothing before [`init`].
pub fn log_line(level: &str, message: &str) {
    let Some(lock) = LOG.get() else {
        return;
    };
    let Ok(path) = lock.lock() else {
        return;
    };
    rotate(&path);
    let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&*path)
    else {
        return;
    };
    let message = message.replace("\r\n", "\n").replace('\n', "\\n");
    let _ = writeln!(file, "{} [{level}] {message}", stamp());
}

/// Write an error line; kept separate so call sites read as intent.
pub fn error(message: &str) {
    log_line("ERROR", message);
}

/// The log file, for the diagnostics page.
pub fn path() -> Option<PathBuf> {
    LOG.get().and_then(|lock| lock.lock().ok().map(|path| path.clone()))
}

/// The last few lines, bounded so diagnostics stay copyable.
pub fn tail() -> String {
    let Some(path) = path() else {
        return String::new();
    };
    let Ok(body) = std::fs::read_to_string(path) else {
        return String::new();
    };
    let lines: Vec<&str> = body.lines().collect();
    let start = lines.len().saturating_sub(TAIL_LINES);
    let mut text = lines[start..].join("\n");
    if text.chars().count() > 6_000 {
        text = text.chars().rev().take(6_000).collect::<String>().chars().rev().collect();
    }
    text
}

/// Move a log past its limit aside; one old file is enough for this size.
fn rotate(path: &PathBuf) {
    let too_big = std::fs::metadata(path).is_ok_and(|meta| meta.len() > MAX_LOG_BYTES);
    if !too_big {
        return;
    }
    let old = path.with_extension("log.1");
    let _ = std::fs::remove_file(&old);
    let _ = std::fs::rename(path, old);
}

/// `YYYY-MM-DD HH:MM:SS` in UTC, matching the timestamps elsewhere.
fn stamp() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs());
    let (days, rest) = (secs / 86_400, secs % 86_400);
    let (year, month, day) = civil_from_days(days as i64);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02}",
        rest / 3600,
        (rest % 3600) / 60,
        rest % 60
    )
}

/// Days since the Unix epoch → (year, month, day), UTC.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if month <= 2 { year + 1 } else { year }, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_match_the_epoch() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(10_957), (2000, 1, 1));
    }
}