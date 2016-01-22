use cargo::{Config, CliResult};
use cargo::core::{Source, Package, PackageId, Resolve};
use cargo::core::dependency::Kind;
use cargo::core::registry::PackageRegistry;
use cargo::core::resolver::Method;
use cargo::ops;
use cargo::util::{important_paths, CargoResult};
use cargo::sources::path::PathSource;
use graph::Graph;

#[derive(Copy, Clone, RustcDecodable)]
pub enum RawKind {
    Normal,
    Dev,
    Build,
}

impl From<RawKind> for Kind {
    fn from(raw: RawKind) -> Kind {
        match raw {
            RawKind::Normal => Kind::Normal,
            RawKind::Dev => Kind::Development,
            RawKind::Build => Kind::Build,
        }
    }
}

pub fn common_main<F>(flags: super::Flags, config: &Config, display_fn: F) -> CliResult<Option<()>>
    where F: Fn(&super::Flags, &PackageId, &Graph) -> ()
{
    let flag_features = flags.flag_features
                             .iter()
                             .flat_map(|s| s.split(" "))
                             .map(|s| s.to_owned())
                             .collect();

    try!(config.shell().set_verbosity(flags.flag_verbose, flags.flag_quiet));

    let mut source = try!(source(config, flags.flag_manifest_path.clone()));
    let package = try!(source.root_package());
    let mut registry = try!(registry(config, &package));
    let resolve = try!(resolve(&mut registry,
                               &package,
                               flag_features,
                               flags.flag_no_default_features));
    let packages = try!(ops::get_resolved_packages(&resolve, &mut registry));

    let root = match flags.flag_package {
        Some(ref pkg) => try!(resolve.query(pkg)),
        None => resolve.root(),
    };

    let kind = Kind::from(flags.flag_kind);

    let target = flags.flag_target.as_ref().unwrap_or(&config.rustc_info().host);

    let graph = Graph::build(&resolve, &packages, package.package_id(), &[kind], target);

    display_fn(&flags, root, &graph);

    Ok(None)
}

fn source(config: &Config, manifest_path: Option<String>) -> CargoResult<PathSource> {
    let root = try!(important_paths::find_root_manifest_for_cwd(manifest_path));
    let mut source = try!(PathSource::for_path(root.parent().unwrap(), config));
    try!(source.update());
    Ok(source)
}

fn registry<'a>(config: &'a Config, package: &Package) -> CargoResult<PackageRegistry<'a>> {
    let mut registry = PackageRegistry::new(config);
    try!(registry.add_sources(&[package.package_id().source_id().clone()]));
    Ok(registry)
}

fn resolve(registry: &mut PackageRegistry,
           package: &Package,
           features: Vec<String>,
           no_default_features: bool)
           -> CargoResult<Resolve> {
    let resolve = try!(ops::resolve_pkg(registry, package));

    let method = Method::Required {
        dev_deps: true,
        features: &features,
        uses_default_features: !no_default_features,
    };

    ops::resolve_with_previous(registry, &package, method, Some(&resolve), None)
}
