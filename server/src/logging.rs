// Credit for logging config file goes to Cassy343

use chrono::{prelude::*, DateTime, Utc};
use chrono_tz::US::Eastern;
use flate2::{write::GzEncoder, Compression};
use linefeed::{terminal::DefaultTerminal, Interface};
use log::*;
use log4rs::{
    append::{
        rolling_file::{
            policy::compound::{roll::Roll, trigger::size::SizeTrigger, CompoundPolicy},
            RollingFileAppender,
        },
        Append,
    },
    config::{Appender, Config, Root},
    encode::pattern::PatternEncoder,
    filter::{Filter, Response},
};
use std::{
    error::Error,
    fmt,
    fs::{read_dir, remove_file, rename, File},
    io,
    path::Path,
    sync::{Arc, Mutex},
    thread,
};

#[cfg(unix)]
use termion::color;

const FILE_SIZE_LIMIT: u64 = 50_000_000;

#[cfg(debug_assertions)]
const LEVEL_FILTER: LevelFilter = LevelFilter::Debug;
#[cfg(not(debug_assertions))]
const LEVEL_FILTER: LevelFilter = LevelFilter::Info;

// Sets up log4rs customized for the server
pub fn init_logger(
    console_interface: Arc<Interface<DefaultTerminal>>,
) -> Result<(), Box<dyn Error>> {
    // Logs info to the console with colors and such
    let console = CustomConsoleAppender { console_interface };

    // Logs to log files
    let log_file = RollingFileAppender::builder()
        .encoder(Box::new(PatternEncoder::new(
            "[{d(%H:%M:%S)(local)} {l}]: {m}\n",
        )))
        .build(
            "logs/latest.log",
            Box::new(CompoundPolicy::new(
                Box::new(SizeTrigger::new(FILE_SIZE_LIMIT)),
                Box::new(CustomLogRoller::new()),
            )),
        )?;

    // Build the log4rs config
    let config = Config::builder()
        .appender(
            Appender::builder()
                .filter(Box::new(CrateFilter))
                .build("console", Box::new(console)),
        )
        .appender(
            Appender::builder()
                .filter(Box::new(CrateFilter))
                .build("log_file", Box::new(log_file)),
        )
        .build(
            Root::builder()
                .appender("console")
                .appender("log_file")
                .build(LEVEL_FILTER),
        )?;

    log4rs::init_config(config)?;

    Ok(())
}

// Called at the end of main, compresses the last log file
pub fn cleanup() {
    // There's no reason to handle an error here
    let _ = CustomLogRoller::new().roll_threaded(Path::new("./logs/latest.log"), false);
}

#[inline]
fn current_time() -> DateTime<chrono_tz::Tz> {
    Utc::now().with_timezone(&Eastern)
}

// Only allow logging from out crate
struct CrateFilter;

impl Filter for CrateFilter {
    #[cfg(debug_assertions)]
    fn filter(&self, record: &Record) -> Response {
        match record.module_path() {
            Some(path) =>
                if path.starts_with("server") {
                    Response::Accept
                } else {
                    Response::Reject
                },
            None => Response::Reject,
        }
    }

    #[cfg(not(debug_assertions))]
    fn filter(&self, _record: &Record) -> Response {
        Response::Neutral
    }
}

impl fmt::Debug for CrateFilter {
    fn fmt(&self, _f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        Ok(())
    }
}

// Custom implementation for a console logger so that it doesn't mangle the user's commands
struct CustomConsoleAppender {
    console_interface: Arc<Interface<DefaultTerminal>>,
}

impl Append for CustomConsoleAppender {
    #[cfg(unix)]
    fn append(&self, record: &Record) -> anyhow::Result<()> {
        let mut writer = self.console_interface.lock_writer_erase()?;
        match record.metadata().level() {
            Level::Error => write!(writer, "{}", color::Fg(color::Red))?,
            Level::Warn => write!(writer, "{}", color::Fg(color::LightYellow))?,
            Level::Debug => write!(writer, "{}", color::Fg(color::LightCyan))?,
            _ => write!(writer, "{}", color::Fg(color::Reset))?,
        }
        writeln!(
            writer,
            "[{} {}]: {}{}",
            current_time().format("%H:%M:%S"),
            record.metadata().level(),
            record.args(),
            color::Fg(color::Reset)
        )?;
        Ok(())
    }

