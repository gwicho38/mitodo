mod cli;
mod logging;
mod prelude;
mod store;

use clap::Parser;

use crate::cli::CliArgs;
use crate::prelude::*;

fn main() -> Result<()> {
    color_eyre::install()?;
    let args = CliArgs::parse();
    logging::init(args.log_file().as_deref(), *args.log_level());
    info!("mitodo starting");
    Ok(())
}
