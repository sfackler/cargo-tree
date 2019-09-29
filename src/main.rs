use cargo::core::dependency::Kind;
use cargo::core::manifest::ManifestMetadata;
use cargo::core::maybe_allow_nightly_features;
use cargo::core::package::PackageSet;
use cargo::core::registry::PackageRegistry;
use cargo::core::resolver::Method;
use cargo::core::shell::Shell;
use cargo::core::{Package, PackageId, Resolve, Workspace};
use cargo::ops;
use cargo::util::{self, important_paths, CargoResult, Cfg, Rustc};
use cargo::{CliResult, Config};
use failure::bail;
use petgraph::graph::NodeIndex;
use petgraph::visit::EdgeRef;
use petgraph::EdgeDirection;
use std::collections::hash_map::Entry;
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::str::{self, FromStr};
use std::rc::Rc;
use structopt::clap::AppSettings;
use structopt::StructOpt;

use crate::format::Pattern;

mod format;

#[derive(StructOpt)]
#[structopt(bin_name = "cargo")]
enum Opts {
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
struct Args {
    #[structopt(long = "package", short = "p", value_name = "SPEC")]
    /// Package to be used as the root of the tree
    package: Option<String>,
    #[structopt(long = "features", value_name = "FEATURES")]
    /// Space-separated list of features to activate
    features: Option<String>,
    #[structopt(long = "all-features")]
    /// Activate all available features
    all_features: bool,
    #[structopt(long = "no-default-features")]
    /// Do not activate the `default` feature
    no_default_features: bool,
    #[structopt(long = "target", value_name = "TARGET")]
    /// Set the target triple
    target: Option<String>,
    /// Directory for all generated artifacts
    #[structopt(long = "target-dir", value_name = "DIRECTORY", parse(from_os_str))]
    target_dir: Option<PathBuf>,
    #[structopt(long = "all-targets")]
    /// Return dependencies for all targets. By default only the host target is matched.
    all_targets: bool,
    #[structopt(long = "no-dev-dependencies")]
    /// Skip dev dependencies.
    no_dev_dependencies: bool,
    #[structopt(long = "depth", short = "D", value_name = "DEPTH")]
    /// Max display depth of the dependency tree
    depth: Option<usize>,
    #[structopt(long = "manifest-path", value_name = "PATH", parse(from_os_str))]
    /// Path to Cargo.toml
    manifest_path: Option<PathBuf>,
    #[structopt(long = "invert", short = "i")]
    /// Invert the tree direction
    invert: bool,
    #[structopt(long = "no-indent")]
    /// Display the dependencies as a list (rather than a tree)
    no_indent: bool,
    #[structopt(long = "prefix-depth")]
    /// Display the dependencies as a list (rather than a tree), but prefixed with the depth
    prefix_depth: bool,
    #[structopt(long = "all", short = "a")]
    /// Don't truncate dependencies that are farther down the dependency tree
    all: bool,
    #[structopt(long = "duplicate", short = "d")]
    /// Show only dependencies which come in multiple versions (implies -i)
    duplicates: bool,
    #[structopt(long = "charset", value_name = "CHARSET", default_value = "utf8")]
    /// Character set to use in output: utf8, ascii
    charset: Charset,
    #[structopt(
        long = "format",
        short = "f",
        value_name = "FORMAT",
        default_value = "{p}"
    )]
    /// Format string used for printing dependencies
    format: String,
    #[structopt(long = "verbose", short = "v", parse(from_occurrences))]
    /// Use verbose output (-vv very verbose/build.rs output)
    verbose: u32,
    #[structopt(long = "quiet", short = "q")]
    /// No output printed to stdout other than the tree
    quiet: Option<bool>,
    #[structopt(long = "color", value_name = "WHEN")]
    /// Coloring: auto, always, never
    color: Option<String>,
    #[structopt(long = "frozen")]
    /// Require Cargo.lock and cache are up to date
    frozen: bool,
    #[structopt(long = "locked")]
    /// Require Cargo.lock is up to date
    locked: bool,
    #[structopt(long = "offline")]
    /// Do not access the network
    offline: bool,
    #[structopt(short = "Z", value_name = "FLAG")]
    /// Unstable (nightly-only) flags to Cargo
    unstable_flags: Vec<String>,
}

enum Charset {
    Utf8,
    Ascii,
}

