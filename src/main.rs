mod agent;
mod cli;
mod config;
mod git;
mod input;
mod logging;
mod messages;
mod prelude;
mod query;
mod store;
mod ui;

use std::io::{self, Write};
use std::path::Path;

use clap::Parser;
use tokio::sync::mpsc::unbounded_channel;
use tokio::task::spawn_blocking;

use crate::cli::{CliArgs, Command};
use crate::messages::Message;
use crate::prelude::*;

#[tokio::main]
async fn main() -> Result<()> {
    color_eyre::install()?;
    let args = CliArgs::parse();
    logging::init(args.log_file().as_deref(), *args.log_level());

    let config_path = match args.config_dir() {
        Some(dir) => dir.join("config.toml"),
        None => config::default_config_path()?,
    };

    let query = args.effective_query();
    match args.command() {
        Some(Command::Init { root, force }) => cmd_init(root, *force, &config_path),
        Some(Command::List) => cmd_list(&config_path, query.as_deref()),
        None => run_tui(&config_path, query.as_deref()).await,
    }
}

fn cmd_init(root: &Path, force: bool, config_path: &Path) -> Result<()> {
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

/// True if the error is a closed downstream pipe, e.g. `mitodo list | head`.
///
/// `println!` panics on EPIPE, which makes piping into `head` look like a
/// crash. Writing explicitly lets us treat it as the normal end of output.
fn is_broken_pipe(err: &io::Error) -> bool {
    err.kind() == io::ErrorKind::BrokenPipe
}

fn cmd_list(config_path: &Path, query: Option<&str>) -> Result<()> {
    let config = config::Config::load(config_path)?;
    let workspace = store::Workspace::load(&config)?;
    let query = parse_query(query)?;

    let stdout = io::stdout();
    let mut out = stdout.lock();

    match write_list(&mut out, &workspace, query.as_ref()) {
        Err(err) if is_broken_pipe(&err) => Ok(()),
        other => Ok(other?),
    }
}

fn write_list(
    out: &mut impl Write,
    workspace: &store::Workspace,
    query: Option<&query::Query>,
) -> io::Result<()> {
    let mut shown = 0usize;
    for group in &workspace.groups {
        let items: Vec<_> = workspace
            .items_for_group(&group.name)
            .into_iter()
            .filter(|item| match query {
                Some(q) => q.matches(item, workspace.group_name_for(item)),
                None => true,
            })
            .collect();
        if items.is_empty() && query.is_some() {
            continue;
        }
        shown += items.len();
        let open = items.iter().filter(|i| !i.done).count();
        writeln!(
            out,
            "\n{} ({} open / {} total)",
            group.name,
            open,
            items.len()
        )?;
        for item in items {
            let box_ = if item.done { "x" } else { " " };
            let indent = " ".repeat(item.indent);
            writeln!(
                out,
                "  {indent}[{box_}] {:<3} {}",
                item.priority.as_str(),
                item.text
            )?;
        }
    }
    match query {
        Some(_) => writeln!(out, "\n{shown} matching item(s)")?,
        None => writeln!(
            out,
            "\n{} open across {} groups",
            workspace.open_count(),
            workspace.groups.len()
        )?,
    }
    Ok(())
}

/// Parse a CLI query, turning a syntax error into a readable failure rather
/// than silently showing everything.
fn parse_query(query: Option<&str>) -> Result<Option<query::Query>> {
    match query {
        None => Ok(None),
        Some(text) => query::Query::parse(text).map_err(|err| eyre!("bad query: {err}")),
    }
}

async fn run_tui(config_path: &Path, query: Option<&str>) -> Result<()> {
    let config = config::Config::load(config_path)
        .map_err(|e| eyre!("{e}\n\nrun `mitodo init <workspace>` first to create a config file"))?;
    let workspace = store::Workspace::load(&config)?;
    info!(
        "loaded {} items across {} groups",
        workspace.items.len(),
        workspace.groups.len()
    );

    let (message_sender, message_receiver) = unbounded_channel::<Message>();
    let app_sender = message_sender.clone();

    // Watch the workspace so edits by Claude, mcli or todos-mcp show up
    // without a restart. Blocking, so it gets its own thread.
    let watch_root = config.workspace.root.clone();
    let watched_files: Vec<_> = workspace
        .groups
        .iter()
        .map(|g| g.todo_file.clone())
        .collect();
    let watch_sender = message_sender.clone();
    let _watch_handle = spawn_blocking(move || {
        store::watch::watch_blocking(&watch_root, watched_files, || {
            watch_sender
                .send(Message::Event(messages::Event::WorkspaceReloaded))
                .is_ok()
        });
    });

    // Drives the chyron. Cheap enough to run unconditionally; the app
    // ignores ticks when the ticker is off.
    let tick_sender = message_sender.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_millis(120));
        loop {
            interval.tick().await;
            if tick_sender
                .send(Message::Event(messages::Event::Tick))
                .is_err()
            {
                return;
            }
        }
    });

    // Terminal polling blocks, so it runs off the async runtime.
    let _input_handle = spawn_blocking(move || {
        if let Err(err) = input::input_reader(message_sender) {
            error!("input reader stopped: {err}");
        }
    });

    let terminal = ratatui::init();
    let mut app = ui::App::new(workspace, config).with_sender(app_sender);
    if let Some(text) = query {
        app.set_query(text)
            .map_err(|err| eyre!("bad query: {err}"))?;
    }
    let result = app.run(message_receiver, terminal).await;
    ratatui::restore();

    result
}
