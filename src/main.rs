extern crate cargo;
extern crate env_logger;
extern crate petgraph;

#[macro_use]
extern crate serde_derive;

use cargo::{CliResult, Config};
use cargo::core::{Package, PackageId, Resolve, Workspace};
use cargo::core::dependency::Kind;
use cargo::core::manifest::ManifestMetadata;
use cargo::core::package::PackageSet;
use cargo::core::registry::PackageRegistry;
use cargo::core::shell::Shell;
use cargo::ops::{self, Packages};
use cargo::util::{self, important_paths, CargoError, CargoResult, Cfg};
use petgraph::EdgeDirection;
use petgraph::graph::NodeIndex;
use petgraph::visit::EdgeRef;
use std::collections::{HashMap, HashSet};
use std::collections::hash_map::Entry;
use std::env;
use std::str::{self, FromStr};

use format::Pattern;

mod format;

#[cfg_attr(rustfmt, rustfmt_skip)]
const USAGE: &'static str = "
Display a tree visualization of a dependency graph

Usage: cargo tree [options]

Options:
    -h, --help              Print this message
    -V, --version           Print version info and exit
    -p, --package PACKAGE   Set the package to be used as the root of the tree
    --features FEATURES     Space separated list of features to include
    --all-features          Include all available features
    --no-default-features   Do not include the `default` feature
    --target TARGET         Set the target triple
    -i, --invert            Invert the tree direction
    --no-indent             Display dependencies as a list (rather than a graph)
    -a, --all               Don't truncate dependencies that have already been
                            displayed
    -d, --duplicates        Show only dependencies which come in multiple
                            versions (implies --invert)
    --charset CHARSET       Set the character set to use in output. Valid
                            values: utf8, ascii [default: utf8]
    -f, --format FORMAT     Format string for printing dependencies
    --manifest-path PATH    Path to the manifest to analyze
    -v, --verbose ...       Use verbose output
    -q, --quiet             No output printed to stdout other than the tree
    --color WHEN            Coloring: auto, always, never
    --frozen                Require Cargo.lock and cache are up to date
    --locked                Require Cargo.lock is up to date
    -Z FLAG ...             Unstable (nightly-only) flags to Cargo
";

#[derive(Deserialize)]
struct Flags {
    flag_version: bool,
    flag_package: Option<String>,
    flag_features: Vec<String>,
    flag_all_features: bool,
    flag_no_default_features: bool,
    flag_target: Option<String>,
    flag_invert: bool,
    flag_no_indent: bool,
    flag_all: bool,
    flag_charset: Charset,
    flag_format: Option<String>,
    flag_manifest_path: Option<String>,
    flag_verbose: u32,
    flag_quiet: Option<bool>,
    flag_color: Option<String>,
    flag_duplicates: bool,
    flag_frozen: bool,
    flag_locked: bool,
    #[serde(rename = "flag_Z")] flag_z: Vec<String>,
}

#[derive(Deserialize)]
enum Charset {
    Utf8,
    Ascii,
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
    env_logger::init().unwrap();

    let mut config = match Config::default() {
        Ok(cfg) => cfg,
        Err(e) => {
            let mut shell = Shell::new();
            cargo::exit_with_error(e.into(), &mut shell)
        }
    };

    let result = (|| {
        let args: Vec<_> = try!(
            env::args_os()
                .map(|s| {
                    s.into_string().map_err(|s| {
                        CargoError::from(format!("invalid unicode in argument: {:?}", s))
                    })
                })
                .collect()
        );
        let rest = &args;
        cargo::call_main_without_stdin(real_main, &mut config, USAGE, rest, false)
    })();

    match result {
        Err(e) => cargo::exit_with_error(e, &mut *config.shell()),
        Ok(()) => {}
    }
}