#[derive(Clone, Copy)]
enum Prefix {
    None,
    Indent,
    Depth,
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

struct Symbols {
    down: &'static str,
    tee: &'static str,
    ell: &'static str,
    right: &'static str,
}

static UTF8_SYMBOLS: Symbols = Symbols {
    down: "│",
    tee: "├",
    ell: "└",
    right: "─",
};

static ASCII_SYMBOLS: Symbols = Symbols {
    down: "|",
    tee: "|",
    ell: "`",
    right: "-",
};

fn main() {
    env_logger::init();

    let mut config = match Config::default() {
        Ok(cfg) => cfg,
        Err(e) => {
            let mut shell = Shell::new();
            cargo::exit_with_error(e.into(), &mut shell)
        }
    };

    let Opts::Tree(args) = Opts::from_args();

    if let Err(e) = real_main(args, &mut config) {
        let mut shell = Shell::new();
        cargo::exit_with_error(e.into(), &mut shell)
    }
}

fn real_main(args: Args, config: &mut Config) -> CliResult {
    config.configure(
        args.verbose,
        args.quiet,
        &args.color,
        args.frozen,
        args.locked,
        args.offline,
        &args.target_dir,
        &args.unstable_flags,
    )?;

    // Needed to allow nightly features
    maybe_allow_nightly_features();

    let workspace = workspace(config, args.manifest_path)?;
    let package = workspace.current()?;
    let mut registry = registry(config, &package)?;
    let (packages, resolve) = resolve(
        &mut registry,
        &workspace,
        args.features,
        args.all_features,
        args.no_default_features,
        args.no_dev_dependencies,
    )?;
    let ids = packages.package_ids().collect::<Vec<_>>();
    let packages = registry.get(&ids)?;

    let root = match args.package {
        Some(ref pkg) => resolve.query(pkg)?,
        None => package.package_id(),
    };

    let rustc = config.load_global_rustc(Some(&workspace))?;

    let target = if args.all_targets {
        None
    } else {
        Some(args.target.as_ref().unwrap_or(&rustc.host).as_str())
    };

    let format = Pattern::new(&args.format).map_err(|e| failure::err_msg(e.to_string()))?;

    let cfgs = get_cfgs(&rustc, &args.target)?;
    let graph = build_graph(
        &resolve,
        &packages,
        package.package_id(),
        target,
        cfgs.as_ref().map(|r| &**r),
        if args.duplicates || args.package.is_some() {
            None
        } else {
            args.depth
        },
    )?;

    let direction = if args.invert || args.duplicates {
        EdgeDirection::Incoming
    } else {
        EdgeDirection::Outgoing
    };

    let symbols = match args.charset {
        Charset::Ascii => &ASCII_SYMBOLS,
        Charset::Utf8 => &UTF8_SYMBOLS,
    };

    let prefix = if args.prefix_depth {
        Prefix::Depth
    } else if args.no_indent {
        Prefix::None
    } else {
        Prefix::Indent
    };

    if args.duplicates {
        let dups = find_duplicates(&graph);
        for dup in &dups {
            print_tree(
                dup, &graph, &format, direction, symbols, prefix, args.all, args.depth,
            )?;
            println!();
        }
    } else {
        print_tree(
            &root, &graph, &format, direction, symbols, prefix, args.all, args.depth,
        )?;
    }

    Ok(())
}

fn find_duplicates<'a>(graph: &Graph<'a>) -> Vec<PackageId> {
    let mut counts = HashMap::new();

    // Count by name only. Source and version are irrelevant here.
    for package in graph.nodes.keys() {
        *counts.entry(package.name()).or_insert(0) += 1;
    }

    // Theoretically inefficient, but in practice we're only listing duplicates and
    // there won't be enough dependencies for it to matter.
    let mut dup_ids = Vec::new();
    for name in counts.drain().filter(|&(_, v)| v > 1).map(|(k, _)| k) {
        dup_ids.extend(graph.nodes.keys().filter(|p| p.name() == name));
    }
    dup_ids.sort();
    dup_ids
}

