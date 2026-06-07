use std::{
    fmt, fs, io,
    path::{Component, Path, PathBuf},
};

use pptx_compose::core::error::{Error as CoreError, ErrorCode, ErrorLocation};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PermissionPolicy {
    pub workspace_root: PathBuf,
    pub temp_dir: PathBuf,
    pub allow_overwrite: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PermissionOperation {
    Read,
    Write,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PermissionError {
    operation: PermissionOperation,
    path: PathBuf,
    reason: PermissionDeniedReason,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum PermissionDeniedReason {
    HomeAlias,
    ParentDir,
    EmptyPath,
    OutsideAllowedRoots,
    MissingParent,
    SilentOverwrite,
    Io(String),
}

impl PermissionPolicy {
    #[must_use]
    pub fn new(workspace_root: PathBuf, temp_dir: PathBuf, allow_overwrite: bool) -> Self {
        Self {
            workspace_root,
            temp_dir,
            allow_overwrite,
        }
    }

    pub fn check_read(&self, path: impl AsRef<Path>) -> Result<PathBuf, PermissionError> {
        let path = path.as_ref();
        let resolved_path = self.resolve_existing(path, PermissionOperation::Read)?;
        self.require_under_allowed_roots(path, resolved_path, PermissionOperation::Read)
    }

    pub fn check_write(&self, path: impl AsRef<Path>) -> Result<PathBuf, PermissionError> {
        self.check_write_with_overwrite(path, self.allow_overwrite)
    }

    pub fn check_write_with_overwrite(
        &self,
        path: impl AsRef<Path>,
        allow_overwrite: bool,
    ) -> Result<PathBuf, PermissionError> {
        let path = path.as_ref();
        let resolved_path = self.resolve_for_write(path)?;
        let allowed_path =
            self.require_under_allowed_roots(path, resolved_path, PermissionOperation::Write)?;

        if allowed_path.exists() && !allow_overwrite {
            return Err(PermissionError::new(
                PermissionOperation::Write,
                path,
                PermissionDeniedReason::SilentOverwrite,
            ));
        }

        Ok(allowed_path)
    }

    fn require_under_allowed_roots(
        &self,
        original_path: &Path,
        resolved_path: PathBuf,
        operation: PermissionOperation,
    ) -> Result<PathBuf, PermissionError> {
        let workspace_root = canonicalize_configured_root(&self.workspace_root, operation)?;
        let temp_dir = canonicalize_configured_root(&self.temp_dir, operation)?;

        if resolved_path.starts_with(&workspace_root) || resolved_path.starts_with(&temp_dir) {
            return Ok(resolved_path);
        }

        Err(PermissionError::new(
            operation,
            original_path,
            PermissionDeniedReason::OutsideAllowedRoots,
        ))
    }

    fn resolve_existing(
        &self,
        path: &Path,
        operation: PermissionOperation,
    ) -> Result<PathBuf, PermissionError> {
        let anchored_path = self.anchor_path(path, operation)?;
        fs::canonicalize(&anchored_path).map_err(|error| {
            PermissionError::new(
                operation,
                path,
                PermissionDeniedReason::Io(error.to_string()),
            )
        })
    }

    fn resolve_for_write(&self, path: &Path) -> Result<PathBuf, PermissionError> {
        let anchored_path = self.anchor_path(path, PermissionOperation::Write)?;

        if anchored_path.exists() {
            return fs::canonicalize(&anchored_path).map_err(|error| {
                PermissionError::new(
                    PermissionOperation::Write,
                    path,
                    PermissionDeniedReason::Io(error.to_string()),
                )
            });
        }

        let Some(parent) = anchored_path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        else {
            return Err(PermissionError::new(
                PermissionOperation::Write,
                path,
                PermissionDeniedReason::MissingParent,
            ));
        };
        let Some(file_name) = path.file_name() else {
            return Err(PermissionError::new(
                PermissionOperation::Write,
                path,
                PermissionDeniedReason::MissingParent,
            ));
        };

        let resolved_parent = fs::canonicalize(parent).map_err(|error| {
            let reason = if error.kind() == io::ErrorKind::NotFound {
                PermissionDeniedReason::MissingParent
            } else {
                PermissionDeniedReason::Io(error.to_string())
            };
            PermissionError::new(PermissionOperation::Write, path, reason)
        })?;

        Ok(resolved_parent.join(file_name))
    }

    fn anchor_path(
        &self,
        path: &Path,
        operation: PermissionOperation,
    ) -> Result<PathBuf, PermissionError> {
        reject_unsafe_syntax(path, operation)?;

        if path.is_absolute() {
            Ok(path.to_path_buf())
        } else {
            Ok(self.workspace_root.join(path))
        }
    }
}

impl Default for PermissionPolicy {
    fn default() -> Self {
        let workspace_root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let temp_dir = std::env::temp_dir();
        Self::new(workspace_root, temp_dir, false)
    }
}

impl PermissionError {
    fn new(
        operation: PermissionOperation,
        path: impl AsRef<Path>,
        reason: PermissionDeniedReason,
    ) -> Self {
        Self {
            operation,
            path: path.as_ref().to_path_buf(),
            reason,
        }
    }

    #[must_use]
    pub fn operation(&self) -> PermissionOperation {
        self.operation
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    #[must_use]
    pub fn into_core_error(self) -> CoreError {
        CoreError::new(ErrorCode::PermissionDenied, self.to_string())
            .with_location(ErrorLocation {
                io_path: Some(self.path.display().to_string()),
                ..ErrorLocation::default()
            })
            .with_suggestion(
                "Use a path under the configured workspace or temp directory, and pass overwrite:true when replacing an existing export.",
            )
    }
}

impl fmt::Display for PermissionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let operation = match self.operation {
            PermissionOperation::Read => "read",
            PermissionOperation::Write => "write",
        };
        let reason = match &self.reason {
            PermissionDeniedReason::HomeAlias => {
                "home-directory aliases are not allowed in MCP file paths".to_owned()
            }
            PermissionDeniedReason::ParentDir => {
                "parent-directory segments are not allowed in MCP file paths".to_owned()
            }
            PermissionDeniedReason::EmptyPath => "empty paths are not allowed".to_owned(),
            PermissionDeniedReason::OutsideAllowedRoots => {
                "resolved path is outside the configured workspace and temp directory".to_owned()
            }
            PermissionDeniedReason::MissingParent => {
                "write path parent directory does not exist".to_owned()
            }
            PermissionDeniedReason::SilentOverwrite => {
                "output already exists and overwrite:true was not provided".to_owned()
            }
            PermissionDeniedReason::Io(message) => {
                format!("path could not be resolved before permission check: {message}")
            }
        };

        write!(
            formatter,
            "Permission denied for {operation} path {}: {reason}.",
            self.path.display()
        )
    }
}

impl std::error::Error for PermissionError {}

fn canonicalize_configured_root(
    path: &Path,
    operation: PermissionOperation,
) -> Result<PathBuf, PermissionError> {
    reject_unsafe_syntax(path, operation)?;
    fs::canonicalize(path).map_err(|error| {
        PermissionError::new(
            operation,
            path,
            PermissionDeniedReason::Io(error.to_string()),
        )
    })
}

fn reject_unsafe_syntax(
    path: &Path,
    operation: PermissionOperation,
) -> Result<(), PermissionError> {
    if path.as_os_str().is_empty() {
        return Err(PermissionError::new(
            operation,
            path,
            PermissionDeniedReason::EmptyPath,
        ));
    }

    if path
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(PermissionError::new(
            operation,
            path,
            PermissionDeniedReason::ParentDir,
        ));
    }

    if path.components().any(|component| {
        matches!(component, Component::Normal(value) if value.to_string_lossy().starts_with('~'))
    }) {
        return Err(PermissionError::new(
            operation,
            path,
            PermissionDeniedReason::HomeAlias,
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_paths_resolve_under_workspace() {
        let fixture = TestDirs::new("relative_paths_resolve_under_workspace");
        let deck = fixture.workspace.join("deck.pptx");
        let output_dir = fixture.workspace.join("out");
        fs::write(&deck, b"pptx").expect("write deck fixture");
        fs::create_dir_all(&output_dir).expect("create output dir");

        let policy = PermissionPolicy::new(fixture.workspace.clone(), fixture.temp.clone(), false);

        assert_eq!(
            policy.check_read("deck.pptx").expect("relative read"),
            fs::canonicalize(&deck).expect("canonical deck")
        );
        assert_eq!(
            policy
                .check_write("out/edited.pptx")
                .expect("relative write"),
            fs::canonicalize(&output_dir)
                .expect("canonical output dir")
                .join("edited.pptx")
        );
    }

    #[test]
    fn relative_paths_reject_parent_dir_escapes() {
        let fixture = TestDirs::new("relative_paths_reject_parent_dir_escapes");
        let policy = PermissionPolicy::new(fixture.workspace.clone(), fixture.temp.clone(), false);

        let read_error = policy
            .check_read("../deck.pptx")
            .expect_err("relative read escape is rejected");
        let write_error = policy
            .check_write("out/../deck.pptx")
            .expect_err("relative write escape is rejected");

        assert_eq!(read_error.operation(), PermissionOperation::Read);
        assert_eq!(write_error.operation(), PermissionOperation::Write);
    }

    #[test]
    fn rejects_path_outside_workspace() {
        let fixture = TestDirs::new("rejects_path_outside_workspace");
        let workspace_file = fixture.workspace.join("deck.pptx");
        fs::write(&workspace_file, b"pptx").expect("write workspace fixture");

        let policy = PermissionPolicy::new(fixture.workspace.clone(), fixture.temp.clone(), false);
        let escaped = fixture.workspace.join("..").join("outside.pptx");
        let escaped_error = policy
            .check_write(&escaped)
            .expect_err("parent-dir escape is rejected");

        assert_eq!(escaped_error.operation(), PermissionOperation::Write);

        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;

            let outside_file = fixture.root.join("outside.pptx");
            fs::write(&outside_file, b"outside").expect("write outside fixture");
            let symlink_path = fixture.workspace.join("linked-outside.pptx");
            symlink(&outside_file, &symlink_path).expect("create escape symlink");

            let symlink_error = policy
                .check_write(&symlink_path)
                .expect_err("symlink escape is rejected");
            assert_eq!(symlink_error.operation(), PermissionOperation::Write);
        }

        assert_eq!(
            policy
                .check_write(&workspace_file)
                .expect_err("no overwrite"),
            PermissionError::new(
                PermissionOperation::Write,
                &workspace_file,
                PermissionDeniedReason::SilentOverwrite
            )
        );
    }

    #[test]
    fn export_no_silent_overwrite() {
        let fixture = TestDirs::new("export_no_silent_overwrite");
        let output = fixture.workspace.join("output.pptx");
        fs::write(&output, b"existing").expect("write output fixture");

        let policy = PermissionPolicy::new(fixture.workspace.clone(), fixture.temp.clone(), false);
        let error = policy
            .check_write_with_overwrite(&output, false)
            .expect_err("existing export requires overwrite");

        assert_eq!(
            error,
            PermissionError::new(
                PermissionOperation::Write,
                &output,
                PermissionDeniedReason::SilentOverwrite
            )
        );
        assert_eq!(
            policy
                .check_write_with_overwrite(&output, true)
                .expect("explicit overwrite is allowed"),
            fs::canonicalize(output).expect("canonical output")
        );
    }

    struct TestDirs {
        root: PathBuf,
        workspace: PathBuf,
        temp: PathBuf,
    }

    impl TestDirs {
        fn new(test_name: &str) -> Self {
            let root = std::env::temp_dir().join(format!(
                "pptx-compose-mcp-{test_name}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .expect("system time")
                    .as_nanos()
            ));
            let workspace = root.join("workspace");
            let temp = root.join("temp");

            fs::create_dir_all(&workspace).expect("create workspace");
            fs::create_dir_all(&temp).expect("create temp dir");

            Self {
                root,
                workspace,
                temp,
            }
        }
    }

    impl Drop for TestDirs {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }
}
