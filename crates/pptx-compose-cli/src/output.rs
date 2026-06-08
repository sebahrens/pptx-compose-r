use std::{
    fs::{self, File, OpenOptions},
    io::{self, IsTerminal, Write},
    path::{Path, PathBuf},
};

use pptx_compose::core::error::ErrorDetails;
use pptx_compose::temp_output_path;
use serde::Serialize;

use crate::{CliError, cli::GlobalArgs};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(dead_code)]
pub(crate) enum LogLevel {
    Error,
    Warn,
    Info,
    Debug,
}

#[derive(Debug)]
#[allow(dead_code)]
pub(crate) enum OutputDest {
    Stdout,
    Path(PathBuf),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct OutputSink {
    quiet: bool,
    verbose: bool,
    color: bool,
    json_errors: bool,
    atomic_temp_dir: Option<PathBuf>,
    keep_temp: bool,
}

impl Default for OutputSink {
    fn default() -> Self {
        Self::new(false, false, true, false)
    }
}

#[derive(Serialize)]
struct ErrorEnvelope<'a> {
    schema: &'static str,
    version: u8,
    status: &'static str,
    error: &'a ErrorDetails,
}

impl OutputSink {
    pub(crate) const fn new(quiet: bool, verbose: bool, no_color: bool, json_errors: bool) -> Self {
        Self::new_with_color_policy(quiet, verbose, no_color, json_errors, false)
    }

    const fn new_with_color_policy(
        quiet: bool,
        verbose: bool,
        no_color: bool,
        json_errors: bool,
        stderr_is_terminal: bool,
    ) -> Self {
        Self {
            quiet,
            verbose,
            color: !no_color && stderr_is_terminal,
            json_errors,
            atomic_temp_dir: None,
            keep_temp: false,
        }
    }

    pub(crate) fn from_global_args(args: &GlobalArgs) -> Self {
        Self::new_with_color_policy(
            args.quiet,
            args.verbose,
            args.no_color,
            args.json_errors,
            io::stderr().is_terminal(),
        )
    }

    pub(crate) fn with_atomic_temp_dir(mut self, temp_dir: PathBuf, keep_temp: bool) -> Self {
        self.atomic_temp_dir = Some(temp_dir);
        self.keep_temp = keep_temp;
        self
    }

    #[allow(dead_code)]
    pub(crate) fn emit_json(&self, doc: &impl Serialize, dest: OutputDest) -> Result<(), CliError> {
        match dest {
            OutputDest::Stdout => {
                let stdout = io::stdout();
                let mut lock = stdout.lock();
                self.emit_json_to_writer(doc, &mut lock)
            }
            OutputDest::Path(path) if path == Path::new("-") => {
                let stdout = io::stdout();
                let mut lock = stdout.lock();
                self.emit_json_to_writer(doc, &mut lock)
            }
            OutputDest::Path(path) => self.emit_json_overwrite(doc, OutputDest::Path(path), false),
        }
    }

    pub(crate) fn emit_json_overwrite(
        &self,
        doc: &impl Serialize,
        dest: OutputDest,
        overwrite: bool,
    ) -> Result<(), CliError> {
        match dest {
            OutputDest::Stdout => self.emit_json(doc, OutputDest::Stdout),
            OutputDest::Path(path) if path == Path::new("-") => {
                self.emit_json(doc, OutputDest::Path(path))
            }
            OutputDest::Path(path) => write_atomic(
                &path,
                overwrite,
                self.atomic_temp_dir.as_deref(),
                self.keep_temp,
                |writer| self.emit_json_to_writer(doc, writer),
            ),
        }
    }

    pub(crate) fn emit_patch_report(
        &self,
        report: &impl Serialize,
        dest: Option<PathBuf>,
        overwrite: bool,
    ) -> Result<(), CliError> {
        self.emit_json_overwrite(report, OutputDest::from(dest), overwrite)
    }

    pub(crate) fn emit_optional_patch_report(
        &self,
        report: &impl Serialize,
        dest: Option<PathBuf>,
        overwrite: bool,
    ) -> Result<(), CliError> {
        if let Some(dest) = dest {
            self.emit_json_overwrite(report, OutputDest::Path(dest), overwrite)
        } else {
            Ok(())
        }
    }

    pub(crate) fn emit_diff(
        &self,
        diff: &impl Serialize,
        dest: Option<PathBuf>,
        overwrite: bool,
    ) -> Result<(), CliError> {
        if let Some(dest) = dest {
            self.emit_json_overwrite(diff, OutputDest::Path(dest), overwrite)
        } else {
            Ok(())
        }
    }

    #[allow(dead_code)]
    pub(crate) fn log(&self, level: LogLevel, msg: &str) -> Result<(), CliError> {
        let stderr = io::stderr();
        let mut lock = stderr.lock();
        self.log_to_writer(level, msg, &mut lock)
    }

