#![deny(warnings)]

mod cli;
mod commands;
mod exit;
mod output;
mod permissions;

use clap::{Parser, error::ErrorKind};
use cli::{Cli, Commands, MediaCmd};
use commands::apply::apply;
use exit::exit_code_for;
use output::{OutputDest, OutputSink};
use permissions::{PathIntent, PermissionContext};
use pptx_compose::{
    AgentViewOptions, MediaPartInfo, OpenOptions, PresentationDocument, ResourceLimits,
    ValidationMode,
    capabilities::{SCHEMA_CAPABILITIES, capabilities_json_schema},
    core::error::{Error, ErrorCode, ErrorDetails, ErrorLocation},
    edit::{media_inputs::media_manifest_json_schema, patch::patch_json_schema},
    json::{
        agent_view::{
            FindTextScope,
            pagination::MAX_PAGE_LIMIT,
            views::{FindTextRequest, ViewMode},
        },
        schemas::{
            JsonError, ResultEnvelope, ResultStatus, ValidationReport, agent_view_json_schema,
            error_json_schema, find_text_json_schema, patch_report_json_schema, result_json_schema,
            validation_report_json_schema,
        },
    },
};
use serde::Serialize;
use serde_json::{Value, json};
use std::{ffi::OsString, path::PathBuf};

