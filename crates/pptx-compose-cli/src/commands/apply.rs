use std::{fs, path::Path};

use pptx_compose::{ApplyPatchOptions, PresentationDocument, WriteMode, WriteOptions};

use crate::{
    CliError, InvalidInputCause,
    cli::ApplyArgs,
    output::OutputSink,
    permissions::{PathIntent, PermissionContext},
};

pub(crate) fn apply(args: ApplyArgs, permissions: &PermissionContext) -> Result<(), CliError> {
    let input = permissions.authorize_read(&args.input, PathIntent::InputPptx)?;
    let patch = permissions.authorize_read(&args.patch, PathIntent::InputPptx)?;
    if let Some(manifest) = &args.media_manifest {
        permissions.authorize_read(manifest, PathIntent::MediaInput)?;
    }
    if let Some(report) = &args.report {
        permissions.authorize_write(report, PathIntent::ReportOutput)?;
    }
    if let Some(diff) = &args.diff {
        permissions.authorize_write(diff, PathIntent::DiffOutput)?;
    }

    let patch = read_patch_json(&patch)?;
    let document = PresentationDocument::open_path(&input).map_err(CliError::from_error)?;
    if args.dry_run {
        let result = document
            .apply_patch_with_options(
                &patch,
                ApplyPatchOptions {
                    dry_run: true,
                    validate: true,
                },
            )
            .map_err(CliError::from_error)?;
        let sink = OutputSink::default();
        sink.emit_patch_report(&result.report, args.report)?;
        sink.emit_diff(&result.diff, args.diff)?;
        return Ok(());
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
    enforce_apply_write_guards(&input, &output, args.overwrite, args.in_place)?;

    let apply_result = document
        .apply_patch_with_options(
            &patch,
            ApplyPatchOptions {
                dry_run: false,
                validate: true,
            },
        )
        .map_err(CliError::from_error)?;
    let write_options = write_options_from_args(&args);
    document
        .write_path_with_options(&output, write_options)
        .map_err(CliError::from_error)?;
    OutputSink::default().emit_optional_patch_report(&apply_result.report, args.report)?;

    Ok(())
}

fn read_patch_json(path: &Path) -> Result<serde_json::Value, CliError> {
    let bytes = fs::read(path).map_err(|source| {
        CliError::invalid_input_with_source(
            InvalidInputCause::PatchSchema,
            "Could not read patch JSON input.",
            source,
        )
    })?;
    serde_json::from_slice(&bytes).map_err(|source| {
        CliError::invalid_input_with_source(
            InvalidInputCause::PatchSchema,
            "Patch input is not valid JSON.",
            source,
        )
    })
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
#[test]
fn dry_run_writes_no_pptx() {
    test_support::dry_run_writes_no_pptx();
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
        fs::write(&patch, br#"{"operations":[]}"#).expect("patch fixture writes");
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
        fs::write(&patch, br#"{"operations":[]}"#).expect("patch fixture writes");
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

    pub(super) fn dry_run_writes_no_pptx() {
        let root = unique_dir();
        let input = root.join("input.pptx");
        let patch = root.join("patch.json");
        let output = root.join("output.pptx");
        let report = root.join("report.json");
        let diff = root.join("diff.json");
        fs::create_dir_all(&root).expect("test dir creates");
        fs::write(&input, include_bytes!("../../../../fixtures/minimal.pptx"))
            .expect("input fixture writes");
        fs::write(&patch, br#"{"operations":[]}"#).expect("patch fixture writes");

        let mut args = args(&input, &patch, &output, false, false);
        args.dry_run = true;
        args.report = Some(report.clone());
        args.diff = Some(diff.clone());
        args.output = None;

        apply(args, &permissions(&root)).expect("dry-run apply succeeds");

        assert!(!output.exists(), "dry-run must not create a PPTX output");
        let report_json: serde_json::Value =
            serde_json::from_slice(&fs::read(&report).expect("report reads"))
                .expect("report is JSON");
        assert_eq!(report_json["schema"], "pptx-compose.patch_report.v1");
        assert_eq!(report_json["version"], 1);
        assert_eq!(report_json["status"], "dry_run_success");
        assert_eq!(report_json["dry_run"], true);

        let diff_json: serde_json::Value =
            serde_json::from_slice(&fs::read(&diff).expect("diff reads")).expect("diff is JSON");
        assert_eq!(diff_json["schema"], "pptx-compose.semantic_diff.v1");
        assert_eq!(diff_json["version"], 1);
        assert_eq!(diff_json["changes"], serde_json::json!([]));

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
