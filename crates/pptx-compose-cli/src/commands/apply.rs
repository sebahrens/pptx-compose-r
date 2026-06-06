use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
};

use pptx_compose::{
    ApplyPatchOptions, OpenOptions, PresentationDocument, WriteMode, WriteOptions,
    core::error::{Error, ErrorCode, ErrorLocation},
    edit::{
        media_inputs::{MediaInputs, MediaManifest},
        patch::{Patch, parse_patch},
    },
};

use crate::{
    CliError, InvalidInputCause,
    cli::ApplyArgs,
    output::OutputSink,
    permissions::{PathIntent, PermissionContext},
};

pub(crate) fn apply(
    args: ApplyArgs,
    permissions: &PermissionContext,
    open_options: OpenOptions,
) -> Result<(), CliError> {
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

    let patch = read_patch(&patch)?;
    let media_inputs = read_media_inputs(args.media_manifest.as_deref())?;
    let mut document = PresentationDocument::open_path_with_options(&input, open_options)
        .map_err(CliError::from_error)?;
    if args.dry_run {
        let output = document
            .apply_patch_with_diff(
                patch,
                media_inputs,
                ApplyPatchOptions {
                    dry_run: true,
                    validate: true,
                },
            )
            .map_err(apply_error)?;
        let sink = OutputSink::default();
        sink.emit_patch_report(&output.report, args.report)?;
        sink.emit_diff(&output.diff, args.diff)?;
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
    let in_place_output = same_path(&input, &output);
    enforce_apply_write_guards(&input, &output, args.overwrite, args.in_place)?;

    let apply_output = document
        .apply_patch_with_diff(
            patch,
            media_inputs,
            ApplyPatchOptions {
                dry_run: false,
                validate: true,
            },
        )
        .map_err(apply_error)?;
    let write_options = write_options_from_args(&args);
    let backup = if in_place_output && !args.no_backup {
        let backup = available_backup_path(&input);
        permissions.authorize_write(&backup, PathIntent::OutputPptx)?;
        create_in_place_backup(&input, &backup)?;
        Some(backup)
    } else {
        None
    };
    if let Err(error) = document.write_path_with_options(&output, write_options) {
        if let Some(backup) = &backup {
            restore_in_place_backup(backup, &output)?;
        }
        return Err(CliError::from_error(error));
    }
    let sink = OutputSink::default();
    sink.emit_optional_patch_report(&apply_output.report, args.report)?;
    sink.emit_diff(&apply_output.diff, args.diff)?;

    Ok(())
}

fn read_patch(path: &Path) -> Result<Patch, CliError> {
    let bytes = fs::read(path).map_err(|source| {
        CliError::invalid_input_with_source(
            InvalidInputCause::PatchSchema,
            "Could not read patch JSON input.",
            source,
        )
    })?;
    let value = serde_json::from_slice(&bytes).map_err(|source| {
        CliError::invalid_input_with_source(
            InvalidInputCause::PatchSchema,
            "Patch input is not valid JSON.",
            source,
        )
    })?;
    reject_known_unsupported_operations(&value).map_err(CliError::from_error)?;
    parse_patch(value).map_err(apply_error)
}

fn read_media_inputs(path: Option<&Path>) -> Result<MediaInputs, CliError> {
    let Some(path) = path else {
        return Ok(MediaInputs::default());
    };
    let bytes = fs::read(path).map_err(|source| {
        CliError::invalid_input_with_source(
            InvalidInputCause::CliArgument,
            "Could not read media manifest JSON input.",
            source,
        )
    })?;
    let manifest = serde_json::from_slice::<MediaManifest>(&bytes).map_err(|source| {
        CliError::invalid_input_with_source(
            InvalidInputCause::CliArgument,
            "Media manifest input is not valid JSON.",
            source,
        )
    })?;
    let media_root = path.parent().unwrap_or_else(|| Path::new("."));
    MediaInputs::from_manifest(&manifest, media_root).map_err(CliError::from_error)
}

fn reject_known_unsupported_operations(patch: &serde_json::Value) -> Result<(), Error> {
    let Some(operations) = patch
        .get("operations")
        .and_then(serde_json::Value::as_array)
    else {
        return Ok(());
    };

    for operation in operations {
        let Some(op_name) = operation.get("op").and_then(serde_json::Value::as_str) else {
            continue;
        };
        if matches!(
            op_name,
            "edit_chart" | "replace_chart_data" | "replace_chart" | "update_chart"
        ) {
            let operation_id = operation
                .get("operation_id")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned);
            return Err(Error::new(
                ErrorCode::UnsupportedEdit,
                "Chart editing is not supported by V1 patch operations.",
            )
            .with_location(ErrorLocation {
                operation_id,
                operation: Some(op_name.to_owned()),
                ..ErrorLocation::default()
            })
            .with_suggestion(
                "Leave chart parts unchanged or use raw XML tools when explicitly enabled.",
            ));
        }
    }

    Ok(())
}

