//! Running an external agent and applying what it returns.
//!
//! The private `mcli todos scan` shells out to `claude --print` with a JSON
//! schema and applies the change-set it gets back. Generalised, that is: run a
//! configured command with a prompt, receive structured JSON, then render it or
//! apply it after review. Four verbs share that one pipeline.
//!
//! The command is any binary that takes a prompt and emits JSON, so no provider
//! is baked in and no email code enters this repository.

// Wired into the UI in the next commit; the pipeline and its tests come first.
#![allow(dead_code, unused_imports)]

pub mod changeset;

use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub use changeset::{Change, ChangeAction, ChangeSet};

#[derive(thiserror::Error, Debug)]
pub enum AgentError {
    #[error("no agent command configured")]
    NotConfigured,
    #[error("could not run {0}: {1}")]
    Spawn(String, std::io::Error),
    #[error("agent exited with {0}: {1}")]
    Failed(String, String),
    #[error("agent output was not valid JSON: {0}")]
    BadJson(String),
    #[error("agent did not finish within {0}s")]
    TimedOut(u64),
}

/// What the agent is being asked to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verb {
    /// Natural language to a query string. Read-only.
    Query,
    /// Summarise the items currently on screen. Read-only.
    Summarize,
    /// Propose sub-items for one item. Writes, after review.
    Breakdown,
    /// Propose a change-set across the workspace. Writes, after review.
    Scan,
}

impl Verb {
    pub fn label(self) -> &'static str {
        match self {
            Verb::Query => "query",
            Verb::Summarize => "summarize",
            Verb::Breakdown => "breakdown",
            Verb::Scan => "scan",
        }
    }

    /// Whether the result mutates the workspace, and so needs review.
    pub fn writes(self) -> bool {
        matches!(self, Verb::Breakdown | Verb::Scan)
    }

    /// JSON Schema the agent is asked to conform to.
    pub fn schema(self) -> &'static str {
        match self {
            Verb::Query => {
                r#"{"type":"object","properties":{"query":{"type":"string"}},"required":["query"]}"#
            }
            Verb::Summarize => {
                r#"{"type":"object","properties":{"brief":{"type":"string"}},"required":["brief"]}"#
            }
            Verb::Breakdown => {
                r#"{"type":"object","properties":{"sub_items":{"type":"array","items":{"type":"string"}}},"required":["sub_items"]}"#
            }
            Verb::Scan => changeset::SCAN_SCHEMA,
        }
    }

    /// Prompt used when the config supplies no template for this verb.
    pub fn default_prompt(self) -> &'static str {
        match self {
            Verb::Query => {
                "Translate the request below into a mitodo query. Fields: acct:<group>, \
                 pri:P0-P3 (optionally <=, >=, <, >), done, !done, sec:\"<section>\", \
                 has:desc, text:\"<substring>\". Combine with AND, OR, NOT and parentheses. \
                 Reply with JSON only.\n\nRequest: {input}"
            }
            Verb::Summarize => {
                "Summarise these todo items in a few sentences: the themes, what looks \
                 stale, and what blocks what. Reply with JSON only.\n\n{items}"
            }
            Verb::Breakdown => {
                "Break this todo item into concrete next actions. Return between two and \
                 six short sub-items. Reply with JSON only.\n\nItem: {input}"
            }
            Verb::Scan => {
                "You are a todo tracking assistant. Read the todo files below, then:\n\
                 1. Find new actionable items from the sources available to you that are \
                 not already listed.\n\
                 2. Identify existing unchecked items that have since been resolved.\n\n\
                 Reply with JSON only. Each change names the file it belongs to, using the \
                 workspace-relative path exactly as shown below.\n\n{files}"
            }
        }
    }
}

/// Substitute the prompt placeholders.
///
/// `{items}` is the view as rendered — enough for summarising what is on
/// screen. `{files}` is every todo file with its workspace-relative path and
/// full contents, which is what a change-set needs: a change names the file it
/// belongs to, so an agent that never saw the paths cannot produce one.
pub fn render_prompt(template: &str, input: &str, items: &str, files: &str) -> String {
    template
        .replace("{input}", input)
        .replace("{items}", items)
        .replace("{files}", files)
}

