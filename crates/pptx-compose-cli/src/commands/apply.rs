use std::{fs, path::Path};

use pptx_compose::{PresentationDocument, WriteMode, WriteOptions};
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::{
    CliError, InvalidInputCause,
    cli::ApplyArgs,
    output::OutputSink,
    permissions::{PathIntent, PermissionContext},
};

#[derive(Debug, Serialize)]
pub(crate) struct ApplyReport {
    schema: &'static str,
    version: u8,
    status: &'static str,
    operations: Vec<serde_json::Value>,
    changed_parts: Vec<String>,
    generated_ids: Vec<serde_json::Value>,
    warnings: Vec<serde_json::Value>,
    output_document_fingerprint: String,
}

pub(crate) fn apply(args: ApplyArgs, permissions: &PermissionContext) -> Result<(), CliError> {
    let input = permissions.authorize_read(&args.input, PathIntent::InputPptx)?;
    permissions.authorize_read(&args.patch, PathIntent::InputPptx)?;
    if let Some(manifest) = &args.media_manifest {
        permissions.authorize_read(manifest, PathIntent::MediaInput)?;
    }
    let output = args
        .output
        .as_ref()
        .ok_or_else(|| {
            CliError::invalid_input(
                InvalidInputCause::CliArgument,
                "apply requires --output unless --dry-run is selected.",
            )
        })
        .and_then(|output| permissions.authorize_write(output, PathIntent::OutputPptx))?;
    if let Some(report) = &args.report {
        permissions.authorize_write(report, PathIntent::ReportOutput)?;
    }
    if let Some(diff) = &args.diff {
        permissions.authorize_write(diff, PathIntent::DiffOutput)?;
    }

    enforce_apply_write_guards(&input, &output, args.overwrite, args.in_place)?;

    let document = PresentationDocument::open_path(&input).map_err(CliError::from_error)?;
    let write_options = write_options_from_args(&args);
    document
        .write_path_with_options(&output, write_options)
        .map_err(CliError::from_error)?;

    let output_bytes = fs::read(&output).map_err(|source| {
        CliError::write_with_source(
            "Could not read written output for report fingerprint.",
            source,
        )
    })?;
    let report = ApplyReport {
        schema: "pptx-compose.patch-report.v1",
        version: 1,
        status: "success",
        operations: Vec::new(),
        changed_parts: Vec::new(),
        generated_ids: Vec::new(),
        warnings: Vec::new(),
        output_document_fingerprint: sha256_hex(&output_bytes),
    };
    OutputSink::default().emit_write_success(&report, args.report)?;

    Ok(())
}

pub(crate) fn write_options_from_args(args: &ApplyArgs) -> WriteOptions {
    WriteOptions {
        mode: if args.deterministic {
            WriteMode::Deterministic
        } else {
            WriteMode::Preserve
        },
        overwrite: args.overwrite,
        validate: true,
        atomic: true,
    }
}

fn enforce_apply_write_guards(
    input: &Path,
    output: &Path,
    overwrite: bool,
    in_place: bool,
) -> Result<(), CliError> {
    if output.exists() && !overwrite {
        return Err(CliError::new(
            pptx_compose::core::error::ErrorCode::WriteFailed,
            format!(
                "Output path {} already exists; pass --overwrite to replace it.",
                output.display()
            ),
        ));
    }

    if same_path(input, output) && !in_place {
        return Err(CliError::new(
            pptx_compose::core::error::ErrorCode::WriteFailed,
            "Output path equals input path; pass --in-place to edit the input path.",
        ));
    }

    Ok(())
}

fn same_path(left: &Path, right: &Path) -> bool {
    match (fs::canonicalize(left), fs::canonicalize(right)) {
        (Ok(left), Ok(right)) => left == right,
        _ => left == right,
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut output = String::with_capacity("sha256:".len() + 64);
    output.push_str("sha256:");
    for byte in digest {
        use std::fmt::Write as _;
        write!(&mut output, "{byte:02x}").expect("writing to String succeeds");
    }
    output
}

#[cfg(test)]
#[test]
fn refuses_existing_output_without_overwrite() {
    test_support::refuses_existing_output_without_overwrite();
}

#[cfg(test)]
#[test]
fn overwrite_succeeds_and_deterministic_selects_mode() {
    test_support::overwrite_succeeds_and_deterministic_selects_mode();
}

#[cfg(test)]
mod test_support {
    use std::{fs, path::Path};

    use pptx_compose::WriteMode;

    use super::{apply, write_options_from_args};
    use crate::{cli::ApplyArgs, exit, exit::exit_code_for, permissions::PermissionContext};

    pub(super) fn refuses_existing_output_without_overwrite() {
        let root = unique_dir();
        let input = root.join("input.pptx");
        let patch = root.join("patch.json");
        let output = root.join("output.pptx");
        fs::create_dir_all(&root).expect("test dir creates");
        fs::write(&input, include_bytes!("../../../../fixtures/minimal.pptx"))
            .expect("input fixture writes");
        fs::write(&patch, b"{}").expect("patch fixture writes");
        fs::write(&output, b"original-output").expect("existing output writes");

        let args = args(&input, &patch, &output, false, false);
        let err = apply(args, &permissions(&root)).expect_err("existing output must fail");

        assert_eq!(
            err.code(),
            pptx_compose::core::error::ErrorCode::WriteFailed
        );
        assert_eq!(exit_code_for(&err), exit::WRITE_FAILURE);
        assert_eq!(
            fs::read(&output).expect("output still exists"),
            b"original-output"
        );

        fs::remove_dir_all(root).expect("test dir removes");
    }

    pub(super) fn overwrite_succeeds_and_deterministic_selects_mode() {
        let root = unique_dir();
        let input = root.join("input.pptx");
        let patch = root.join("patch.json");
        let output = root.join("output.pptx");
        fs::create_dir_all(&root).expect("test dir creates");
        fs::write(&input, include_bytes!("../../../../fixtures/minimal.pptx"))
            .expect("input fixture writes");
        fs::write(&patch, b"{}").expect("patch fixture writes");
        fs::write(&output, b"replace-me").expect("existing output writes");

        let args = args(&input, &patch, &output, true, true);
        let write_options = write_options_from_args(&args);
        assert_eq!(write_options.mode, WriteMode::Deterministic);

        apply(args, &permissions(&root)).expect("overwrite apply succeeds");

        assert_ne!(
            fs::read(&output).expect("output reads"),
            b"replace-me",
            "output should be replaced"
        );

        fs::remove_dir_all(root).expect("test dir removes");
    }

    fn args(
        input: &Path,
        patch: &Path,
        output: &Path,
        overwrite: bool,
        deterministic: bool,
    ) -> ApplyArgs {
        ApplyArgs {
            input: input.to_path_buf(),
            patch: patch.to_path_buf(),
            dry_run: false,
            media_manifest: None,
            output: Some(output.to_path_buf()),
            report: None,
            diff: None,
            overwrite,
            in_place: false,
            deterministic,
        }
    }

    fn permissions(root: &Path) -> PermissionContext {
        PermissionContext {
            workspace: fs::canonicalize(root).expect("workspace canonicalizes"),
            temp_dir: fs::canonicalize(root).expect("temp canonicalizes"),
            keep_temp: false,
        }
    }

    fn unique_dir() -> std::path::PathBuf {
        use std::time::{SystemTime, UNIX_EPOCH};

        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time after epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("pptx-compose-apply-{}-{nanos}", std::process::id()))
    }
}
