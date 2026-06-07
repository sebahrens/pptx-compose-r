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
    #[arg(long)]
    pub keep_temp: bool,
    #[arg(long, value_name = "N")]
    pub max_compressed_bytes: Option<u64>,
    #[arg(long, value_name = "N")]
    pub max_uncompressed_bytes: Option<u64>,
    #[arg(long, value_name = "N")]
    pub max_part_count: Option<u64>,
    #[arg(long, value_name = "N")]
    pub max_media_bytes: Option<u64>,
}

#[derive(Debug, Eq, PartialEq, Subcommand)]
pub enum Commands {
    Capabilities,
    Inspect(InspectArgs),
    FindText(FindTextArgs),
    Validate(ValidateArgs),
    Apply(ApplyArgs),
    ToJson(LegacyToJsonArgs),
    ToPptx(LegacyToPptxArgs),
    Convert(LegacyConvertArgs),
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
    #[arg(
        long,
        value_name = "N|slide-N|N-M",
        help = "Slides to inspect: 1-based numbers, canonical slide-N ids, comma lists, or numeric ranges"
    )]
    pub slides: Option<String>,
    #[arg(long, value_enum)]
    pub detail: Option<InspectDetail>,
    #[arg(long)]
    pub overwrite: bool,
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
pub struct FindTextArgs {
    pub input: PathBuf,
    pub query: String,
    #[arg(long)]
    pub slide_id: Option<String>,
    #[arg(long)]
    pub cursor: Option<String>,
    #[arg(long, value_name = "N")]
    pub limit: Option<u32>,
    #[arg(long)]
    pub output: Option<PathBuf>,
}

#[derive(Args, Debug, Eq, PartialEq)]
pub struct ValidateArgs {
    pub input: PathBuf,
    #[arg(long)]
    pub report: Option<PathBuf>,
    #[arg(long)]
    pub overwrite: bool,
}

#[derive(Args, Debug, Eq, PartialEq)]
pub struct ApplyArgs {
    pub input: PathBuf,
    pub patch: PathBuf,
    #[arg(long)]
    pub dry_run: bool,
    #[arg(long)]
    pub media_manifest: Option<PathBuf>,
    #[arg(long, value_name = "DIR")]
    pub media_root: Option<PathBuf>,
    #[arg(long = "media", value_name = "MEDIA_REF=PATH")]
    pub media: Vec<String>,
    #[arg(long)]
    pub output: Option<PathBuf>,
    #[arg(long)]
    pub report: Option<PathBuf>,
    #[arg(long)]
    pub diff: Option<PathBuf>,
    #[arg(long)]
    pub overwrite: bool,
    #[arg(
        long,
        help = "Write back to INPUT atomically; creates INPUT.bak unless --no-backup is set"
    )]
    pub in_place: bool,
    #[arg(long)]
    pub no_backup: bool,
    #[arg(long)]
    pub deterministic: bool,
}

#[derive(Args, Debug, Eq, PartialEq)]
pub struct LegacyToJsonArgs {
    pub input: PathBuf,
    pub output: PathBuf,
    #[arg(long)]
    pub compat_json: bool,
}

#[derive(Args, Debug, Eq, PartialEq)]
pub struct LegacyToPptxArgs {
    pub input: PathBuf,
    pub output: PathBuf,
    #[arg(long)]
    pub compat_json: bool,
    #[arg(long)]
    pub overwrite: bool,
}

#[derive(Args, Debug, Eq, PartialEq)]
pub struct LegacyConvertArgs {
    pub input: PathBuf,
    pub output: PathBuf,
    #[arg(long)]
    pub compat_json: bool,
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
    #[arg(long)]
    pub overwrite: bool,
}

#[derive(Args, Debug, Eq, PartialEq)]
pub struct SchemaArgs {
    pub name: String,
}

#[cfg(test)]
#[test]
fn parses_capabilities() {
    use clap::Parser;

    let cli = Cli::try_parse_from(["pptx-compose", "capabilities"])
        .expect("capabilities command should parse");

    assert!(matches!(cli.command, Commands::Capabilities));
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
    assert_eq!(args.media_root, None);
    assert_eq!(args.media, Vec::<String>::new());
    assert_eq!(args.output, None);
    assert_eq!(args.diff, None);
    assert!(!args.overwrite);
    assert!(!args.in_place);
    assert!(!args.no_backup);
    assert!(!args.deterministic);
}

#[cfg(test)]
#[test]
fn parses_apply_media_bindings() {
    use clap::Parser;
    use std::path::PathBuf;

    let cli = Cli::try_parse_from([
        "pptx-compose",
        "apply",
        "in.pptx",
        "p.json",
        "--media-manifest",
        "media.json",
        "--media-root",
        "assets",
        "--media",
        "hero=override.png",
        "--media",
        "logo=logo.gif",
    ])
    .expect("apply media arguments should parse");

    let Commands::Apply(args) = cli.command else {
        unreachable!("expected apply command");
    };

    assert_eq!(args.media_manifest, Some(PathBuf::from("media.json")));
    assert_eq!(args.media_root, Some(PathBuf::from("assets")));
    assert_eq!(
        args.media,
        vec!["hero=override.png".to_owned(), "logo=logo.gif".to_owned()]
    );
}
