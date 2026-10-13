use anyhow::{Context, Result};
use serde_yaml::Value;
use std::{fs, path::Path};

#[derive(Clone, Debug)]
pub struct Ticket {
    pub id: String,
    pub title: String,
    pub status: String,
    pub priority: u8,
    pub kind: String,
    pub assignee: String,
    pub tags: Vec<String>,
    pub dependencies: Vec<String>,
    pub description: String,
}

impl Ticket {
    fn parse(path: &Path) -> Result<Self> {
        let source = fs::read_to_string(path)
            .with_context(|| format!("could not read {}", path.display()))?;
        let mut lines = source.lines();
        let first = lines
            .next()
            .unwrap_or_default()
            .trim_start_matches('\u{feff}');
        if first.trim() != "---" {
            anyhow::bail!("{} has no YAML frontmatter", path.display());
        }

        let mut frontmatter = String::new();
        let mut body = String::new();
        let mut in_frontmatter = true;
        let mut found_end = false;
        for line in lines {
            if in_frontmatter && line.trim() == "---" {
                in_frontmatter = false;
                found_end = true;
                continue;
            }
            if in_frontmatter {
                frontmatter.push_str(line);
                frontmatter.push('\n');
            } else {
                body.push_str(line);
                body.push('\n');
            }
        }
        if !found_end {
            anyhow::bail!("{} has unterminated YAML frontmatter", path.display());
        }

        let yaml: Value = serde_yaml::from_str(&frontmatter)
            .with_context(|| format!("could not parse frontmatter in {}", path.display()))?;
        let id = string_field(&yaml, "id").unwrap_or_else(|| {
            path.file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or("unknown")
                .to_owned()
        });
        let heading_title = body
            .lines()
            .find_map(|line| line.strip_prefix("# "))
            .map(str::trim)
            .filter(|title| !title.is_empty());
        let title = string_field(&yaml, "title")
            .or_else(|| heading_title.map(str::to_owned))
            .unwrap_or_else(|| id.clone());
        let description = body
            .lines()
            .filter(|line| !line.starts_with("# "))
            .collect::<Vec<_>>()
            .join("\n")
            .trim()
            .to_owned();

        Ok(Self {
            id,
            title,
            status: string_field(&yaml, "status").unwrap_or_else(|| "open".to_owned()),
            priority: number_field(&yaml, "priority").unwrap_or(2).clamp(0, 4) as u8,
            kind: string_field(&yaml, "type").unwrap_or_else(|| "task".to_owned()),
            assignee: string_field(&yaml, "assignee").unwrap_or_default(),
            tags: string_list(&yaml, "tags"),
            dependencies: string_list(&yaml, "deps"),
            description,
        })
    }
}

pub fn load_tickets(tickets_dir: &Path) -> Result<Vec<Ticket>> {
    if !tickets_dir.exists() {
        return Ok(Vec::new());
    }
    let mut tickets = Vec::new();
    for entry in fs::read_dir(tickets_dir)
        .with_context(|| format!("could not list {}", tickets_dir.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("md") {
            continue;
        }
        match Ticket::parse(&path) {
            Ok(ticket) => tickets.push(ticket),
            Err(error) => eprintln!("herdr-ticket: skipped {}: {error:#}", path.display()),
        }
    }
    tickets.sort_by(|left, right| {
        left.priority
            .cmp(&right.priority)
            .then_with(|| left.id.cmp(&right.id))
    });
    Ok(tickets)
}

fn string_field(yaml: &Value, key: &str) -> Option<String> {
    yaml.get(key).and_then(Value::as_str).map(str::to_owned)
}

fn number_field(yaml: &Value, key: &str) -> Option<i64> {
    yaml.get(key).and_then(|value| {
        value
            .as_i64()
            .or_else(|| value.as_str().and_then(|value| value.parse().ok()))
    })
}

fn string_list(yaml: &Value, key: &str) -> Vec<String> {
    let Some(value) = yaml.get(key) else {
        return Vec::new();
    };
    match value {
        Value::Sequence(items) => items
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect(),
        Value::String(items) => items
            .trim_matches(['[', ']'])
            .split(',')
            .map(|item| item.trim().trim_matches(['\'', '"']))
            .filter(|item| !item.is_empty())
            .map(str::to_owned)
            .collect(),
        _ => Vec::new(),
    }
}

pub fn status_label(status: &str) -> String {
    status
        .split('_')
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}
