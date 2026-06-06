use std::{
    fs::File,
    io::{self, BufWriter, Write},
    path::{Path, PathBuf},
};

use pptx_compose::core::error::ErrorDetails;
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct OutputSink {
    quiet: bool,
    verbose: bool,
    no_color: bool,
    json_errors: bool,
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
        Self {
            quiet,
            verbose,
            no_color,
            json_errors,
        }
    }

    pub(crate) const fn from_global_args(args: &GlobalArgs) -> Self {
        Self::new(args.quiet, args.verbose, args.no_color, args.json_errors)
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
            OutputDest::Path(path) => {
                let file = File::create(&path).map_err(|source| {
                    CliError::write_with_source("Could not open JSON output path.", source)
                })?;
                let mut writer = BufWriter::new(file);
                self.emit_json_to_writer(doc, &mut writer)
            }
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

    const fn log_prefix(self, level: LogLevel) -> &'static str {
        match (self.no_color, level) {
            (_, LogLevel::Error) => "error",
            (_, LogLevel::Warn) => "warning",
            (_, LogLevel::Info) => "info",
            (_, LogLevel::Debug) => "debug",
        }
    }
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
fn json_errors_emits_single_envelope_to_stderr() {
    use pptx_compose::core::error::ErrorCode;

    let sink = OutputSink::new(true, false, true, true);
    let err = CliError::unsupported("inspect command is not implemented yet");
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