    pub(crate) fn emit_error(&self, err: &CliError) -> Result<(), CliError> {
        let stderr = io::stderr();
        let mut lock = stderr.lock();
        self.emit_error_to_writer(err, &mut lock)
    }

    fn emit_json_to_writer(
        &self,
        doc: &impl Serialize,
        writer: &mut impl Write,
    ) -> Result<(), CliError> {
        serde_json::to_writer(&mut *writer, doc).map_err(|source| {
            CliError::write_with_source("Could not serialize JSON output.", source)
        })?;
        writeln!(writer)
            .map_err(|source| CliError::write_with_source("Could not finish JSON output.", source))
    }

    fn log_to_writer(
        &self,
        level: LogLevel,
        msg: &str,
        writer: &mut impl Write,
    ) -> Result<(), CliError> {
        if self.quiet || matches!(level, LogLevel::Debug) && !self.verbose {
            return Ok(());
        }
        let prefix = self.log_prefix(level);
        writeln!(writer, "{prefix}: {msg}")
            .map_err(|source| CliError::write_with_source("Could not write log output.", source))
    }

    fn emit_error_to_writer(
        &self,
        err: &CliError,
        writer: &mut impl Write,
    ) -> Result<(), CliError> {
        if self.json_errors {
            let envelope = ErrorEnvelope {
                schema: "pptx-compose.error.v1",
                version: 1,
                status: "error",
                error: err.details(),
            };
            return self.emit_json_to_writer(&envelope, writer);
        }

        writeln!(writer, "{err}")
            .map_err(|source| CliError::write_with_source("Could not write error output.", source))
    }

    const fn log_prefix(&self, level: LogLevel) -> &'static str {
        match (self.color, level) {
            (false, LogLevel::Error) => "error",
            (false, LogLevel::Warn) => "warning",
            (false, LogLevel::Info) => "info",
            (false, LogLevel::Debug) => "debug",
            (true, LogLevel::Error) => "\x1b[31merror\x1b[0m",
            (true, LogLevel::Warn) => "\x1b[33mwarning\x1b[0m",
            (true, LogLevel::Info) => "\x1b[36minfo\x1b[0m",
            (true, LogLevel::Debug) => "\x1b[90mdebug\x1b[0m",
        }
    }
}

pub(crate) fn write_bytes_atomic(
    path: &Path,
    bytes: &[u8],
    overwrite: bool,
    temp_dir: Option<&Path>,
    keep_temp: bool,
) -> Result<(), CliError> {
    write_atomic(path, overwrite, temp_dir, keep_temp, |writer| {
        writer
            .write_all(bytes)
            .map_err(|source| CliError::write_with_source("Could not write output bytes.", source))
    })
}

fn create_file_atomic(path: &Path, overwrite: bool) -> Result<File, CliError> {
    let mut options = OpenOptions::new();
    options.write(true);
    if overwrite {
        options.create(true).truncate(true);
    } else {
        options.create_new(true);
    }
    options.open(path).map_err(|source| {
        if source.kind() == io::ErrorKind::AlreadyExists && !overwrite {
            output_exists_error(path)
        } else {
            CliError::write_with_source(
                format!("Could not open output path {}.", path.display()),
                source,
            )
        }
    })
}

fn write_atomic(
    path: &Path,
    overwrite: bool,
    temp_dir: Option<&Path>,
    keep_temp: bool,
    write: impl FnOnce(&mut File) -> Result<(), CliError>,
) -> Result<(), CliError> {
    if path.exists() && !overwrite {
        return Err(output_exists_error(path));
    }
    let parent = output_parent(path);
    let temp_path = temp_output_path(path, temp_dir);
    let result = (|| {
        let mut temp = create_file_atomic(&temp_path, false)?;
        write(&mut temp)?;
        temp.sync_all().map_err(|source| {
            CliError::write_with_source(
                format!("Could not fsync temporary output {}.", temp_path.display()),
                source,
            )
        })?;
        if overwrite {
            fs::rename(&temp_path, path).map_err(|source| {
                CliError::write_with_source(
                    format!(
                        "Could not atomically rename {} to {}.",
                        temp_path.display(),
                        path.display()
                    ),
                    source,
                )
            })?;
        } else {
            fs::hard_link(&temp_path, path).map_err(|source| {
                if source.kind() == io::ErrorKind::AlreadyExists {
                    output_exists_error(path)
                } else if source.kind() == io::ErrorKind::CrossesDevices {
                    cross_device_publish_error(&temp_path, path, source)
                } else {
                    CliError::write_with_source(
                        format!(
                            "Could not atomically publish {} to {} without replacing an existing file.",
                            temp_path.display(),
                            path.display()
                        ),
                        source,
                    )
                }
            })?;
            fs::remove_file(&temp_path).map_err(|source| {
                CliError::write_with_source(
                    format!("Could not remove temporary output {}.", temp_path.display()),
                    source,
                )
            })?;
        }
        fsync_dir(parent)?;
        Ok(())
    })();
    if result.is_err() && !keep_temp {
        let _ = fs::remove_file(&temp_path);
    }
    result
}

