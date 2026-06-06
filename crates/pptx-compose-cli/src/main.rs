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
use output::OutputSink;
use permissions::{PathIntent, PermissionContext};
use pptx_compose::{
    PresentationDocument,
    core::error::{Error, ErrorCode, ErrorDetails, ErrorLocation},
    json::agent_view::{FindTextScope, views::FindTextRequest},
};

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
    match cli.command {
        Commands::Inspect(args) => inspect(args, &permissions),
        Commands::FindText(args) => find_text(args, &permissions, sink),
        Commands::Validate(args) => validate(args, &permissions),
        Commands::Apply(args) => apply(args, &permissions),
        Commands::ToJson(args) => run_to_json(args, &permissions),
        Commands::ToPptx(args) => run_to_pptx(args, &permissions),
        Commands::Convert(args) => run_convert(args, &permissions),
        Commands::Media(MediaCmd::List(args)) => media_list(args, &permissions),
        Commands::Media(MediaCmd::Get(args)) => media_get(args, &permissions),
        Commands::Schema(args) => schema(args),
    }
}

fn find_text(
    args: cli::FindTextArgs,
    permissions: &PermissionContext,
    sink: OutputSink,
) -> Result<(), CliError> {
    permissions.authorize_read(&args.input, PathIntent::InputPptx)?;
    if let Some(output) = &args.output {
        permissions.authorize_write(output, PathIntent::ReportOutput)?;
    }

    let document = PresentationDocument::open_path(&args.input).map_err(|error| {
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

fn inspect(args: cli::InspectArgs, permissions: &PermissionContext) -> Result<(), CliError> {
    permissions.authorize_read(&args.input, PathIntent::InputPptx)?;
    if let Some(output) = &args.output {
        permissions.authorize_write(output, PathIntent::OutputPptx)?;
    }
    if let Some(report) = &args.report {
        permissions.authorize_write(report, PathIntent::ReportOutput)?;
    }
    Err(CliError::unsupported(
        "inspect command is not implemented yet",
    ))
}

fn validate(args: cli::ValidateArgs, permissions: &PermissionContext) -> Result<(), CliError> {
    permissions.authorize_read(&args.input, PathIntent::InputPptx)?;
    if let Some(report) = &args.report {
        permissions.authorize_write(report, PathIntent::ReportOutput)?;
    }
    Err(CliError::unsupported(
        "validate command is not implemented yet",
    ))
}

fn media_list(args: cli::MediaListArgs, permissions: &PermissionContext) -> Result<(), CliError> {
    permissions.authorize_read(&args.input, PathIntent::InputPptx)?;
    Err(CliError::unsupported(
        "media list command is not implemented yet",
    ))
}

fn media_get(args: cli::MediaGetArgs, permissions: &PermissionContext) -> Result<(), CliError> {
    permissions.authorize_read(&args.input, PathIntent::InputPptx)?;
    permissions.authorize_write(&args.output, PathIntent::OutputPptx)?;
    if let Some(report) = &args.report {
        permissions.authorize_write(report, PathIntent::ReportOutput)?;
    }
    Err(CliError::unsupported(
        "media get command is not implemented yet",
    ))
}

fn schema(_args: cli::SchemaArgs) -> Result<(), CliError> {
    Err(CliError::unsupported(
        "schema command is not implemented yet",
    ))
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

    fn unsupported(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::UnsupportedEdit, message)
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
