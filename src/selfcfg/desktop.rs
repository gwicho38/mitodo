//! The one client with no CLI, so the one config mitodo edits itself.
//!
//! Exactly one key is touched and every other byte is preserved: read, modify,
//! write a temp file in the same directory, rename over the original. The same
//! discipline `store::write` applies to markdown, for the same reason — it is
//! someone else's file.

use std::path::Path;

use serde_json::{Map, Value, json};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub command: String,
    pub args: Vec<String>,
}

fn load(path: &Path) -> Result<Value, String> {
    let raw = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if raw.trim().is_empty() {
        return Ok(json!({}));
    }
    serde_json::from_str(&raw).map_err(|e| format!("{} is not valid JSON: {e}", path.display()))
}

fn servers_of(root: &Value) -> Result<Map<String, Value>, String> {
    match root.get("mcpServers") {
        None => Ok(Map::new()),
        Some(Value::Object(map)) => Ok(map.clone()),
        Some(_) => Err("mcpServers is not an object; refusing to rewrite it".to_string()),
    }
}

/// What `name` is registered as, if it is.
pub fn read_entry(path: &Path, name: &str) -> Result<Option<Entry>, String> {
    if !path.is_file() {
        return Ok(None);
    }
    let servers = servers_of(&load(path)?)?;
    Ok(servers.get(name).map(|entry| Entry {
        command: entry
            .get("command")
            .and_then(|c| c.as_str())
            .unwrap_or_default()
            .to_string(),
        args: entry
            .get("args")
            .and_then(|a| a.as_array())
            .map(|list| {
                list.iter()
                    .filter_map(|a| a.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default(),
    }))
}

/// Write `name` into the file's `mcpServers`, leaving everything else alone.
pub fn merge(path: &Path, name: &str, entry: &Entry) -> Result<(), String> {
    let mut root = if path.is_file() {
        load(path)?
    } else {
        json!({})
    };
    let mut servers = servers_of(&root)?;
    servers.insert(
        name.to_string(),
        json!({"command": entry.command, "args": entry.args}),
    );
    root.as_object_mut()
        .ok_or_else(|| format!("{} is not a JSON object", path.display()))?
        .insert("mcpServers".to_string(), Value::Object(servers));
    write_atomically(path, &root)
}

/// Drop `name`. `Ok(false)` means it was not there.
pub fn remove(path: &Path, name: &str) -> Result<bool, String> {
    if !path.is_file() {
        return Ok(false);
    }
    let mut root = load(path)?;
    let mut servers = servers_of(&root)?;
    if servers.remove(name).is_none() {
        return Ok(false);
    }
    root.as_object_mut()
        .ok_or_else(|| format!("{} is not a JSON object", path.display()))?
        .insert("mcpServers".to_string(), Value::Object(servers));
    write_atomically(path, &root)?;
    Ok(true)
}

/// The temp file goes in the same directory, so the rename stays on one
/// filesystem and is therefore atomic.
fn write_atomically(path: &Path, root: &Value) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("{} has no parent directory", path.display()))?;
    std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;

    let mut text = serde_json::to_string_pretty(root).map_err(|e| e.to_string())?;
    text.push('\n');

    let temp = tempfile::Builder::new()
        .prefix(".mitodo-")
        .suffix(".json")
        .tempfile_in(parent)
        .map_err(|e| format!("{}: {e}", parent.display()))?;
    std::fs::write(temp.path(), text).map_err(|e| format!("{}: {e}", temp.path().display()))?;
    temp.persist(path)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ours() -> Entry {
        Entry {
            command: "/abs/mitodo".to_string(),
            args: vec!["mcp-server".to_string()],
        }
    }

    fn config(body: &str) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("claude_desktop_config.json");
        std::fs::write(&path, body).unwrap();
        (dir, path)
    }

    const EXISTING: &str = r#"{
  "mcpServers": {
    "gmail-multi": {
      "command": "/opt/gmail",
      "args": []
    }
  },
  "preferences": {
    "theme": "dark"
  }
}
"#;

    #[test]
    fn merging_leaves_the_other_servers_and_keys_intact() {
        let (_dir, path) = config(EXISTING);
        merge(&path, "mitodo", &ours()).unwrap();

        let root: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(root["mcpServers"]["gmail-multi"]["command"], "/opt/gmail");
        assert_eq!(
            root["preferences"]["theme"], "dark",
            "unrelated keys survive"
        );
        assert_eq!(root["mcpServers"]["mitodo"]["command"], "/abs/mitodo");
        assert_eq!(root["mcpServers"]["mitodo"]["args"][0], "mcp-server");
    }

    #[test]
    fn merging_twice_changes_nothing_the_second_time() {
        let (_dir, path) = config(EXISTING);
        merge(&path, "mitodo", &ours()).unwrap();
        let once = std::fs::read_to_string(&path).unwrap();
        merge(&path, "mitodo", &ours()).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), once);
    }

    #[test]
    fn a_stale_command_is_replaced() {
        let (_dir, path) = config(EXISTING);
        merge(
            &path,
            "mitodo",
            &Entry {
                command: "/old/mitodo".to_string(),
                args: vec!["mcp-server".to_string()],
            },
        )
        .unwrap();
        merge(&path, "mitodo", &ours()).unwrap();
        assert_eq!(
            read_entry(&path, "mitodo").unwrap().unwrap().command,
            "/abs/mitodo"
        );
    }

    #[test]
    fn read_entry_reports_absence_and_presence() {
        let (_dir, path) = config(EXISTING);
        assert!(read_entry(&path, "mitodo").unwrap().is_none());
        merge(&path, "mitodo", &ours()).unwrap();
        assert_eq!(read_entry(&path, "mitodo").unwrap(), Some(ours()));
    }

    #[test]
    fn removing_drops_only_our_entry() {
        let (_dir, path) = config(EXISTING);
        merge(&path, "mitodo", &ours()).unwrap();
        assert!(remove(&path, "mitodo").unwrap());

        let root: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert!(root["mcpServers"].get("mitodo").is_none());
        assert_eq!(root["mcpServers"]["gmail-multi"]["command"], "/opt/gmail");
    }

    #[test]
    fn removing_something_absent_reports_false_and_writes_nothing() {
        let (_dir, path) = config(EXISTING);
        let before = std::fs::read_to_string(&path).unwrap();
        assert!(!remove(&path, "mitodo").unwrap());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
    }

    // Never overwrite a file we could not read.
    #[test]
    fn unparseable_json_is_refused_and_the_file_is_untouched() {
        let (_dir, path) = config("{ this is not json");
        let before = std::fs::read_to_string(&path).unwrap();
        let failure = merge(&path, "mitodo", &ours()).unwrap_err();
        assert!(failure.contains("not valid JSON"), "{failure}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
    }

    #[test]
    fn a_non_object_mcpservers_is_refused() {
        let (_dir, path) = config(r#"{"mcpServers": ["not", "an", "object"]}"#);
        let failure = merge(&path, "mitodo", &ours()).unwrap_err();
        assert!(failure.contains("not an object"), "{failure}");
    }

    #[test]
    fn a_missing_mcpservers_key_is_created() {
        let (_dir, path) = config(r#"{"preferences": {"theme": "light"}}"#);
        merge(&path, "mitodo", &ours()).unwrap();
        let root: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(root["mcpServers"]["mitodo"]["command"], "/abs/mitodo");
        assert_eq!(root["preferences"]["theme"], "light");
    }

    #[test]
    fn an_empty_file_is_treated_as_an_empty_config() {
        let (_dir, path) = config("");
        merge(&path, "mitodo", &ours()).unwrap();
        assert!(read_entry(&path, "mitodo").unwrap().is_some());
    }

    #[test]
    fn a_file_that_does_not_exist_yet_is_created() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("claude_desktop_config.json");
        merge(&path, "mitodo", &ours()).unwrap();
        assert!(path.is_file());
        assert_eq!(read_entry(&path, "mitodo").unwrap(), Some(ours()));
    }
}
