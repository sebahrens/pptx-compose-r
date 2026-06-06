use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};

#[derive(Debug, Parser)]
#[command(
    name = "pptx-compose",
    version,
    about = "Scriptable PPTX compose CLI",
    disable_help_subcommand = true
)]
pub struct Cli {
    #[command(flatten)]
    pub global: GlobalArgs,
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Args, Debug, Default, Eq, PartialEq)]
pub struct GlobalArgs {
    #[arg(long)]
    pub json_errors: bool,
    #[arg(long)]
    pub quiet: bool,
    #[arg(long)]
    pub verbose: bool,
    #[arg(long)]
    pub no_color: bool,
    #[arg(long, value_name = "DIR")]
    pub workspace: Option<PathBuf>,
    #[arg(long, value_name = "DIR")]
    pub temp_dir: Option<PathBuf>,
    #[arg(long, value_name = "N")]
    pub max_uncompressed_bytes: Option<u64>,
    #[arg(long, value_name = "N")]
    pub max_part_count: Option<u64>,
}

#[derive(Debug, Eq, PartialEq, Subcommand)]
pub enum Commands {
    Inspect(InspectArgs),
    Validate(ValidateArgs),
    Apply(ApplyArgs),
    #[command(subcommand)]
    Media(MediaCmd),
    Schema(SchemaArgs),
}

#[derive(Args, Debug, Eq, PartialEq)]
pub struct InspectArgs {
    pub input: PathBuf,
    #[arg(long, value_enum)]
    pub format: Option<InspectFormat>,
    #[arg(long)]
    pub output: Option<PathBuf>,
    #[arg(long)]
    pub report: Option<PathBuf>,
    #[arg(long)]
    pub slides: Option<String>,
    #[arg(long, value_enum)]
    pub detail: Option<InspectDetail>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum InspectFormat {
    AgentJson,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum InspectDetail {
    Summary,
    Full,
}

#[derive(Args, Debug, Eq, PartialEq)]
pub struct ValidateArgs {
    pub input: PathBuf,
    #[arg(long)]
    pub report: Option<PathBuf>,
}

#[derive(Args, Debug, Eq, PartialEq)]
pub struct ApplyArgs {
    pub input: PathBuf,
    pub patch: PathBuf,
    #[arg(long)]
    pub dry_run: bool,
    #[arg(long)]
    pub media_manifest: Option<PathBuf>,
    #[arg(long)]
    pub output: Option<PathBuf>,
    #[arg(long)]
    pub report: Option<PathBuf>,
    #[arg(long)]
    pub diff: Option<PathBuf>,
    #[arg(long)]
    pub overwrite: bool,
    #[arg(long)]
    pub in_place: bool,
    #[arg(long)]
    pub deterministic: bool,
}

#[derive(Debug, Eq, PartialEq, Subcommand)]
pub enum MediaCmd {
    List(MediaListArgs),
    Get(MediaGetArgs),
}

#[derive(Args, Debug, Eq, PartialEq)]
pub struct MediaListArgs {
    pub input: PathBuf,
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug, Eq, PartialEq)]
pub struct MediaGetArgs {
    pub input: PathBuf,
    pub package_path: String,
    #[arg(long)]
    pub output: PathBuf,
    #[arg(long)]
    pub report: Option<PathBuf>,
}

#[derive(Args, Debug, Eq, PartialEq)]
pub struct SchemaArgs {
    pub name: String,
}

#[cfg(test)]
#[test]
fn parses_apply_dry_run() {
    use clap::Parser;
    use std::path::PathBuf;

    let cli = Cli::try_parse_from([
        "pptx-compose",
        "apply",
        "in.pptx",
        "p.json",
        "--dry-run",
        "--report",
        "r.json",
    ])
    .expect("apply dry-run arguments should parse");

    assert!(matches!(cli.command, Commands::Apply(_)));
    let Commands::Apply(args) = cli.command else {
        unreachable!("asserted apply command above");
    };

    assert_eq!(args.input, PathBuf::from("in.pptx"));
    assert_eq!(args.patch, PathBuf::from("p.json"));
    assert!(args.dry_run);
    assert_eq!(args.report, Some(PathBuf::from("r.json")));
    assert_eq!(args.media_manifest, None);
    assert_eq!(args.output, None);
    assert_eq!(args.diff, None);
    assert!(!args.overwrite);
    assert!(!args.in_place);
    assert!(!args.deterministic);
}