fn main() {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error) => {
            if should_emit_json_parse_error(&error) && json_errors_requested(std::env::args_os()) {
                let sink = OutputSink::new(false, false, true, true);
                let cli_error = CliError::invalid_input(
                    InvalidInputCause::CliArgument,
                    parse_error_message(&error),
                );
                if let Err(emit_error) = sink.emit_error(&cli_error) {
                    eprintln!("{emit_error}");
                    std::process::exit(exit::WRITE_FAILURE);
                }
                std::process::exit(exit::USAGE);
            }
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
    let sink = OutputSink::from_global_args(&cli.global)
        .with_atomic_temp_dir(permissions.temp_dir.clone(), false);
    let open_options = open_options_from_global_args(&cli.global)?;
    match cli.command {
        Commands::Capabilities => capabilities(sink),
        Commands::Inspect(args) => inspect(args, &permissions, sink, open_options),
        Commands::FindText(args) => find_text(args, &permissions, sink, open_options),
        Commands::Validate(args) => validate(args, &permissions, sink, open_options),
        Commands::Apply(args) => apply(args, &permissions, open_options),
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
    if let Some(max_compressed_bytes) = global.max_compressed_bytes {
        resource_limits.max_compressed_package_bytes = max_compressed_bytes;
    }
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
    validate_page_limit(args.limit)?;
    let input = permissions.authorize_read(&args.input, PathIntent::InputPptx)?;
    let output = args
        .output
        .as_deref()
        .map(|output| permissions.authorize_write(output, PathIntent::ReportOutput))
        .transpose()?;

    let document = PresentationDocument::open_path_with_options(&input, open_options)
        .map_err(|error| open_input_error(error, &input))?;
    let scope = match args.slides {
        Some(slides) => {
            let slide_ids = parse_slide_scope(&slides)?;
            if slide_ids.len() != 1 {
                return Err(CliError::invalid_input(
                    InvalidInputCause::CliArgument,
                    "find-text --slides currently accepts exactly one slide.",
                ));
            }
            FindTextScope::Slide {
                slide_id: slide_ids.into_iter().next().ok_or_else(|| {
                    CliError::invalid_input(
                        InvalidInputCause::CliArgument,
                        "find-text --slides currently accepts exactly one slide.",
                    )
                })?,
            }
        }
        None => FindTextScope::Deck,
    };
    validate_find_text_scope(&document, &scope)?;
    let result = document
        .find_text(FindTextRequest {
            query: args.query,
            scope,
            cursor: args.cursor,
            limit: args.limit,
        })
        .map_err(read_scope_error)?;
    sink.emit_json_overwrite(&result, output::OutputDest::from(output), args.overwrite)
}

fn inspect(
    args: cli::InspectArgs,
    permissions: &PermissionContext,
    sink: OutputSink,
    open_options: OpenOptions,
) -> Result<(), CliError> {
    reject_inspect_stdout_collision(&args)?;

    let input = permissions.authorize_read(&args.input, PathIntent::InputPptx)?;
    let output = args
        .output
        .as_deref()
        .map(|output| permissions.authorize_write(output, PathIntent::ReportOutput))
        .transpose()?;
    let report_output = args
        .report
        .as_deref()
        .map(|report| permissions.authorize_write(report, PathIntent::ReportOutput))
        .transpose()?;

    let document = PresentationDocument::open_path_with_options(&input, open_options)
        .map_err(|error| open_input_error(error, &input))?;
    let view_options = inspect_view_options(&args)?;
    validate_agent_view_scope(&document, &view_options)?;
    let view = document
        .to_agent_json_with_options(view_options)
        .map_err(read_scope_error)?;
    sink.emit_json_overwrite(&view, OutputDest::from(output), args.overwrite)?;
    if report_output.is_some() {
        let report = document
            .validate_with_mode(ValidationMode::NoEdit)
            .map_err(CliError::from_error)?;
        sink.emit_json_overwrite(&report, OutputDest::from(report_output), args.overwrite)?;
    }
    Ok(())
}

fn reject_inspect_stdout_collision(args: &cli::InspectArgs) -> Result<(), CliError> {
    let view_targets_stdout = args
        .output
        .as_deref()
        .is_none_or(|output| output == std::path::Path::new("-"));
    let report_targets_stdout = args
        .report
        .as_deref()
        .is_some_and(|report| report == std::path::Path::new("-"));

    if view_targets_stdout && report_targets_stdout {
        return Err(CliError::invalid_input(
            InvalidInputCause::CliArgument,
            "inspect cannot write both the view and --report to stdout; give --output or --report a file path.",
        ));
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
    let report_output = args
        .report
        .as_deref()
        .map(|report| permissions.authorize_write(report, PathIntent::ReportOutput))
        .transpose()?;
    let document = PresentationDocument::open_path_with_options(&input, open_options)
        .map_err(CliError::from_error)?;
    let report = document
        .validate_with_mode(ValidationMode::NoEdit)
        .map_err(CliError::from_error)?;
    sink.emit_json_overwrite(&report, OutputDest::from(report_output), args.overwrite)?;
    if validation_report_has_blocking_findings(&report) {
        return Err(CliError::new(
            ErrorCode::ValidationFailed,
            "Package failed validation.",
        ));
    }
    Ok(())
}

fn validation_report_has_blocking_findings(report: &ValidationReport) -> bool {
    report.findings.iter().any(|finding| finding.blocking)
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
    sink.emit_json(&success_envelope(json!(result)), OutputDest::Stdout)
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
        .map_err(cli_argument_error)?;
    output::write_bytes_atomic(
        &output,
        &bytes,
        args.overwrite,
        Some(&permissions.temp_dir),
        false,
    )?;

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
        sink.emit_json_overwrite(
            &success_envelope(json!(report)),
            OutputDest::from(args.report),
            args.overwrite,
        )?;
    }
    Ok(())
}

fn schema(args: cli::SchemaArgs, sink: OutputSink) -> Result<(), CliError> {
    let schema = match args.name.as_str() {
        "capabilities-v1" => capabilities_json_schema().map_err(schema_error)?,
        "agent-view-v1" => agent_view_json_schema().map_err(schema_error)?,
        "find-text-v1" => find_text_json_schema().map_err(schema_error)?,
        "patch-v1" => patch_json_schema().map_err(CliError::from_error)?,
        "media-manifest-v1" => media_manifest_json_schema().map_err(CliError::from_error)?,
        "patch-report-v1" => patch_report_json_schema().map_err(schema_error)?,
        "validation-report-v1" => validation_report_json_schema().map_err(schema_error)?,
        "result-v1" => result_json_schema().map_err(schema_error)?,
        "error-v1" => error_json_schema().map_err(schema_error)?,
        _ => {
            let expected = SCHEMA_CAPABILITIES
                .iter()
                .map(|capability| capability.name)
                .collect::<Vec<_>>()
                .join(", ");
            return Err(CliError::invalid_input(
                InvalidInputCause::CliArgument,
                format!("Unknown schema name. Expected one of: {expected}."),
            ));
        }
    };
    sink.emit_json(&schema, OutputDest::Stdout)
}

fn should_emit_json_parse_error(error: &clap::Error) -> bool {
    !matches!(
        error.kind(),
        ErrorKind::DisplayHelp | ErrorKind::DisplayVersion
    )
}

fn parse_error_message(error: &clap::Error) -> String {
    error
        .to_string()
        .lines()
        .find(|line| !line.trim().is_empty())
        .map(|line| line.trim_start_matches("error: ").to_owned())
        .unwrap_or_else(|| "Command-line arguments are invalid.".to_owned())
}

fn json_errors_requested(args: impl IntoIterator<Item = OsString>) -> bool {
    args.into_iter().any(|arg| arg == "--json-errors")
}

fn schema_error(error: JsonError) -> CliError {
    match error {
        JsonError::SerializeSchema(message) => {
            CliError::from_error(Error::new(ErrorCode::InternalError, message))
        }
        JsonError::InvalidCursor(message)
        | JsonError::MalformedLegacyEnvelope(message)
        | JsonError::Projection(message) => {
            CliError::from_error(Error::new(ErrorCode::InvalidInput, message))
        }
        JsonError::ResourceLimitExceeded(message) => {
            CliError::from_error(Error::resource_limit_exceeded(message))
        }
        JsonError::Core(error) => CliError::from_error(error),
        JsonError::NotFound { kind, id } => CliError::from_error(Error::new(
            ErrorCode::InvalidInput,
            format!("{kind} `{id}` was not found."),
        )),
    }
}

fn inspect_view_options(args: &cli::InspectArgs) -> Result<AgentViewOptions, CliError> {
    let mut options = AgentViewOptions::default();
    if matches!(args.detail, Some(cli::InspectDetail::Full)) {
        options.mode = ViewMode::SlidePage;
        options.include_elements = true;
        options.limit = Some(MAX_PAGE_LIMIT);
    }
    if let Some(slides) = &args.slides {
        let slide_ids = parse_slide_scope(slides)?;
        if slide_ids.len() == 1 {
            options.mode = ViewMode::SlideDetail;
            options.slide_id = slide_ids.into_iter().next();
        } else {
            options.mode = ViewMode::SlidePage;
            options.slide_ids = slide_ids;
        }
    }
    options.cursor.clone_from(&args.cursor);
    if let Some(limit) = args.limit {
        validate_page_limit(Some(limit))?;
        options.limit = Some(limit);
    }
    Ok(options)
}

fn validate_find_text_scope(
    document: &PresentationDocument,
    scope: &FindTextScope,
) -> Result<(), CliError> {
    if let FindTextScope::Slide { slide_id } = scope {
        validate_slide_id_exists(document, slide_id)?;
    }
    Ok(())
}

fn validate_agent_view_scope(
    document: &PresentationDocument,
    options: &AgentViewOptions,
) -> Result<(), CliError> {
    if let Some(slide_id) = &options.slide_id {
        validate_slide_id_exists(document, slide_id)?;
    }
    for slide_id in &options.slide_ids {
        validate_slide_id_exists(document, slide_id)?;
    }
    Ok(())
}

fn validate_slide_id_exists(
    document: &PresentationDocument,
    slide_id: &str,
) -> Result<(), CliError> {
    let slide_number = parse_slide_id_token(slide_id)?;
    let slide_count = document.slide_count();
    if slide_number > slide_count {
        return Err(CliError::invalid_input(
            InvalidInputCause::CliArgument,
            format!("Slide `{slide_id}` was not found."),
        ));
    }
    Ok(())
}

fn validate_page_limit(limit: Option<u32>) -> Result<(), CliError> {
    if let Some(limit) = limit
        && (limit == 0 || limit > MAX_PAGE_LIMIT)
    {
        return Err(CliError::invalid_input(
            InvalidInputCause::CliArgument,
            format!("--limit must be between 1 and {MAX_PAGE_LIMIT}."),
        ));
    }
    Ok(())
}

fn open_input_error(error: Error, input: &std::path::Path) -> CliError {
    let error = error.with_location(ErrorLocation {
        part: Some(input.display().to_string()),
        ..ErrorLocation::default()
    });
    if error.code() == ErrorCode::InvalidInput
        && error.message().starts_with("Could not read PPTX input")
    {
        CliError::invalid_input_with_source(
            InvalidInputCause::InputPath,
            error.message().to_owned(),
            error,
        )
    } else {
        CliError::from_error(error)
    }
}

fn read_scope_error(error: Error) -> CliError {
    match error.code() {
        ErrorCode::InvalidInput => CliError::invalid_input_with_source(
            InvalidInputCause::CliArgument,
            error.message().to_owned(),
            error,
        ),
        ErrorCode::SelectorNotFound => CliError::invalid_input_with_source(
            InvalidInputCause::CliArgument,
            error.message().to_owned(),
            error,
        ),
        _ => CliError::from_error(error),
    }
}

fn cli_argument_error(error: Error) -> CliError {
    if error.code() == ErrorCode::InvalidInput {
        CliError::invalid_input_with_source(
            InvalidInputCause::CliArgument,
            error.message().to_owned(),
            error,
        )
    } else {
        CliError::from_error(error)
    }
}

fn parse_slide_scope(slides: &str) -> Result<Vec<String>, CliError> {
    let trimmed = slides.trim();
    if trimmed.is_empty() {
        return Err(CliError::invalid_input(
            InvalidInputCause::CliArgument,
            "--slides must not be empty.",
        ));
    }

    let mut slide_ids = Vec::new();
    for token in trimmed.split(',') {
        let token = token.trim();
        if token.is_empty() {
            return Err(CliError::invalid_input(
                InvalidInputCause::CliArgument,
                "--slides contains an empty list item.",
            ));
        }
        if token.starts_with("slide-") {
            let number = parse_slide_id_token(token)?;
            push_unique_slide_id(&mut slide_ids, format!("slide-{number}"));
        } else if let Some((start, end)) = token.split_once('-') {
            let start = parse_slide_number(start)?;
            let end = parse_slide_number(end)?;
            if start > end {
                return Err(CliError::invalid_input(
                    InvalidInputCause::CliArgument,
                    "--slides ranges must be ascending.",
                ));
            }
            for number in start..=end {
                push_unique_slide_id(&mut slide_ids, format!("slide-{number}"));
            }
        } else {
            let number = parse_slide_number(token)?;
            push_unique_slide_id(&mut slide_ids, format!("slide-{number}"));
        }
    }
    Ok(slide_ids)
}

fn parse_slide_id_token(value: &str) -> Result<u32, CliError> {
    let number = value.strip_prefix("slide-").ok_or_else(|| {
        CliError::invalid_input(
            InvalidInputCause::CliArgument,
            "--slides canonical slide ids must use the slide-N form.",
        )
    })?;
    parse_slide_number(number)
}

fn parse_slide_number(value: &str) -> Result<u32, CliError> {
    let number = value.trim().parse::<u32>().map_err(|source| {
        CliError::invalid_input_with_source(
            InvalidInputCause::CliArgument,
            "--slides values must be positive slide numbers.",
            source,
        )
    })?;
    if number == 0 {
        return Err(CliError::invalid_input(
            InvalidInputCause::CliArgument,
            "--slides values are 1-based and must be greater than zero.",
        ));
    }
    Ok(number)
}

fn push_unique_slide_id(slide_ids: &mut Vec<String>, slide_id: String) {
    if !slide_ids.contains(&slide_id) {
        slide_ids.push(slide_id);
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
        checksum: pptx_compose::part_checksum(bytes),
    }
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

    fn into_error(self) -> Error {
        self.error
    }

    fn with_state_changed(mut self) -> Self {
        self.error = self.error.with_state_changed(true);
        self
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
