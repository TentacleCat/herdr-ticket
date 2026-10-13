mod backend;
mod model;
mod ui;
mod watch;

use anyhow::{Context, Result};
use std::{env, path::PathBuf};

fn main() {
    if let Err(error) = run() {
        eprintln!("herdr-ticket: {error:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let mut args = env::args().skip(1);
    let mut root: Option<PathBuf> = None;
    let mut selftest = false;
    let mut compact = false;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--root" => {
                let path = args.next().context("--root requires a directory")?;
                root = Some(PathBuf::from(path));
            }
            "--selftest" => selftest = true,
            "--compact" => compact = true,
            "open-board" => return backend::open_pane("board"),
            "open-sidebar" => return backend::open_pane("sidebar"),
            "--help" | "-h" => {
                println!(
                    "herdr-ticket — a Herdr board for wedow/ticket\n\n\
                     Usage: herdr-ticket [--root <project>] [--selftest] [--compact]\n\
                       herdr-ticket open-board\n\
                       herdr-ticket open-sidebar\n\n\
                     Set TICKETS_DIR to choose a specific .tickets directory."
                );
                return Ok(());
            }
            other => anyhow::bail!("unknown argument `{other}`; use --help"),
        }
    }

    let tickets_dir = backend::resolve_tickets_dir(root.as_deref())?;
    let tickets = backend::load(&tickets_dir)?;
    if selftest {
        return backend::print_board(&tickets_dir);
    }
    ui::run(tickets_dir, tickets, compact)
}
