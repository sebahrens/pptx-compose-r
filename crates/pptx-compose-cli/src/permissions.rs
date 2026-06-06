use std::{
    fs,
    path::{Component, Path, PathBuf},
};

use pptx_compose::core::error::ErrorCode;

use crate::{CliError, cli::GlobalArgs};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PermissionContext {
    pub(crate) workspace: PathBuf,
    pub(crate) temp_dir: PathBuf,
    pub(crate) keep_temp: bool,
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
            keep_temp: args.keep_temp,
        })
    }

    pub(crate) fn authorize_read(
        &self,
        path: &Path,
        intent: PathIntent,
    ) -> Result<PathBuf, CliError> {
        let candidate = self.anchor_path(path)?;
        let resolved = fs::canonicalize(&candidate).map_err(|source| {
            path_error_with_source(
                ErrorCode::PermissionDenied,
                format!("Could not resolve readable {} path.", intent.label()),
                source,
            )
        })?;

        if self.is_allowed(&resolved) {
            Ok(resolved)
        } else {
            Err(path_error(
                ErrorCode::PermissionDenied,
                format!(
                    "{} path is outside the configured workspace or temp directory.",
                    intent.label()
                ),
            ))
        }
    }

    pub(crate) fn authorize_write(
        &self,
        path: &Path,
        intent: PathIntent,
    ) -> Result<PathBuf, CliError> {
        if intent.allows_stdio() && path == Path::new("-") {
            return Ok(PathBuf::from("-"));
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
                format!(
                    "{} path is outside the configured workspace or temp directory.",
                    intent.label()
                ),
            ))
        }
    }

    #[allow(dead_code)]
    pub(crate) fn cleanup_temp_on_failure(&self, temp_path: &Path) -> Result<(), CliError> {
        if self.keep_temp {
            return Ok(());
        }

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
        matches!(
            self,
            Self::ReportOutput | Self::DiffOutput | Self::OutputPptx
        )
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
        keep_temp: false,
    };

    let err = ctx
        .authorize_read(&outside_input, PathIntent::InputPptx)
        .expect_err("outside workspace read must fail");
    assert!(matches!(
        err.code(),
        ErrorCode::PermissionDenied | ErrorCode::UnsafePath
    ));

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

    let partial = temp_dir.join("partial.pptx.tmp");
    fs::write(&partial, b"partial").expect("partial temp fixture");
    ctx.cleanup_temp_on_failure(&partial)
        .expect("cleanup should remove temp file");
    assert!(!partial.exists());

    let keep_ctx = PermissionContext {
        keep_temp: true,
        ..ctx
    };
    let kept = temp_dir.join("kept.pptx.tmp");
    fs::write(&kept, b"partial").expect("kept temp fixture");
    keep_ctx
        .cleanup_temp_on_failure(&kept)
        .expect("keep-temp cleanup should be a no-op");
    assert!(kept.exists());

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
