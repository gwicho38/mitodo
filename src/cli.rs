use std::path::PathBuf;

use clap::{Parser, Subcommand};
use getset::Getters;
use log::LevelFilter;

#[derive(Parser, Debug, Getters)]
#[command(
    name = "mitodo",
    version,
    about = "a TUI todo tracker over plain markdown checklists"
)]
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

    /// Filter with a query, e.g. "pri:P0 acct:work !done"
    #[arg(short, long)]
    query: Option<String>,

    /// Shorthand for `pri:<VALUE>`, combined with --query
    #[arg(short, long)]
    priority: Option<String>,

    /// Shorthand for `acct:<VALUE>`, combined with --query
    #[arg(short, long)]
    account: Option<String>,

    #[command(subcommand)]
    command: Option<Command>,
}

impl CliArgs {
    /// The query implied by the flags, if any.
    ///
    /// `--priority` and `--account` are sugar for the equivalent query terms,
    /// kept because they are what `mcli todos` used; they AND with `--query`
    /// rather than replacing it.
    pub fn effective_query(&self) -> Option<String> {
        let mut terms: Vec<String> = Vec::new();
        if let Some(priority) = &self.priority {
            terms.push(format!("pri:{priority}"));
        }
        if let Some(account) = &self.account {
            terms.push(format!("acct:{account}"));
        }
        if let Some(query) = &self.query
            && !query.trim().is_empty()
        {
            terms.push(query.clone());
        }
        if terms.is_empty() {
            None
        } else {
            Some(terms.join(" "))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    fn parse(args: &[&str]) -> CliArgs {
        let mut argv = vec!["mitodo"];
        argv.extend_from_slice(args);
        CliArgs::parse_from(argv)
    }

    #[test]
    fn no_filter_flags_means_no_query() {
        assert_eq!(parse(&[]).effective_query(), None);
    }

    #[test]
    fn a_bare_query_passes_through() {
        assert_eq!(
            parse(&["--query", "pri:P0 !done"]).effective_query(),
            Some("pri:P0 !done".to_string())
        );
    }

    #[test]
    fn priority_and_account_become_query_terms() {
        assert_eq!(
            parse(&["--priority", "P0"]).effective_query(),
            Some("pri:P0".to_string())
        );
        assert_eq!(
            parse(&["--account", "lefv"]).effective_query(),
            Some("acct:lefv".to_string())
        );
    }

    #[test]
    fn the_flags_and_the_query_are_anded_together() {
        // This is the `mcli todos act -p P0 -a lefv` invocation, preserved.
        assert_eq!(
            parse(&["-p", "P0", "-a", "lefv", "-q", "!done"]).effective_query(),
            Some("pri:P0 acct:lefv !done".to_string())
        );
    }

    #[test]
    fn an_empty_query_string_is_ignored() {
        assert_eq!(parse(&["--query", "  "]).effective_query(), None);
        assert_eq!(
            parse(&["-p", "P1", "-q", ""]).effective_query(),
            Some("pri:P1".to_string())
        );
    }
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
    /// Serve the workspace to an MCP client over stdio
    McpServer,
    /// Manage this installation of mitodo
    #[command(name = "self")]
    Selfie {
        #[command(subcommand)]
        action: SelfAction,
    },
}

#[derive(Subcommand, Debug, Clone)]
pub enum SelfAction {
    /// Register mitodo's MCP server with the clients on this machine
    Mcp {
        #[command(subcommand)]
        action: McpAction,
    },
}

#[derive(Subcommand, Debug, Clone)]
pub enum McpAction {
    /// Register with every supported client found
    Setup {
        /// Print what would change, and change nothing
        #[arg(long)]
        dry_run: bool,
    },
    /// Show where mitodo is registered, and whether its path still resolves
    Status,
    /// Unregister from every client that has it
    Remove {
        #[arg(long)]
        dry_run: bool,
    },
}