    #[cfg(not(unix))]
    fn append(&self, record: &Record) -> anyhow::Result<()> {
        let mut writer = self.console_interface.lock_writer_erase()?;
        writeln!(
            writer,
            "[{} {}]: {}",
            current_time().format("%H:%M:%S"),
            record.metadata().level(),
            record.args()
        )?;
        Ok(())
    }

    fn flush(&self) {}
}

impl fmt::Debug for CustomConsoleAppender {
    fn fmt(&self, _f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        Ok(())
    }
}

struct CustomLogRoller {
    name_info: Mutex<(u32, u32)>, // current day, log count for today
}

impl CustomLogRoller {
    pub fn new() -> Self {
        let mut max_index = 0;

        if let Ok(paths) = read_dir("./logs/") {
            let today = format!("{}", current_time().format("%Y-%m-%d"));

            // Find the logs that match today's date and determine the highest index ({date}-{index}.log).
            for path in paths
                .flatten()
                .map(|entry| entry.file_name().into_string())
                .flatten()
                .filter(|name| name.starts_with(&today))
            {
                if let Some(index) = Self::index_from_path(&path) {
                    if index > max_index {
                        max_index = index;
                    }
                }
            }
        }

        CustomLogRoller {
            name_info: Mutex::new((current_time().ordinal(), max_index)),
        }
    }

    fn index_from_path(path: &str) -> Option<u32> {
        let dash_index = path.rfind("-")?;
        let dot_index = path.find(".")?;
        if dash_index + 1 < dot_index {
            path[dash_index + 1 .. dot_index].parse::<u32>().ok()
        } else {
            None
        }
    }

    pub fn roll_threaded(&self, file: &Path, threaded: bool) -> anyhow::Result<()> {
        let mut guard = match self.name_info.lock() {
            Ok(g) => g,

            // Since the mutex is privately managed and errors are handled correctly, this shouldn't be an issue
            Err(_) => unreachable!("Logger mutex poisoned."),
        };

        // Check to make sure the log name info is still accurate
        let local_datetime = current_time();
        if local_datetime.ordinal() != guard.0 {
            guard.0 = local_datetime.ordinal();
            guard.1 = 1;
        } else {
            guard.1 += 1;
        }

        // Rename the file in case it's large and will take a while to compress
        let log = "./logs/latest-tmp.log";
        rename(file, log)?;

        let output = format!(
            "./logs/{}-{}.log.gz",
            local_datetime.format("%Y-%m-%d"),
            guard.1
        );

        if threaded {
            thread::spawn(move || {
                Self::try_compress_log(log, &output);
            });
        } else {
            Self::try_compress_log(log, &output);
        }

        Ok(())
    }

    // Attempts compress_log and prints an error if it fails
    fn try_compress_log(input_path: &str, output_path: &str) {
        if let Err(_) = Self::compress_log(Path::new(input_path), Path::new(output_path)) {
            error!("Failed to compress log file");
        }
    }

    // Takes the source file and compresses it, writing to the output path. Removes the source when done.
    fn compress_log(input_path: &Path, output_path: &Path) -> Result<(), io::Error> {
        let mut input = File::open(input_path)?;
        let mut output = GzEncoder::new(File::create(output_path)?, Compression::default());
        io::copy(&mut input, &mut output)?;
        drop(output.finish()?);
        drop(input); // This needs to occur before file deletion on some OS's
        remove_file(input_path)
    }
}

impl Roll for CustomLogRoller {
    fn roll(&self, file: &Path) -> anyhow::Result<()> {
        self.roll_threaded(file, true)
    }
}

impl fmt::Debug for CustomLogRoller {
    fn fmt(&self, _f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        Ok(())
    }
}
