mod cli;
mod config;
mod logging;
mod prelude;
mod store;

use std::path::{Path, PathBuf};

use clap::Parser;

use crate::cli::{CliArgs, Command};
use crate::prelude::*;

fn main() -> Result<()> {
    color_eyre::install()?;
    let args = CliArgs::parse();
    logging::init(args.log_file().as_deref(), *args.log_level());

    let config_path = match args.config_dir() {
        Some(dir) => dir.join("config.toml"),
        None => config::default_config_path()?,
    };

    match args.command() {
        Some(Command::Init { root, force }) => cmd_init(root, *force, &config_path),
        Some(Command::List) => Err(eyre!("`list` arrives in Task 8")),
        None => Err(eyre!("no subcommand given; try `mitodo --help`")),
    }
}

fn cmd_init(root: &PathBuf, force: bool, config_path: &Path) -> Result<()> {
    if config_path.exists() && !force {
        return Err(eyre!(
            "{} already exists; pass --force to overwrite",
            config_path.display()
        ));
    }

    let found = store::detect(root)?;
    println!("scanning {}...", root.display());
    for note in &found.notes {
        println!("  ✓ {note}");
    }
    found.config.save(config_path)?;
    println!("wrote {}", config_path.display());
    Ok(())
}