fn real_main(flags: Flags, config: &mut Config) -> CliResult {
    if flags.flag_version {
        println!("cargo-tree {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    config.configure(
        flags.flag_verbose,
        flags.flag_quiet,
        &flags.flag_color,
        flags.flag_frozen,
        flags.flag_locked,
        &flags.flag_z,
    )?;

    let workspace = workspace(config, flags.flag_manifest_path)?;
    let package = workspace.current()?;
    let registry = registry(config, &package)?;
    let (packages, resolve) = resolve(
        &workspace,
        flags.flag_features,
        flags.flag_all_features,
        flags.flag_no_default_features,
    )?;
    let ids = packages.package_ids().cloned().collect::<Vec<_>>();
    let packages = registry.get(&ids);

    let root = match flags.flag_package {
        Some(ref pkg) => resolve.query(pkg)?,
        None => package.package_id(),
    };

    let target = flags.flag_target.as_ref().unwrap_or(&config.rustc()?.host);

    let format = match flags.flag_format {
        Some(ref r) => &**r,
        None => "{p}",
    };
    let format = Pattern::new(format).map_err(|e| CargoError::from(e.to_string()))?;

    let cfgs = get_cfgs(config, &flags.flag_target)?;
    let graph = build_graph(
        &resolve,
        &packages,
        package.package_id(),
        target,
        cfgs.as_ref().map(|r| &**r),
    )?;

    let direction = if flags.flag_invert || flags.flag_duplicates {
        EdgeDirection::Incoming
    } else {
        EdgeDirection::Outgoing
    };

    let symbols = match flags.flag_charset {
        Charset::Ascii => &ASCII_SYMBOLS,
        Charset::Utf8 => &UTF8_SYMBOLS,
    };

    if flags.flag_duplicates {
        let dups = find_duplicates(&graph);
        for dup in &dups {
            print_tree(
                dup,
                &graph,
                &format,
                direction,
                symbols,
                flags.flag_no_indent,
                flags.flag_all,
            );
            println!("");
        }
    } else {
        print_tree(
            root,
            &graph,
            &format,
            direction,
            symbols,
            flags.flag_no_indent,
            flags.flag_all,
        );
    }

    Ok(())
}

fn find_duplicates<'a>(graph: &Graph<'a>) -> Vec<&'a PackageId> {
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

fn get_cfgs(config: &Config, target: &Option<String>) -> CargoResult<Option<Vec<Cfg>>> {
    let mut process = util::process(&config.rustc()?.path);
    process.arg("--print=cfg").env_remove("RUST_LOG");
    if let Some(ref s) = *target {
        process.arg("--target").arg(s);
    }

    let output = match process.exec_with_output() {
        Ok(output) => output,
        Err(_) => return Ok(None),
    };
    let output = str::from_utf8(&output.stdout).unwrap();
    let lines = output.lines();
    Ok(Some(
        lines.map(Cfg::from_str).collect::<CargoResult<Vec<_>>>()?,
    ))
}

fn workspace(config: &Config, manifest_path: Option<String>) -> CargoResult<Workspace> {
    let root = important_paths::find_root_manifest_for_wd(manifest_path, config.cwd())?;
    Workspace::new(&root, config)
}

fn registry<'a>(config: &'a Config, package: &Package) -> CargoResult<PackageRegistry<'a>> {
    let mut registry = PackageRegistry::new(config)?;
    registry.add_sources(&[package.package_id().source_id().clone()])?;
    Ok(registry)
}

fn resolve<'a>(
    workspace: &'a Workspace,
    features: Vec<String>,
    all_features: bool,
    no_default_features: bool,
) -> CargoResult<(PackageSet<'a>, Resolve)> {
    let features = features
        .iter()
        .flat_map(|s| s.split_whitespace())
        .flat_map(|s| s.split(','))
        .filter(|s| s.len() > 0)
        .map(|s| s.to_string())
        .collect::<Vec<String>>();

    let specs = Packages::All.into_package_id_specs(workspace)?;

    ops::resolve_ws_precisely(
        workspace,
        None,
        &features,
        all_features,
        no_default_features,
        &specs,
    )
}

struct Node<'a> {
    id: &'a PackageId,
    metadata: &'a ManifestMetadata,
}

struct Graph<'a> {
    graph: petgraph::Graph<Node<'a>, Kind>,
    nodes: HashMap<&'a PackageId, NodeIndex>,
}