/// Run the configured agent and return its raw stdout.
///
/// Blocking: callers put this on a dedicated thread so the UI keeps drawing.
pub fn run(
    command: &[String],
    schema_flag: Option<&str>,
    schema: &str,
    prompt: &str,
    cwd: &Path,
    timeout_secs: u64,
) -> Result<String, AgentError> {
    let Some((program, leading)) = command.split_first() else {
        return Err(AgentError::NotConfigured);
    };

    let mut cmd = Command::new(program);
    cmd.args(leading);
    if let Some(flag) = schema_flag {
        cmd.arg(flag).arg(schema);
    }
    cmd.arg(prompt)
        .current_dir(cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = cmd
        .spawn()
        .map_err(|err| AgentError::Spawn(program.clone(), err))?;

    // Poll rather than block, so a wedged agent is killed instead of leaving
    // the UI waiting on it for the rest of the session. The replies are small
    // JSON documents, so the pipe buffer will not fill while we wait.
    let deadline = Instant::now() + Duration::from_secs(timeout_secs);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(AgentError::TimedOut(timeout_secs));
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(err) => return Err(AgentError::Spawn(program.clone(), err)),
        }
    }

    let output = child
        .wait_with_output()
        .map_err(|err| AgentError::Spawn(program.clone(), err))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(AgentError::Failed(
            output.status.to_string(),
            stderr.lines().next().unwrap_or_default().to_string(),
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// Pull a single string field out of an agent's JSON reply.
pub fn field(json: &str, name: &str) -> Result<String, AgentError> {
    let value: serde_json::Value =
        serde_json::from_str(json.trim()).map_err(|e| AgentError::BadJson(e.to_string()))?;
    value
        .get(name)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| AgentError::BadJson(format!("missing string field {name:?}")))
}

/// Pull an array-of-strings field out of an agent's JSON reply.
pub fn string_list(json: &str, name: &str) -> Result<Vec<String>, AgentError> {
    let value: serde_json::Value =
        serde_json::from_str(json.trim()).map_err(|e| AgentError::BadJson(e.to_string()))?;
    let array = value
        .get(name)
        .and_then(|v| v.as_array())
        .ok_or_else(|| AgentError::BadJson(format!("missing array field {name:?}")))?;
    Ok(array
        .iter()
        .filter_map(|v| v.as_str().map(|s| s.to_string()))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verbs_declare_whether_they_write() {
        assert!(!Verb::Query.writes());
        assert!(!Verb::Summarize.writes());
        assert!(Verb::Breakdown.writes());
        assert!(Verb::Scan.writes());
    }

    #[test]
    fn every_verb_has_valid_json_schema() {
        for verb in [Verb::Query, Verb::Summarize, Verb::Breakdown, Verb::Scan] {
            let parsed: Result<serde_json::Value, _> = serde_json::from_str(verb.schema());
            assert!(parsed.is_ok(), "{} schema is valid JSON", verb.label());
        }
    }

    #[test]
    fn prompt_templates_substitute_placeholders() {
        let out = render_prompt("do {input} with {items} and {files}", "X", "Y", "Z");
        assert_eq!(out, "do X with Y and Z");
    }

    #[test]
    fn unused_placeholders_are_left_alone() {
        let out = render_prompt("only {input}", "X", "Y", "Z");
        assert_eq!(out, "only X");
    }

    #[test]
    fn the_scan_prompt_asks_for_the_files_not_the_view() {
        // A change-set names the file it belongs to, so scan must see paths.
        let prompt = Verb::Scan.default_prompt();
        assert!(prompt.contains("{files}"), "scan needs the file dump");
        assert!(
            !prompt.contains("{items}"),
            "the rendered view is not enough"
        );
    }

    #[test]
    fn runs_a_command_and_returns_stdout() {
        let dir = tempfile::tempdir().unwrap();
        let command = vec!["echo".to_string()];
        let out = run(&command, None, "{}", "hello", dir.path(), 30).unwrap();
        assert_eq!(out.trim(), "hello");
    }

    #[test]
    fn passes_the_schema_behind_the_configured_flag() {
        let dir = tempfile::tempdir().unwrap();
        let command = vec!["echo".to_string()];
        let out = run(
            &command,
            Some("--schema"),
            "SCHEMA",
            "PROMPT",
            dir.path(),
            30,
        )
        .unwrap();
        assert!(out.contains("--schema SCHEMA PROMPT"), "got {out:?}");
    }

    #[test]
    fn an_empty_command_is_not_configured() {
        let dir = tempfile::tempdir().unwrap();
        assert!(matches!(
            run(&[], None, "{}", "p", dir.path(), 30),
            Err(AgentError::NotConfigured)
        ));
    }

    #[test]
    fn a_missing_program_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        let command = vec!["definitely-not-a-real-program".to_string()];
        assert!(matches!(
            run(&command, None, "{}", "p", dir.path(), 30),
            Err(AgentError::Spawn(..))
        ));
    }

    #[test]
    fn a_nonzero_exit_is_reported_with_stderr() {
        let dir = tempfile::tempdir().unwrap();
        let command = vec![
            "sh".to_string(),
            "-c".to_string(),
            "echo boom >&2; exit 2".to_string(),
        ];
        match run(&command, None, "{}", "ignored", dir.path(), 30) {
            Err(AgentError::Failed(_, stderr)) => assert!(stderr.contains("boom")),
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    #[test]
    fn a_wedged_agent_is_killed_rather_than_waited_on_forever() {
        let dir = tempfile::tempdir().unwrap();
        // The prompt is appended as the last argument, so the command has to
        // tolerate an extra one.
        let command = vec!["sh".to_string(), "-c".to_string(), "sleep 60".to_string()];
        let started = std::time::Instant::now();
        let result = run(&command, None, "{}", "ignored", dir.path(), 1);
        assert!(matches!(result, Err(AgentError::TimedOut(1))), "{result:?}");
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "gave up promptly"
        );
    }

    #[test]
    fn a_prompt_config_is_still_honoured_within_the_timeout() {
        let dir = tempfile::tempdir().unwrap();
        let command = vec!["echo".to_string()];
        assert!(run(&command, None, "{}", "quick", dir.path(), 30).is_ok());
    }

    #[test]
    fn extracts_string_fields() {
        assert_eq!(field(r#"{"query":"pri:P0"}"#, "query").unwrap(), "pri:P0");
        assert_eq!(field("  {\"brief\":\"ok\"}  ", "brief").unwrap(), "ok");
    }

    #[test]
    fn extracts_string_lists() {
        let items = string_list(r#"{"sub_items":["a","b"]}"#, "sub_items").unwrap();
        assert_eq!(items, vec!["a", "b"]);
    }

    #[test]
    fn malformed_json_is_rejected_cleanly() {
        assert!(matches!(
            field("not json", "query"),
            Err(AgentError::BadJson(_))
        ));
        assert!(matches!(
            field(r#"{"other":1}"#, "query"),
            Err(AgentError::BadJson(_))
        ));
        assert!(matches!(
            string_list(r#"{"sub_items":"not an array"}"#, "sub_items"),
            Err(AgentError::BadJson(_))
        ));
    }
}
