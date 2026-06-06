#![deny(warnings)]

mod cli;
mod commands;
mod exit;
mod output;
mod permissions;

use clap::{Parser, error::ErrorKind};
use cli::{Cli, Commands, MediaCmd};
use commands::apply::apply;
use commands::legacy::{run_convert, run_to_json, run_to_pptx};
use exit::exit_code_for;
use output::{OutputDest, OutputSink};
use permissions::{PathIntent, PermissionContext};
use pptx_compose::{
    AgentViewOptions, MediaPartInfo, OpenOptions, Patch, PresentationDocument,
    core::{
        error::{Error, ErrorCode, ErrorDetails, ErrorLocation},
        zip::limits::ResourceLimits,
    },
    edit::{
        media_inputs::{MEDIA_MANIFEST_SCHEMA, MediaManifest},
        patch::PATCH_SCHEMA,
    },
    json::{
        agent_view::{
            AgentView, FindTextScope,
            views::{FindTextRequest, ViewMode},
        },
        schema_versions::{
            AGENT_VIEW_SCHEMA, ERROR_SCHEMA, PATCH_REPORT_SCHEMA, VALIDATION_REPORT_SCHEMA,
        },
        schemas::{ErrorEnvelope, PatchReport, ResultEnvelope, ResultStatus, ValidationReport},
    },
};
use schemars::JsonSchema;
use serde::Serialize;
use serde_json::{Value, json};
use std::{fs, path::PathBuf};

fn main() {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error) => {
            let exit_code = match error.kind() {
                ErrorKind::DisplayHelp
                | ErrorKind::DisplayVersion
                | ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand => exit::SUCCESS,
                _ => exit::USAGE,
            };
            let _ = error.print();
            std::process::exit(exit_code);
        }
    };

    let sink = OutputSink::from_global_args(&cli.global);
    if let Err(error) = run(cli) {
        if let Err(emit_error) = sink.emit_error(&error) {
            eprintln!("{emit_error}");
            std::process::exit(exit::WRITE_FAILURE);
        }
        std::process::exit(exit_code_for(&error));
    }
    std::process::exit(exit::SUCCESS);
}

fn run(cli: Cli) -> Result<(), CliError> {
    let permissions = PermissionContext::from_global_args(&cli.global)?;
    let sink = OutputSink::from_global_args(&cli.global);
    let open_options = open_options_from_global_args(&cli.global)?;
    match cli.command {
        Commands::Capabilities => capabilities(sink),
        Commands::Inspect(args) => inspect(args, &permissions, sink, open_options),
        Commands::FindText(args) => find_text(args, &permissions, sink, open_options),
        Commands::Validate(args) => validate(args, &permissions, sink, open_options),
        Commands::Apply(args) => apply(args, &permissions, open_options),
        Commands::ToJson(args) => run_to_json(args, &permissions, open_options),
        Commands::ToPptx(args) => run_to_pptx(args, &permissions),
        Commands::Convert(args) => run_convert(args, &permissions, open_options),
        Commands::Media(MediaCmd::List(args)) => media_list(args, &permissions, sink, open_options),
        Commands::Media(MediaCmd::Get(args)) => media_get(args, &permissions, sink, open_options),
        Commands::Schema(args) => schema(args, sink),
    }
}

fn capabilities(sink: OutputSink) -> Result<(), CliError> {
    let document = pptx_compose::capabilities::capabilities(
        pptx_compose::capabilities::CapabilitiesOptions::new(
            env!("CARGO_PKG_NAME"),
            env!("CARGO_PKG_VERSION"),
        ),
    );
    sink.emit_json(&document, OutputDest::Stdout)
}

