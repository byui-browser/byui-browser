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
//! | `warn`  | Something went wrong but the browser recovered or degraded: a subresource was skipped, a font fell back, a request was blocked by policy. Also an uncaught exception from page script, because the browser has no developer console to show it. | `page script failed: TypeError: x is not a function` |
//! | `info`  | One line per major user-visible milestone: startup, window creation, navigation start and finish, storage opened. Readable by a non-developer. | `Navigating to byui://welcome` |
//! | `debug` | Per-subsystem detail a developer needs to follow one operation: request and response summaries, cache hits, parse and layout summaries, script evaluation start and end. | `GET https://example.com -> 200 (1532 bytes)` |
//! | `trace` | Very high-volume detail that would drown everything else: individual tokens, DOM nodes, selector matches, bytecode instructions, every repaint and input event. | `token StartTag(<p>)` |
//!
//! The default level is `warn`, so a normal run prints only problems.
//!
//! Report each failure **once** at `warn` or `error`, in the layer that owns
//! the outcome (for a page load, `browser::load`). Lower layers that see the
//! same failure, and Web APIs whose error the page already receives as a JS
//! exception, log it at `debug`. One exception: the layer that applies a
//! security policy (CORS, mixed content, credentials in a URL) may add one
//! `warn` with the reason, because the outcome message does not say why.
//! Code that reruns on every window resize or
//! page click (styling, layout, painting) never logs above `debug`.
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
//! - `-q` / `--quiet` lowers the level to `error`; `-qq` turns logging off.
//!   Both also quiet third-party crates. When flags repeat, the last wins
//!   (`-vv -v` is `info`).
//! - `--log-file <path>` appends log lines to a file instead of standard
//!   error, which is useful on Windows where the GUI has no console.
//! - Programs that keep a log by default (the browser and its native shells)
//!   pass a default log file to [`init_from_command_line_with_default_file`].
//!   It is written **in addition to** standard error, so the terminal is
//!   unchanged. Each launch moves the previous run's file aside (see
//!   [`previous_log_path`]) and starts a fresh one, so at most two runs are
//!   kept. `--no-log-file` turns it off, and `--log-file <path>` replaces it.
//!   If it cannot be opened, logging continues on standard error only.
//! - When no level option is given, the `BYUI_LOG` environment variable is
//!   read as a spec instead.
//!
//! # Credentials
//!
//! Every line is passed through [`redact_credentials`] before it is written,
//! so a `user:password@` inside a URL never reaches the terminal or log file,
//! even when it arrives inside an error message or a debug dump. Crates should
//! still avoid logging secrets in the first place; this is a safety net.
//!
//! # Limitations
//!
//! Lines are written synchronously as they are logged; there is no
//! background writer, colorized output, or log rotation.

use std::fmt;
use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
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
  -q, -qq              Log only errors, or nothing at all (including
                       third-party crates).
  --log-file <path>    Append log lines to <path> instead of standard error.
  --no-log-file        Do not write the default log file (the browser keeps
                       one for each run; see below).
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
    /// File to append log lines to instead of standard error, from
    /// `--log-file`. When set, [`LogConfig::default_file`] is not used.
    pub file: Option<PathBuf>,
    /// Log file written alongside standard error when `file` is `None`.
    ///
    /// Never set from the command line: programs that keep a log by default
    /// set it through [`init_from_command_line_with_default_file`]. [`init`]
    /// moves an existing file aside to [`previous_log_path`] and starts a
    /// fresh one; if that fails, logging continues on standard error only.
    pub default_file: Option<PathBuf>,
    /// Whether `--no-log-file` was given, which turns off
    /// [`LogConfig::default_file`].
    pub default_file_disabled: bool,
    /// Whether the command line asked for help (`-h` / `--help`). Callers
    /// should print their usage plus [`USAGE`] and exit.
    pub help_requested: bool,
    /// Whether a `deps=<level>` directive set `dependency_level`; when it
    /// did, later bare levels and `-v`/`-q` flags leave it alone.
    dependency_level_explicit: bool,
}

