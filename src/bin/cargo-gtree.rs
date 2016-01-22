extern crate cargo;
extern crate petgraph;
extern crate rustc_serialize;
extern crate dot;

#[path = "../common.rs"]
mod common;
#[path = "../graph.rs"]
mod graph;

use cargo::{Config, CliResult};
use cargo::core::dependency::Kind;
use std::borrow::Cow;
use std::io;
use petgraph::graph::{NodeIndex, EdgeIndex};
use dot::{Labeller, GraphWalk};
use common::RawKind;
use graph::Graph;

#[cfg_attr(rustfmt, rustfmt_skip)]
const USAGE: &'static str = "
Export the dependency graph in the dot format

Usage: cargo gtree [options]
       cargo gtree --help

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
    flag_manifest_path: Option<String>,
    flag_verbose: bool,
    flag_quiet: bool,
}

fn main() {
    cargo::execute_main_without_stdin(gtree_main, false, USAGE);
}

fn gtree_main(flags: Flags, config: &Config) -> CliResult<Option<()>> {
    if flags.flag_version {
        println!("cargo-gtree {}", env!("CARGO_PKG_VERSION"));
        return Ok(None);
    }

    common::common_main(flags, config, |_flags, _root, graph| {
        dot::render(graph, &mut io::stdout()).unwrap();
    })
}

impl<'a, 'b> Labeller<'a, NodeIndex, EdgeIndex> for Graph<'b> {
    fn graph_id(&'a self) -> dot::Id<'a> {
        dot::Id::new("dependencies").unwrap()
    }

    fn node_id(&'a self, n: &NodeIndex) -> dot::Id<'a> {
        dot::Id::new(format!("N{}", n.index())).unwrap()
    }

    fn node_label(&'a self, n: &NodeIndex) -> dot::LabelText<'a> {
        let pkg = self.graph[*n];
        dot::LabelText::label(format!("{} {}", pkg.name(), pkg.version()))
    }

    fn edge_style(&'a self, e: &EdgeIndex) -> dot::Style {
        use dot::Style::*;

        match self.graph.edge_weight(*e).unwrap() {
            &Kind::Normal => Solid,
            &Kind::Development => Dashed,
            &Kind::Build => Dotted,
        }
    }
}

impl<'a, 'b> GraphWalk<'a, NodeIndex, EdgeIndex> for Graph<'b> {
    fn nodes(&'a self) -> dot::Nodes<'a, NodeIndex> {
        Cow::Owned(self.graph.node_indices().collect::<Vec<_>>())
    }

    fn edges(&'a self) -> dot::Edges<'a, EdgeIndex> {
        Cow::Owned(self.graph.edge_indices().collect::<Vec<_>>())
    }

    fn source(&'a self, edge: &EdgeIndex) -> NodeIndex {
        self.graph.edge_endpoints(*edge).unwrap().0
    }

    fn target(&'a self, edge: &EdgeIndex) -> NodeIndex {
        self.graph.edge_endpoints(*edge).unwrap().1
    }
}
