extern crate cargo;
extern crate petgraph;
extern crate rustc_serialize;

mod common;
mod graph;

use cargo::{Config, CliResult};
use cargo::core::PackageId;
use std::collections::HashSet;
use petgraph::EdgeDirection;
use common::RawKind;
use graph::Graph;

#[cfg_attr(rustfmt, rustfmt_skip)]
const USAGE: &'static str = "
Display a tree visualization of a dependency graph

Usage: cargo tree [options]
       cargo tree --help

Options:
    -h, --help              Print this message
    -V, --version           Print version info and exit
    -p, --package PACKAGE   Set the package to be used as the root of the tree
    -k, --kind KIND         Set the kind of dependencies to analyze. Valid
                            values: normal, dev, build [default: normal]
    --features FEATURES     Space separated list of features to include
    --no-default-features   Do not include the `default` feature
    --target TARGET         Set the target triple
    -i, --invert            Invert the tree direction
    --charset CHARSET       Set the character set to use in output. Valid
                            values: utf8, ascii [default: utf8]
    --manifest-path PATH    Path to the manifest to analyze
    -v, --verbose           Use verbose output
    -q, --quiet             No output printed to stdout other than the tree
";

#[derive(RustcDecodable)]
pub struct Flags {
    flag_version: bool,
    flag_package: Option<String>,
    flag_kind: RawKind,
    flag_features: Vec<String>,
    flag_no_default_features: bool,
    flag_target: Option<String>,
    flag_invert: bool,
    flag_charset: Charset,
    flag_manifest_path: Option<String>,
    flag_verbose: bool,
    flag_quiet: bool,
}

#[derive(RustcDecodable)]
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
    cargo::execute_main_without_stdin(tree_main, false, USAGE);
}

fn tree_main(flags: Flags, config: &Config) -> CliResult<Option<()>> {
    if flags.flag_version {
        println!("cargo-tree {}", env!("CARGO_PKG_VERSION"));
        return Ok(None);
    }

    common::common_main(flags, config, |flags, root, graph| {
        let symbols = match flags.flag_charset {
            Charset::Ascii => &ASCII_SYMBOLS,
            Charset::Utf8 => &UTF8_SYMBOLS,
        };

        print_tree(root, graph, symbols);
    })
}

fn print_tree<'a>(package: &'a PackageId, graph: &Graph<'a>, symbols: &Symbols) {
    let mut visited_deps = HashSet::new();
    let mut levels_continue = vec![];

    print_dependency(package,
                     &graph,
                     symbols,
                     &mut visited_deps,
                     &mut levels_continue);
}

fn print_dependency<'a>(package: &'a PackageId,
                        graph: &Graph<'a>,
                        symbols: &Symbols,
                        visited_deps: &mut HashSet<&'a PackageId>,
                        levels_continue: &mut Vec<bool>) {
    if let Some((&last_continues, rest)) = levels_continue.split_last() {
        for &continues in rest {
            let c = if continues {
                symbols.down
            } else {
                " "
            };
            print!("{}   ", c);
        }

        let c = if last_continues {
            symbols.tee
        } else {
            symbols.ell
        };
        print!("{0}{1}{1} ", c, symbols.right);
    }

    let new = visited_deps.insert(package);
    let star = if new {
        ""
    } else {
        " (*)"
    };

    println!("{}{}", package, star);

    if !new {
        return;
    }

    // Resolve uses Hash data types internally but we want consistent output ordering
    let mut deps = graph.graph
                        .neighbors_directed(graph.nodes[&package], EdgeDirection::Outgoing)
                        .map(|i| graph.graph[i])
                        .collect::<Vec<_>>();
    deps.sort();
    let mut it = deps.iter().peekable();
    while let Some(dependency) = it.next() {
        levels_continue.push(it.peek().is_some());
        print_dependency(dependency, graph, symbols, visited_deps, levels_continue);
        levels_continue.pop();
    }
}
