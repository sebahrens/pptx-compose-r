use std::{
    collections::HashSet,
    error::Error as StdError,
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
};

use pptx_compose::{
    ApplyPatchOptions, OpenOptions, PresentationDocument, WriteMode, WriteOptions,
    core::error::{Error, ErrorCode, ErrorLocation},
    edit::{
        media_inputs::{MediaInputs, MediaLimits, MediaManifest},
        patch::{Patch, parse_patch},
    },
    json::schemas::{ErrorView, PatchStatus},
    temp_output_path,
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
    enforce_single_apply_stdout_json(&args)?;

    let input = permissions.authorize_read(&args.input, PathIntent::InputPptx)?;
    let patch = permissions.authorize_read(&args.patch, PathIntent::InputPptx)?;
    let media_manifest = args
        .media_manifest
        .as_ref()
        .map(|manifest| permissions.authorize_read(manifest, PathIntent::MediaInput))
        .transpose()?;
    let report = args
        .report
        .as_ref()
        .map(|report| permissions.authorize_write(report, PathIntent::ReportOutput))
        .transpose()?;
    let diff = args
        .diff
        .as_ref()
        .map(|diff| permissions.authorize_write(diff, PathIntent::DiffOutput))
        .transpose()?;
    enforce_secondary_output_write_guards(report.as_deref(), diff.as_deref(), args.overwrite)?;
    let sink = OutputSink::default().with_atomic_temp_dir(permissions.temp_dir.clone(), false);

    let patch = read_patch(&patch)?;
    let media_inputs = read_media_inputs(
        media_manifest.as_deref(),
        args.media_root.as_deref(),
        &args.media,
        permissions,
        MediaLimits {
            max_media_bytes: open_options.resource_limits.max_media_part_bytes,
        },
    )?;
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
        sink.emit_patch_report(&output.report, report, args.overwrite)?;
        sink.emit_diff(&output.diff, diff, args.overwrite)?;
        if output.report.status == PatchStatus::DryRunFailed {
            let error = output_operation_error(&output.report, true);
            return Err(apply_error(error));
        }
        return Ok(());
    }

    let output = resolve_apply_output(&args, &input, permissions)?;
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
    if apply_output.report.status == PatchStatus::Failed {
        sink.emit_patch_report(&apply_output.report, report, args.overwrite)?;
        sink.emit_diff(&apply_output.diff, diff, args.overwrite)?;
        let error = output_operation_error(&apply_output.report, false);
        return Err(apply_error(error));
    }
    let mut write_options = write_options_from_args(&args);
    if write_options.atomic {
        let temp_path = temp_output_path(&output, Some(&permissions.temp_dir));
        write_options.atomic_temp_path =
            Some(permissions.authorize_write(&temp_path, PathIntent::TempFile)?);
    }
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
    sink.emit_optional_patch_report(&apply_output.report, report, args.overwrite)?;
    sink.emit_diff(&apply_output.diff, diff, args.overwrite)?;

    Ok(())
}

fn enforce_single_apply_stdout_json(args: &ApplyArgs) -> Result<(), CliError> {
    let mut stdout_outputs = Vec::new();

    if args.dry_run {
        match &args.report {
            Some(report) if report == Path::new("-") => stdout_outputs.push("--report -"),
            None => stdout_outputs.push("omitted --report"),
            Some(_) => {}
        }
    } else if args.report.as_deref() == Some(Path::new("-")) {
        stdout_outputs.push("--report -");
    }

    if args.diff.as_deref() == Some(Path::new("-")) {
        stdout_outputs.push("--diff -");
    }

    if stdout_outputs.len() > 1 {
        return Err(CliError::invalid_input(
            InvalidInputCause::CliArgument,
            format!(
                "apply may emit at most one machine-readable JSON document to stdout; {} resolve to stdout.",
                stdout_outputs.join(" and ")
            ),
        ));
    }

    Ok(())
}

