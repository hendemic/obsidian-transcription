//! Routing whisper.cpp and ggml logs to stderr.
//!
//! stdout is the sidecar's protocol channel, so no log line may ever reach it.
//! whisper.cpp's default log callback already writes to stderr, but we install
//! whisper-rs's hooks anyway so that routing is explicit, independent of the
//! vendored whisper.cpp version, and the verbosity is controllable.

use std::io::Write;
use std::sync::Once;

use log::{LevelFilter, Log, Metadata, Record};

struct StderrLogger;

impl Log for StderrLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= log::max_level()
    }

    fn log(&self, record: &Record) {
        if self.enabled(record.metadata()) {
            let mut stderr = std::io::stderr().lock();
            let _ = writeln!(stderr, "[{}] {}", record.level(), record.args());
        }
    }

    fn flush(&self) {
        let _ = std::io::stderr().flush();
    }
}

static LOGGER: StderrLogger = StderrLogger;
static INSTALL_HOOKS: Once = Once::new();

/// Redirects whisper.cpp/ggml logging into the `log` facade.
///
/// Called by [`crate::WhisperCpp::new`]. Without a `log` logger installed the
/// messages are dropped, which is still never stdout.
pub(crate) fn install_hooks() {
    INSTALL_HOOKS.call_once(whisper_rs::install_logging_hooks);
}

/// Sends whisper.cpp/ggml logs (and any other `log` records) to stderr at
/// `level` and above.
///
/// If another `log` logger is already installed it is kept and only the level
/// is changed.
pub fn log_to_stderr(level: LevelFilter) {
    let _ = log::set_logger(&LOGGER);
    log::set_max_level(level);
    install_hooks();
}