fn cross_device_publish_error(temp_path: &Path, path: &Path, source: io::Error) -> CliError {
    CliError::write_with_source(
        format!(
            "Could not atomically publish {} to {} without replacing an existing file because the temporary output is on a different filesystem. Use --temp-dir on the same filesystem as the output path.",
            temp_path.display(),
            path.display()
        ),
        source,
    )
}

fn output_exists_error(path: &Path) -> CliError {
    CliError::new(
        pptx_compose::core::error::ErrorCode::WriteFailed,
        format!(
            "Output path {} already exists; pass --overwrite to replace it.",
            path.display()
        ),
    )
}

fn output_parent(path: &Path) -> &Path {
    match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    }
}

#[cfg(test)]
fn unique_counter() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);
    COUNTER.fetch_add(1, Ordering::Relaxed)
}

fn fsync_dir(path: &Path) -> Result<(), CliError> {
    File::open(path)
        .and_then(|dir| dir.sync_all())
        .map_err(|source| {
            CliError::write_with_source(
                format!("Could not fsync output directory {}.", path.display()),
                source,
            )
        })
}

impl From<Option<PathBuf>> for OutputDest {
    fn from(path: Option<PathBuf>) -> Self {
        match path {
            Some(path) => Self::Path(path),
            None => Self::Stdout,
        }
    }
}

#[cfg(test)]
#[test]
fn write_bytes_atomic_accepts_bare_relative_output() {
    use std::sync::{Mutex, OnceLock};

    static CURRENT_DIR_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

    let _guard = CURRENT_DIR_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .expect("current-dir test lock acquired");
    let root = unique_dir();
    let previous_dir = std::env::current_dir().expect("current dir reads");
    std::env::set_current_dir(&root).expect("current dir changes to test root");

    write_bytes_atomic(
        Path::new("bare-output.pptx"),
        b"pptx bytes",
        false,
        None,
        false,
    )
    .expect("bare relative atomic output succeeds");

    let output = root.join("bare-output.pptx");
    assert_eq!(
        fs::read(&output).expect("bare relative output reads"),
        b"pptx bytes"
    );
    let temp_prefix = ".bare-output.pptx.";
    let temp_remains = fs::read_dir(&root).expect("test root reads").any(|entry| {
        entry
            .expect("test root entry reads")
            .file_name()
            .to_string_lossy()
            .starts_with(temp_prefix)
    });
    assert!(
        !temp_remains,
        "successful atomic write removes temp sibling"
    );

    std::env::set_current_dir(previous_dir).expect("current dir restores");
    fs::remove_dir_all(root).expect("test dir removes");
}

#[cfg(test)]
#[test]
fn write_bytes_atomic_uses_configured_temp_dir() {
    let root = unique_dir();
    let temp_dir = unique_dir();
    let output = root.join("media.bin");

    write_bytes_atomic(&output, b"media bytes", false, Some(&temp_dir), false)
        .expect("atomic output with configured temp dir succeeds");

    assert_eq!(
        fs::read(&output).expect("output reads"),
        b"media bytes",
        "output file is published at requested path"
    );
    let output_temp_prefix = ".media.bin.";
    let sibling_temp_remains = fs::read_dir(&root).expect("output dir reads").any(|entry| {
        entry
            .expect("output dir entry reads")
            .file_name()
            .to_string_lossy()
            .starts_with(output_temp_prefix)
    });
    assert!(
        !sibling_temp_remains,
        "atomic write must not create sibling temps next to the output"
    );
    let configured_temp_remains = fs::read_dir(&temp_dir)
        .expect("temp dir reads")
        .next()
        .is_some();
    assert!(
        !configured_temp_remains,
        "successful atomic write removes configured temp file"
    );

    fs::remove_dir_all(root).expect("output test dir removes");
    fs::remove_dir_all(temp_dir).expect("temp test dir removes");
}

