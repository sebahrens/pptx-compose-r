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
    #[arg(long, global = true)]
    pub json_errors: bool,
    #[arg(long, global = true)]
    pub quiet: bool,
    #[arg(long, global = true)]
    pub verbose: bool,
    #[arg(long, global = true)]
    pub no_color: bool,
    #[arg(long, global = true, value_name = "DIR")]
    pub workspace: Option<PathBuf>,
    #[arg(long, global = true, value_name = "DIR")]
    pub temp_dir: Option<PathBuf>,
    #[arg(long, global = true)]
    pub keep_temp: bool,
    #[arg(long, global = true, value_name = "N")]
    pub max_compressed_bytes: Option<u64>,
    #[arg(long, global = true, value_name = "N")]
    pub max_uncompressed_bytes: Option<u64>,
    #[arg(long, global = true, value_name = "N")]
    pub max_part_count: Option<u64>,
    #[arg(long, global = true, value_name = "N")]
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
    pub cursor: Option<String>,
    #[arg(long, value_name = "N")]
    pub limit: Option<u32>,
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
    #[arg(
        long,
        value_name = "N|slide-N",
        help = "Single slide to search: a 1-based number or canonical slide-N id"
    )]
    pub slides: Option<String>,
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

#[cfg(test)]
#[test]
fn parses_find_text_slides_scope() {
    use clap::Parser;
    use std::path::PathBuf;

    let cli = Cli::try_parse_from([
        "pptx-compose",
        "find-text",
        "in.pptx",
        "needle",
        "--slides",
        "slide-2",
        "--limit",
        "5",
        "--output",
        "matches.json",
    ])
    .expect("find-text arguments should parse");

    let Commands::FindText(args) = cli.command else {
        unreachable!("expected find-text command");
    };

    assert_eq!(args.input, PathBuf::from("in.pptx"));
    assert_eq!(args.query, "needle");
    assert_eq!(args.slides, Some("slide-2".to_owned()));
    assert_eq!(args.cursor, None);
    assert_eq!(args.limit, Some(5));
    assert_eq!(args.output, Some(PathBuf::from("matches.json")));
}

#[cfg(test)]
#[test]
fn parses_inspect_pagination_flags() {
    use clap::Parser;
    use std::path::PathBuf;

    let cli = Cli::try_parse_from([
        "pptx-compose",
        "inspect",
        "in.pptx",
        "--detail",
        "full",
        "--cursor",
        "opaque",
        "--limit",
        "5",
    ])
    .expect("inspect pagination arguments should parse");

    let Commands::Inspect(args) = cli.command else {
        unreachable!("expected inspect command");
    };

    assert_eq!(args.input, PathBuf::from("in.pptx"));
    assert_eq!(args.detail, Some(InspectDetail::Full));
    assert_eq!(args.cursor, Some("opaque".to_owned()));
    assert_eq!(args.limit, Some(5));
}

#[cfg(test)]
#[test]
fn spec_071_documents_command_variants() {
    use clap::CommandFactory;

    let spec = include_str!("../../../specs/071-cli-agent-contract.md");
    let command = Cli::command();
    let documented_commands = [
        "capabilities",
        "inspect",
        "find-text",
        "validate",
        "apply",
        "to-json",
        "to-pptx",
        "convert",
        "media",
        "schema",
    ];

    for subcommand in command.get_subcommands() {
        let name = subcommand.get_name();
        assert!(
            documented_commands.contains(&name),
            "test must enumerate CLI command variant `{name}`"
        );
        assert!(
            spec.contains(&format!("### `{name}`"))
                || spec.contains(&format!("#### `{name}`"))
                || spec.contains(&format!("`{name}`")),
            "spec 071 must document CLI command variant `{name}`"
        );
    }

    assert_eq!(
        command.get_subcommands().count(),
        documented_commands.len(),
        "test command list must match cli.rs Commands variants"
    );
}

#[cfg(test)]
#[test]
fn parses_trailing_global_flags_after_subcommands() {
    use clap::Parser;
    use std::path::PathBuf;

    let inspect = Cli::try_parse_from([
        "pptx-compose",
        "inspect",
        "INPUT.pptx",
        "--slides",
        "1-5",
        "--detail",
        "summary",
        "--output",
        "-",
        "--json-errors",
    ])
    .expect("inspect with trailing global flag should parse");

    assert!(inspect.global.json_errors);
    let Commands::Inspect(args) = inspect.command else {
        unreachable!("expected inspect command");
    };
    assert_eq!(args.input, PathBuf::from("INPUT.pptx"));
    assert_eq!(args.output, Some(PathBuf::from("-")));

    let validate = Cli::try_parse_from([
        "pptx-compose",
        "validate",
        "INPUT.pptx",
        "--report",
        "-",
        "--json-errors",
    ])
    .expect("validate with trailing global flag should parse");

    assert!(validate.global.json_errors);
    let Commands::Validate(args) = validate.command else {
        unreachable!("expected validate command");
    };
    assert_eq!(args.input, PathBuf::from("INPUT.pptx"));
    assert_eq!(args.report, Some(PathBuf::from("-")));
}
