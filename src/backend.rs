use crate::model::{load_tickets, Ticket};
use anyhow::{Context, Result};
use serde_json::Value;
use std::{
    env,
    path::{Path, PathBuf},
    process::{Command, Output},
};

pub fn resolve_tickets_dir(explicit_root: Option<&Path>) -> Result<PathBuf> {
    if let Some(path) = env::var_os("TICKETS_DIR") {
        return Ok(PathBuf::from(path));
    }
    if let Some(root) = explicit_root {
        return Ok(as_tickets_dir(root));
    }

    let plugin_root = env::var_os("HERDR_PLUGIN_ROOT").map(PathBuf::from);
    let mut candidates = Vec::new();
    if let Ok(context) = env::var("HERDR_PLUGIN_CONTEXT_JSON") {
        if let Ok(context) = serde_json::from_str::<Value>(&context) {
            collect_context_paths(&context, &mut candidates);
        }
    }

    let current = env::current_dir().context("could not read the current directory")?;
    candidates.push(current);

    for candidate in &candidates {
        if let Some(found) = find_existing_tickets_dir(candidate) {
            return Ok(found);
        }
    }

    for candidate in candidates {
        if let Some(plugin_root) = &plugin_root {
            if candidate == *plugin_root || candidate.starts_with(plugin_root) {
                continue;
            }
        }
        if candidate.is_dir() {
            return Ok(as_tickets_dir(&candidate));
        }
    }

    anyhow::bail!(
        "could not locate a project with .tickets/; launch from that project, pass --root <path>, or set TICKETS_DIR"
    )
}

fn as_tickets_dir(path: &Path) -> PathBuf {
    if path.file_name().and_then(|name| name.to_str()) == Some(".tickets") {
        path.to_path_buf()
    } else {
        path.join(".tickets")
    }
}

fn find_existing_tickets_dir(path: &Path) -> Option<PathBuf> {
    if path.file_name().and_then(|name| name.to_str()) == Some(".tickets") && path.is_dir() {
        return Some(path.to_path_buf());
    }
    for parent in path.ancestors() {
        let tickets = parent.join(".tickets");
        if tickets.is_dir() {
            return Some(tickets);
        }
    }
    None
}

fn collect_context_paths(value: &Value, paths: &mut Vec<PathBuf>) {
    const PATH_KEYS: &[&str] = &[
        "cwd",
        "focused_pane_cwd",
        "working_directory",
        "workspace_cwd",
        "worktree_path",
        "project_path",
        "project_root",
        "repo_path",
        "root_path",
        "workspace_path",
        "path",
        "root",
    ];
    match value {
        Value::Object(object) => {
            for (key, value) in object {
                if PATH_KEYS.contains(&key.as_str()) {
                    if let Some(path) = value.as_str() {
                        let path = PathBuf::from(path);
                        if path.is_absolute() && !paths.contains(&path) {
                            paths.push(path);
                        }
                    }
                }
                collect_context_paths(value, paths);
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_context_paths(item, paths);
            }
        }
        _ => {}
    }
}

pub fn load(tickets_dir: &Path) -> Result<Vec<Ticket>> {
    load_tickets(tickets_dir)
}

pub fn set_status(tickets_dir: &Path, id: &str, status: &str) -> Result<()> {
    run_tk(tickets_dir, &["status", id, status]).map(|_| ())
}

pub fn create(tickets_dir: &Path, title: &str, description: &str) -> Result<String> {
    let mut args = vec![
        "create".to_owned(),
        title.to_owned(),
        "--type".to_owned(),
        "task".to_owned(),
        "--priority".to_owned(),
        "2".to_owned(),
    ];
    if !description.trim().is_empty() {
        args.push("--description".to_owned());
        args.push(description.to_owned());
    }
    let refs = args.iter().map(String::as_str).collect::<Vec<_>>();
    run_tk(tickets_dir, &refs)
}

fn run_tk(tickets_dir: &Path, args: &[&str]) -> Result<String> {
    let mut command = Command::new("tk");
    if let Some(project_root) = tickets_dir.parent().filter(|path| path.is_dir()) {
        command.current_dir(project_root);
    }
    let output: Output = command
        .args(args)
        .env("TICKETS_DIR", tickets_dir)
        .output()
        .with_context(|| {
            "could not start `tk`; install wedow/ticket and make sure it is on PATH"
        })?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        anyhow::bail!("tk {} failed: {stderr}", args.join(" "));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

pub fn print_board(tickets_dir: &Path) -> Result<()> {
    let tickets = load(tickets_dir)?;
    println!("Tickets: {}", tickets_dir.display());
    for status in ["open", "in_progress", "closed"] {
        let group = tickets
            .iter()
            .filter(|ticket| ticket.status == status)
            .collect::<Vec<_>>();
        println!("\n{} ({})", crate::model::status_label(status), group.len());
        for ticket in group {
            println!(
                "  {} [P{}] {} — {}",
                ticket.id, ticket.priority, ticket.kind, ticket.title
            );
        }
    }
    Ok(())
}

pub fn open_pane(entrypoint: &str) -> Result<()> {
    let herdr = env::var("HERDR_BIN_PATH").unwrap_or_else(|_| "herdr".to_owned());
    let output = Command::new(herdr)
        .args([
            "plugin",
            "pane",
            "open",
            "--plugin",
            "herdr-ticket",
            "--entrypoint",
            entrypoint,
        ])
        .output()
        .context("could not start Herdr; invoke this action from a Herdr session")?;
    if !output.status.success() {
        anyhow::bail!(
            "Herdr could not open the {entrypoint} pane: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}
