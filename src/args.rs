use std::path::PathBuf;
use std::str::FromStr;
use structopt::clap::AppSettings;
use structopt::StructOpt;

#[derive(StructOpt)]
#[structopt(bin_name = "cargo")]
pub enum Opts {
    #[structopt(
    name = "tree",
    setting = AppSettings::UnifiedHelpMessage,
    setting = AppSettings::DeriveDisplayOrder,
    setting = AppSettings::DontCollapseArgsInUsage
    )]
    /// Display a tree visualization of a dependency graph
    Tree(Args),
}

#[derive(StructOpt)]
pub struct Args {
    #[structopt(long = "package", short = "p", value_name = "SPEC")]
    /// Package to be used as the root of the tree
    pub package: Option<String>,
    #[structopt(long = "features", value_name = "FEATURES")]
    /// Space-separated list of features to activate
    pub features: Option<String>,
    #[structopt(long = "all-features")]
    /// Activate all available features
    pub all_features: bool,
    #[structopt(long = "no-default-features")]
    /// Do not activate the `default` feature
    pub no_default_features: bool,
    #[structopt(long = "target", value_name = "TARGET")]
    /// Set the target triple
    pub target: Option<String>,
    #[structopt(long = "all-targets")]
    /// Return dependencies for all targets. By default only the host target is matched.
    pub all_targets: bool,
    #[structopt(long = "no-dev-dependencies")]
    /// Skip dev dependencies.
    pub no_dev_dependencies: bool,
    #[structopt(long = "manifest-path", value_name = "PATH", parse(from_os_str))]
    /// Path to Cargo.toml
    pub manifest_path: Option<PathBuf>,
    #[structopt(long = "invert", short = "i")]
    /// Invert the tree direction
    pub invert: bool,
    #[structopt(long = "no-indent")]
    /// Display the dependencies as a list (rather than a tree)
    pub no_indent: bool,
    #[structopt(long = "prefix-depth")]
    /// Display the dependencies as a list (rather than a tree), but prefixed with the depth
    pub prefix_depth: bool,
    #[structopt(long = "all", short = "a")]
    /// Don't truncate dependencies that have already been displayed
    pub all: bool,
    #[structopt(long = "duplicate", short = "d")]
    /// Show only dependencies which come in multiple versions (implies -i)
    pub duplicates: bool,
    #[structopt(long = "charset", value_name = "CHARSET", default_value = "utf8")]
    /// Character set to use in output: utf8, ascii
    pub charset: Charset,
    #[structopt(
        long = "format",
        short = "f",
        value_name = "FORMAT",
        default_value = "{p}"
    )]
    /// Format string used for printing dependencies
    pub format: String,
    #[structopt(long = "verbose", short = "v", parse(from_occurrences))]
    /// Use verbose output (-vv very verbose/build.rs output)
    pub verbose: u32,
    #[structopt(long = "quiet", short = "q")]
    /// No output printed to stdout other than the tree
    pub quiet: bool,
    #[structopt(long = "color", value_name = "WHEN")]
    /// Coloring: auto, always, never
    pub color: Option<String>,
    #[structopt(long = "frozen")]
    /// Require Cargo.lock and cache are up to date
    pub frozen: bool,
    #[structopt(long = "locked")]
    /// Require Cargo.lock is up to date
    pub locked: bool,
    #[structopt(long = "offline")]
    /// Do not access the network
    pub offline: bool,
    #[structopt(short = "Z", value_name = "FLAG")]
    /// Unstable (nightly-only) flags to Cargo
    pub unstable_flags: Vec<String>,
}

pub enum Charset {
    Utf8,
    Ascii,
}

impl FromStr for Charset {
    type Err = &'static str;

    fn from_str(s: &str) -> Result<Charset, &'static str> {
        match s {
            "utf8" => Ok(Charset::Utf8),
            "ascii" => Ok(Charset::Ascii),
            _ => Err("invalid charset"),
        }
    }
}
