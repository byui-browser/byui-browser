//! Leveled, per-crate logging selected from the command line.
//!
//! Every crate logs through the [`log`] facade macros (`log::error!`,
//! `log::warn!`, `log::info!`, `log::debug!`, `log::trace!`). This module
//! owns the one logger that prints those records, and parses the command-line
//! options that choose how much of it is shown.
//!
//! # Levels
//!
//! Pick the level by asking "who needs to see this, and how often will it
//! print?":
//!
//! | Level   | Use it for | Example |
//! | ------- | ---------- | ------- |
//! | `error` | An operation failed and the user will notice: a page did not load, storage could not open, a script threw. | `Failed to load https://example.com: connection refused` |
//! | `warn`  | Something went wrong but the browser recovered or degraded: a subresource was skipped, a font fell back, a request was blocked by policy, an unknown CSS property was dropped. | `No page font could be loaded; text will not render` |
//! | `info`  | One line per major user-visible milestone: startup, window creation, navigation start and finish, storage opened. Readable by a non-developer. | `Navigating to byui://welcome` |
//! | `debug` | Per-subsystem detail a developer needs to follow one operation: request and response summaries, cache hits, parse and layout summaries, script evaluation start and end. | `GET https://example.com -> 200 (1532 bytes)` |
//! | `trace` | Very high-volume detail that would drown everything else: individual tokens, DOM nodes, selector matches, bytecode instructions, every repaint and input event. | `token StartTag(<p>)` |
//!
//! The default level is `warn`, so a normal run prints only problems.
//!
//! # Workspace crates and dependencies
//!
//! A bare level (`--log-level debug`, `-v`) applies to this workspace's
//! crates, listed in [`WORKSPACE_CRATES`]. Third-party dependencies that also
//! log through `log` (winit, reqwest, hyper, rustls, ...) stay at
//! [`DEFAULT_DEPENDENCY_LEVEL`] so they do not drown out browser output. Raise
//! them with the `deps=<level>` directive, or one at a time by name
//! (`hyper=debug`). **When you add a workspace crate, add its name to
//! [`WORKSPACE_CRATES`]**; a test fails until you do.
//!
//! # Command-line options
//!
//! See [`USAGE`] for the help text. In short:
//!
//! - `--log-level <spec>` (or `--log-level=<spec>`) sets the filter. A spec is
//!   a comma-separated list of `level` and `target=level` directives, such as
//!   `debug` or `warn,net=debug,js::vm=trace`. A target is a crate name
//!   (`net`, `browser-shell`) or a module path inside one (`html::tokenizer`);
//!   the longest matching target wins. The special target `deps` sets the
//!   level for every third-party crate.
//! - `-v`, `-vv`, `-vvv` raise the default level to `info`, `debug`, `trace`.
//! - `-q` / `--quiet` lowers the default level to `error`; `-qq` turns
//!   logging off.
//! - `--log-file <path>` appends log lines to a file instead of standard
//!   error, which is useful on Windows where the GUI has no console.
//! - When no level option is given, the `BYUI_LOG` environment variable is
//!   read as a spec instead.
//!
//! # Limitations
//!
//! Lines are written synchronously as they are logged; there is no
//! background writer, colorized output, or log rotation.

use std::fmt;
use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Instant;

pub use log::LevelFilter;

/// Environment variable read as a filter spec when the command line sets no
/// level option. Uses the same syntax as `--log-level`.
pub const LOG_ENV_VAR: &str = "BYUI_LOG";

/// Level used when neither the command line nor [`LOG_ENV_VAR`] chooses one.
pub const DEFAULT_LEVEL: LevelFilter = LevelFilter::Warn;

/// Level for third-party crates' records unless a `deps=<level>` or
/// crate-specific directive says otherwise.
pub const DEFAULT_DEPENDENCY_LEVEL: LevelFilter = LevelFilter::Warn;

/// Directive target that sets the level for all third-party crates, as in
/// `--log-level debug,deps=info`.
pub const DEPENDENCIES_TARGET: &str = "deps";