impl Default for LogConfig {
    fn default() -> Self {
        Self {
            default_level: DEFAULT_LEVEL,
            dependency_level: DEFAULT_DEPENDENCY_LEVEL,
            targets: Vec::new(),
            file: None,
            default_file: None,
            default_file_disabled: false,
            help_requested: false,
            dependency_level_explicit: false,
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
    /// command line, or was followed by another option instead of a value.
    /// Holds the option name.
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
                    let spec = option_value(args.next(), "--log-level")?;
                    config.reset_levels();
                    config.apply_spec(&spec)?;
                    level_from_args = true;
                }
                "--log-file" => {
                    let path = option_value(args.next(), "--log-file")?;
                    config.file = Some(PathBuf::from(path));
                }
                "--no-log-file" => config.default_file_disabled = true,
                "--verbose" => {
                    config.set_default_level(verbosity_level(1));
                    level_from_args = true;
                }
                "--quiet" => {
                    config.set_default_level(LevelFilter::Error);
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
                        config.set_default_level(verbosity_level(count));
                        level_from_args = true;
                    } else if let Some(count) = repeated_flag(&arg, 'q') {
                        config.set_default_level(if count == 1 {
                            LevelFilter::Error
                        } else {
                            LevelFilter::Off
                        });
                        level_from_args = true;
                    } else {
                        remaining.push(arg);
                    }
                }
            }
        }

        // Skip the environment for `--help`, so a malformed `BYUI_LOG` cannot
        // stop the help text from printing.
        if let Some(spec) = env_spec.filter(|_| !level_from_args && !config.help_requested) {
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
        self.dependency_level_explicit = false;
        self.targets.clear();
    }

    /// Sets the workspace level. Unless `deps=<level>` was given, a level
    /// quieter than [`DEFAULT_DEPENDENCY_LEVEL`] also quiets dependencies, so
    /// `-q`, `-qq`, and `off` silence everything while `-v` leaves
    /// dependencies at their default.
    fn set_default_level(&mut self, level: LevelFilter) {
        self.default_level = level;
        if !self.dependency_level_explicit {
            self.dependency_level = DEFAULT_DEPENDENCY_LEVEL.min(level);
        }
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
                        self.dependency_level_explicit = true;
                        continue;
                    }
                    self.targets.retain(|entry| entry.target != target);
                    self.targets.push(TargetLevel { target, level });
                }
                None => self.set_default_level(parse_level(directive)?),
            }
        }
        Ok(())
    }
}