pub(crate) fn write_options_from_args(args: &ApplyArgs) -> WriteOptions {
    WriteOptions {
        mode: WriteMode::Deterministic,
        overwrite: args.overwrite || args.in_place,
        validate: true,
        atomic: true,
    }
}

fn apply_error(error: Error) -> CliError {
    if error.code() == pptx_compose::core::error::ErrorCode::InvalidInput {
        CliError::invalid_input_with_source(
            InvalidInputCause::PatchSchema,
            "Patch input failed schema validation.",
            error,
        )
    } else {
        CliError::from_error(error)
    }
}

fn enforce_apply_write_guards(
    input: &Path,
    output: &Path,
    overwrite: bool,
    in_place: bool,
) -> Result<(), CliError> {
    let same_output = same_path(input, output);
    if same_output && !in_place {
        return Err(CliError::new(
            pptx_compose::core::error::ErrorCode::WriteFailed,
            "Output path equals input path; pass --in-place to edit the input path.",
        ));
    }

    if output.exists() && !overwrite && !same_output {
        return Err(CliError::new(
            pptx_compose::core::error::ErrorCode::WriteFailed,
            format!(
                "Output path {} already exists; pass --overwrite to replace it.",
                output.display()
            ),
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

fn create_in_place_backup(input: &Path, backup: &Path) -> Result<(), CliError> {
    copy_file_exclusive(input, backup).map_err(|source| {
        CliError::write_with_source(
            format!("Could not create in-place backup {}.", backup.display()),
            source,
        )
    })?;
    Ok(())
}

fn available_backup_path(input: &Path) -> PathBuf {
    let base = PathBuf::from(format!("{}.bak", input.display()));
    if !base.exists() {
        return base;
    }

    for index in 1.. {
        let candidate = PathBuf::from(format!("{}.bak.{index}", input.display()));
        if !candidate.exists() {
            return candidate;
        }
    }
    unreachable!("unbounded backup suffix search must eventually return")
}

fn copy_file_exclusive(from: &Path, to: &Path) -> io::Result<()> {
    let mut input = fs::File::open(from)?;
    let mut output = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(to)?;
    io::copy(&mut input, &mut output)?;
    output.flush()?;
    output.sync_all()
}

fn restore_in_place_backup(backup: &Path, output: &Path) -> Result<(), CliError> {
    fs::copy(backup, output).map_err(|source| {
        CliError::write_with_source(
            format!(
                "Could not restore in-place output {} from backup {}.",
                output.display(),
                backup.display()
            ),
            source,
        )
    })?;
    Ok(())
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
fn default_apply_selects_deterministic_mode() {
    test_support::default_apply_selects_deterministic_mode();
}

#[cfg(test)]
#[test]
fn repeated_apply_without_flag_is_byte_identical() {
    test_support::repeated_apply_without_flag_is_byte_identical();
}

#[cfg(test)]
#[test]
fn in_place_apply_writes_backup() {
    test_support::in_place_apply_writes_backup();
}

#[cfg(test)]
#[test]
fn in_place_no_backup_suppresses_backup() {
    test_support::in_place_no_backup_suppresses_backup();
}

#[cfg(test)]
#[test]
fn in_place_apply_restores_from_backup_on_write_failure() {
    test_support::in_place_apply_restores_from_backup_on_write_failure();
}

#[cfg(test)]
#[test]
fn dry_run_writes_no_pptx() {
    test_support::dry_run_writes_no_pptx();
}

#[cfg(test)]
#[test]
fn replace_text_writes_mutated_output() {
    test_support::replace_text_writes_mutated_output();
}

#[cfg(test)]
#[test]
fn dry_run_add_image_uses_media_manifest() {
    test_support::dry_run_add_image_uses_media_manifest();
}

#[cfg(test)]
#[test]
fn media_manifest_mismatches_return_structured_errors() {
    test_support::media_manifest_mismatches_return_structured_errors();
}

#[cfg(test)]
mod test_support {
    use std::{fs, io::Cursor, io::Write, path::Path};

    use pptx_compose::{
        OpenOptions, PresentationDocument, WriteMode,
        core::{error::ErrorCode, provenance::checksum::part_checksum, zip::reader::from_bytes},
    };
    use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

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
        fs::write(&patch, valid_noop_patch()).expect("patch fixture writes");
        fs::write(&output, b"original-output").expect("existing output writes");

        let args = args(&input, &patch, &output, false, false);
        let err = apply(args, &permissions(&root), OpenOptions::default())
            .expect_err("existing output must fail");

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
        fs::write(&patch, valid_noop_patch()).expect("patch fixture writes");
        fs::write(&output, b"replace-me").expect("existing output writes");

        let args = args(&input, &patch, &output, true, true);
        let write_options = write_options_from_args(&args);
        assert_eq!(write_options.mode, WriteMode::Deterministic);

        apply(args, &permissions(&root), OpenOptions::default()).expect("overwrite apply succeeds");

        assert_ne!(
            fs::read(&output).expect("output reads"),
            b"replace-me",
            "output should be replaced"
        );

        fs::remove_dir_all(root).expect("test dir removes");
    }

    pub(super) fn default_apply_selects_deterministic_mode() {
        let root = unique_dir();
        let input = root.join("input.pptx");
        let patch = root.join("patch.json");
        let output = root.join("output.pptx");
        fs::create_dir_all(&root).expect("test dir creates");
        fs::write(&input, include_bytes!("../../../../fixtures/minimal.pptx"))
            .expect("input fixture writes");
        fs::write(&patch, valid_noop_patch()).expect("patch fixture writes");

        let args = args(&input, &patch, &output, false, false);
        let write_options = write_options_from_args(&args);
        assert_eq!(write_options.mode, WriteMode::Deterministic);

        fs::remove_dir_all(root).expect("test dir removes");
    }

    pub(super) fn repeated_apply_without_flag_is_byte_identical() {
        let root = unique_dir();
        let input = root.join("input.pptx");
        let patch = root.join("patch.json");
        let first_output = root.join("first.pptx");
        let second_output = root.join("second.pptx");
        fs::create_dir_all(&root).expect("test dir creates");
        let input_bytes = text_deck();
        fs::write(&input, &input_bytes).expect("input fixture writes");
        fs::write(&patch, replace_text_patch(&input_bytes)).expect("patch fixture writes");

        apply(
            args(&input, &patch, &first_output, false, false),
            &permissions(&root),
            OpenOptions::default(),
        )
        .expect("first apply succeeds");
        apply(
            args(&input, &patch, &second_output, false, false),
            &permissions(&root),
            OpenOptions::default(),
        )
        .expect("second apply succeeds");

        assert_eq!(
            fs::read(&first_output).expect("first output reads"),
            fs::read(&second_output).expect("second output reads")
        );

        fs::remove_dir_all(root).expect("test dir removes");
    }

    pub(super) fn in_place_apply_writes_backup() {
        let root = unique_dir();
        let input = root.join("input.pptx");
        let patch = root.join("patch.json");
        fs::create_dir_all(&root).expect("test dir creates");
        let input_bytes = text_deck();
        fs::write(&input, &input_bytes).expect("input fixture writes");
        fs::write(&patch, replace_text_patch(&input_bytes)).expect("patch fixture writes");

        let mut args = args(&input, &patch, &input, false, false);
        args.in_place = true;
        let write_options = write_options_from_args(&args);
        assert!(
            write_options.overwrite,
            "in-place mode permits same-path overwrite"
        );
        apply(args, &permissions(&root), OpenOptions::default()).expect("in-place apply succeeds");

        let backup = root.join("input.pptx.bak");
        assert_eq!(
            fs::read(&backup).expect("backup reads"),
            input_bytes,
            "backup preserves original input bytes"
        );
        assert_ne!(
            fs::read(&input).expect("input reads after apply"),
            input_bytes,
            "input is replaced in-place"
        );

        fs::remove_dir_all(root).expect("test dir removes");
    }

    pub(super) fn in_place_no_backup_suppresses_backup() {
        let root = unique_dir();
        let input = root.join("input.pptx");
        let patch = root.join("patch.json");
        fs::create_dir_all(&root).expect("test dir creates");
        let input_bytes = text_deck();
        fs::write(&input, &input_bytes).expect("input fixture writes");
        fs::write(&patch, replace_text_patch(&input_bytes)).expect("patch fixture writes");

        let mut args = args(&input, &patch, &input, false, false);
        args.in_place = true;
        args.no_backup = true;
        apply(args, &permissions(&root), OpenOptions::default())
            .expect("in-place apply without backup succeeds");

        assert!(
            !root.join("input.pptx.bak").exists(),
            "--no-backup suppresses the sibling backup"
        );
        assert_ne!(
            fs::read(&input).expect("input reads after apply"),
            input_bytes,
            "input is replaced in-place"
        );

        fs::remove_dir_all(root).expect("test dir removes");
    }

    pub(super) fn in_place_apply_restores_from_backup_on_write_failure() {
        let root = unique_dir();
        let input = root.join("input.pptx");
        let patch = root.join("patch.json");
        fs::create_dir_all(&root).expect("test dir creates");
        let input_bytes = text_deck();
        fs::write(&input, &input_bytes).expect("input fixture writes");
        fs::write(&patch, replace_text_patch(&input_bytes)).expect("patch fixture writes");
        precreate_atomic_temp_outputs(&input);

        let mut args = args(&input, &patch, &input, false, false);
        args.in_place = true;
        let err = apply(args, &permissions(&root), OpenOptions::default())
            .expect_err("pre-created atomic temp output forces write failure");

        assert_eq!(err.code(), ErrorCode::WriteFailed);
        assert_eq!(
            fs::read(&input).expect("input reads after failed write"),
            input_bytes,
            "input is restored from backup after write failure"
        );
        assert_eq!(
            fs::read(root.join("input.pptx.bak")).expect("backup reads"),
            input_bytes
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
        fs::write(&patch, valid_noop_patch()).expect("patch fixture writes");

        let mut args = args(&input, &patch, &output, false, false);
        args.dry_run = true;
        args.report = Some(report.clone());
        args.diff = Some(diff.clone());
        args.output = None;

        apply(args, &permissions(&root), OpenOptions::default()).expect("dry-run apply succeeds");

        assert!(!output.exists(), "dry-run must not create a PPTX output");
        let report_json: serde_json::Value =
            serde_json::from_slice(&fs::read(&report).expect("report reads"))
                .expect("report is JSON");
        assert_eq!(report_json["schema"], "pptx-compose.patch_report.v1");
        assert_eq!(report_json["version"], 1);
        assert_eq!(report_json["status"], "dry_run_success");
        assert_eq!(report_json["dry_run"], true);
        assert_eq!(report_json["operation_reports"], serde_json::json!([]));
        assert_eq!(report_json["changed_parts"], serde_json::json!([]));

        let diff_json: serde_json::Value =
            serde_json::from_slice(&fs::read(&diff).expect("diff reads")).expect("diff is JSON");
        assert_eq!(diff_json["schema"], "pptx-compose.semantic_diff.v1");
        assert_eq!(diff_json["version"], 1);
        assert_eq!(diff_json["changes"], serde_json::json!([]));

        fs::remove_dir_all(root).expect("test dir removes");
    }

    pub(super) fn replace_text_writes_mutated_output() {
        let root = unique_dir();
        let input = root.join("input.pptx");
        let patch = root.join("patch.json");
        let output = root.join("output.pptx");
        let report = root.join("report.json");
        let diff = root.join("diff.json");
        fs::create_dir_all(&root).expect("test dir creates");
        let input_bytes = text_deck();
        fs::write(&input, &input_bytes).expect("input fixture writes");
        fs::write(&patch, replace_text_patch(&input_bytes)).expect("patch fixture writes");

        let mut args = args(&input, &patch, &output, false, false);
        args.report = Some(report.clone());
        args.diff = Some(diff.clone());
        apply(args, &permissions(&root), OpenOptions::default())
            .expect("replace_text apply succeeds");

        let output_bytes = fs::read(&output).expect("output reads");
        assert_ne!(output_bytes, input_bytes);
        let output_entries = from_bytes(&output_bytes).expect("output entries read");
        let slide = output_entries
            .iter()
            .find(|entry| entry.name.zip_entry_name() == "ppt/slides/slide1.xml")
            .expect("slide entry exists");
        let slide_xml = std::str::from_utf8(&slide.bytes).expect("slide XML is UTF-8");
        assert!(slide_xml.contains(">Updated title<"));

        let report_json: serde_json::Value =
            serde_json::from_slice(&fs::read(&report).expect("report reads"))
                .expect("report is JSON");
        assert_eq!(report_json["status"], "applied");
        assert_eq!(
            report_json["changed_parts"],
            serde_json::json!(["ppt/slides/slide1.xml"])
        );
        assert_eq!(
            report_json["operation_reports"].as_array().map(Vec::len),
            Some(1)
        );

        let diff_json: serde_json::Value =
            serde_json::from_slice(&fs::read(&diff).expect("diff reads")).expect("diff is JSON");
        assert_eq!(diff_json["schema"], "pptx-compose.semantic_diff.v1");
        assert_eq!(
            diff_json["changed_parts"][0]["part"],
            serde_json::json!("ppt/slides/slide1.xml")
        );
        assert_eq!(
            diff_json["changed_parts"][0]["change_kind"],
            serde_json::json!("modified_xml")
        );
        assert_ne!(
            diff_json["changed_parts"][0]["before_checksum"],
            diff_json["changed_parts"][0]["after_checksum"]
        );
        assert_eq!(diff_json["changes"].as_array().map(Vec::len), Some(1));

        fs::remove_dir_all(root).expect("test dir removes");
    }

    pub(super) fn dry_run_add_image_uses_media_manifest() {
        let root = unique_dir();
        let input = root.join("input.pptx");
        let patch = root.join("patch.json");
        let manifest = root.join("media.json");
        let assets = root.join("assets");
        let image = assets.join("hero.png");
        let output = root.join("output.pptx");
        let report = root.join("report.json");
        fs::create_dir_all(&assets).expect("asset dir creates");
        let input_bytes = text_deck();
        fs::write(&input, &input_bytes).expect("input fixture writes");
        fs::write(&image, png_bytes()).expect("image writes");
        fs::write(&patch, add_image_patch(&input_bytes)).expect("patch fixture writes");
        fs::write(
            &manifest,
            media_manifest(
                "assets/hero.png",
                "image/png",
                Some(part_checksum(png_bytes())),
                None,
            ),
        )
        .expect("manifest writes");

        let mut args = args(&input, &patch, &output, false, false);
        args.dry_run = true;
        args.output = None;
        args.report = Some(report.clone());
        args.media_manifest = Some(manifest);

        apply(args, &permissions(&root), OpenOptions::default())
            .expect("dry-run add_image uses manifest media");

        let report_json: serde_json::Value =
            serde_json::from_slice(&fs::read(&report).expect("report reads"))
                .expect("report is JSON");
        assert_eq!(report_json["status"], "dry_run_success");
        assert!(!output.exists(), "dry-run must not create a PPTX output");

        fs::remove_dir_all(root).expect("test dir removes");
    }

    pub(super) fn media_manifest_mismatches_return_structured_errors() {
        let cases = [
            (
                "sha",
                media_manifest(
                    "assets/hero.png",
                    "image/png",
                    Some(
                        "sha256:0000000000000000000000000000000000000000000000000000000000000000"
                            .to_owned(),
                    ),
                    None,
                ),
                ErrorCode::MediaChecksumMismatch,
            ),
            (
                "content_type",
                media_manifest("assets/hero.png", "image/jpeg", None, None),
                ErrorCode::UnsupportedMediaType,
            ),
            (
                "byte_length",
                media_manifest("assets/hero.png", "image/png", None, Some(1)),
                ErrorCode::InvalidInput,
            ),
        ];

        for (name, manifest_json, expected_code) in cases {
            let root = unique_dir();
            let input = root.join("input.pptx");
            let patch = root.join("patch.json");
            let manifest = root.join("media.json");
            let assets = root.join("assets");
            let image = assets.join("hero.png");
            let output = root.join("output.pptx");
            fs::create_dir_all(&assets).expect("asset dir creates");
            let input_bytes = text_deck();
            fs::write(&input, &input_bytes).expect("input fixture writes");
            fs::write(&image, png_bytes()).expect("image writes");
            fs::write(&patch, add_image_patch(&input_bytes)).expect("patch fixture writes");
            fs::write(&manifest, manifest_json).expect("manifest writes");

            let mut args = args(&input, &patch, &output, false, false);
            args.dry_run = true;
            args.output = None;
            args.media_manifest = Some(manifest);

            let err = match apply(args, &permissions(&root), OpenOptions::default()) {
                Ok(()) => panic!("{name} mismatch must fail"),
                Err(error) => error,
            };
            assert_eq!(err.code(), expected_code, "{name} mismatch");
            assert_ne!(
                err.code(),
                ErrorCode::MissingMediaRef,
                "{name} mismatch should prove manifest binding was used"
            );

            fs::remove_dir_all(root).expect("test dir removes");
        }
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
            no_backup: false,
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

    fn valid_noop_patch() -> Vec<u8> {
        let document_id =
            PresentationDocument::from_bytes(include_bytes!("../../../../fixtures/minimal.pptx"))
                .expect("fixture opens")
                .validate()
                .expect("fixture validates")
                .document_id;
        format!(
            r#"{{
            "schema": "pptx-compose.patch.v1",
            "version": 1,
            "document_id": "{document_id}",
            "base_revision": 1,
            "client_request_id": "apply-test-noop",
            "operations": []
        }}"#
        )
        .into_bytes()
    }

    fn replace_text_patch(bytes: &[u8]) -> Vec<u8> {
        let document_id = PresentationDocument::from_bytes(bytes)
            .expect("fixture opens")
            .validate()
            .expect("fixture validates")
            .document_id;
        format!(
            r#"{{
            "schema": "pptx-compose.patch.v1",
            "version": 1,
            "document_id": "{document_id}",
            "base_revision": 1,
            "client_request_id": "apply-test-replace-text",
            "operations": [{{
                "operation_id": "replace-title",
                "op": "replace_text",
                "element_id": "slide-1:shape-3",
                "text": "Updated title"
            }}]
        }}"#
        )
        .into_bytes()
    }

    fn add_image_patch(bytes: &[u8]) -> Vec<u8> {
        let document_id = PresentationDocument::from_bytes(bytes)
            .expect("fixture opens")
            .validate()
            .expect("fixture validates")
            .document_id;
        format!(
            r#"{{
            "schema": "pptx-compose.patch.v1",
            "version": 1,
            "document_id": "{document_id}",
            "base_revision": 1,
            "client_request_id": "apply-test-add-image",
            "operations": [{{
                "operation_id": "add-hero",
                "op": "add_image",
                "slide_id": "slide-1",
                "media_ref": "hero",
                "content_type": "image/png",
                "bounds": {{ "x": 914400, "y": 914400, "cx": 1828800, "cy": 914400 }}
            }}]
        }}"#
        )
        .into_bytes()
    }

    fn media_manifest(
        path: &str,
        content_type: &str,
        sha256: Option<String>,
        byte_length: Option<u64>,
    ) -> Vec<u8> {
        let sha256 = sha256
            .map(|value| format!(r#", "sha256": "{value}""#))
            .unwrap_or_default();
        let byte_length = byte_length
            .map(|value| format!(r#", "byte_length": {value}"#))
            .unwrap_or_default();
        format!(
            r#"{{
            "schema": "pptx-compose.media_manifest.v1",
            "version": 1,
            "media": {{
                "hero": {{
                    "path": "{path}",
                    "content_type": "{content_type}"{sha256}{byte_length}
                }}
            }}
        }}"#
        )
        .into_bytes()
    }

    fn png_bytes() -> &'static [u8] {
        b"\x89PNG\r\n\x1a\npptx-compose-test-image"
    }

    fn text_deck() -> Vec<u8> {
        zip_entries(
            [
                ("[Content_Types].xml", content_types().as_bytes()),
                ("_rels/.rels", root_rels().as_bytes()),
                ("ppt/presentation.xml", presentation().as_bytes()),
                (
                    "ppt/_rels/presentation.xml.rels",
                    presentation_rels().as_bytes(),
                ),
                ("ppt/slides/slide1.xml", text_slide().as_bytes()),
            ],
            CompressionMethod::Stored,
        )
    }

    fn zip_entries<const N: usize>(
        entries: [(&str, &[u8]); N],
        method: CompressionMethod,
    ) -> Vec<u8> {
        let mut bytes = Vec::new();
        {
            let mut writer = ZipWriter::new(Cursor::new(&mut bytes));
            let options = SimpleFileOptions::default().compression_method(method);
            for (name, data) in entries {
                writer.start_file(name, options).expect("start ZIP entry");
                writer.write_all(data).expect("write ZIP entry");
            }
            writer.finish().expect("finish ZIP");
        }
        bytes
    }

    fn content_types() -> String {
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
  <Default Extension="xml" ContentType="application/xml"/>
  <Override PartName="/ppt/presentation.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml"/>
  <Override PartName="/ppt/slides/slide1.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slide+xml"/>
</Types>"#
            .to_owned()
    }

    fn root_rels() -> String {
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="ppt/presentation.xml"/>
</Relationships>"#
            .to_owned()
    }

    fn presentation() -> String {
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:presentation xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <p:sldIdLst><p:sldId id="256" r:id="rId1"/></p:sldIdLst>
</p:presentation>"#
            .to_owned()
    }

    fn presentation_rels() -> String {
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide" Target="slides/slide1.xml"/>
</Relationships>"#
            .to_owned()
    }

    fn text_slide() -> String {
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main">
  <p:cSld>
    <p:spTree>
      <p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr>
      <p:grpSpPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="0" cy="0"/><a:chOff x="0" y="0"/><a:chExt cx="0" cy="0"/></a:xfrm></p:grpSpPr>
      <p:sp>
        <p:nvSpPr><p:cNvPr id="3" name="Title"/><p:cNvSpPr txBox="1"/><p:nvPr/></p:nvSpPr>
        <p:spPr><a:xfrm><a:off x="914400" y="457200"/><a:ext cx="3657600" cy="914400"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></p:spPr>
        <p:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:t>Original title</a:t></a:r></a:p></p:txBody>
      </p:sp>
    </p:spTree>
  </p:cSld>
</p:sld>"#
            .to_owned()
    }

    fn unique_dir() -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        use std::time::{SystemTime, UNIX_EPOCH};

        static NEXT_ID: AtomicU64 = AtomicU64::new(0);

        let base_nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time after epoch")
            .as_nanos();
        let temp_dir = std::env::temp_dir();

        for _ in 0..100 {
            let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
            let path = temp_dir.join(format!(
                "pptx-compose-apply-{}-{base_nanos}-{id}",
                std::process::id()
            ));
            match fs::create_dir(&path) {
                Ok(()) => return path,
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("test dir creates: {error}"),
            }
        }

        panic!("could not create a unique apply test directory")
    }

    fn precreate_atomic_temp_outputs(output: &Path) {
        let file_name = output
            .file_name()
            .and_then(|name| name.to_str())
            .expect("test output filename is UTF-8");
        for counter in 0..4096 {
            let temp = output.with_file_name(format!(
                ".{file_name}.{}.{}.tmp",
                std::process::id(),
                counter
            ));
            fs::write(temp, b"precreated temp output").expect("temp output writes");
        }
    }
}