/// Log targets (crate names with `-` written as `_`) of this workspace's own
/// crates. A bare level applies to these; every other target is a
/// dependency. Keep in sync with the workspace `members` in `Cargo.toml`.
pub const WORKSPACE_CRATES: &[&str] = &[
    "browser",
    "browser_js",
    "browser_linux",
    "browser_macos",
    "browser_shell",
    "byui_browser_windows",
    "chrome",
    "common",
    "css",
    "devtools",
    "html",
    "js",
    "layout",
    "net",
    "paint",
    "render",
    "security",
    "storage",
    "webapis",
];

/// Help text for the logging command-line options, ready to print under a
/// program's own usage line.
pub const USAGE: &str = "\
Logging options:
  --log-level <spec>   Set which messages are logged. <spec> is a level
                       (off, error, warn, info, debug, trace) or a
                       comma-separated list such as `warn,net=debug,js=trace`
                       that sets levels per crate or module. Default: warn.
                       A bare level covers the browser's own crates;
                       third-party crates stay at warn unless you add
                       `deps=<level>` or name them (e.g. `hyper=debug`).
  -v, -vv, -vvv        Log at info, debug, or trace level.
  -q, -qq              Log only errors, or nothing at all.
  --log-file <path>    Append log lines to <path> instead of standard error.
  -h, --help           Print this help and exit.

  When no level option is given, the BYUI_LOG environment variable is read
  as a <spec> instead.";

/// A per-target level override such as `net=debug`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetLevel {
    /// Crate name or `::`-separated module path, with `-` normalized to `_`
    /// so `browser-shell` matches the `browser_shell` crate's records.
    pub target: String,
    /// Most verbose level logged for records from this target.
    pub level: LevelFilter,
}

/// Which records are logged and where they go.
///
/// Build one with [`LogConfig::from_args`] (command line plus environment)
/// or [`LogConfig::parse_spec`] (a spec string alone), then pass it to
/// [`init`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogConfig {
    /// Level for records from [`WORKSPACE_CRATES`] whose target matches no
    /// entry in `targets`.
    pub default_level: LevelFilter,
    /// Level for records from third-party crates whose target matches no
    /// entry in `targets`. Set with the `deps=<level>` directive.
    pub dependency_level: LevelFilter,
    /// Per-target overrides; the longest matching target wins.
    pub targets: Vec<TargetLevel>,
    /// File to append log lines to; `None` writes to standard error.
    pub file: Option<PathBuf>,
    /// Whether the command line asked for help (`-h` / `--help`). Callers
    /// should print their usage plus [`USAGE`] and exit.
    pub help_requested: bool,
}

impl Default for LogConfig {
    fn default() -> Self {
        Self {
            default_level: DEFAULT_LEVEL,
            dependency_level: DEFAULT_DEPENDENCY_LEVEL,
            targets: Vec::new(),
            file: None,
            help_requested: false,
        }
    }
}

/// A logging option on the command line or in [`LOG_ENV_VAR`] was malformed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LogConfigError {
    /// A level name was not one of `off`, `error`, `warn`, `info`, `debug`,
    /// `trace`. Holds the rejected text.
    UnknownLevel(String),
    /// A `target=level` directive had an empty target. Holds the directive.
    EmptyTarget(String),
    /// An option that takes a value (such as `--log-level`) was last on the
    /// command line. Holds the option name.
    MissingValue(&'static str),
}

impl fmt::Display for LogConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownLevel(level) => write!(
                f,
                "unknown log level `{level}` (expected off, error, warn, info, debug, or trace)"
            ),
            Self::EmptyTarget(directive) => {
                write!(f, "log directive `{directive}` has an empty target")
            }
            Self::MissingValue(option) => write!(f, "{option} requires a value"),
        }
    }
}

impl std::error::Error for LogConfigError {}

impl LogConfig {
    /// Parses a filter spec such as `debug` or `warn,net=debug,js::vm=trace`.
    ///
    /// A bare level sets [`LogConfig::default_level`]; `target=level` adds a
    /// [`TargetLevel`]. Whitespace around directives is ignored, as are empty
    /// directives, so `""` yields the default configuration.
    pub fn parse_spec(spec: &str) -> Result<Self, LogConfigError> {
        let mut config = Self::default();
        config.apply_spec(spec)?;
        Ok(config)
    }