fn get_cfgs(rustc: &Rustc, target: &Option<String>) -> CargoResult<Option<Vec<Cfg>>> {
    let mut process = util::process(&rustc.path);
    process.arg("--print=cfg").env_remove("RUST_LOG");
    if let Some(ref s) = *target {
        process.arg("--target").arg(s);
    }

    let output = match process.exec_with_output() {
        Ok(output) => output,
        Err(e) => return Err(e),
    };
    let output = str::from_utf8(&output.stdout).unwrap();
    let lines = output.lines();
    Ok(Some(
        lines.map(Cfg::from_str).collect::<CargoResult<Vec<_>>>()?,
    ))
}

fn workspace(config: &Config, manifest_path: Option<PathBuf>) -> CargoResult<Workspace<'_>> {
    let root = match manifest_path {
        Some(path) => path,
        None => important_paths::find_root_manifest_for_wd(config.cwd())?,
    };
    Workspace::new(&root, config)
}

fn registry<'a>(config: &'a Config, package: &Package) -> CargoResult<PackageRegistry<'a>> {
    let mut registry = PackageRegistry::new(config)?;
    registry.add_sources(Some(package.package_id().source_id().clone()))?;
    Ok(registry)
}

fn resolve<'a, 'cfg>(
    registry: &mut PackageRegistry<'cfg>,
    workspace: &'a Workspace<'cfg>,
    features: Option<String>,
    all_features: bool,
    no_default_features: bool,
    no_dev_dependencies: bool,
) -> CargoResult<(PackageSet<'a>, Resolve)> {
    let features = Method::split_features(&features.into_iter().collect::<Vec<_>>());

    let (packages, resolve) = ops::resolve_ws(workspace)?;

    let method = Method::Required {
        dev_deps: !no_dev_dependencies,
        features: Rc::new(features),
        all_features,
        uses_default_features: !no_default_features,
    };

    let resolve = ops::resolve_with_previous(
        registry,
        workspace,
        method,
        Some(&resolve),
        None,
        &[],
        true,
    )?;
    Ok((packages, resolve))
}

struct Node<'a> {
    id: PackageId,
    metadata: &'a ManifestMetadata,
    depth_first_seen: usize,
    is_duplicate: bool,
}

struct Graph<'a> {
    graph: petgraph::Graph<Node<'a>, Kind>,
    nodes: HashMap<PackageId, NodeIndex>,
}

fn build_graph<'a>(
    resolve: &'a Resolve,
    packages: &'a PackageSet<'_>,
    root: PackageId,
    target: Option<&str>,
    cfgs: Option<&[Cfg]>,
    max_depth: Option<usize>,
) -> CargoResult<Graph<'a>> {
    let mut graph = Graph {
        graph: petgraph::Graph::new(),
        nodes: HashMap::new(),
    };
    let node = Node {
        id: root.clone(),
        metadata: packages.get_one(root)?.manifest().metadata(),
        depth_first_seen: 0,
        is_duplicate: false,
    };
    graph.nodes.insert(root.clone(), graph.graph.add_node(node));

    if Some(0) == max_depth {
        return Ok(graph);
    }

    let mut pending = VecDeque::new();
    pending.push_back(root);

    let mut current_depth: usize = 1;

    while let Some(pkg_id) = pending.pop_front() {
        let idx = graph.nodes[&pkg_id];
        let pkg = packages.get_one(pkg_id)?;

        for raw_dep_id in resolve.deps_not_replaced(pkg_id) {
            let it = pkg
                .dependencies()
                .iter()
                .filter(|d| d.matches_ignoring_source(raw_dep_id))
                .filter(|d| {
                    d.platform()
                        .and_then(|p| target.map(|t| p.matches(t, cfgs)))
                        .unwrap_or(true)
                });
            let dep_id = match resolve.replacement(raw_dep_id) {
                Some(id) => id,
                None => raw_dep_id,
            };
            for dep in it {
                let dep_idx = match graph.nodes.entry(dep_id) {
                    Entry::Occupied(e) => {
                        let key = *e.get();

                        let mut node = &mut graph.graph[key];
                        node.is_duplicate = true;

                        key
                    }
                    Entry::Vacant(e) => {
                        if let Some(depth) = max_depth {
                            if current_depth < depth {
                                pending.push_back(dep_id);
                            }
                        } else {
                            pending.push_back(dep_id);
                        }

                        let node = Node {
                            id: dep_id,
                            metadata: packages.get_one(dep_id)?.manifest().metadata(),
                            depth_first_seen: current_depth,
                            is_duplicate: false,
                        };
                        *e.insert(graph.graph.add_node(node))
                    }
                };
                graph.graph.add_edge(idx, dep_idx, dep.kind());
            }
        }
        current_depth += 1;
    }

    Ok(graph)
}