fn open_options_from_global_args(global: &cli::GlobalArgs) -> Result<OpenOptions, CliError> {
    let mut resource_limits = ResourceLimits::default();
    if let Some(max_uncompressed_bytes) = global.max_uncompressed_bytes {
        resource_limits.max_uncompressed_package_bytes = max_uncompressed_bytes;
    }
    if let Some(max_part_count) = global.max_part_count {
        resource_limits.max_part_count = usize::try_from(max_part_count).map_err(|source| {
            CliError::invalid_input_with_source(
                InvalidInputCause::CliArgument,
                "--max-part-count exceeds this platform's supported usize range.",
                source,
            )
        })?;
    }
    if let Some(max_media_bytes) = global.max_media_bytes {
        resource_limits.max_media_part_bytes = max_media_bytes;
    }
    Ok(OpenOptions { resource_limits })
}

fn find_text(
    args: cli::FindTextArgs,
    permissions: &PermissionContext,
    sink: OutputSink,
    open_options: OpenOptions,
) -> Result<(), CliError> {
    permissions.authorize_read(&args.input, PathIntent::InputPptx)?;
    if let Some(output) = &args.output {
        permissions.authorize_write(output, PathIntent::ReportOutput)?;
    }

    let document = PresentationDocument::open_path_with_options(&args.input, open_options)
        .map_err(|error| {
            CliError::from_error(error.with_location(ErrorLocation {
                part: Some(args.input.display().to_string()),
                ..ErrorLocation::default()
            }))
        })?;
    let scope = match args.slide_id {
        Some(slide_id) => FindTextScope::Slide { slide_id },
        None => FindTextScope::Deck,
    };
    let result = document
        .find_text(FindTextRequest {
            query: args.query,
            scope,
            cursor: args.cursor,
            limit: args.limit,
        })
        .map_err(CliError::from_error)?;
    sink.emit_json(&result, output::OutputDest::from(args.output))
}

fn inspect(
    args: cli::InspectArgs,
    permissions: &PermissionContext,
    sink: OutputSink,
    open_options: OpenOptions,
) -> Result<(), CliError> {
    let input = permissions.authorize_read(&args.input, PathIntent::InputPptx)?;
    if let Some(output) = &args.output {
        permissions.authorize_write(output, PathIntent::ReportOutput)?;
    }
    if let Some(report) = &args.report {
        permissions.authorize_write(report, PathIntent::ReportOutput)?;
    }

    let document = PresentationDocument::open_path_with_options(&input, open_options)
        .map_err(CliError::from_error)?;
    let view = document
        .to_agent_json_with_options(inspect_view_options(&args)?)
        .map_err(CliError::from_error)?;
    sink.emit_json(&view, OutputDest::from(args.output))?;
    if args.report.is_some() {
        let report = document.validate().map_err(CliError::from_error)?;
        sink.emit_json(&report, OutputDest::from(args.report))?;
    }
    Ok(())
}

fn validate(
    args: cli::ValidateArgs,
    permissions: &PermissionContext,
    sink: OutputSink,
    open_options: OpenOptions,
) -> Result<(), CliError> {
    let input = permissions.authorize_read(&args.input, PathIntent::InputPptx)?;
    if let Some(report) = &args.report {
        permissions.authorize_write(report, PathIntent::ReportOutput)?;
    }
    let document = PresentationDocument::open_path_with_options(&input, open_options)
        .map_err(CliError::from_error)?;
    let report = document.validate().map_err(CliError::from_error)?;
    sink.emit_json(&report, OutputDest::from(args.report))
}

fn media_list(
    args: cli::MediaListArgs,
    permissions: &PermissionContext,
    sink: OutputSink,
    open_options: OpenOptions,
) -> Result<(), CliError> {
    let input = permissions.authorize_read(&args.input, PathIntent::InputPptx)?;
    let document = PresentationDocument::open_path_with_options(&input, open_options)
        .map_err(CliError::from_error)?;
    let result = MediaListResult {
        media: document.media_parts().map_err(CliError::from_error)?,
    };
    let output = if args.json {
        json!(result)
    } else {
        json!(result.media)
    };
    sink.emit_json(&success_envelope(output), OutputDest::Stdout)
}

