use crate::args::Args;
use anyhow::{anyhow, Error};
use cargo_metadata::{DependencyKind, Metadata, Package, PackageId};
use petgraph::graph::NodeIndex;
use petgraph::stable_graph::StableGraph;
use petgraph::visit::Dfs;
use std::collections::HashMap;

pub struct Graph {
    pub graph: StableGraph<Package, DependencyKind>,
    pub nodes: HashMap<PackageId, NodeIndex>,
    pub root: Option<PackageId>,
}

pub fn build(args: &Args, metadata: Metadata) -> Result<Graph, Error> {
    let resolve = metadata.resolve.unwrap();

    let mut graph = Graph {
        graph: StableGraph::new(),
        nodes: HashMap::new(),
        root: resolve.root,
    };

    for package in metadata.packages {
        let id = package.id.clone();
        let index = graph.graph.add_node(package);
        graph.nodes.insert(id, index);
    }

    for node in resolve.nodes {
        if node.deps.len() != node.dependencies.len() {
            return Err(anyhow!("cargo tree requires cargo 1.41 or newer"));
        }

        let from = graph.nodes[&node.id];
        for dep in node.deps {
            if dep.dep_kinds.is_empty() {
                return Err(anyhow!("cargo tree requires cargo 1.41 or newer"));
            }

            // https://github.com/rust-lang/cargo/issues/7752
            let mut kinds = vec![];
            for kind in dep.dep_kinds {
                if !kinds.iter().any(|k| *k == kind.kind) {
                    kinds.push(kind.kind);
                }
            }

            let to = graph.nodes[&dep.pkg];
            for kind in kinds {
                if args.no_dev_dependencies && kind == DependencyKind::Development {
                    continue;
                }

                graph.graph.add_edge(from, to, kind);
            }
        }
    }

    // prune nodes not reachable from the root package (directionally)
    if let Some(root) = &graph.root {
        let mut dfs = Dfs::new(&graph.graph, graph.nodes[root]);
        while dfs.next(&graph.graph).is_some() {}

        let g = &mut graph.graph;
        graph.nodes.retain(|_, idx| {
            if !dfs.discovered.contains(idx.index()) {
                g.remove_node(*idx);
                false
            } else {
                true
            }
        });
    }

    Ok(graph)
}
