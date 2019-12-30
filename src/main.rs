use crate::args::Opts;
use anyhow::Error;
use structopt::StructOpt;

mod args;
mod format;
mod graph;
mod metadata;
mod tree;

fn main() -> Result<(), Error> {
    let Opts::Tree(args) = Opts::from_args();
    let metadata = metadata::get(&args)?;
    let graph = graph::build(&args, metadata)?;
    tree::print(&args, &graph)?;

    Ok(())
}
