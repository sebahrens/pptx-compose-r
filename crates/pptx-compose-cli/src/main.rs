#![deny(warnings)]

mod cli;
mod permissions;

use clap::Parser;
use cli::{Cli, Commands, MediaCmd};
use permissions::{PathIntent, PermissionContext};
use pptx_compose::core::error::{Error, ErrorCode};

fn main() {
    let cli = Cli::parse();

    if let Err(error) = run(cli) {
        eprintln!("{error}");
        std::process::exit(exit_code(error.code()));
    }
}

fn run(cli: Cli) -> Result<(), CliError> {
    let permissions = PermissionContext::from_global_args(&cli.global)?;
    match cli.command {
        Commands::Inspect(args) => inspect(args, &permissions),
        Commands::Validate(args) => validate(args, &permissions),
        Commands::Apply(args) => apply(args, &permissions),
        Commands::Media(MediaCmd::List(args)) => media_list(args, &permissions),
        Commands::Media(MediaCmd::Get(args)) => media_get(args, &permissions),
        Commands::Schema(args) => schema(args),
    }
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

fn apply(args: cli::ApplyArgs, permissions: &PermissionContext) -> Result<(), CliError> {
    permissions.authorize_read(&args.input, PathIntent::InputPptx)?;
    permissions.authorize_read(&args.patch, PathIntent::InputPptx)?;
    if let Some(manifest) = &args.media_manifest {
        permissions.authorize_read(manifest, PathIntent::MediaInput)?;
    }
    if let Some(output) = &args.output {
        permissions.authorize_write(output, PathIntent::OutputPptx)?;
    }
    if let Some(report) = &args.report {
        permissions.authorize_write(report, PathIntent::ReportOutput)?;
    }
    if let Some(diff) = &args.diff {
        permissions.authorize_write(diff, PathIntent::DiffOutput)?;
    }
    Err(CliError::unsupported(
        "apply command is not implemented yet",
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
pub(crate) struct CliError(Error);

impl CliError {
    fn unsupported(message: impl Into<String>) -> Self {
        Self(Error::new(ErrorCode::UnsupportedEdit, message))
    }

    const fn code(&self) -> ErrorCode {
        self.0.code()
    }
}

impl std::fmt::Display for CliError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(formatter)
    }
}

impl std::error::Error for CliError {}

const fn exit_code(code: ErrorCode) -> i32 {
    match code {
        ErrorCode::InvalidInput => 1,
        ErrorCode::UnsafePath | ErrorCode::PermissionDenied => 3,
        ErrorCode::ParseError | ErrorCode::MalformedXml => 10,
        ErrorCode::UnsupportedPackage => 11,
        ErrorCode::ResourceLimitExceeded => 12,
        ErrorCode::InvalidBounds => 20,
        ErrorCode::StalePatch => 21,
        ErrorCode::SelectorNotFound
        | ErrorCode::SelectorAmbiguous
        | ErrorCode::SelectorGuardFailed => 22,
        ErrorCode::MissingMediaRef
        | ErrorCode::MediaChecksumMismatch
        | ErrorCode::UnsupportedMediaType => 23,
        ErrorCode::UnsupportedEdit => 24,
        ErrorCode::ValidationFailed => 30,
        ErrorCode::WriteFailed => 40,
        ErrorCode::InternalError => 50,
    }
}
