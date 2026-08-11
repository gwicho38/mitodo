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
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

pub use changeset::{Change, ChangeAction, ChangeSet};

use crate::config::{SchemaMode, ServiceConfig};

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
    #[error("cancelled")]
    Cancelled,
}

/// What the agent is being asked to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verb {
    /// Natural language to a query string. Read-only.
    Query,
    /// Summarise the items currently on screen. Read-only.
    Summarize,
    /// Explain the selected item on its own. Read-only.
    Explain,
    /// Carry out an instruction against one item, with whatever tools the
    /// agent has. Reports what it did and offers to mark the item done.
    Act,
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
            Verb::Explain => "explain",
            Verb::Act => "act",
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
            Verb::Summarize | Verb::Explain => {
                r#"{"type":"object","properties":{"brief":{"type":"string"}},"required":["brief"]}"#
            }
            Verb::Act => {
                r#"{"type":"object","properties":{"report":{"type":"string"},"done":{"type":"boolean"}},"required":["report"]}"#
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
                "Summarise these todo items: the themes, what looks stale, and what \
                 blocks what.\n\nReply with JSON matching the schema. The \"brief\" value \
                 must be plain prose written for a person to read — a few sentences or \
                 short dashed lines. Do not put JSON, objects or arrays inside it.\n\n{items}"
            }
            Verb::Explain => {
                "Explain this one todo item: what it actually asks for, what is needed to \
                 finish it, anything it appears to be waiting on, and how urgent it looks. \
                 Say plainly if the item is too vague to act on.\n\nReply with JSON \
                 matching the schema. The \"brief\" value must be plain prose written for a \
                 person to read. Do not put JSON, objects or arrays inside it.\n\n{item}"
            }
            Verb::Act => {
                "Carry out the request below against this todo item, using whatever tools \
                 you have. Do the work; do not just describe it.\n\nReply with JSON \
                 matching the schema. \"report\" is plain prose saying what you actually \
                 did — not what you intend to do — and must not contain JSON. Set \"done\" \
                 to true only if the item is now genuinely finished.\n\n{item}\n\nRequest: \
                 {input}"
            }
            Verb::Breakdown => {
                "Break this todo item into concrete next actions. Return between two and \
                 six short sub-items. Reply with JSON only.\n\n{item}"
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
pub fn render_prompt(template: &str, input: &str, items: &str, files: &str, item: &str) -> String {
    template
        .replace("{input}", input)
        .replace("{items}", items)
        .replace("{files}", files)
        .replace("{item}", item)
}

/// Run a service and return its raw stdout.
///
/// Blocking: callers put this on a dedicated thread so the UI keeps drawing.
/// `cancel` is polled on the same tick as the child, so setting it kills the
/// child rather than waiting out `timeout_secs`.
pub fn run(
    service: &ServiceConfig,
    schema: &str,
    prompt: &str,
    cwd: &Path,
    cancel: &Arc<AtomicBool>,
) -> Result<String, AgentError> {
    let Some((program, leading)) = service.command.split_first() else {
        return Err(AgentError::NotConfigured);
    };

    // Held for the whole call: dropping it removes the file, on every exit path.
    let mut schema_file: Option<tempfile::NamedTempFile> = None;

    let mut cmd = Command::new(program);
    cmd.args(leading);
    let prompt = match service.schema_mode {
        SchemaMode::Flag => {
            if let Some(flag) = &service.schema_flag {
                cmd.arg(flag).arg(schema);
            }
            prompt.to_string()
        }
        SchemaMode::File => {
            let mut file = tempfile::Builder::new()
                .prefix("mitodo-schema-")
                .suffix(".json")
                .tempfile()
                .map_err(|err| AgentError::Spawn(program.clone(), err))?;
            std::io::Write::write_all(&mut file, schema.as_bytes())
                .map_err(|err| AgentError::Spawn(program.clone(), err))?;
            if let Some(flag) = &service.schema_flag {
                cmd.arg(flag).arg(file.path());
            }
            schema_file = Some(file);
            prompt.to_string()
        }
        // `--format json` buys valid JSON, not the right shape, so say the shape.
        SchemaMode::Prompt => {
            format!("{prompt}\n\nReply with JSON matching this schema:\n{schema}")
        }
    };

    cmd.arg(&prompt)
        .current_dir(cwd)
        // Nested inside an agent session this makes the child see itself.
        .env_remove("CLAUDECODE")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = cmd
        .spawn()
        .map_err(|err| AgentError::Spawn(program.clone(), err))?;

    // Poll rather than block, so a wedged or unwanted agent is killed instead of
    // leaving the UI waiting on it for the rest of the session. The replies are
    // small JSON documents, so the pipe buffer will not fill while we wait.
    let deadline = Instant::now() + Duration::from_secs(service.timeout_secs);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => {
                if cancel.load(Ordering::Relaxed) {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(AgentError::Cancelled);
                }
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(AgentError::TimedOut(service.timeout_secs));
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(err) => return Err(AgentError::Spawn(program.clone(), err)),
        }
    }

    let output = child
        .wait_with_output()
        .map_err(|err| AgentError::Spawn(program.clone(), err))?;
    drop(schema_file);

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(AgentError::Failed(
            output.status.to_string(),
            stderr.lines().next().unwrap_or_default().to_string(),
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// Find the JSON object in an agent's reply.
///
/// Models wrap their answers: a markdown fence, a sentence of preamble, or a
/// transport envelope carrying the real payload as a string. Insisting on a
/// bare object is what leaks raw JSON into the UI, so this digs the object out
/// of whatever it arrived in.
pub fn extract_json(raw: &str) -> Option<serde_json::Value> {
    let text = raw.trim();

    // A fenced block, with or without a language tag.
    let unfenced = text
        .strip_prefix("```json")
        .or_else(|| text.strip_prefix("```"))
        .and_then(|rest| rest.rsplit_once("```").map(|(body, _)| body))
        .unwrap_or(text)
        .trim();

    let candidate = match serde_json::from_str::<serde_json::Value>(unfenced) {
        Ok(value) => value,
        // Prose around the object: take the outermost braces.
        Err(_) => {
            let start = unfenced.find('{')?;
            let end = unfenced.rfind('}')?;
            serde_json::from_str(unfenced.get(start..=end)?).ok()?
        }
    };

    // Some CLIs return {"result": "<the real json>"}.
    for envelope in ["result", "content", "text", "output"] {
        if let Some(inner) = candidate.get(envelope).and_then(|v| v.as_str())
            && let Some(parsed) = extract_json(inner)
        {
            return Some(parsed);
        }
    }
    Some(candidate)
}

/// Render a JSON value as something a person can read.
///
/// Models sometimes satisfy a string schema by stuffing JSON into the string.
/// Rather than showing that raw, flatten it: keys become labels and arrays
/// become dashed lines.
pub fn humanise(value: &serde_json::Value, depth: usize) -> Vec<String> {
    let pad = "  ".repeat(depth);
    match value {
        serde_json::Value::String(text) => vec![format!("{pad}{text}")],
        serde_json::Value::Number(n) => vec![format!("{pad}{n}")],
        serde_json::Value::Bool(b) => vec![format!("{pad}{b}")],
        serde_json::Value::Null => Vec::new(),
        serde_json::Value::Array(items) => items
            .iter()
            .flat_map(|item| match item {
                serde_json::Value::String(text) => vec![format!("{pad}- {text}")],
                other => humanise(other, depth),
            })
            .collect(),
        serde_json::Value::Object(fields) => fields
            .iter()
            .flat_map(|(key, field)| {
                let label = key.replace('_', " ");
                match field {
                    serde_json::Value::String(text) => vec![format!("{pad}{label}: {text}")],
                    other => {
                        let mut lines = vec![format!("{pad}{label}:")];
                        lines.extend(humanise(other, depth + 1));
                        lines
                    }
                }
            })
            .collect(),
    }
}

/// Turn whatever the agent said into readable text.
///
/// A string that is itself JSON gets flattened, because a schema-obedient
/// model will happily nest one inside the other.
fn readable(text: &str) -> String {
    match extract_json(text) {
        Some(value) if !value.is_string() => humanise(&value, 0).join("\n"),
        _ => text.to_string(),
    }
}

/// Pull a single string field out of an agent's JSON reply.
///
/// Falls back to the first string in the object, then to the raw text, because
/// a readable answer in the wrong field beats showing the user JSON.
pub fn field(json: &str, name: &str) -> Result<String, AgentError> {
    let Some(value) = extract_json(json) else {
        let text = json.trim();
        return if text.is_empty() {
            Err(AgentError::BadJson("the agent said nothing".to_string()))
        } else {
            Ok(text.to_string())
        };
    };

    if let Some(found) = value.get(name).and_then(|v| v.as_str()) {
        return Ok(readable(found));
    }
    if let Some(object) = value.as_object()
        && let Some(first) = object.values().find_map(|v| v.as_str())
    {
        return Ok(readable(first));
    }
    if let Some(text) = value.as_str() {
        return Ok(readable(text));
    }
    // Structured, but not in the shape asked for: show it readably rather than
    // failing or putting JSON on screen.
    Ok(humanise(&value, 0).join("\n"))
}

/// Read a boolean out of an agent's reply, if it said one either way.
pub fn flag(json: &str, name: &str) -> Option<bool> {
    extract_json(json)?.get(name)?.as_bool()
}

/// Pull an array-of-strings field out of an agent's JSON reply.
pub fn string_list(json: &str, name: &str) -> Result<Vec<String>, AgentError> {
    let value = extract_json(json).ok_or_else(|| AgentError::BadJson("not JSON".to_string()))?;
    let array = value
        .get(name)
        .and_then(|v| v.as_array())
        // Any array of strings will do if the name does not match.
        .or_else(|| {
            value
                .as_object()?
                .values()
                .find(|v| {
                    v.as_array()
                        .is_some_and(|a| a.iter().all(|i| i.is_string()))
                })?
                .as_array()
        })
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
        let out = render_prompt("do {input} with {items} {files} {item}", "X", "Y", "Z", "W");
        assert_eq!(out, "do X with Y Z W");
    }

    #[test]
    fn unused_placeholders_are_left_alone() {
        let out = render_prompt("only {input}", "X", "Y", "Z", "W");
        assert_eq!(out, "only X");
    }

    #[test]
    fn the_item_verbs_are_sent_the_item_not_the_whole_view() {
        for verb in [Verb::Explain, Verb::Breakdown] {
            let prompt = verb.default_prompt();
            assert!(prompt.contains("{item}"), "{} needs the item", verb.label());
            assert!(
                !prompt.contains("{items}"),
                "{} must not be sent the whole list",
                verb.label()
            );
        }
    }

    #[test]
    fn explain_is_read_only() {
        assert!(!Verb::Explain.writes());
    }

    #[test]
    fn act_is_sent_both_the_item_and_the_instruction() {
        let prompt = Verb::Act.default_prompt();
        assert!(prompt.contains("{item}"), "which item");
        assert!(prompt.contains("{input}"), "and what to do with it");
        assert!(
            prompt.contains("do not just describe"),
            "it is meant to act"
        );
    }

    #[test]
    fn the_act_schema_carries_a_report_and_a_done_flag() {
        let schema: serde_json::Value = serde_json::from_str(Verb::Act.schema()).unwrap();
        let props = schema.get("properties").unwrap();
        assert!(props.get("report").is_some());
        assert!(props.get("done").is_some());
    }

    #[test]
    fn a_boolean_field_is_read_from_a_reply() {
        assert_eq!(
            flag(r#"{"report":"did it","done":true}"#, "done"),
            Some(true)
        );
        assert_eq!(
            flag(r#"{"report":"tried","done":false}"#, "done"),
            Some(false)
        );
        assert_eq!(
            flag(r#"{"report":"tried"}"#, "done"),
            None,
            "absent is unknown"
        );
        assert_eq!(flag("not json", "done"), None);
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

    fn service(command: &[&str], mode: SchemaMode, flag: Option<&str>) -> ServiceConfig {
        ServiceConfig {
            name: "test".to_string(),
            command: command.iter().map(|s| s.to_string()).collect(),
            schema_mode: mode,
            schema_flag: flag.map(|f| f.to_string()),
            timeout_secs: 30,
        }
    }

    fn go(service: &ServiceConfig, prompt: &str) -> Result<String, AgentError> {
        let dir = tempfile::tempdir().unwrap();
        run(
            service,
            "{\"type\":\"object\"}",
            prompt,
            dir.path(),
            &Arc::new(AtomicBool::new(false)),
        )
    }

    #[test]
    fn runs_a_command_and_returns_stdout() {
        let out = go(&service(&["echo"], SchemaMode::Prompt, None), "hello").unwrap();
        assert!(out.contains("hello"));
    }

    #[test]
    fn flag_mode_passes_the_schema_inline_behind_the_flag() {
        let svc = service(&["echo"], SchemaMode::Flag, Some("--json-schema"));
        let out = go(&svc, "PROMPT").unwrap();
        assert!(out.contains("--json-schema"), "got {out:?}");
        assert!(out.contains("\"type\":\"object\""), "got {out:?}");
        assert!(out.trim().ends_with("PROMPT"), "prompt comes last: {out:?}");
    }

    // codex takes `--output-schema <FILE>`, so the schema reaches it as a path.
    #[test]
    fn file_mode_passes_a_readable_path_behind_the_flag() {
        let svc = service(
            &["sh", "-c", r#"printf '%s' "$(cat "$2")""#, "sh"],
            SchemaMode::File,
            Some("--x"),
        );
        let out = go(&svc, "ignored").unwrap();
        assert!(
            out.contains("\"type\":\"object\""),
            "the file held the schema: {out:?}"
        );
    }

    #[test]
    fn the_schema_file_is_gone_once_the_call_returns() {
        let svc = service(
            &["sh", "-c", r#"printf '%s' "$2""#, "sh"],
            SchemaMode::File,
            Some("--x"),
        );
        let printed = go(&svc, "ignored").unwrap();
        let path = std::path::PathBuf::from(printed.trim());
        assert!(
            !path.exists(),
            "temp schema outlived the call: {}",
            path.display()
        );
    }

    #[test]
    fn prompt_mode_appends_the_schema_to_the_prompt_and_passes_no_flag() {
        let svc = service(&["echo"], SchemaMode::Prompt, Some("--ignored"));
        let out = go(&svc, "PROMPT").unwrap();
        assert!(
            !out.contains("--ignored"),
            "prompt mode sends no flag: {out:?}"
        );
        assert!(out.contains("PROMPT"));
        assert!(
            out.contains("\"type\":\"object\""),
            "schema is in the prompt: {out:?}"
        );
    }

    // Nested inside a Claude Code session, the child sees itself running inside
    // itself unless this is stripped.
    #[test]
    fn claudecode_is_stripped_from_the_child_environment() {
        unsafe { std::env::set_var("CLAUDECODE", "1") };
        let svc = service(&["sh", "-c", "env", "sh"], SchemaMode::Prompt, None);
        let out = go(&svc, "ignored").unwrap();
        unsafe { std::env::remove_var("CLAUDECODE") };
        assert!(
            !out.contains("CLAUDECODE"),
            "leaked into the child: {out:?}"
        );
    }

    #[test]
    fn an_empty_command_is_not_configured() {
        let svc = service(&[], SchemaMode::Flag, None);
        assert!(matches!(go(&svc, "p"), Err(AgentError::NotConfigured)));
    }

    #[test]
    fn a_missing_program_is_reported() {
        let svc = service(&["definitely-not-a-real-program"], SchemaMode::Prompt, None);
        assert!(matches!(go(&svc, "p"), Err(AgentError::Spawn(..))));
    }

    #[test]
    fn a_nonzero_exit_is_reported_with_stderr() {
        let svc = service(
            &["sh", "-c", "echo boom >&2; exit 2"],
            SchemaMode::Prompt,
            None,
        );
        match go(&svc, "ignored") {
            Err(AgentError::Failed(_, stderr)) => assert!(stderr.contains("boom")),
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    #[test]
    fn a_wedged_agent_is_killed_rather_than_waited_on_forever() {
        let dir = tempfile::tempdir().unwrap();
        let mut svc = service(&["sh", "-c", "sleep 60"], SchemaMode::Prompt, None);
        svc.timeout_secs = 1;
        let started = std::time::Instant::now();
        let result = run(
            &svc,
            "{}",
            "ignored",
            dir.path(),
            &Arc::new(AtomicBool::new(false)),
        );
        assert!(matches!(result, Err(AgentError::TimedOut(1))), "{result:?}");
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "gave up promptly"
        );
    }

    #[test]
    fn a_cancelled_call_kills_the_child_promptly() {
        let dir = tempfile::tempdir().unwrap();
        let svc = service(&["sh", "-c", "sleep 60"], SchemaMode::Prompt, None);
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(200));
            flag.store(true, Ordering::Relaxed);
        });
        let started = std::time::Instant::now();
        let result = run(&svc, "{}", "ignored", dir.path(), &cancel);
        assert!(matches!(result, Err(AgentError::Cancelled)), "{result:?}");
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "cancel is not a timeout wait"
        );
    }

    #[test]
    fn a_prompt_config_is_still_honoured_within_the_timeout() {
        assert!(go(&service(&["echo"], SchemaMode::Prompt, None), "quick").is_ok());
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
    fn a_fenced_reply_is_unwrapped() {
        let fenced = "```json\n{\"brief\":\"all quiet\"}\n```";
        assert_eq!(field(fenced, "brief").unwrap(), "all quiet");
        let bare_fence = "```\n{\"brief\":\"all quiet\"}\n```";
        assert_eq!(field(bare_fence, "brief").unwrap(), "all quiet");
    }

    #[test]
    fn preamble_around_the_object_is_ignored() {
        let chatty = "Sure! Here you go:\n{\"brief\":\"all quiet\"}\nHope that helps.";
        assert_eq!(field(chatty, "brief").unwrap(), "all quiet");
    }

    #[test]
    fn a_transport_envelope_is_unwrapped() {
        let enveloped = r#"{"type":"result","result":"{\"brief\":\"all quiet\"}"}"#;
        assert_eq!(field(enveloped, "brief").unwrap(), "all quiet");
    }

    #[test]
    fn a_differently_named_field_still_yields_its_text() {
        // Better a readable answer from the wrong key than raw JSON on screen.
        assert_eq!(
            field(r#"{"summary":"all quiet"}"#, "brief").unwrap(),
            "all quiet"
        );
    }

    #[test]
    fn json_stuffed_into_the_string_field_is_flattened() {
        // What a schema-obedient model actually did: satisfy {brief: string}
        // by putting a JSON document inside the string.
        let nested = r#"{"brief":"{\"themes\":[\"deadlines\",\"chasing signatures\"]}"}"#;
        let out = field(nested, "brief").unwrap();
        assert!(!out.contains('{'), "no raw JSON on screen: {out}");
        assert!(out.contains("- deadlines"), "arrays become lines: {out}");
        assert!(out.contains("themes:"), "keys become labels: {out}");
    }

    #[test]
    fn a_reply_in_the_wrong_shape_is_still_readable() {
        let wrong = r#"{"themes":["a","b"],"stale":[{"item":"x","why":"late"}]}"#;
        let out = field(wrong, "brief").unwrap();
        assert!(!out.contains("{\""), "no raw JSON: {out}");
        assert!(out.contains("- a") && out.contains("- b"));
        assert!(out.contains("item: x") && out.contains("why: late"));
    }

    #[test]
    fn prose_is_left_alone() {
        let plain = r#"{"brief":"Two filings are overdue and one needs a notary."}"#;
        assert_eq!(
            field(plain, "brief").unwrap(),
            "Two filings are overdue and one needs a notary."
        );
    }

    #[test]
    fn the_summarize_prompt_asks_for_prose() {
        let prompt = Verb::Summarize.default_prompt();
        assert!(
            prompt.contains("plain prose"),
            "the schema alone is not enough"
        );
        assert!(prompt.contains("Do not put JSON"));
    }

    #[test]
    fn a_plain_text_reply_is_shown_as_it_is() {
        assert_eq!(
            field("just a sentence", "brief").unwrap(),
            "just a sentence"
        );
    }

    #[test]
    fn an_empty_reply_is_an_error() {
        assert!(matches!(field("   ", "brief"), Err(AgentError::BadJson(_))));
    }

    #[test]
    fn a_list_under_another_name_is_still_found() {
        let items = string_list(r#"{"items":["a","b"]}"#, "sub_items").unwrap();
        assert_eq!(items, vec!["a", "b"]);
    }

    #[test]
    fn a_reply_with_no_list_at_all_is_rejected() {
        assert!(matches!(
            string_list(r#"{"sub_items":"not an array"}"#, "sub_items"),
            Err(AgentError::BadJson(_))
        ));
    }
}