fn build_graph<'a>(
    resolve: &'a Resolve,
    packages: &'a PackageSet,
    root: &'a PackageId,
    target: &str,
    cfgs: Option<&[Cfg]>,
) -> CargoResult<Graph<'a>> {
    let mut graph = Graph {
        graph: petgraph::Graph::new(),
        nodes: HashMap::new(),
    };
    let node = Node {
        id: root,
        metadata: packages.get(root)?.manifest().metadata(),
    };
    graph.nodes.insert(root, graph.graph.add_node(node));

    let mut pending = vec![root];

    while let Some(pkg_id) = pending.pop() {
        let idx = graph.nodes[&pkg_id];
        let pkg = packages.get(pkg_id)?;

        for raw_dep_id in resolve.deps_not_replaced(pkg_id) {
            let it = pkg.dependencies()
                .iter()
                .filter(|d| d.matches_id(raw_dep_id))
                .filter(|d| {
                    d.platform()
                        .map(|p| p.matches(target, cfgs))
                        .unwrap_or(true)
                });
            let dep_id = match resolve.replacement(raw_dep_id) {
                Some(id) => id,
                None => raw_dep_id,
            };
            for dep in it {
                let dep_idx = match graph.nodes.entry(dep_id) {
                    Entry::Occupied(e) => *e.get(),
                    Entry::Vacant(e) => {
                        pending.push(dep_id);
                        let node = Node {
                            id: dep_id,
                            metadata: packages.get(dep_id)?.manifest().metadata(),
                        };
                        *e.insert(graph.graph.add_node(node))
                    }
                };
                graph.graph.add_edge(idx, dep_idx, dep.kind());
            }
        }
    }

    Ok(graph)
}

fn print_tree<'a>(
    package: &'a PackageId,
    graph: &Graph<'a>,
    format: &Pattern,
    direction: EdgeDirection,
    symbols: &Symbols,
    no_indent: bool,
    all: bool,
) {
    let mut visited_deps = HashSet::new();
    let mut levels_continue = vec![];

    let node = &graph.graph[graph.nodes[&package]];
    print_dependency(
        node,
        &graph,
        format,
        direction,
        symbols,
        &mut visited_deps,
        &mut levels_continue,
        no_indent,
        all,
    );
}

fn print_dependency<'a>(
    package: &Node<'a>,
    graph: &Graph<'a>,
    format: &Pattern,
    direction: EdgeDirection,
    symbols: &Symbols,
    visited_deps: &mut HashSet<&'a PackageId>,
    levels_continue: &mut Vec<bool>,
    no_indent: bool,
    all: bool,
) {
    let new = all || visited_deps.insert(package.id);
    let star = if new { "" } else { " (*)" };

    if !no_indent {
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

    println!("{}{}", format.display(package.id, package.metadata), star);

    if !new {
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
        visited_deps,
        levels_continue,
        no_indent,
        all,
    );
    print_dependency_kind(
        Kind::Build,
        build,
        graph,
        format,
        direction,
        symbols,
        visited_deps,
        levels_continue,
        no_indent,
        all,
    );
    print_dependency_kind(
        Kind::Development,
        development,
        graph,
        format,
        direction,
        symbols,
        visited_deps,
        levels_continue,
        no_indent,
        all,
    );
}

fn print_dependency_kind<'a>(
    kind: Kind,
    mut deps: Vec<&Node<'a>>,
    graph: &Graph<'a>,
    format: &Pattern,
    direction: EdgeDirection,
    symbols: &Symbols,
    visited_deps: &mut HashSet<&'a PackageId>,
    levels_continue: &mut Vec<bool>,
    no_indent: bool,
    all: bool,
) {
    if deps.is_empty() {
        return;
    }

    // Resolve uses Hash data types internally but we want consistent output ordering
    deps.sort_by_key(|n| n.id);

    for &continues in &**levels_continue {
        let c = if continues { symbols.down } else { " " };
        print!("{}   ", c);
    }
    let name = match kind {
        Kind::Normal => "[dependencies]",
        Kind::Build => "[build-dependencies]",
        Kind::Development => "[dev-dependencies]",
    };
    println!("{}", name);

    let mut it = deps.iter().peekable();
    while let Some(dependency) = it.next() {
        levels_continue.push(it.peek().is_some());
        print_dependency(
            dependency,
            graph,
            format,
            direction,
            symbols,
            visited_deps,
            levels_continue,
            no_indent,
            all,
        );
        levels_continue.pop();
    }
}