#[cfg(test)]
#[test]
fn emit_json_overwrite_uses_configured_temp_dir() {
    let root = unique_dir();
    let temp_dir = unique_dir();
    let output = root.join("report.json");
    let sink = OutputSink::default().with_atomic_temp_dir(temp_dir.clone(), false);

    sink.emit_json_overwrite(
        &serde_json::json!({"status": "ok"}),
        OutputDest::Path(output.clone()),
        false,
    )
    .expect("JSON output with configured temp dir succeeds");

    let value: serde_json::Value =
        serde_json::from_slice(&fs::read(&output).expect("JSON output reads"))
            .expect("JSON output parses");
    assert_eq!(value, serde_json::json!({"status": "ok"}));
    let sibling_temp_remains = fs::read_dir(&root).expect("output dir reads").any(|entry| {
        entry
            .expect("output dir entry reads")
            .file_name()
            .to_string_lossy()
            .starts_with(".report.json.")
    });
    assert!(
        !sibling_temp_remains,
        "JSON atomic write must not create sibling temps next to the output"
    );
    assert!(
        fs::read_dir(&temp_dir)
            .expect("temp dir reads")
            .next()
            .is_none(),
        "successful JSON atomic write removes configured temp file"
    );

    fs::remove_dir_all(root).expect("output test dir removes");
    fs::remove_dir_all(temp_dir).expect("temp test dir removes");
}

#[cfg(test)]
fn unique_dir() -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "pptx-compose-cli-output-{}-{}",
        std::process::id(),
        unique_counter()
    ));
    fs::create_dir_all(&root).expect("test dir creates");
    root
}

#[cfg(test)]
#[test]
fn cross_device_atomic_publish_error_is_actionable() {
    use pptx_compose::core::error::ErrorCode;

    let error = cross_device_publish_error(
        Path::new("/tmp/report.json.tmp"),
        Path::new("/workspace/report.json"),
        io::Error::from(io::ErrorKind::CrossesDevices),
    );

    assert_eq!(error.code(), ErrorCode::WriteFailed);
    assert!(error.details().message.contains("different filesystem"));
    assert!(error.details().message.contains("--temp-dir"));
}

#[cfg(test)]
#[test]
fn json_errors_emits_single_envelope_to_stderr() {
    use pptx_compose::core::error::ErrorCode;

    let sink = OutputSink::new(true, false, true, true);
    let err = CliError::new(
        ErrorCode::UnsupportedEdit,
        "inspect command is not implemented yet",
    );
    let stdout: Vec<u8> = Vec::new();
    let mut stderr = Vec::new();

    sink.log_to_writer(LogLevel::Info, "progress", &mut stderr)
        .expect("quiet log suppression succeeds");
    sink.emit_error_to_writer(&err, &mut stderr)
        .expect("JSON error envelope emits");

    assert!(stdout.is_empty());

    let stderr_text = String::from_utf8(stderr).expect("stderr is UTF-8");
    assert_eq!(stderr_text.lines().count(), 1);

    let envelope: serde_json::Value =
        serde_json::from_str(&stderr_text).expect("stderr is one JSON document");
    assert_eq!(envelope["schema"], "pptx-compose.error.v1");
    assert_eq!(envelope["version"], 1);
    assert_eq!(envelope["status"], "error");
    assert_eq!(
        envelope["error"]["code"],
        ErrorCode::UnsupportedEdit.as_str()
    );
    assert_eq!(
        envelope["error"]["message"],
        "inspect command is not implemented yet"
    );
}

#[cfg(test)]
#[test]
fn emit_json_writes_one_newline_terminated_document() {
    let sink = OutputSink::new(false, false, true, false);
    let mut stdout = Vec::new();

    sink.emit_json_to_writer(&serde_json::json!({"status": "success"}), &mut stdout)
        .expect("JSON output emits");

    let expected = br#"{"status":"success"}"#.iter().copied().chain([b'\n']).collect::<Vec<_>>();
    assert_eq!(stdout, expected);
}

#[cfg(test)]
#[test]
fn no_color_suppresses_ansi_log_prefixes_even_for_terminal_stderr() {
    let color_sink = OutputSink::new_with_color_policy(false, false, false, false, true);
    let no_color_sink = OutputSink::new_with_color_policy(false, false, true, false, true);

    let mut colored = Vec::new();
    color_sink
        .log_to_writer(LogLevel::Warn, "check", &mut colored)
        .expect("colored log emits");

    let mut plain = Vec::new();
    no_color_sink
        .log_to_writer(LogLevel::Warn, "check", &mut plain)
        .expect("plain log emits");

    assert_eq!(
        String::from_utf8(colored).expect("UTF-8"),
        "\x1b[33mwarning\x1b[0m: check\n"
    );
    assert_eq!(String::from_utf8(plain).expect("UTF-8"), "warning: check\n");
}

#[cfg(test)]
#[test]
fn non_terminal_stderr_keeps_log_prefixes_plain_without_no_color() {
    let sink = OutputSink::new_with_color_policy(false, false, false, false, false);
    let mut stderr = Vec::new();

    sink.log_to_writer(LogLevel::Error, "check", &mut stderr)
        .expect("plain log emits");

    assert_eq!(String::from_utf8(stderr).expect("UTF-8"), "error: check\n");
}
