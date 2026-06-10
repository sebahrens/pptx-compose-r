use std::{
    fs, io,
    path::{Component, Path, PathBuf},
};

use pptx_compose::core::error::ErrorCode;

use crate::{CliError, InvalidInputCause, cli::GlobalArgs};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PermissionContext {
    pub(crate) workspace: PathBuf,
    pub(crate) temp_dir: PathBuf,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PathIntent {
    InputPptx,
    OutputPptx,
    MediaInput,
    ReportOutput,
    DiffOutput,
    #[allow(dead_code)]
    TempFile,
}

impl PermissionContext {
    pub(crate) fn from_global_args(args: &GlobalArgs) -> Result<Self, CliError> {
        let workspace = match &args.workspace {
            Some(path) => canonical_root(path, "workspace")?,
            None => std::env::current_dir().map_err(|source| {
                path_error_with_source(
                    ErrorCode::PermissionDenied,
                    "Could not resolve current workspace.",
                    source,
                )
            })?,
        };
        let temp_dir = match &args.temp_dir {
            Some(path) => canonical_root(path, "temp directory")?,
            None => canonical_root(&std::env::temp_dir(), "temp directory")?,
        };

        Ok(Self {
            workspace,
            temp_dir,
        })
    }

    pub(crate) fn authorize_read(
        &self,
        path: &Path,
        intent: PathIntent,
    ) -> Result<PathBuf, CliError> {
        let candidate = self.anchor_path(path)?;
        let resolved =
            fs::canonicalize(&candidate).map_err(|source| read_path_error(intent, source))?;

        if self.is_allowed(&resolved) {
            Ok(resolved)
        } else {
            Err(path_error(
                ErrorCode::PermissionDenied,
                self.outside_allowed_dirs_message(intent),
            ))
        }
    }

    pub(crate) fn authorize_write(
        &self,
        path: &Path,
        intent: PathIntent,
    ) -> Result<PathBuf, CliError> {
        if path == Path::new("-") {
            if intent.allows_stdio() {
                return Ok(PathBuf::from("-"));
            }
            return Err(CliError::invalid_input(
                InvalidInputCause::CliArgument,
                format!("{} path does not support stdout/stdin '-'.", intent.label()),
            ));
        }

        let candidate = self.anchor_path(path)?;
        let resolved = if candidate.exists() {
            fs::canonicalize(&candidate).map_err(|source| {
                path_error_with_source(
                    ErrorCode::PermissionDenied,
                    format!("Could not resolve writable {} path.", intent.label()),
                    source,
                )
            })?
        } else {
            let file_name = candidate.file_name().ok_or_else(|| {
                path_error(
                    ErrorCode::UnsafePath,
                    format!("{} path must name a file.", intent.label()),
                )
            })?;
            let parent = candidate.parent().ok_or_else(|| {
                path_error(
                    ErrorCode::UnsafePath,
                    format!("{} path must have a parent directory.", intent.label()),
                )
            })?;
            let parent = fs::canonicalize(parent).map_err(|source| {
                path_error_with_source(
                    ErrorCode::PermissionDenied,
                    format!(
                        "Could not resolve parent directory for {} path.",
                        intent.label()
                    ),
                    source,
                )
            })?;
            parent.join(file_name)
        };

        if self.is_allowed(&resolved) {
            Ok(resolved)
        } else {
            Err(path_error(
                ErrorCode::PermissionDenied,
                self.outside_allowed_dirs_message(intent),
            ))
        }
    }

    #[allow(dead_code)]
    pub(crate) fn cleanup_temp_on_failure(&self, temp_path: &Path) -> Result<(), CliError> {
        let authorized = self.authorize_write(temp_path, PathIntent::TempFile)?;
        if authorized.exists() {
            fs::remove_file(&authorized).map_err(|source| {
                path_error_with_source(
                    ErrorCode::WriteFailed,
                    "Could not remove partial temporary file.",
                    source,
                )
            })?;
        }
        Ok(())
    }

    fn anchor_path(&self, path: &Path) -> Result<PathBuf, CliError> {
        reject_aliases(path)?;
        if path.is_absolute() {
            Ok(path.to_path_buf())
        } else {
            Ok(self.workspace.join(path))
        }
    }

    fn is_allowed(&self, path: &Path) -> bool {
        path.starts_with(&self.workspace) || path.starts_with(&self.temp_dir)
    }

    fn outside_allowed_dirs_message(&self, intent: PathIntent) -> String {
        format!(
            "{} path is outside the configured workspace or temp directory. Allowed directories: workspace={}, temp_dir={}. Pass --workspace or --temp-dir to allow a different directory.",
            intent.label(),
            self.workspace.display(),
            self.temp_dir.display()
        )
    }
}

impl PathIntent {
    const fn label(self) -> &'static str {
        match self {
            Self::InputPptx => "input PPTX",
            Self::OutputPptx => "output PPTX",
            Self::MediaInput => "media input",
            Self::ReportOutput => "report output",
            Self::DiffOutput => "diff output",
            Self::TempFile => "temporary file",
        }
    }

    const fn allows_stdio(self) -> bool {
        matches!(self, Self::ReportOutput | Self::DiffOutput)
    }
}

