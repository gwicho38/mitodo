use std::path::PathBuf;

use clap::{Parser, Subcommand};
use getset::Getters;
use log::LevelFilter;

#[derive(Parser, Debug, Getters)]
#[command(name = "mitodo", version, about = "a TUI todo tracker over plain markdown checklists")]
#[getset(get = "pub")]
pub struct CliArgs {
    /// Log file (must be writable)
    #[arg(long)]
    log_file: Option<PathBuf>,

    /// Log level (OFF, ERROR, WARN, INFO, DEBUG, TRACE)
    #[arg(long)]
    log_level: Option<LevelFilter>,

    /// Directory holding config.toml
    #[arg(short, long)]
    config_dir: Option<PathBuf>,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand, Debug, Clone)]
pub enum Command {
    /// Detect the layout of a todo workspace and write a config file
    Init {
        /// Workspace root directory
        root: PathBuf,
        /// Overwrite an existing config file
        #[arg(long)]
        force: bool,
    },
    /// Print the workspace to stdout
    List,
}
