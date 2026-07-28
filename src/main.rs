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
        Some(Command::List) => cmd_list(&config_path),
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

fn cmd_list(config_path: &Path) -> Result<()> {
    let config = config::Config::load(config_path)?;
    let workspace = store::Workspace::load(&config)?;

    for group in &workspace.groups {
        let items = workspace.items_for_group(&group.name);
        let open = items.iter().filter(|i| !i.done).count();
        println!("\n{} ({} open / {} total)", group.name, open, items.len());
        for item in items {
            let box_ = if item.done { "x" } else { " " };
            let indent = " ".repeat(item.indent);
            println!(
                "  {indent}[{box_}] {:<3} {}",
                item.priority.as_str(),
                item.text
            );
        }
    }
    println!(
        "\n{} open across {} groups",
        workspace.open_count(),
        workspace.groups.len()
    );
    Ok(())
}
