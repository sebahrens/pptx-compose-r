#![deny(warnings)]

mod cli;

use clap::Parser;
use cli::{Cli, Commands, MediaCmd};
use pptx_compose::core::error::{Error, ErrorCode};

fn main() {
    let cli = Cli::parse();

    if let Err(error) = run(cli) {
        eprintln!("{error}");
        std::process::exit(exit_code(error.code()));
    }
}

fn run(cli: Cli) -> Result<(), CliError> {
    match cli.command {
        Commands::Inspect(args) => inspect(args),
        Commands::Validate(args) => validate(args),
        Commands::Apply(args) => apply(args),
        Commands::Media(MediaCmd::List(args)) => media_list(args),
        Commands::Media(MediaCmd::Get(args)) => media_get(args),
        Commands::Schema(args) => schema(args),
    }
}

fn inspect(_args: cli::InspectArgs) -> Result<(), CliError> {
    Err(CliError::unsupported(
        "inspect command is not implemented yet",
    ))
}

fn validate(_args: cli::ValidateArgs) -> Result<(), CliError> {
    Err(CliError::unsupported(
        "validate command is not implemented yet",
    ))
}

fn apply(_args: cli::ApplyArgs) -> Result<(), CliError> {
    Err(CliError::unsupported(
        "apply command is not implemented yet",
    ))
}

fn media_list(_args: cli::MediaListArgs) -> Result<(), CliError> {
    Err(CliError::unsupported(
        "media list command is not implemented yet",
    ))
}

fn media_get(_args: cli::MediaGetArgs) -> Result<(), CliError> {
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
struct CliError(Error);

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