    /// Builds a configuration from command-line arguments (without the
    /// program name) and [`LOG_ENV_VAR`].
    ///
    /// Returns the configuration and the arguments that were not logging
    /// options, in their original order, so the caller can handle its own.
    /// Precedence is: the command line, then [`LOG_ENV_VAR`], then
    /// [`DEFAULT_LEVEL`]. When several level options are given the last one
    /// wins.
    pub fn from_args<I, S>(args: I) -> Result<(Self, Vec<String>), LogConfigError>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self::from_args_and_env(args, std::env::var(LOG_ENV_VAR).ok().as_deref())
    }

    /// Like [`LogConfig::from_args`], but takes the [`LOG_ENV_VAR`] value
    /// explicitly so tests do not depend on the process environment.
    pub fn from_args_and_env<I, S>(
        args: I,
        env_spec: Option<&str>,
    ) -> Result<(Self, Vec<String>), LogConfigError>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let mut config = Self::default();
        let mut level_from_args = false;
        let mut remaining = Vec::new();
        let mut args = args.into_iter().map(Into::into);

        while let Some(arg) = args.next() {
            match arg.as_str() {
                "-h" | "--help" => config.help_requested = true,
                "--log-level" => {
                    let spec = args
                        .next()
                        .ok_or(LogConfigError::MissingValue("--log-level"))?;
                    config.reset_levels();
                    config.apply_spec(&spec)?;
                    level_from_args = true;
                }
                "--log-file" => {
                    let path = args
                        .next()
                        .ok_or(LogConfigError::MissingValue("--log-file"))?;
                    config.file = Some(PathBuf::from(path));
                }
                "--verbose" => {
                    config.default_level = verbosity_level(1);
                    level_from_args = true;
                }
                "--quiet" => {
                    config.default_level = LevelFilter::Error;
                    level_from_args = true;
                }
                _ => {
                    if let Some(spec) = arg.strip_prefix("--log-level=") {
                        config.reset_levels();
                        config.apply_spec(spec)?;
                        level_from_args = true;
                    } else if let Some(path) = arg.strip_prefix("--log-file=") {
                        config.file = Some(PathBuf::from(path));
                    } else if let Some(count) = repeated_flag(&arg, 'v') {
                        config.default_level = verbosity_level(count);
                        level_from_args = true;
                    } else if let Some(count) = repeated_flag(&arg, 'q') {
                        config.default_level = if count == 1 {
                            LevelFilter::Error
                        } else {
                            LevelFilter::Off
                        };
                        level_from_args = true;
                    } else {
                        remaining.push(arg);
                    }
                }
            }
        }

        if let Some(spec) = env_spec.filter(|_| !level_from_args) {
            config.apply_spec(spec)?;
        }
        Ok((config, remaining))
    }

    /// Returns the most verbose level logged for records from `target`
    /// (a module path such as `net::fetch`).
    pub fn level_for(&self, target: &str) -> LevelFilter {
        self.targets
            .iter()
            .filter(|entry| target_matches(&entry.target, target))
            .max_by_key(|entry| entry.target.len())
            .map_or_else(
                || {
                    if is_workspace_target(target) {
                        self.default_level
                    } else {
                        self.dependency_level
                    }
                },
                |entry| entry.level,
            )
    }

    /// Returns the most verbose level any target can log, which callers use
    /// as the global [`log::max_level`] so filtered-out macros stay cheap.
    pub fn max_level(&self) -> LevelFilter {
        self.targets
            .iter()
            .map(|entry| entry.level)
            .fold(self.default_level.max(self.dependency_level), Ord::max)
    }

    fn reset_levels(&mut self) {
        self.default_level = DEFAULT_LEVEL;
        self.dependency_level = DEFAULT_DEPENDENCY_LEVEL;
        self.targets.clear();
    }

    fn apply_spec(&mut self, spec: &str) -> Result<(), LogConfigError> {
        for directive in spec.split(',').map(str::trim).filter(|d| !d.is_empty()) {
            match directive.split_once('=') {
                Some((target, level)) => {
                    let target = target.trim().replace('-', "_");
                    if target.is_empty() {
                        return Err(LogConfigError::EmptyTarget(directive.to_owned()));
                    }
                    let level = parse_level(level.trim())?;
                    if target == DEPENDENCIES_TARGET {
                        self.dependency_level = level;
                        continue;
                    }
                    self.targets.retain(|entry| entry.target != target);
                    self.targets.push(TargetLevel { target, level });
                }
                None => self.default_level = parse_level(directive)?,
            }
        }
        Ok(())
    }
}

