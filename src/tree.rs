use crate::args::{Args, Charset};
use crate::format::Pattern;
use crate::graph::Graph;
use anyhow::{anyhow, Context, Error};
use cargo_metadata::{DependencyKind, Package, PackageId};
use petgraph::visit::EdgeRef;
use petgraph::EdgeDirection;
use semver::Version;
use std::collections::{HashMap, HashSet};

#[derive(Clone, Copy)]
enum Prefix {
    None,
    Indent,
    Depth,
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

pub fn print(args: &Args, graph: &Graph) -> Result<(), Error> {
    let format = Pattern::new(&args.format)?;

    let direction = if args.invert || args.duplicates {
        EdgeDirection::Incoming
    } else {
        EdgeDirection::Outgoing
    };

    let symbols = match args.charset {
        Charset::Utf8 => &UTF8_SYMBOLS,
        Charset::Ascii => &ASCII_SYMBOLS,
    };

    let prefix = if args.prefix_depth {
        Prefix::Depth
    } else if args.no_indent {
        Prefix::None
    } else {
        Prefix::Indent
    };

    if args.duplicates {
        for (i, package) in find_duplicates(graph).iter().enumerate() {
            if i != 0 {
                println!();
            }

            let root = &graph.graph[graph.nodes[*package]];
            print_tree(graph, root, &format, direction, symbols, prefix, args.all);
        }
    } else {
        let root = match &args.package {
            Some(package) => find_package(package, graph)?,
            None => graph.root.as_ref().ok_or_else(|| {
                anyhow!("this command requires running against an actual package in this workspace")
            })?,
        };
        let root = &graph.graph[graph.nodes[root]];

        print_tree(graph, root, &format, direction, symbols, prefix, args.all);
    }

    Ok(())
}

fn find_package<'a>(package: &str, graph: &'a Graph) -> Result<&'a PackageId, Error> {
    let mut it = package.split(':');
    let name = it.next().unwrap();
    let version = it
        .next()
        .map(Version::parse)
        .transpose()
        .context("error parsing package version")?;

    let mut candidates = vec![];
    for idx in graph.graph.node_indices() {
        let package = &graph.graph[idx];
        if package.name != name {
            continue;
        }

        if let Some(version) = &version {
            if package.version != *version {
                continue;
            }
        }

        candidates.push(package);
    }

    if candidates.is_empty() {
        Err(anyhow!("no crates found for package `{}`", package))
    } else if candidates.len() > 1 {
        let specs = candidates
            .iter()
            .map(|p| format!("{}:{}", p.name, p.version))
            .collect::<Vec<_>>()
            .join(", ");
        Err(anyhow!(
            "multiple crates found for package `{}`: {}",
            package,
            specs,
        ))
    } else {
        Ok(&candidates[0].id)
    }
}

fn find_duplicates(graph: &Graph) -> Vec<&PackageId> {
    let mut packages = HashMap::new();

    for idx in graph.graph.node_indices() {
        let package = &graph.graph[idx];
        packages
            .entry(&package.name)
            .or_insert_with(Vec::new)
            .push(&package.id);
    }

    let mut duplicates = vec![];
    for ids in packages.values() {
        if ids.len() > 1 {
            duplicates.extend(ids.iter().cloned());
        }
    }

    duplicates.sort();
    duplicates
}

fn print_tree<'a>(
    graph: &'a Graph,
    root: &'a Package,
    format: &Pattern,
    direction: EdgeDirection,
    symbols: &Symbols,
    prefix: Prefix,
    all: bool,
) {
    let mut visited_deps = HashSet::new();
    let mut levels_continue = vec![];

    print_package(
        graph,
        root,
        format,
        direction,
        symbols,
        prefix,
        all,
        &mut visited_deps,
        &mut levels_continue,
    );
}

fn print_package<'a>(
    graph: &'a Graph,
    package: &'a Package,
    format: &Pattern,
    direction: EdgeDirection,
    symbols: &Symbols,
    prefix: Prefix,
    all: bool,
    visited_deps: &mut HashSet<&'a PackageId>,
    levels_continue: &mut Vec<bool>,
) {
    let new = all || visited_deps.insert(&package.id);

    match prefix {
        Prefix::Depth => print!("{}", levels_continue.len()),
        Prefix::Indent => {
            if let Some((last_continues, rest)) = levels_continue.split_last() {
                for continues in rest {
                    let c = if *continues { symbols.down } else { " " };
                    print!("{}   ", c);
                }

                let c = if *last_continues {
                    symbols.tee
                } else {
                    symbols.ell
                };
                print!("{0}{1}{1} ", c, symbols.right);
            }
        }
        Prefix::None => {}
    }

    let star = if new { "" } else { " (*)" };
    println!("{}{}", format.display(package), star);

    if !new {
        return;
    }

    for kind in &[
        DependencyKind::Normal,
        DependencyKind::Build,
        DependencyKind::Development,
    ] {
        print_dependencies(
            graph,
            package,
            format,
            direction,
            symbols,
            prefix,
            all,
            visited_deps,
            levels_continue,
            *kind,
        );
    }
}

fn print_dependencies<'a>(
    graph: &'a Graph,
    package: &'a Package,
    format: &Pattern,
    direction: EdgeDirection,
    symbols: &Symbols,
    prefix: Prefix,
    all: bool,
    visited_deps: &mut HashSet<&'a PackageId>,
    levels_continue: &mut Vec<bool>,
    kind: DependencyKind,
) {
    let idx = graph.nodes[&package.id];
    let mut deps = vec![];
    for edge in graph.graph.edges_directed(idx, direction) {
        if *edge.weight() != kind {
            continue;
        }

        let dep = match direction {
            EdgeDirection::Incoming => &graph.graph[edge.source()],
            EdgeDirection::Outgoing => &graph.graph[edge.target()],
        };
        deps.push(dep);
    }

    if deps.is_empty() {
        return;
    }

    // ensure a consistent output ordering
    deps.sort_by_key(|p| &p.id);

    let name = match kind {
        DependencyKind::Normal => None,
        DependencyKind::Build => Some("[build-dependencies]"),
        DependencyKind::Development => Some("[dev-dependencies]"),
        _ => unreachable!(),
    };

    if let Prefix::Indent = prefix {
        if let Some(name) = name {
            for continues in &**levels_continue {
                let c = if *continues { symbols.down } else { " " };
                print!("{}   ", c);
            }

            println!("{}", name);
        }
    }

    let mut it = deps.iter().peekable();
    while let Some(dependency) = it.next() {
        levels_continue.push(it.peek().is_some());
        print_package(
            graph,
            dependency,
            format,
            direction,
            symbols,
            prefix,
            all,
            visited_deps,
            levels_continue,
        );
        levels_continue.pop();
    }
}
