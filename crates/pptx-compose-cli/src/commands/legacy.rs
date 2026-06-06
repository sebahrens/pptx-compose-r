use std::{fs, path::Path};

use pptx_compose::{
    OpenOptions, PresentationDocument, WriteMode, WriteOptions, core::error::ErrorCode,
    json::legacy_path_map,
};

use crate::{
    CliError, InvalidInputCause,
    cli::{LegacyConvertArgs, LegacyToJsonArgs, LegacyToPptxArgs},
    permissions::{PathIntent, PermissionContext},
};

pub(crate) fn run_to_json(
    args: LegacyToJsonArgs,
    permissions: &PermissionContext,
    open_options: OpenOptions,
) -> Result<(), CliError> {
    require_compat_json(args.compat_json)?;
    let input = permissions.authorize_read(&args.input, PathIntent::InputPptx)?;
    let output = permissions.authorize_write(&args.output, PathIntent::LegacyJsonOutput)?;

    let document = PresentationDocument::open_path_with_options(&input, open_options)
        .map_err(CliError::from_error)?;
    let legacy_json = document.to_legacy_json().map_err(CliError::from_error)?;
    let legacy_json = legacy_path_map::from_legacy_map(legacy_json)
        .and_then(|package| legacy_path_map::to_legacy_map(&package))
        .map_err(legacy_json_error)?;
    write_pretty_json(&output, &legacy_json)
}

pub(crate) fn run_convert(
    args: LegacyConvertArgs,
    permissions: &PermissionContext,
    open_options: OpenOptions,
) -> Result<(), CliError> {
    run_to_json(
        LegacyToJsonArgs {
            input: args.input,
            output: args.output,
            compat_json: args.compat_json,
        },
        permissions,
        open_options,
    )
}

pub(crate) fn run_to_pptx(
    args: LegacyToPptxArgs,
    permissions: &PermissionContext,
) -> Result<(), CliError> {
    require_compat_json(args.compat_json)?;
    let input = permissions.authorize_read(&args.input, PathIntent::LegacyJsonInput)?;
    let output = permissions.authorize_write(&args.output, PathIntent::OutputPptx)?;

    if output.exists() && !args.overwrite {
        return Err(CliError::new(
            ErrorCode::WriteFailed,
            format!(
                "Output path {} already exists; pass --overwrite to replace it.",
                output.display()
            ),
        ));
    }

    let legacy_json = read_legacy_json(&input)?;
    let document =
        PresentationDocument::from_legacy_json(legacy_json).map_err(CliError::from_error)?;
    document
        .write_path_with_options(
            &output,
            WriteOptions {
                mode: WriteMode::Preserve,
                overwrite: args.overwrite,
                validate: true,
                atomic: true,
            },
        )
        .map_err(CliError::from_error)
}

fn require_compat_json(enabled: bool) -> Result<(), CliError> {
    if enabled {
        Ok(())
    } else {
        Err(CliError::invalid_input(
            InvalidInputCause::CliArgument,
            "Legacy conversion commands require --compat-json.",
        ))
    }
}

fn read_legacy_json(path: &Path) -> Result<serde_json::Value, CliError> {
    let bytes = fs::read(path).map_err(|source| {
        CliError::invalid_input_with_source(
            InvalidInputCause::InputPath,
            "Could not read legacy JSON input.",
            source,
        )
    })?;
    serde_json::from_slice(&bytes).map_err(|source| {
        CliError::invalid_input_with_source(
            InvalidInputCause::CliArgument,
            "Legacy JSON input is not valid JSON.",
            source,
        )
    })
}

fn write_pretty_json(path: &Path, value: &serde_json::Value) -> Result<(), CliError> {
    if path == Path::new("-") {
        let stdout = std::io::stdout();
        let mut lock = stdout.lock();
        serde_json::to_writer_pretty(&mut lock, value).map_err(|source| {
            CliError::write_with_source("Could not serialize JSON output.", source)
        })?;
        use std::io::Write as _;
        writeln!(lock)
            .map_err(|source| CliError::write_with_source("Could not finish JSON output.", source))
    } else {
        let file = fs::File::create(path).map_err(|source| {
            CliError::write_with_source("Could not open JSON output path.", source)
        })?;
        let mut writer = std::io::BufWriter::new(file);
        serde_json::to_writer_pretty(&mut writer, value).map_err(|source| {
            CliError::write_with_source("Could not serialize JSON output.", source)
        })?;
        use std::io::Write as _;
        writeln!(writer)
            .map_err(|source| CliError::write_with_source("Could not finish JSON output.", source))
    }
}

fn legacy_json_error(error: pptx_compose::json::schemas::JsonError) -> CliError {
    let err = match error {
        pptx_compose::json::schemas::JsonError::SerializeSchema(message)
        | pptx_compose::json::schemas::JsonError::InvalidCursor(message)
        | pptx_compose::json::schemas::JsonError::MalformedLegacyEnvelope(message)
        | pptx_compose::json::schemas::JsonError::Projection(message) => {
            pptx_compose::core::error::Error::new(ErrorCode::InvalidInput, message)
        }
        pptx_compose::json::schemas::JsonError::NotFound { kind, id } => {
            pptx_compose::core::error::Error::new(
                ErrorCode::SelectorNotFound,
                format!("{kind} `{id}` was not found."),
            )
        }
    };
    CliError::from_error(err)
}