fn resolve_apply_output(
    args: &ApplyArgs,
    input: &Path,
    permissions: &PermissionContext,
) -> Result<PathBuf, CliError> {
    match (&args.output, args.in_place) {
        (Some(output), true) if !same_path(input, output) => Err(CliError::invalid_input(
            InvalidInputCause::CliArgument,
            "--in-place cannot be combined with a different --output path.",
        )),
        (Some(output), _) => permissions.authorize_write(output, PathIntent::OutputPptx),
        (None, true) => permissions.authorize_write(input, PathIntent::OutputPptx),
        (None, false) => Err(CliError::invalid_input(
            InvalidInputCause::CliArgument,
            "apply requires --output unless --dry-run or --in-place is selected.",
        )),
    }
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

fn read_media_inputs(
    path: Option<&Path>,
    media_root: Option<&Path>,
    explicit_media: &[String],
    permissions: &PermissionContext,
    limits: MediaLimits,
) -> Result<MediaInputs, CliError> {
    let mut inputs = if let Some(path) = path {
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
        let root = resolve_media_root(path, media_root, permissions)?;
        MediaInputs::from_manifest_with_limits_and_path_resolver(
            &manifest,
            &root,
            limits,
            |root, manifest_path| {
                resolve_cli_manifest_media_path(root, manifest_path, permissions)
                    .map_err(CliError::into_error)
            },
        )
        .map_err(CliError::from_error)?
    } else {
        if media_root.is_some() {
            return Err(CliError::invalid_input(
                InvalidInputCause::CliArgument,
                "--media-root requires --media-manifest.",
            ));
        }
        MediaInputs::with_limits(Default::default(), limits)
    };

    apply_explicit_media_bindings(&mut inputs, explicit_media, permissions, limits)?;
    Ok(inputs)
}

fn resolve_media_root(
    manifest_path: &Path,
    media_root: Option<&Path>,
    permissions: &PermissionContext,
) -> Result<PathBuf, CliError> {
    match media_root {
        Some(root) => permissions.authorize_read(root, PathIntent::MediaInput),
        None => Ok(manifest_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf()),
    }
}

fn apply_explicit_media_bindings(
    inputs: &mut MediaInputs,
    explicit_media: &[String],
    permissions: &PermissionContext,
    limits: MediaLimits,
) -> Result<(), CliError> {
    let mut seen = HashSet::new();
    for binding in explicit_media {
        let (media_ref, path) = parse_explicit_media_binding(binding)?;
        if !seen.insert(media_ref.to_owned()) {
            return Err(CliError::invalid_input(
                InvalidInputCause::CliArgument,
                format!("Duplicate explicit --media binding for media_ref `{media_ref}`."),
            ));
        }

        let resolved = permissions.authorize_read(Path::new(path), PathIntent::MediaInput)?;
        let media_binding = MediaInputs::sniffed_path_binding(media_ref, &resolved, limits)
            .map_err(CliError::from_error)?;
        inputs.insert_or_replace(media_ref.to_owned(), media_binding);
    }

    Ok(())
}

fn parse_explicit_media_binding(binding: &str) -> Result<(&str, &str), CliError> {
    let Some((media_ref, path)) = binding.split_once('=') else {
        return Err(CliError::invalid_input(
            InvalidInputCause::CliArgument,
            "--media must use MEDIA_REF=PATH.",
        ));
    };
    if media_ref.is_empty() || path.is_empty() {
        return Err(CliError::invalid_input(
            InvalidInputCause::CliArgument,
            "--media requires both MEDIA_REF and PATH.",
        ));
    }
    Ok((media_ref, path))
}

fn resolve_cli_manifest_media_path(
    media_root: &Path,
    manifest_path: &Path,
    permissions: &PermissionContext,
) -> Result<PathBuf, CliError> {
    let resolved =
        pptx_compose::edit::media_inputs::resolve_manifest_media_path(media_root, manifest_path)
            .map_err(CliError::from_error)?;
    permissions.authorize_read(&resolved, PathIntent::MediaInput)
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
                "Leave chart parts unchanged; chart editing is unsupported in V1 patch operations.",
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
        ..WriteOptions::default()
    }
}

fn apply_error(error: Error) -> CliError {
    if error.code() == pptx_compose::core::error::ErrorCode::InvalidInput {
        let message = if let Some(source) = StdError::source(&error) {
            format!("{}: {source}", error.message())
        } else {
            error.message().to_owned()
        };
        CliError::invalid_input_with_source(InvalidInputCause::PatchSchema, message, error)
    } else {
        CliError::from_error(error)
    }
}

fn output_operation_error(
    report: &pptx_compose::json::schemas::PatchReport,
    dry_run: bool,
) -> Error {
    report
        .operation_reports
        .iter()
        .filter_map(|operation| operation.error.as_ref())
        .next()
        .map(operation_error)
        .unwrap_or_else(|| {
            Error::new(
                ErrorCode::ValidationFailed,
                if dry_run {
                    "Dry-run patch validation failed; inspect the patch report for per-operation errors."
                } else {
                    "Patch application failed; inspect the patch report for per-operation errors."
                },
            )
        })
}

fn operation_error(error: &ErrorView) -> Error {
    Error::new(core_error_code(error.code), error.message.clone())
        .with_location(error_location(&error.location))
}