fn media_get(
    args: cli::MediaGetArgs,
    permissions: &PermissionContext,
    sink: OutputSink,
    open_options: OpenOptions,
) -> Result<(), CliError> {
    let input = permissions.authorize_read(&args.input, PathIntent::InputPptx)?;
    let output = permissions.authorize_write(&args.output, PathIntent::OutputPptx)?;
    if let Some(report) = &args.report {
        permissions.authorize_write(report, PathIntent::ReportOutput)?;
    }
    let document = PresentationDocument::open_path_with_options(&input, open_options)
        .map_err(CliError::from_error)?;
    let bytes = document
        .media_part_bytes(&args.package_path)
        .map_err(CliError::from_error)?;
    fs::write(&output, &bytes).map_err(|source| {
        CliError::write_with_source("Could not write extracted media output.", source)
    })?;

    let info = document
        .media_parts()
        .map_err(CliError::from_error)?
        .into_iter()
        .find(|info| info.package_path == normalized_media_path(&args.package_path))
        .unwrap_or_else(|| media_info_from_bytes(&args.package_path, &bytes));
    let report = MediaGetReport {
        package_path: info.package_path,
        output,
        content_type: info.content_type,
        byte_length: info.byte_length,
        checksum: info.checksum,
    };
    if args.report.is_some() {
        sink.emit_json(
            &success_envelope(json!(report)),
            OutputDest::from(args.report),
        )?;
    }
    Ok(())
}

fn schema(args: cli::SchemaArgs, sink: OutputSink) -> Result<(), CliError> {
    let schema = match args.name.as_str() {
        "agent-view-v1" => schema_value::<AgentView>(AGENT_VIEW_SCHEMA)?,
        "patch-v1" => schema_value::<Patch>(PATCH_SCHEMA)?,
        "media-manifest-v1" => schema_value::<MediaManifest>(MEDIA_MANIFEST_SCHEMA)?,
        "patch-report-v1" => schema_value::<PatchReport>(PATCH_REPORT_SCHEMA)?,
        "validation-report-v1" => schema_value::<ValidationReport>(VALIDATION_REPORT_SCHEMA)?,
        "error-v1" => schema_value::<ErrorEnvelope>(ERROR_SCHEMA)?,
        _ => {
            return Err(CliError::invalid_input(
                InvalidInputCause::CliArgument,
                "Unknown schema name. Expected one of: agent-view-v1, patch-v1, media-manifest-v1, patch-report-v1, validation-report-v1, error-v1.",
            ));
        }
    };
    sink.emit_json(&schema, OutputDest::Stdout)
}

fn inspect_view_options(args: &cli::InspectArgs) -> Result<AgentViewOptions, CliError> {
    if !matches!(args.format, None | Some(cli::InspectFormat::AgentJson)) {
        return Err(CliError::invalid_input(
            InvalidInputCause::CliArgument,
            "inspect only supports --format agent-json.",
        ));
    }

    let mut options = AgentViewOptions::default();
    if matches!(args.detail, Some(cli::InspectDetail::Full)) {
        options.mode = ViewMode::SlidePage;
    }
    if let Some(slides) = &args.slides {
        if let Some(slide_id) = single_slide_id(slides) {
            options.mode = ViewMode::SlideDetail;
            options.slide_id = Some(slide_id);
        } else {
            options.mode = ViewMode::SlidePage;
        }
    }
    Ok(options)
}

fn single_slide_id(slides: &str) -> Option<String> {
    let trimmed = slides.trim();
    if trimmed.is_empty() || trimmed.contains(',') || trimmed.contains('-') {
        return None;
    }
    if trimmed.starts_with("slide-") {
        Some(trimmed.to_owned())
    } else {
        Some(format!("slide-{trimmed}"))
    }
}