/// Parses a level name, ignoring ASCII case.
pub fn parse_level(level: &str) -> Result<LevelFilter, LogConfigError> {
    level
        .parse()
        .map_err(|_| LogConfigError::UnknownLevel(level.to_owned()))
}

/// Maps a `-v` count to a level: 1 is `info`, 2 is `debug`, 3+ is `trace`.
fn verbosity_level(count: usize) -> LevelFilter {
    match count {
        0 => DEFAULT_LEVEL,
        1 => LevelFilter::Info,
        2 => LevelFilter::Debug,
        _ => LevelFilter::Trace,
    }
}

/// Returns how many times `flag` repeats in a short option like `-vvv`, or
/// `None` when `arg` is anything else.
fn repeated_flag(arg: &str, flag: char) -> Option<usize> {
    let letters = arg.strip_prefix('-')?;
    (!letters.is_empty() && letters.chars().all(|c| c == flag)).then_some(letters.len())
}

/// Whether a record target belongs to one of [`WORKSPACE_CRATES`].
fn is_workspace_target(target: &str) -> bool {
    let crate_name = target.split("::").next().unwrap_or(target);
    WORKSPACE_CRATES.contains(&crate_name)
}

/// Whether directive `filter` covers record target `target`: an exact match
/// or a `::`-separated prefix of it.
fn target_matches(filter: &str, target: &str) -> bool {
    target
        .strip_prefix(filter)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with("::"))
}

/// An error installing the logger.
#[derive(Debug)]
pub enum LogInitError {
    /// The `--log-file` path could not be opened for appending.
    OpenFile {
        /// The path that failed to open.
        path: PathBuf,
        /// Why it failed.
        source: io::Error,
    },
    /// Another logger was already installed in this process.
    AlreadyInitialized,
}

impl fmt::Display for LogInitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OpenFile { path, source } => {
                write!(f, "cannot open log file {}: {source}", path.display())
            }
            Self::AlreadyInitialized => f.write_str("a logger is already installed"),
        }
    }
}

impl std::error::Error for LogInitError {}

/// Installs the process-wide logger for `config`.
///
/// Call this once, as early as possible in `main`. Records logged before it
/// runs are dropped. Each line looks like
/// `[   0.042s DEBUG net::fetch] GET https://example.com -> 200`, where the
/// time is seconds since `init` ran.
pub fn init(config: LogConfig) -> Result<(), LogInitError> {
    let sink = match &config.file {
        Some(path) => Sink::File(Mutex::new(
            OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .map_err(|source| LogInitError::OpenFile {
                    path: path.clone(),
                    source,
                })?,
        )),
        None => Sink::Stderr,
    };
    let max_level = config.max_level();
    log::set_boxed_logger(Box::new(Logger {
        config,
        started: Instant::now(),
        sink,
    }))
    .map_err(|_| LogInitError::AlreadyInitialized)?;
    log::set_max_level(max_level);
    Ok(())
}

/// Parses logging options from the process's command line and
/// [`LOG_ENV_VAR`], then installs the logger.
///
/// Returns the remaining (non-logging) arguments, without the program name,
/// and whether `-h` / `--help` was given. On a malformed option or an
/// unopenable log file this prints the problem and [`USAGE`] to standard
/// error and exits the process with status 2, because no logger exists yet
/// to report it.
pub fn init_from_command_line() -> CommandLine {
    let (config, remaining) = match LogConfig::from_args(std::env::args().skip(1)) {
        Ok(parsed) => parsed,
        Err(error) => {
            eprintln!("error: {error}\n\n{USAGE}");
            std::process::exit(2);
        }
    };
    let help_requested = config.help_requested;
    let summary = format!("{config:?}");
    if let Err(error) = init(config) {
        eprintln!("error: {error}");
        std::process::exit(2);
    }
    log::debug!("logging initialized: {summary}");
    CommandLine {
        remaining,
        help_requested,
    }
}

/// The parts of the command line left for the program after
/// [`init_from_command_line`] consumed the logging options.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandLine {
    /// Arguments that were not logging options, in their original order.
    pub remaining: Vec<String>,
    /// Whether `-h` / `--help` was given; print usage and exit when set.
    pub help_requested: bool,
}

