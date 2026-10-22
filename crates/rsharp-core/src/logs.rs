//! The IDE's log: one folder, `logs/` under the state root, `rsharp.log` there, written off a queue and rotated
//! by size. Everything that logs through the `log` crate lands in it, gpui's own warnings included.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::mpsc::{self, Sender};

/// A file grows to this before it is rotated, and this many old ones are kept.
const ROTATE_AT: u64 = 5 * 1024 * 1024;
const KEPT: usize = 3;

struct Sink {
    lines: Mutex<Sender<String>>,
    level: log::LevelFilter,
}

impl log::Log for Sink {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= self.level
    }

    fn log(&self, record: &log::Record) {
        if self.enabled(record.metadata()) {
            let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
            let line =
                format!("{} {} {}: {}\n", utc(now.as_millis() as u64), record.level(), record.target(), record.args());
            let _ = self.lines.lock().unwrap().send(line);
        }
    }

    fn flush(&self) {}
}

/// Starts the log in `folder`; from then on errors, warnings and the IDE's own notes are written there.
/// A second call (a second window, a test) changes nothing.
pub fn start(folder: &Path) {
    let (lines, queue) = mpsc::channel::<String>();
    let sink = Sink { lines: Mutex::new(lines), level: log::LevelFilter::Info };
    if log::set_boxed_logger(Box::new(sink)).is_err() {
        return;
    }
    log::set_max_level(log::LevelFilter::Info);
    let file = folder.join("rsharp.log");
    let _ = std::fs::create_dir_all(folder);
    std::thread::Builder::new()
        .name("log-writer".into())
        .spawn(move || {
            for line in queue {
                if std::fs::metadata(&file).is_ok_and(|m| m.len() > ROTATE_AT) {
                    rotate(&file);
                }
                if let Ok(mut out) = std::fs::OpenOptions::new().create(true).append(true).open(&file) {
                    let _ = out.write_all(line.as_bytes());
                }
            }
        })
        .expect("the log writer starts");
}

/// A time as `2026-10-22 13:41:07.123` in UTC, from milliseconds since 1970 (Howard Hinnant's civil_from_days).
fn utc(millis: u64) -> String {
    let (days, day_millis) = ((millis / 86_400_000) as i64, millis % 86_400_000);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    let (h, m, s, ms) = (day_millis / 3_600_000, day_millis / 60_000 % 60, day_millis / 1000 % 60, day_millis % 1000);
    format!("{year:04}-{month:02}-{day:02} {h:02}:{m:02}:{s:02}.{ms:03}")
}

/// `rsharp.log` becomes `rsharp.1.log`, which becomes `rsharp.2.log`, and the oldest goes.
fn rotate(file: &Path) {
    let numbered = |n: usize| -> PathBuf { file.with_extension(format!("{n}.log")) };
    let _ = std::fs::remove_file(numbered(KEPT));
    for n in (1..KEPT).rev() {
        let _ = std::fs::rename(numbered(n), numbered(n + 1));
    }
    let _ = std::fs::rename(file, numbered(1));
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_log_line_says_when_in_a_date_a_person_reads() {
        assert_eq!(super::utc(0), "1970-01-01 00:00:00.000");
        assert_eq!(super::utc(1_792_670_754_123), "2026-10-22 12:05:54.123");
        assert_eq!(super::utc(951_782_400_000), "2000-02-29 00:00:00.000", "a leap day");
    }
}
