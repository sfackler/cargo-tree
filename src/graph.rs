use cargo::core::{PackageId, Package, Resolve};
use cargo::core::dependency::Kind;
use std::collections::HashMap;
use petgraph::Graph as Pgraph;
use petgraph::EdgeDirection;
use petgraph::graph::NodeIndex;

pub struct Graph<'a> {
    pub graph: Pgraph<&'a PackageId, Kind>,
    pub nodes: HashMap<&'a PackageId, NodeIndex>,
}

impl<'a> Graph<'a> {
    pub fn build(resolve: &'a Resolve,
                 packages: &[Package],
                 root: &'a PackageId,
                 kinds: &[Kind],
                 target: &str)
                 -> Graph<'a> {
        let packages = packages.iter()
                               .map(|p| (p.package_id().clone(), p))
                               .collect::<HashMap<_, _>>();

        let mut graph = Graph {
            graph: Pgraph::new(),
            nodes: HashMap::new(),
        };
        graph.nodes.insert(root, graph.graph.add_node(root));

        let mut pending = vec![root];

        while let Some(pkg_id) = pending.pop() {
            let idx = graph.nodes[&pkg_id];

            let pkg = packages[pkg_id];
            for dep_id in resolve.deps(pkg_id).unwrap() {
                let kind = pkg.dependencies()
                              .iter()
                              .filter(|d| d.matches_id(dep_id))
                              .filter(|d| {
                                  (pkg_id == root && kinds.contains(&d.kind())) ||
                                  (pkg_id != root && d.kind() == Kind::Normal)
                              })
                              .filter(|d| {
                                  d.only_for_platform().map(|t| t == target).unwrap_or(true)
                              })
                              .map(|d| d.kind())
                              .next();
                if let Some(kind) = kind {
                    let dep_idx = {
                        let g = &mut graph.graph;
                        *graph.nodes.entry(dep_id).or_insert_with(|| g.add_node(dep_id))
                    };
                    graph.graph.update_edge(idx, dep_idx, kind);
                    pending.push(dep_id);
                }
            }
        }

        graph
    }

    pub fn extract(&self, root: &'a PackageId, dir: EdgeDirection) -> Graph<'a> {
        assert!(self.nodes.contains_key(root),
                format!("{} is not in the dependency graph", root));
        let mut graph = Graph {
            graph: Pgraph::new(),
            nodes: HashMap::new(),
        };
        graph.nodes.insert(root, graph.graph.add_node(root));

        let mut pending = vec![root];

        while let Some(pkg_id) = pending.pop() {
            let idx = self.nodes[&pkg_id];
            let new_idx = graph.nodes[&pkg_id];

            for (dep_idx, &kind) in self.graph.edges_directed(idx, dir) {
                let dep_id = self.graph[dep_idx];
                let new_dep_idx = {
                    let g = &mut graph.graph;
                    *graph.nodes.entry(dep_id).or_insert_with(|| g.add_node(dep_id))
                };
                graph.graph.update_edge(new_idx, new_dep_idx, kind);
                pending.push(dep_id);
            }
        }

        graph
    }
}