fn print_tree<'a>(
    package: &'a PackageId,
    graph: &Graph<'a>,
    format: &Pattern,
    direction: EdgeDirection,
    symbols: &Symbols,
    prefix: Prefix,
    all: bool,
    max_depth: Option<usize>,
) -> CargoResult<()> {
    let mut levels_continue = vec![];

    let package = match graph.nodes.get(package) {
        Some(package) => package,
        None => bail!("package {} not found", package),
    };
    let node = &graph.graph[*package];
    print_dependency(
        node,
        &graph,
        format,
        direction,
        symbols,
        &mut levels_continue,
        prefix,
        all,
        max_depth,
    );
    Ok(())
}

fn print_dependency<'a>(
    package: &Node<'a>,
    graph: &Graph<'a>,
    format: &Pattern,
    direction: EdgeDirection,
    symbols: &Symbols,
    levels_continue: &mut Vec<bool>,
    prefix: Prefix,
    all: bool,
    max_depth: Option<usize>,
) {
    let new = all || (!package.is_duplicate || levels_continue.len() <= package.depth_first_seen);

    let star = if new { "" } else { " (*)" };

    match prefix {
        Prefix::Depth => print!("{} ", levels_continue.len()),
        Prefix::Indent => {
            if let Some((&last_continues, rest)) = levels_continue.split_last() {
                for &continues in rest {
                    let c = if continues { symbols.down } else { " " };
                    print!("{}   ", c);
                }

                let c = if last_continues {
                    symbols.tee
                } else {
                    symbols.ell
                };
                print!("{0}{1}{1} ", c, symbols.right);
            }
        }
        Prefix::None => (),
    }

    println!("{}{}", format.display(&package.id, package.metadata), star);

    if !new || max_depth.map_or_else(|| false, |d| levels_continue.len() >= d) {
        return;
    }

    let mut normal = vec![];
    let mut build = vec![];
    let mut development = vec![];
    for edge in graph
        .graph
        .edges_directed(graph.nodes[&package.id], direction)
    {
        let dep = match direction {
            EdgeDirection::Incoming => &graph.graph[edge.source()],
            EdgeDirection::Outgoing => &graph.graph[edge.target()],
        };
        match *edge.weight() {
            Kind::Normal => normal.push(dep),
            Kind::Build => build.push(dep),
            Kind::Development => development.push(dep),
        }
    }

    print_dependency_kind(
        Kind::Normal,
        normal,
        graph,
        format,
        direction,
        symbols,
        levels_continue,
        prefix,
        all,
        max_depth,
    );
    print_dependency_kind(
        Kind::Build,
        build,
        graph,
        format,
        direction,
        symbols,
        levels_continue,
        prefix,
        all,
        max_depth,
    );
    print_dependency_kind(
        Kind::Development,
        development,
        graph,
        format,
        direction,
        symbols,
        levels_continue,
        prefix,
        all,
        max_depth,
    );
}

fn print_dependency_kind<'a>(
    kind: Kind,
    mut deps: Vec<&Node<'a>>,
    graph: &Graph<'a>,
    format: &Pattern,
    direction: EdgeDirection,
    symbols: &Symbols,
    levels_continue: &mut Vec<bool>,
    prefix: Prefix,
    all: bool,
    max_depth: Option<usize>,
) {
    if deps.is_empty() {
        return;
    }

    // Resolve uses Hash data types internally but we want consistent output ordering
    deps.sort_by_key(|n| n.id);

    let name = match kind {
        Kind::Normal => None,
        Kind::Build => Some("[build-dependencies]"),
        Kind::Development => Some("[dev-dependencies]"),
    };
    if let Prefix::Indent = prefix {
        if let Some(name) = name {
            for &continues in &**levels_continue {
                let c = if continues { symbols.down } else { " " };
                print!("{}   ", c);
            }

            println!("{}", name);
        }
    }

    let mut it = deps.iter().peekable();
    while let Some(dependency) = it.next() {
        levels_continue.push(it.peek().is_some());
        print_dependency(
            dependency,
            graph,
            format,
            direction,
            symbols,
            levels_continue,
            prefix,
            all,
            max_depth,
        );
        levels_continue.pop();
    }
}