#[derive(Debug, Serialize)]
#[serde(deny_unknown_fields)]
struct MediaListResult {
    media: Vec<MediaPartInfo>,
}

#[derive(Debug, Serialize)]
#[serde(deny_unknown_fields)]
struct MediaGetReport {
    package_path: String,
    output: PathBuf,
    content_type: Option<String>,
    byte_length: u64,
    checksum: String,
}

fn success_envelope(result: Value) -> ResultEnvelope {
    ResultEnvelope {
        schema: pptx_compose::json::schema_versions::RESULT_SCHEMA.to_owned(),
        version: pptx_compose::json::schema_versions::RESULT_VERSION,
        status: ResultStatus::Success,
        result,
        warnings: Vec::new(),
        next_cursor: None,
    }
}

fn normalized_media_path(package_path: &str) -> String {
    package_path.trim_start_matches('/').to_owned()
}

fn media_info_from_bytes(package_path: &str, bytes: &[u8]) -> MediaPartInfo {
    MediaPartInfo {
        package_path: normalized_media_path(package_path),
        content_type: None,
        byte_length: u64::try_from(bytes.len()).map_or(u64::MAX, |len| len),
        checksum: pptx_compose::core::provenance::checksum::part_checksum(bytes),
    }
}

fn schema_value<T: JsonSchema>(id: &str) -> Result<Value, CliError> {
    let schema = schemars::schema_for!(T);
    let mut value = serde_json::to_value(schema).map_err(|source| {
        CliError::with_source(
            ErrorCode::InternalError,
            "Could not serialize JSON schema.",
            source,
        )
    })?;
    if let Some(object) = value.as_object_mut() {
        object.insert("$id".to_owned(), Value::String(id.to_owned()));
    }
    Ok(value)
}

#[derive(Debug)]
pub(crate) struct CliError {
    error: Error,
    invalid_input_cause: Option<InvalidInputCause>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(dead_code)]
pub(crate) enum InvalidInputCause {
    CliArgument,
    InputPath,
    PatchSchema,
}

impl CliError {
    fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        debug_assert_ne!(
            code,
            ErrorCode::InvalidInput,
            "invalid_input errors need an explicit CLI sub-cause"
        );
        Self::from_error(Error::new(code, message))
    }

    fn with_source(
        code: ErrorCode,
        message: impl Into<String>,
        source: impl std::error::Error + Send + Sync + 'static,
    ) -> Self {
        debug_assert_ne!(
            code,
            ErrorCode::InvalidInput,
            "invalid_input errors need an explicit CLI sub-cause"
        );
        Self::from_error(Error::with_source(code, message, source))
    }

    #[allow(dead_code)]
    fn invalid_input(cause: InvalidInputCause, message: impl Into<String>) -> Self {
        Self {
            error: Error::new(ErrorCode::InvalidInput, message),
            invalid_input_cause: Some(cause),
        }
    }

    fn invalid_input_with_source(
        cause: InvalidInputCause,
        message: impl Into<String>,
        source: impl std::error::Error + Send + Sync + 'static,
    ) -> Self {
        Self {
            error: Error::with_source(ErrorCode::InvalidInput, message, source),
            invalid_input_cause: Some(cause),
        }
    }

    fn from_error(error: Error) -> Self {
        Self {
            error,
            invalid_input_cause: None,
        }
    }

    fn write_with_source(
        message: impl Into<String>,
        source: impl std::error::Error + Send + Sync + 'static,
    ) -> Self {
        Self::with_source(ErrorCode::WriteFailed, message, source)
    }

    const fn code(&self) -> ErrorCode {
        self.error.code()
    }

    fn details(&self) -> &ErrorDetails {
        self.error.details()
    }

    const fn invalid_input_cause(&self) -> Option<InvalidInputCause> {
        self.invalid_input_cause
    }
}

impl std::fmt::Display for CliError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.error.fmt(formatter)
    }
}

impl std::error::Error for CliError {}