const fn core_error_code(code: pptx_compose::json::schemas::ErrorCode) -> ErrorCode {
    match code {
        pptx_compose::json::schemas::ErrorCode::InvalidInput => ErrorCode::InvalidInput,
        pptx_compose::json::schemas::ErrorCode::UnsafePath => ErrorCode::UnsafePath,
        pptx_compose::json::schemas::ErrorCode::ResourceLimitExceeded => {
            ErrorCode::ResourceLimitExceeded
        }
        pptx_compose::json::schemas::ErrorCode::UnsupportedPackage => ErrorCode::UnsupportedPackage,
        pptx_compose::json::schemas::ErrorCode::UnsupportedEdit => ErrorCode::UnsupportedEdit,
        pptx_compose::json::schemas::ErrorCode::UnsupportedMediaType => {
            ErrorCode::UnsupportedMediaType
        }
        pptx_compose::json::schemas::ErrorCode::InvalidBounds => ErrorCode::InvalidBounds,
        pptx_compose::json::schemas::ErrorCode::ParseError => ErrorCode::ParseError,
        pptx_compose::json::schemas::ErrorCode::MalformedXml => ErrorCode::MalformedXml,
        pptx_compose::json::schemas::ErrorCode::ValidationFailed => ErrorCode::ValidationFailed,
        pptx_compose::json::schemas::ErrorCode::StalePatch => ErrorCode::StalePatch,
        pptx_compose::json::schemas::ErrorCode::SelectorNotFound => ErrorCode::SelectorNotFound,
        pptx_compose::json::schemas::ErrorCode::SelectorAmbiguous => ErrorCode::SelectorAmbiguous,
        pptx_compose::json::schemas::ErrorCode::SelectorGuardFailed => {
            ErrorCode::SelectorGuardFailed
        }
        pptx_compose::json::schemas::ErrorCode::MissingMediaRef => ErrorCode::MissingMediaRef,
        pptx_compose::json::schemas::ErrorCode::MediaChecksumMismatch => {
            ErrorCode::MediaChecksumMismatch
        }
        pptx_compose::json::schemas::ErrorCode::PermissionDenied => ErrorCode::PermissionDenied,
        pptx_compose::json::schemas::ErrorCode::WriteFailed => ErrorCode::WriteFailed,
        pptx_compose::json::schemas::ErrorCode::InternalError => ErrorCode::InternalError,
    }
}