fn canonical_root(path: &Path, label: &str) -> Result<PathBuf, CliError> {
    reject_aliases(path)?;
    let anchored = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|source| {
                path_error_with_source(
                    ErrorCode::PermissionDenied,
                    format!("Could not resolve current directory for {label}."),
                    source,
                )
            })?
            .join(path)
    };
    fs::canonicalize(&anchored).map_err(|source| {
        path_error_with_source(
            ErrorCode::PermissionDenied,
            format!("Could not resolve {label}."),
            source,
        )
    })
}

fn reject_aliases(path: &Path) -> Result<(), CliError> {
    let mut components = path.components();
    if let Some(component) = components.next() {
        match component {
            Component::Prefix(_) => {
                return Err(path_error(
                    ErrorCode::UnsafePath,
                    "Platform path prefixes are not allowed.",
                ));
            }
            Component::Normal(value) if value.to_string_lossy().starts_with('~') => {
                return Err(path_error(
                    ErrorCode::UnsafePath,
                    "Home-directory aliases are not allowed.",
                ));
            }
            _ => {}
        }
    }
    Ok(())
}

fn path_error(code: ErrorCode, message: impl Into<String>) -> CliError {
    CliError::new(code, message)
}

fn path_error_with_source(
    code: ErrorCode,
    message: impl Into<String>,
    source: impl std::error::Error + Send + Sync + 'static,
) -> CliError {
    CliError::with_source(code, message, source)
}

fn read_path_error(intent: PathIntent, source: io::Error) -> CliError {
    let message = format!("Could not resolve readable {} path.", intent.label());
    if intent == PathIntent::InputPptx
        && matches!(
            source.kind(),
            io::ErrorKind::NotFound | io::ErrorKind::PermissionDenied
        )
    {
        CliError::invalid_input_with_source(InvalidInputCause::InputPath, message, source)
    } else {
        path_error_with_source(ErrorCode::PermissionDenied, message, source)
    }
}

#[cfg(test)]
#[test]
fn rejects_escape_and_cleans_temp() {
    use pptx_compose::core::error::ErrorCode;
    use std::fs;

    let root = unique_dir();
    let workspace = root.join("workspace");
    let temp_dir = root.join("tmp");
    let outside = root.join("outside");
    fs::create_dir_all(&workspace).expect("workspace fixture dir");
    fs::create_dir_all(&temp_dir).expect("temp fixture dir");
    fs::create_dir_all(&outside).expect("outside fixture dir");

    let outside_input = outside.join("deck.pptx");
    fs::write(&outside_input, b"pptx").expect("outside input fixture");

    let ctx = PermissionContext {
        workspace: fs::canonicalize(&workspace).expect("canonical workspace"),
        temp_dir: fs::canonicalize(&temp_dir).expect("canonical temp"),
    };

    let err = ctx
        .authorize_read(&outside_input, PathIntent::InputPptx)
        .expect_err("outside workspace read must fail");
    assert!(matches!(
        err.code(),
        ErrorCode::PermissionDenied | ErrorCode::UnsafePath
    ));
    let message = err.to_string();
    assert!(message.contains(ctx.workspace.to_string_lossy().as_ref()));
    assert!(message.contains(ctx.temp_dir.to_string_lossy().as_ref()));
    assert!(message.contains("--workspace"));

    let symlink = workspace.join("escape.pptx");
    create_symlink(&outside_input, &symlink);
    let err = ctx
        .authorize_read(&symlink, PathIntent::InputPptx)
        .expect_err("symlink escape must fail");
    assert!(matches!(
        err.code(),
        ErrorCode::PermissionDenied | ErrorCode::UnsafePath
    ));

    let sibling_write = root.join("sibling-output.pptx");
    let err = ctx
        .authorize_write(&sibling_write, PathIntent::OutputPptx)
        .expect_err("sibling output write must fail");
    assert!(matches!(
        err.code(),
        ErrorCode::PermissionDenied | ErrorCode::UnsafePath
    ));

    let err = ctx
        .authorize_write(Path::new("-"), PathIntent::OutputPptx)
        .expect_err("binary output must not authorize stdout");
    assert_eq!(err.code(), ErrorCode::InvalidInput);
    assert_eq!(
        err.invalid_input_cause(),
        Some(crate::InvalidInputCause::CliArgument)
    );
    assert!(
        !workspace.join("-").exists(),
        "rejecting stdout must not create a literal '-' file"
    );
    assert_eq!(
        ctx.authorize_write(Path::new("-"), PathIntent::ReportOutput)
            .expect("JSON report stdout remains authorized"),
        PathBuf::from("-")
    );

    let partial = temp_dir.join("partial.pptx.tmp");
    fs::write(&partial, b"partial").expect("partial temp fixture");
    ctx.cleanup_temp_on_failure(&partial)
        .expect("cleanup should remove temp file");
    assert!(!partial.exists());

    fs::remove_dir_all(root).expect("remove permission test fixture");
}

#[cfg(test)]
fn unique_dir() -> PathBuf {
    use std::time::{SystemTime, UNIX_EPOCH};

    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time after epoch")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "pptx-compose-permissions-{}-{nanos}",
        std::process::id()
    ))
}

#[cfg(all(test, unix))]
fn create_symlink(target: &Path, link: &Path) {
    std::os::unix::fs::symlink(target, link).expect("symlink fixture");
}

#[cfg(all(test, windows))]
fn create_symlink(target: &Path, link: &Path) {
    std::os::windows::fs::symlink_file(target, link).expect("symlink fixture");
}