enum Sink {
    Stderr,
    File(Mutex<File>),
}

struct Logger {
    config: LogConfig,
    started: Instant,
    sink: Sink,
}

impl log::Log for Logger {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        metadata.level() <= self.config.level_for(metadata.target())
    }

    fn log(&self, record: &log::Record<'_>) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let line = format!(
            "[{:>8.3}s {:<5} {}] {}\n",
            self.started.elapsed().as_secs_f64(),
            record.level(),
            record.target(),
            record.args()
        );
        // A failed log write has nowhere better to be reported, so it is
        // ignored rather than allowed to disturb the browser.
        match &self.sink {
            Sink::Stderr => {
                let _ = io::stderr().lock().write_all(line.as_bytes());
            }
            Sink::File(file) => {
                if let Ok(mut file) = file.lock() {
                    let _ = file.write_all(line.as_bytes());
                }
            }
        }
    }

    fn flush(&self) {
        match &self.sink {
            Sink::Stderr => {
                let _ = io::stderr().flush();
            }
            Sink::File(file) => {
                if let Ok(mut file) = file.lock() {
                    let _ = file.flush();
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> (LogConfig, Vec<String>) {
        LogConfig::from_args_and_env(args.iter().copied(), None).expect("valid arguments")
    }

    #[test]
    fn default_is_warn() {
        let (config, rest) = parse(&[]);
        assert_eq!(config.default_level, LevelFilter::Warn);
        assert!(rest.is_empty());
    }

    #[test]
    fn spec_sets_default_and_targets() {
        let config = LogConfig::parse_spec("info, net=debug ,js::vm=trace").unwrap();
        assert_eq!(config.default_level, LevelFilter::Info);
        assert_eq!(config.level_for("net"), LevelFilter::Debug);
        assert_eq!(config.level_for("net::fetch"), LevelFilter::Debug);
        assert_eq!(config.level_for("js::vm::frame"), LevelFilter::Trace);
        assert_eq!(config.level_for("js::lexer"), LevelFilter::Info);
        // `net=` must not match the unrelated `network` crate, which is a
        // dependency and so stays at the dependency level.
        assert_eq!(config.level_for("network"), LevelFilter::Warn);
        assert_eq!(config.level_for("css"), LevelFilter::Info);
        assert_eq!(config.max_level(), LevelFilter::Trace);
    }

    #[test]
    fn longest_target_wins_and_dashes_normalize() {
        let config = LogConfig::parse_spec("browser-shell=error,browser_shell::app=trace").unwrap();
        assert_eq!(
            config.level_for("browser_shell::toolbar"),
            LevelFilter::Error
        );
        assert_eq!(config.level_for("browser_shell::app"), LevelFilter::Trace);
    }

    #[test]
    fn levels_are_case_insensitive_and_validated() {
        assert_eq!(parse_level("DEBUG"), Ok(LevelFilter::Debug));
        assert_eq!(parse_level("off"), Ok(LevelFilter::Off));
        assert_eq!(
            LogConfig::parse_spec("loud"),
            Err(LogConfigError::UnknownLevel("loud".into()))
        );
        assert_eq!(
            LogConfig::parse_spec("=debug"),
            Err(LogConfigError::EmptyTarget("=debug".into()))
        );
    }

    #[test]
    fn verbosity_and_quiet_flags() {
        assert_eq!(parse(&["-v"]).0.default_level, LevelFilter::Info);
        assert_eq!(parse(&["-vv"]).0.default_level, LevelFilter::Debug);
        assert_eq!(parse(&["-vvvv"]).0.default_level, LevelFilter::Trace);
        assert_eq!(parse(&["--verbose"]).0.default_level, LevelFilter::Info);
        assert_eq!(parse(&["-q"]).0.default_level, LevelFilter::Error);
        assert_eq!(parse(&["-qq"]).0.default_level, LevelFilter::Off);
        assert_eq!(parse(&["--quiet"]).0.default_level, LevelFilter::Error);
    }

    #[test]
    fn log_level_forms_and_remaining_args() {
        let (config, rest) = parse(&[
            "https://example.com",
            "--log-level",
            "debug",
            "--log-file=out.log",
            "--other",
        ]);
        assert_eq!(config.default_level, LevelFilter::Debug);
        assert_eq!(config.file, Some(PathBuf::from("out.log")));
        assert_eq!(rest, vec!["https://example.com", "--other"]);

        let (config, _) = parse(&["--log-level=net=trace"]);
        assert_eq!(config.default_level, LevelFilter::Warn);
        assert_eq!(config.level_for("net"), LevelFilter::Trace);
    }

    #[test]
    fn later_level_options_win() {
        let (config, _) = parse(&["--log-level", "net=trace", "--log-level", "info"]);
        assert_eq!(config.default_level, LevelFilter::Info);
        assert!(config.targets.is_empty());
        assert_eq!(parse(&["-vvv", "-q"]).0.default_level, LevelFilter::Error);
    }

    #[test]
    fn missing_values_are_errors() {
        assert_eq!(
            LogConfig::from_args_and_env(["--log-level"], None),
            Err(LogConfigError::MissingValue("--log-level"))
        );
        assert_eq!(
            LogConfig::from_args_and_env(["--log-file"], None),
            Err(LogConfigError::MissingValue("--log-file"))
        );
    }

    #[test]
    fn env_spec_applies_only_without_cli_level() {
        let (config, _) =
            LogConfig::from_args_and_env(std::iter::empty::<&str>(), Some("debug")).unwrap();
        assert_eq!(config.default_level, LevelFilter::Debug);
        let (config, _) = LogConfig::from_args_and_env(["-q"], Some("debug")).unwrap();
        assert_eq!(config.default_level, LevelFilter::Error);
        assert!(LogConfig::from_args_and_env(std::iter::empty::<&str>(), Some("nope")).is_err());
    }

    #[test]
    fn help_flag_is_reported_and_consumed() {
        let (config, rest) = parse(&["--help"]);
        assert!(config.help_requested);
        assert!(rest.is_empty());
        assert!(parse(&["-h"]).0.help_requested);
    }

    #[test]
    fn dependencies_stay_at_warn_unless_asked() {
        let (config, _) = parse(&["-vvv"]);
        assert_eq!(config.level_for("net::engine"), LevelFilter::Trace);
        assert_eq!(config.level_for("hyper::proto"), LevelFilter::Warn);
        assert_eq!(config.level_for("winit"), LevelFilter::Warn);

        let config = LogConfig::parse_spec("debug,deps=info,hyper=trace").unwrap();
        assert_eq!(config.level_for("browser_shell::app"), LevelFilter::Debug);
        assert_eq!(config.level_for("rustls::client"), LevelFilter::Info);
        assert_eq!(config.level_for("hyper::client"), LevelFilter::Trace);
        assert!(
            config
                .targets
                .iter()
                .all(|t| t.target != DEPENDENCIES_TARGET)
        );

        let config = LogConfig::parse_spec("deps=off").unwrap();
        assert_eq!(config.level_for("reqwest"), LevelFilter::Off);
        assert_eq!(config.level_for("net"), LevelFilter::Warn);
    }

    /// Every workspace member's package name must be in `WORKSPACE_CRATES`,
    /// or `-v` would treat that crate's logs as a dependency's.
    #[test]
    fn workspace_crates_match_cargo_members() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let manifest = std::fs::read_to_string(root.join("Cargo.toml")).unwrap();
        let members = manifest
            .split("members = [")
            .nth(1)
            .and_then(|rest| rest.split(']').next())
            .expect("workspace members list");
        let mut names = Vec::new();
        for member in members.split(',').map(|m| m.trim().trim_matches('"')) {
            if member.is_empty() {
                continue;
            }
            let package = std::fs::read_to_string(root.join(member).join("Cargo.toml")).unwrap();
            let name = package
                .lines()
                .find_map(|line| line.strip_prefix("name = "))
                .expect("package name")
                .trim_matches('"')
                .replace('-', "_");
            names.push(name);
        }
        for name in &names {
            assert!(
                WORKSPACE_CRATES.contains(&name.as_str()),
                "add `{name}` to common::logging::WORKSPACE_CRATES"
            );
        }
        assert_eq!(
            names.len(),
            WORKSPACE_CRATES.len(),
            "stale entry in WORKSPACE_CRATES"
        );
    }

    #[test]
    fn lone_dash_is_not_a_flag() {
        let (_, rest) = parse(&["-", "-x"]);
        assert_eq!(rest, vec!["-", "-x"]);
    }
}