/// Returns the value that follows a value-taking option, rejecting a missing
/// value or another option (such as `--log-file --log-level`) in its place.
fn option_value(value: Option<String>, option: &'static str) -> Result<String, LogConfigError> {
    match value {
        Some(value) if !(value.starts_with('-') && value.len() > 1) => Ok(value),
        _ => Err(LogConfigError::MissingValue(option)),
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

/// Removes the `user:password@` part of every URL in `text`.
///
/// A URL here is any `scheme://` followed by an authority; the authority ends
/// at the first `/`, `?`, `#`, whitespace, quote, `<`, `>`, `(`, `)`, `[`,
/// `]`, or end of text, and everything up to and including its last `@` is
/// removed. Returns `text` unchanged (and unallocated) when nothing matches.
pub fn redact_credentials(text: &str) -> std::borrow::Cow<'_, str> {
    if !text.contains('@') || !text.contains("://") {
        return std::borrow::Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(index) = rest.find("://") {
        let authority_start = index + 3;
        out.push_str(&rest[..authority_start]);
        let after = &rest[authority_start..];
        let authority_end = after
            .find(|c: char| c.is_whitespace() || "/?#\"'<>()[]".contains(c))
            .unwrap_or(after.len());
        match after[..authority_end].rfind('@') {
            Some(at) => rest = &after[at + 1..],
            None => rest = after,
        }
    }
    out.push_str(rest);
    std::borrow::Cow::Owned(out)
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
///
/// Where lines go:
/// - `config.file` set (`--log-file`): that file only, appended to. Failing
///   to open it is an error, because the user asked for it.
/// - otherwise, `config.default_file` set and not disabled (and help not
///   requested): standard error **and** that file, after moving the previous
///   run's file aside. Failing to open it logs a warning and continues on
///   standard error only, so a read-only data folder never stops the
///   program.
/// - otherwise: standard error only.
pub fn init(config: LogConfig) -> Result<(), LogInitError> {
    let mut default_file_status = None;
    let sink = match (&config.file, default_log_target(&config)) {
        (Some(path), _) => Sink::File(Mutex::new(
            OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .map_err(|source| LogInitError::OpenFile {
                    path: path.clone(),
                    source,
                })?,
        )),
        (None, Some(path)) => match open_default_log_file(path) {
            Ok(file) => {
                default_file_status = Some(Ok(path.clone()));
                Sink::StderrAndFile(Mutex::new(file))
            }
            Err(error) => {
                default_file_status = Some(Err((path.clone(), error)));
                Sink::Stderr
            }
        },
        (None, None) => Sink::Stderr,
    };
    let max_level = config.max_level();
    log::set_boxed_logger(Box::new(Logger {
        config,
        started: Instant::now(),
        sink,
    }))
    .map_err(|_| LogInitError::AlreadyInitialized)?;
    log::set_max_level(max_level);
    match default_file_status {
        Some(Ok(path)) => log::info!("writing log file {}", path.display()),
        Some(Err((path, error))) => log::warn!(
            "cannot write log file {}: {error}; logging to standard error only \
             (--no-log-file turns the log file off)",
            path.display()
        ),
        None => {}
    }
    Ok(())
}

/// The default log file `init` should open, if any.
fn default_log_target(config: &LogConfig) -> Option<&PathBuf> {
    config
        .default_file
        .as_ref()
        .filter(|_| !config.default_file_disabled && !config.help_requested)
}

/// Returns where the previous run's log is kept when a default log file
/// starts fresh: `browser.log` becomes `browser.previous.log` in the same
/// folder (a name without an extension gets `.previous` appended).
pub fn previous_log_path(path: &Path) -> PathBuf {
    let stem = path
        .file_stem()
        .map_or_else(|| "log".into(), |stem| stem.to_string_lossy().into_owned());
    let name = match path.extension() {
        Some(extension) => format!("{stem}.previous.{}", extension.to_string_lossy()),
        None => format!("{stem}.previous"),
    };
    path.with_file_name(name)
}

/// Creates the log folder, moves last run's file to [`previous_log_path`]
/// (replacing an older one), and opens a fresh file with a header line.
///
/// If the old file cannot be moved (on Windows, while another browser still
/// has it open) the new run is appended to it instead, so nothing is lost.
fn open_default_log_file(path: &Path) -> io::Result<File> {
    if let Some(folder) = path
        .parent()
        .filter(|folder| !folder.as_os_str().is_empty())
    {
        std::fs::create_dir_all(folder)?;
    }
    if path.exists() {
        let _ = std::fs::rename(path, previous_log_path(path));
    }
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    let started = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_secs());
    writeln!(
        file,
        "# BYUI Browser {} log, started at unix time {started}, process {}",
        env!("CARGO_PKG_VERSION"),
        std::process::id()
    )?;
    Ok(file)
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
    init_from_command_line_with_default_file(None)
}

/// Like [`init_from_command_line`], but also writes `default_file` (when
/// `Some`) alongside standard error unless the command line says otherwise;
/// see [`LogConfig::default_file`]. The browser passes its data folder's
/// `logs/browser.log` here.
pub fn init_from_command_line_with_default_file(default_file: Option<PathBuf>) -> CommandLine {
    let (mut config, remaining) = match LogConfig::from_args(std::env::args().skip(1)) {
        Ok(parsed) => parsed,
        Err(error) => {
            eprintln!("error: {error}\n\n{USAGE}");
            std::process::exit(2);
        }
    };
    config.default_file = default_file;
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
    StderrAndFile(Mutex<File>),
}

impl Sink {
    fn file(&self) -> Option<&Mutex<File>> {
        match self {
            Self::Stderr => None,
            Self::File(file) | Self::StderrAndFile(file) => Some(file),
        }
    }

    fn writes_stderr(&self) -> bool {
        !matches!(self, Self::File(_))
    }
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
        let message = record.args().to_string();
        let line = format!(
            "[{:>8.3}s {:<5} {}] {}\n",
            self.started.elapsed().as_secs_f64(),
            record.level(),
            record.target(),
            redact_credentials(&message)
        );
        // A failed log write has nowhere better to be reported, so it is
        // ignored rather than allowed to disturb the browser.
        if self.sink.writes_stderr() {
            let _ = io::stderr().lock().write_all(line.as_bytes());
        }
        if let Some(file) = self.sink.file()
            && let Ok(mut file) = file.lock()
        {
            let _ = file.write_all(line.as_bytes());
        }
    }

    fn flush(&self) {
        if self.sink.writes_stderr() {
            let _ = io::stderr().flush();
        }
        if let Some(file) = self.sink.file()
            && let Ok(mut file) = file.lock()
        {
            let _ = file.flush();
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
    fn quiet_levels_also_quiet_dependencies() {
        assert_eq!(parse(&["-q"]).0.level_for("winit"), LevelFilter::Error);
        assert_eq!(parse(&["-qq"]).0.level_for("hyper"), LevelFilter::Off);
        let (config, _) = parse(&["--log-level", "off"]);
        assert_eq!(config.level_for("reqwest"), LevelFilter::Off);
        assert_eq!(config.max_level(), LevelFilter::Off);
        // Louder workspace levels leave dependencies at their default ...
        assert_eq!(parse(&["-vvv"]).0.level_for("hyper"), LevelFilter::Warn);
        // ... the last flag wins ...
        assert_eq!(
            parse(&["-qq", "-v"]).0.level_for("hyper"),
            LevelFilter::Warn
        );
        // ... and an explicit `deps=` is kept.
        let (config, _) = parse(&["--log-level", "deps=debug,off"]);
        assert_eq!(config.level_for("hyper"), LevelFilter::Debug);
        assert_eq!(config.level_for("net"), LevelFilter::Off);
    }

    #[test]
    fn options_reject_another_option_as_their_value() {
        assert_eq!(
            LogConfig::from_args_and_env(["--log-file", "--log-level", "debug"], None),
            Err(LogConfigError::MissingValue("--log-file"))
        );
        assert_eq!(
            LogConfig::from_args_and_env(["--log-level", "-v"], None),
            Err(LogConfigError::MissingValue("--log-level"))
        );
        // A lone `-` is still a value.
        let (config, _) = parse(&["--log-file", "-"]);
        assert_eq!(config.file, Some(PathBuf::from("-")));
    }

    #[test]
    fn help_ignores_a_malformed_environment_spec() {
        let (config, _) = LogConfig::from_args_and_env(["--help"], Some("nope")).unwrap();
        assert!(config.help_requested);
    }

    #[test]
    fn credentials_are_redacted_from_urls() {
        assert_eq!(
            redact_credentials("GET http://alice:hunter2@example.com/a?b=c failed"),
            "GET http://example.com/a?b=c failed"
        );
        assert_eq!(
            redact_credentials(
                "load \"https://u@h.test\" then text: \"ftp://x:y@z:21\" and wss://p:q@r#f"
            ),
            "load \"https://h.test\" then text: \"ftp://z:21\" and wss://r#f"
        );
        // A password containing `@` is removed up to the last `@`.
        assert_eq!(redact_credentials("http://a:b@c@host/"), "http://host/");
        // An `@` after the authority is left alone.
        assert_eq!(
            redact_credentials("mailto via http://host/user@example.com"),
            "mailto via http://host/user@example.com"
        );
        let plain = "no urls here, just user@example.com";
        assert!(matches!(
            redact_credentials(plain),
            std::borrow::Cow::Borrowed(_)
        ));
    }

    /// A fresh, empty folder under the system temp directory for one test.
    fn temp_folder(test: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |since| since.as_nanos());
        let folder = std::env::temp_dir().join(format!(
            "byui-logging-{test}-{}-{nanos}",
            std::process::id()
        ));
        std::fs::create_dir_all(&folder).unwrap();
        folder
    }

    #[test]
    fn no_log_file_flag_is_consumed() {
        let (config, rest) = parse(&["--no-log-file", "page.html"]);
        assert!(config.default_file_disabled);
        assert_eq!(rest, vec!["page.html"]);
        assert!(!parse(&[]).0.default_file_disabled);
    }

    #[test]
    fn previous_log_keeps_the_extension() {
        assert_eq!(
            previous_log_path(Path::new("/data/logs/browser.log")),
            PathBuf::from("/data/logs/browser.previous.log")
        );
        assert_eq!(
            previous_log_path(Path::new("trace")),
            PathBuf::from("trace.previous")
        );
    }

    #[test]
    fn default_log_file_starts_fresh_and_keeps_one_previous_run() {
        let folder = temp_folder("rotation");
        let path = folder.join("logs").join("browser.log");
        let previous = previous_log_path(&path);

        // First run creates the missing `logs` folder and a header line.
        let mut first = open_default_log_file(&path).unwrap();
        writeln!(first, "run one").unwrap();
        drop(first);
        let contents = std::fs::read_to_string(&path).unwrap();
        assert!(contents.starts_with("# BYUI Browser "), "{contents}");
        assert!(contents.contains("run one"));
        assert!(!previous.exists());

        // Each later run moves the last one aside, replacing an older one.
        for run in ["run two", "run three"] {
            let mut file = open_default_log_file(&path).unwrap();
            writeln!(file, "{run}").unwrap();
        }
        let current = std::fs::read_to_string(&path).unwrap();
        let kept = std::fs::read_to_string(&previous).unwrap();
        assert!(current.contains("run three") && !current.contains("run two"));
        assert!(kept.contains("run two") && !kept.contains("run one"));

        std::fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn unwritable_default_log_folder_is_an_error_not_a_panic() {
        let folder = temp_folder("unwritable");
        // A file where the `logs` folder should be makes the folder
        // impossible to create on every OS.
        std::fs::write(folder.join("logs"), "not a folder").unwrap();
        assert!(open_default_log_file(&folder.join("logs").join("browser.log")).is_err());
        std::fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn default_log_file_is_skipped_when_disabled_or_for_help() {
        let with_default = |args: &[&str]| {
            let (mut config, _) = parse(args);
            config.default_file = Some(PathBuf::from("browser.log"));
            config
        };
        assert!(default_log_target(&with_default(&[])).is_some());
        assert!(default_log_target(&with_default(&["--no-log-file"])).is_none());
        // `--help` must not move last run's log aside.
        assert!(default_log_target(&with_default(&["--help"])).is_none());
        assert!(default_log_target(&LogConfig::default()).is_none());
    }

    #[test]
    fn lone_dash_is_not_a_flag() {
        let (_, rest) = parse(&["-", "-x"]);
        assert_eq!(rest, vec!["-", "-x"]);
    }
}