fn error_location(location: &serde_json::Value) -> ErrorLocation {
    ErrorLocation {
        current_revision: location
            .get("current_revision")
            .and_then(serde_json::Value::as_u64),
        io_path: location_string(location, "io_path"),
        zip_entry: location_string(location, "zip_entry"),
        part: location_string(location, "part"),
        relationship_id: location_string(location, "relationship_id"),
        slide_id: location_string(location, "slide_id"),
        element_id: location_string(location, "element_id"),
        operation_id: location_string(location, "operation_id"),
        operation: location_string(location, "operation"),
        expected: location_string(location, "expected"),
        actual: location_string(location, "actual"),
        candidates: location
            .get("candidates")
            .and_then(serde_json::Value::as_array)
            .map(|values| {
                values
                    .iter()
                    .filter_map(serde_json::Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default(),
    }
}

fn location_string(location: &serde_json::Value, key: &str) -> Option<String> {
    location
        .get(key)
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
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

fn enforce_secondary_output_write_guards(
    report: Option<&Path>,
    diff: Option<&Path>,
    overwrite: bool,
) -> Result<(), CliError> {
    if overwrite {
        return Ok(());
    }

    for output in [report, diff].into_iter().flatten() {
        if output != Path::new("-") && output.exists() {
            return Err(CliError::new(
                pptx_compose::core::error::ErrorCode::WriteFailed,
                format!(
                    "Output path {} already exists; pass --overwrite to replace it.",
                    output.display()
                ),
            ));
        }
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
fn overwrite_succeeds_and_default_selects_deterministic_mode() {
    test_support::overwrite_succeeds_and_default_selects_deterministic_mode();
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
fn in_place_apply_without_output_writes_input_and_backup() {
    test_support::in_place_apply_without_output_writes_input_and_backup();
}

#[cfg(test)]
#[test]
fn in_place_apply_rejects_different_output() {
    test_support::in_place_apply_rejects_different_output();
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

#[test]
fn apply_uses_configured_temp_dir_for_atomic_write() {
    test_support::apply_uses_configured_temp_dir_for_atomic_write();
}

#[cfg(test)]
#[test]
fn dry_run_writes_no_pptx() {
    test_support::dry_run_writes_no_pptx();
}

#[cfg(test)]
#[test]
fn dry_run_rejects_default_report_and_stdout_diff() {
    test_support::dry_run_rejects_default_report_and_stdout_diff();
}

#[cfg(test)]
#[test]
fn apply_rejects_stdout_report_and_stdout_diff() {
    test_support::apply_rejects_stdout_report_and_stdout_diff();
}

#[cfg(test)]
#[test]
fn dry_run_writes_report_for_all_failed_operations() {
    test_support::dry_run_writes_report_for_all_failed_operations();
}

#[cfg(test)]
#[test]
fn non_dry_run_writes_failed_report_without_output_pptx() {
    test_support::non_dry_run_writes_failed_report_without_output_pptx();
}

#[cfg(test)]
#[test]
fn replace_text_writes_mutated_output() {
    test_support::replace_text_writes_mutated_output();
}

#[cfg(test)]
#[test]
fn apply_rejects_existing_secondary_outputs_before_writing_pptx() {
    test_support::apply_rejects_existing_secondary_outputs_before_writing_pptx();
}

#[cfg(test)]
#[test]
fn parse_patch_invalid_input_preserves_underlying_message() {
    test_support::parse_patch_invalid_input_preserves_underlying_message();
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
#[test]
fn media_manifest_symlink_escape_is_rejected() {
    test_support::media_manifest_symlink_escape_is_rejected();
}

#[cfg(test)]
#[test]
fn unused_manifest_media_refs_are_reported() {
    test_support::unused_manifest_media_refs_are_reported();
}

#[cfg(test)]
#[test]
fn media_root_resolves_manifest_paths() {
    test_support::media_root_resolves_manifest_paths();
}

#[cfg(test)]
#[test]
fn explicit_media_binding_overrides_manifest_binding() {
    test_support::explicit_media_binding_overrides_manifest_binding();
}

#[cfg(test)]
mod test_support {
    use std::{fs, io::Cursor, io::Write, path::Path};

    use pptx_compose::{
        OpenOptions, PresentationDocument, WriteMode,
        core::error::ErrorCode,
        json::agent_view::{FindTextScope, views::FindTextRequest},
        part_checksum,
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

        let args = args(&input, &patch, &output, false);
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

    pub(super) fn overwrite_succeeds_and_default_selects_deterministic_mode() {
        let root = unique_dir();
        let input = root.join("input.pptx");
        let patch = root.join("patch.json");
        let output = root.join("output.pptx");
        fs::create_dir_all(&root).expect("test dir creates");
        fs::write(&input, include_bytes!("../../../../fixtures/minimal.pptx"))
            .expect("input fixture writes");
        fs::write(&patch, valid_noop_patch()).expect("patch fixture writes");
        fs::write(&output, b"replace-me").expect("existing output writes");

        let args = args(&input, &patch, &output, true);
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

        let args = args(&input, &patch, &output, false);
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
            args(&input, &patch, &first_output, false),
            &permissions(&root),
            OpenOptions::default(),
        )
        .expect("first apply succeeds");
        apply(
            args(&input, &patch, &second_output, false),
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

        let mut args = args(&input, &patch, &input, false);
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

    pub(super) fn in_place_apply_without_output_writes_input_and_backup() {
        let root = unique_dir();
        let input = root.join("input.pptx");
        let patch = root.join("patch.json");
        fs::create_dir_all(&root).expect("test dir creates");
        let input_bytes = text_deck();
        fs::write(&input, &input_bytes).expect("input fixture writes");
        fs::write(&patch, replace_text_patch(&input_bytes)).expect("patch fixture writes");

        let mut args = args(&input, &patch, &input, false);
        args.output = None;
        args.in_place = true;
        apply(args, &permissions(&root), OpenOptions::default())
            .expect("in-place apply without explicit output succeeds");

        assert_eq!(
            fs::read(root.join("input.pptx.bak")).expect("backup reads"),
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

    pub(super) fn in_place_apply_rejects_different_output() {
        let root = unique_dir();
        let input = root.join("input.pptx");
        let patch = root.join("patch.json");
        let output = root.join("output.pptx");
        fs::create_dir_all(&root).expect("test dir creates");
        let input_bytes = text_deck();
        fs::write(&input, &input_bytes).expect("input fixture writes");
        fs::write(&patch, replace_text_patch(&input_bytes)).expect("patch fixture writes");

        let mut args = args(&input, &patch, &output, false);
        args.in_place = true;
        let err = apply(args, &permissions(&root), OpenOptions::default())
            .expect_err("in-place apply with different output must fail");

        assert_eq!(err.code(), ErrorCode::InvalidInput);
        assert!(
            !output.exists(),
            "rejected in-place apply must not write the different output"
        );
        assert_eq!(
            fs::read(&input).expect("input reads after rejected apply"),
            input_bytes,
            "rejected in-place apply must not modify input"
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

        let mut args = args(&input, &patch, &input, false);
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

        let mut args = args(&input, &patch, &input, false);
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

    pub(super) fn apply_uses_configured_temp_dir_for_atomic_write() {
        let root = unique_dir();
        let workspace = root.join("workspace");
        let temp_dir = root.join("tmp");
        let input = workspace.join("input.pptx");
        let patch = workspace.join("patch.json");
        let output = workspace.join("output.pptx");
        fs::create_dir_all(&workspace).expect("workspace dir creates");
        fs::create_dir_all(&temp_dir).expect("temp dir creates");
        let input_bytes = text_deck();
        fs::write(&input, &input_bytes).expect("input fixture writes");
        fs::write(&patch, replace_text_patch(&input_bytes)).expect("patch fixture writes");

        apply(
            args(&input, &patch, &output, false),
            &permissions_with_temp(&workspace, &temp_dir),
            OpenOptions::default(),
        )
        .expect("apply succeeds with separate configured temp dir");

        assert!(output.exists(), "apply writes the explicit output path");
        let workspace_entries = fs::read_dir(&workspace)
            .expect("workspace reads")
            .map(|entry| entry.expect("workspace entry reads").file_name())
            .collect::<Vec<_>>();
        assert!(
            !workspace_entries
                .iter()
                .any(|name| name.to_string_lossy().starts_with(".output.pptx.")),
            "atomic temp output must not be created as an output sibling"
        );
        assert!(
            fs::read_dir(&temp_dir)
                .expect("temp dir reads")
                .next()
                .is_none(),
            "successful apply removes the temporary output"
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

        let mut args = args(&input, &patch, &output, false);
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

    pub(super) fn dry_run_rejects_default_report_and_stdout_diff() {
        let root = unique_dir();
        let input = root.join("missing.pptx");
        let patch = root.join("missing-patch.json");
        let output = root.join("output.pptx");

        let mut args = args(&input, &patch, &output, false);
        args.dry_run = true;
        args.output = None;
        args.diff = Some(Path::new("-").to_path_buf());

        let err = apply(args, &permissions(&root), OpenOptions::default())
            .expect_err("dry-run default report and stdout diff must be rejected");
        assert_eq!(err.code(), ErrorCode::InvalidInput);
        assert!(
            err.details()
                .message
                .contains("omitted --report and --diff - resolve to stdout")
        );

        fs::remove_dir_all(root).expect("test dir removes");
    }

    pub(super) fn apply_rejects_stdout_report_and_stdout_diff() {
        let root = unique_dir();
        let input = root.join("missing.pptx");
        let patch = root.join("missing-patch.json");
        let output = root.join("output.pptx");

        let mut args = args(&input, &patch, &output, false);
        args.report = Some(Path::new("-").to_path_buf());
        args.diff = Some(Path::new("-").to_path_buf());

        let err = apply(args, &permissions(&root), OpenOptions::default())
            .expect_err("stdout report and stdout diff must be rejected");
        assert_eq!(err.code(), ErrorCode::InvalidInput);
        assert!(
            err.details()
                .message
                .contains("--report - and --diff - resolve to stdout")
        );

        fs::remove_dir_all(root).expect("test dir removes");
    }

    pub(super) fn dry_run_writes_report_for_all_failed_operations() {
        let root = unique_dir();
        let input = root.join("input.pptx");
        let patch = root.join("patch.json");
        let output = root.join("output.pptx");
        let report = root.join("report.json");
        let input_bytes = text_deck();
        fs::write(&input, &input_bytes).expect("input fixture writes");
        fs::write(&patch, invalid_multi_op_patch(&input_bytes)).expect("patch fixture writes");

        let mut args = args(&input, &patch, &output, false);
        args.dry_run = true;
        args.output = None;
        args.report = Some(report.clone());

        let err = apply(args, &permissions(&root), OpenOptions::default())
            .expect_err("dry-run with invalid operations exits nonzero");
        assert_eq!(err.code(), ErrorCode::SelectorNotFound);
        assert!(!output.exists(), "dry-run must not create a PPTX output");

        let report_json: serde_json::Value =
            serde_json::from_slice(&fs::read(&report).expect("report reads"))
                .expect("report is JSON");
        assert_eq!(report_json["status"], "dry_run_failed");
        let operation_reports = report_json["operation_reports"]
            .as_array()
            .expect("operation reports is an array");
        assert_eq!(operation_reports.len(), 2);
        assert!(operation_reports.iter().all(|operation| {
            operation["status"] == "failed" && operation["error"]["code"] == "selector_not_found"
        }));

        fs::remove_dir_all(root).expect("test dir removes");
    }

    pub(super) fn non_dry_run_writes_failed_report_without_output_pptx() {
        let root = unique_dir();
        let input = root.join("input.pptx");
        let patch = root.join("patch.json");
        let output = root.join("output.pptx");
        let report = root.join("report.json");
        let input_bytes = text_deck();
        fs::write(&input, &input_bytes).expect("input fixture writes");
        fs::write(&patch, one_valid_one_invalid_patch(&input_bytes)).expect("patch fixture writes");

        let mut args = args(&input, &patch, &output, false);
        args.report = Some(report.clone());

        let err = apply(args, &permissions(&root), OpenOptions::default())
            .expect_err("non-dry-run with invalid operation exits nonzero");
        assert_eq!(err.code(), ErrorCode::SelectorNotFound);
        assert!(
            !output.exists(),
            "failed apply must not create a PPTX output"
        );

        let report_json: serde_json::Value =
            serde_json::from_slice(&fs::read(&report).expect("report reads"))
                .expect("report is JSON");
        assert_eq!(report_json["status"], "failed");
        assert_eq!(report_json["dry_run"], false);
        let operation_reports = report_json["operation_reports"]
            .as_array()
            .expect("operation reports is an array");
        assert_eq!(operation_reports.len(), 2);
        assert_eq!(operation_reports[0]["operation_id"], "good-1");
        assert_eq!(operation_reports[0]["status"], "skipped");
        assert_eq!(operation_reports[0]["changed_parts"], serde_json::json!([]));
        assert_eq!(operation_reports[1]["operation_id"], "bad-2");
        assert_eq!(operation_reports[1]["status"], "failed");
        assert_eq!(operation_reports[1]["error"]["code"], "selector_not_found");

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

        let mut args = args(&input, &patch, &output, false);
        args.report = Some(report.clone());
        args.diff = Some(diff.clone());
        apply(args, &permissions(&root), OpenOptions::default())
            .expect("replace_text apply succeeds");

        let output_bytes = fs::read(&output).expect("output reads");
        assert_ne!(output_bytes, input_bytes);
        let output_document =
            PresentationDocument::from_bytes(&output_bytes).expect("output deck opens");
        let matches = output_document
            .find_text(FindTextRequest {
                query: "Updated title".to_owned(),
                scope: FindTextScope::Deck,
                cursor: None,
                limit: None,
            })
            .expect("updated text is searchable");
        assert_eq!(matches.matches.len(), 1);

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

    pub(super) fn apply_rejects_existing_secondary_outputs_before_writing_pptx() {
        for secondary_output in ["report", "diff"] {
            let root = unique_dir();
            let input = root.join("input.pptx");
            let patch = root.join("patch.json");
            let output = root.join("output.pptx");
            let existing = root.join(format!("{secondary_output}.json"));
            fs::create_dir_all(&root).expect("test dir creates");
            let input_bytes = text_deck();
            fs::write(&input, &input_bytes).expect("input fixture writes");
            fs::write(&patch, replace_text_patch(&input_bytes)).expect("patch fixture writes");
            fs::write(&existing, b"existing-json").expect("existing secondary output writes");

            let mut args = args(&input, &patch, &output, false);
            match secondary_output {
                "report" => args.report = Some(existing.clone()),
                "diff" => args.diff = Some(existing.clone()),
                _ => unreachable!("test cases are exhaustive"),
            }

            let err = apply(args, &permissions(&root), OpenOptions::default())
                .expect_err("existing secondary output must fail before PPTX write");

            assert_eq!(err.code(), ErrorCode::WriteFailed);
            assert!(
                err.details()
                    .message
                    .contains("already exists; pass --overwrite")
            );
            assert!(
                !output.exists(),
                "apply must not write PPTX output when {secondary_output} already exists"
            );
            assert_eq!(
                fs::read(&existing).expect("existing secondary output reads"),
                b"existing-json",
                "apply must not replace existing {secondary_output}"
            );

            fs::remove_dir_all(root).expect("test dir removes");
        }
    }

    pub(super) fn parse_patch_invalid_input_preserves_underlying_message() {
        let root = unique_dir();
        let input = root.join("input.pptx");
        let patch = root.join("patch.json");
        let output = root.join("output.pptx");
        fs::create_dir_all(&root).expect("test dir creates");
        let input_bytes = text_deck();
        fs::write(&input, &input_bytes).expect("input fixture writes");
        fs::write(&patch, removed_replace_text_policy_patch(&input_bytes))
            .expect("patch fixture writes");

        let err = apply(
            args(&input, &patch, &output, false),
            &permissions(&root),
            OpenOptions::default(),
        )
        .expect_err("semantic patch invalid_input must fail");

        assert_eq!(err.code(), ErrorCode::InvalidInput);
        assert!(err.details().message.contains("overflow_policy"));
        assert!(!err.details().message.contains("schema validation"));

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

        let mut args = args(&input, &patch, &output, false);
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

            let mut args = args(&input, &patch, &output, false);
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

    pub(super) fn media_manifest_symlink_escape_is_rejected() {
        let root = unique_dir();
        let workspace = root.join("workspace");
        let outside = root.join("outside");
        let input = workspace.join("input.pptx");
        let patch = workspace.join("patch.json");
        let manifest = workspace.join("media.json");
        let assets = workspace.join("assets");
        let image = assets.join("hero.png");
        let outside_image = outside.join("secret.png");
        let output = workspace.join("output.pptx");
        fs::create_dir_all(&assets).expect("asset dir creates");
        fs::create_dir_all(&outside).expect("outside dir creates");
        let input_bytes = text_deck();
        fs::write(&input, &input_bytes).expect("input fixture writes");
        fs::write(&outside_image, png_bytes()).expect("outside image writes");
        create_symlink(&outside_image, &image);
        fs::write(&patch, add_image_patch(&input_bytes)).expect("patch fixture writes");
        fs::write(
            &manifest,
            media_manifest("assets/hero.png", "image/png", None, None),
        )
        .expect("manifest writes");

        let mut args = args(&input, &patch, &output, false);
        args.dry_run = true;
        args.output = None;
        args.media_manifest = Some(manifest);

        let err = apply(args, &permissions(&workspace), OpenOptions::default())
            .expect_err("manifest symlink escape must fail");
        assert!(matches!(
            err.code(),
            ErrorCode::PermissionDenied | ErrorCode::UnsafePath
        ));

        fs::remove_dir_all(root).expect("test dir removes");
    }

    pub(super) fn unused_manifest_media_refs_are_reported() {
        let root = unique_dir();
        let input = root.join("input.pptx");
        let patch = root.join("patch.json");
        let manifest = root.join("media.json");
        let assets = root.join("assets");
        let image = assets.join("hero.png");
        let unused = assets.join("unused.png");
        let output = root.join("output.pptx");
        let report = root.join("report.json");
        fs::create_dir_all(&assets).expect("asset dir creates");
        let input_bytes = text_deck();
        fs::write(&input, &input_bytes).expect("input fixture writes");
        fs::write(&image, png_bytes()).expect("image writes");
        fs::write(&unused, png_bytes()).expect("unused image writes");
        fs::write(&patch, add_image_patch(&input_bytes)).expect("patch fixture writes");
        fs::write(
            &manifest,
            media_manifest_with_entries(&[
                ("hero", "assets/hero.png", "image/png"),
                ("unused", "assets/unused.png", "image/png"),
            ]),
        )
        .expect("manifest writes");

        let mut args = args(&input, &patch, &output, false);
        args.dry_run = true;
        args.output = None;
        args.report = Some(report.clone());
        args.media_manifest = Some(manifest);

        apply(args, &permissions(&root), OpenOptions::default())
            .expect("dry-run with unused manifest binding succeeds");

        let report_json: serde_json::Value =
            serde_json::from_slice(&fs::read(&report).expect("report reads"))
                .expect("report is JSON");
        assert_eq!(
            report_json["warnings"],
            serde_json::json!([{
                "category": "media_input",
                "code": "unused_media_ref",
                "media_ref": "unused",
                "message": "Media input `unused` is bound but not referenced."
            }])
        );

        fs::remove_dir_all(root).expect("test dir removes");
    }

    pub(super) fn media_root_resolves_manifest_paths() {
        let root = unique_dir();
        let input = root.join("input.pptx");
        let patch = root.join("patch.json");
        let manifest_dir = root.join("manifests");
        let manifest = manifest_dir.join("media.json");
        let assets = root.join("assets");
        let image = assets.join("hero.png");
        let output = root.join("output.pptx");
        let report = root.join("report.json");
        fs::create_dir_all(&manifest_dir).expect("manifest dir creates");
        fs::create_dir_all(&assets).expect("asset dir creates");
        let input_bytes = text_deck();
        fs::write(&input, &input_bytes).expect("input fixture writes");
        fs::write(&image, png_bytes()).expect("image writes");
        fs::write(&patch, add_image_patch(&input_bytes)).expect("patch fixture writes");
        fs::write(
            &manifest,
            media_manifest("hero.png", "image/png", None, None),
        )
        .expect("manifest writes");

        let mut args = args(&input, &patch, &output, false);
        args.dry_run = true;
        args.output = None;
        args.report = Some(report.clone());
        args.media_manifest = Some(manifest);
        args.media_root = Some(assets);

        apply(args, &permissions(&root), OpenOptions::default())
            .expect("media-root resolves manifest path");

        let report_json: serde_json::Value =
            serde_json::from_slice(&fs::read(&report).expect("report reads"))
                .expect("report is JSON");
        assert_eq!(report_json["status"], "dry_run_success");

        fs::remove_dir_all(root).expect("test dir removes");
    }

    pub(super) fn explicit_media_binding_overrides_manifest_binding() {
        let root = unique_dir();
        let input = root.join("input.pptx");
        let patch = root.join("patch.json");
        let manifest = root.join("media.json");
        let assets = root.join("assets");
        let manifest_image = assets.join("hero.jpg");
        let explicit_image = assets.join("hero.png");
        let output = root.join("output.pptx");
        fs::create_dir_all(&assets).expect("asset dir creates");
        let input_bytes = text_deck();
        fs::write(&input, &input_bytes).expect("input fixture writes");
        fs::write(&manifest_image, jpeg_bytes()).expect("manifest image writes");
        fs::write(&explicit_image, png_bytes()).expect("explicit image writes");
        fs::write(&patch, add_image_patch(&input_bytes)).expect("patch fixture writes");
        fs::write(
            &manifest,
            media_manifest("assets/hero.jpg", "image/jpeg", None, None),
        )
        .expect("manifest writes");

        let mut args = args(&input, &patch, &output, false);
        args.dry_run = true;
        args.output = None;
        args.media_manifest = Some(manifest);
        args.media
            .push(format!("hero={}", explicit_image.display()));

        apply(args, &permissions(&root), OpenOptions::default())
            .expect("explicit --media binding overrides manifest binding");

        fs::remove_dir_all(root).expect("test dir removes");
    }

    fn args(input: &Path, patch: &Path, output: &Path, overwrite: bool) -> ApplyArgs {
        ApplyArgs {
            input: input.to_path_buf(),
            patch: patch.to_path_buf(),
            dry_run: false,
            media_manifest: None,
            media_root: None,
            media: Vec::new(),
            output: Some(output.to_path_buf()),
            report: None,
            diff: None,
            overwrite,
            in_place: false,
            no_backup: false,
        }
    }

    fn permissions(root: &Path) -> PermissionContext {
        permissions_with_temp(root, root)
    }

    fn permissions_with_temp(root: &Path, temp_dir: &Path) -> PermissionContext {
        PermissionContext {
            workspace: fs::canonicalize(root).expect("workspace canonicalizes"),
            temp_dir: fs::canonicalize(temp_dir).expect("temp canonicalizes"),
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

    fn removed_replace_text_policy_patch(bytes: &[u8]) -> Vec<u8> {
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
            "client_request_id": "apply-test-invalid-format-policy",
            "operations": [{{
                "operation_id": "replace-title",
                "op": "replace_text",
                "element_id": "slide-1:shape-3",
                "text": "Updated title",
                "overflow_policy": "allow"
            }}]
        }}"#
        )
        .into_bytes()
    }

    fn invalid_multi_op_patch(bytes: &[u8]) -> Vec<u8> {
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
            "client_request_id": "apply-test-invalid-multi-op",
            "operations": [
                {{
                    "operation_id": "bad-1",
                    "op": "replace_text",
                    "element_id": "slide-1:missing-1",
                    "text": "Updated title"
                }},
                {{
                    "operation_id": "bad-2",
                    "op": "replace_text",
                    "element_id": "slide-1:missing-2",
                    "text": "Updated subtitle"
                }}
            ]
        }}"#
        )
        .into_bytes()
    }

    fn one_valid_one_invalid_patch(bytes: &[u8]) -> Vec<u8> {
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
            "client_request_id": "apply-test-one-valid-one-invalid",
            "operations": [
                {{
                    "operation_id": "good-1",
                    "op": "replace_text",
                    "element_id": "slide-1:shape-3",
                    "text": "Updated title"
                }},
                {{
                    "operation_id": "bad-2",
                    "op": "replace_text",
                    "element_id": "slide-1:missing-2",
                    "text": "Updated subtitle"
                }}
            ]
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

    fn jpeg_bytes() -> &'static [u8] {
        b"\xff\xd8\xffpptx-compose-test-image"
    }

    fn media_manifest_with_entries(entries: &[(&str, &str, &str)]) -> Vec<u8> {
        let entries = entries
            .iter()
            .map(|(media_ref, path, content_type)| {
                format!(
                    r#""{media_ref}": {{ "path": "{path}", "content_type": "{content_type}" }}"#
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        format!(
            r#"{{
            "schema": "pptx-compose.media_manifest.v1",
            "version": 1,
            "media": {{ {entries} }}
        }}"#
        )
        .into_bytes()
    }

    #[cfg(unix)]
    fn create_symlink(target: &Path, link: &Path) {
        std::os::unix::fs::symlink(target, link).expect("symlink fixture");
    }

    #[cfg(windows)]
    fn create_symlink(target: &Path, link: &Path) {
        std::os::windows::fs::symlink_file(target, link).expect("symlink fixture");
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
