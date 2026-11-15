//! Tiny diagnostics helper.
//!
//! The release build has no console window, so anything that would normally be
//! printed to stderr is appended to `lumen.log` next to the settings file.

use std::io::Write;
use std::path::PathBuf;

/// Full path of the diagnostic log: `<config dir>/Lumen/lumen.log`.
pub fn log_path() -> PathBuf {
    crate::settings::config_dir().join("lumen.log")
}

/// Appends one line to the log file. Never panics - logging must never be able
/// to take the application down.
pub fn log(message: &str) {
    let path = log_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        let _ = writeln!(file, "[{}] {}", timestamp(), message);
    }
}

/// Installs a panic hook that writes the panic message to the log file before
/// the process aborts (the release profile is built with `panic = "abort"`).
pub fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        log(&format!("panic: {info}"));
        previous(info);
    }));
}

/// Routes the `log` records emitted by eframe/winit/glutin into the log file.
///
/// Only enabled when `LUMEN_LOG` is set, so normal runs stay completely silent
/// and pay no logging cost.
pub fn install_logger() {
    if std::env::var_os("LUMEN_LOG").is_none() {
        return;
    }

    struct FileLogger;
    impl log::Log for FileLogger {
        fn enabled(&self, _metadata: &log::Metadata<'_>) -> bool {
            true
        }
        fn log(&self, record: &log::Record<'_>) {
            log(&format!("{} {}: {}", record.level(), record.target(), record.args()));
        }
        fn flush(&self) {}
    }

    static LOGGER: FileLogger = FileLogger;
    let _ = log::set_logger(&LOGGER);
    log::set_max_level(log::LevelFilter::Trace);
}

/// UTC timestamp in `YYYY-MM-DD HH:MM:SSZ` form, without pulling in a date crate.
fn timestamp() -> String {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days = (seconds / 86_400) as i64;
    let rem = seconds % 86_400;
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02}Z",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

/// Converts days since the Unix epoch into a civil date (Howard Hinnant's
/// `civil_from_days` algorithm). Shared with the file-info formatter.
pub fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if month <= 2 { year + 1 } else { year }, month, day)
}
