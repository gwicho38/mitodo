//! The tool catalogue, as data.
//!
//! Names keep todos-mcp's `todos_` prefix so prompts written against that server
//! keep working and the two stay recognisably one surface.

pub struct Tool {
    pub name: &'static str,
    pub description: &'static str,
    /// JSON Schema for the tool's arguments, as a literal.
    pub schema: &'static str,
}

pub const TOOLS: [Tool; 13] = [
    Tool {
        name: "todos_list",
        description: "List todo items. Optionally filter with mitodo's query language \
                      (for example \"pri:P0 !done\" or \"acct:lysk text:\\\"bank\\\"\") \
                      and/or restrict to one group. Completed items are excluded unless \
                      include_done is true.",
        schema: r#"{"type":"object","properties":{"query":{"type":"string"},"group":{"type":"string"},"include_done":{"type":"boolean"}},"additionalProperties":false}"#,
    },
    Tool {
        name: "todos_get_item",
        description: "Read one item in full by id, including its notes and its direct children.",
        schema: r#"{"type":"object","properties":{"id":{"type":"string"}},"required":["id"],"additionalProperties":false}"#,
    },
    Tool {
        name: "todos_get_file",
        description: "Read a group's TODO.md verbatim, for when the raw markdown matters.",
        schema: r#"{"type":"object","properties":{"group":{"type":"string"}},"required":["group"],"additionalProperties":false}"#,
    },
    Tool {
        name: "todos_list_groups",
        description: "List the workspace's groups with their open and total item counts.",
        schema: r#"{"type":"object","properties":{},"additionalProperties":false}"#,
    },
    Tool {
        name: "todos_create_item",
        description: "Add a new item to a group. section names the heading to place it under. \
                      Optionally attach notes and child items.",
        schema: r#"{"type":"object","properties":{"group":{"type":"string"},"text":{"type":"string"},"section":{"type":"string"},"notes":{"type":"string"},"children":{"type":"array","items":{"type":"string"}}},"required":["group","text"],"additionalProperties":false}"#,
    },
    Tool {
        name: "todos_add_child",
        description: "Add a child item beneath an existing item, indented under it.",
        schema: r#"{"type":"object","properties":{"parent_id":{"type":"string"},"text":{"type":"string"}},"required":["parent_id","text"],"additionalProperties":false}"#,
    },
    Tool {
        name: "todos_update_item",
        description: "Edit an item's text and/or set whether it is done, in one write. \
                      Returns the item, whose id changes when the text changed.",
        schema: r#"{"type":"object","properties":{"id":{"type":"string"},"new_text":{"type":"string"},"done":{"type":"boolean"}},"required":["id"],"additionalProperties":false}"#,
    },
    Tool {
        name: "todos_set_notes",
        description: "Replace the notes beneath an item. An empty string removes them.",
        schema: r#"{"type":"object","properties":{"id":{"type":"string"},"notes":{"type":"string"}},"required":["id","notes"],"additionalProperties":false}"#,
    },
    Tool {
        name: "todos_delete_item",
        description: "Delete an item and its notes outright. Prefer todos_archive_item, \
                      which moves it into the archive instead.",
        schema: r#"{"type":"object","properties":{"id":{"type":"string"}},"required":["id"],"additionalProperties":false}"#,
    },
    Tool {
        name: "todos_archive_item",
        description: "Move one item, and everything nested under it, into the group's \
                      archive file under a dated heading. A move, not a delete.",
        schema: r#"{"type":"object","properties":{"id":{"type":"string"}},"required":["id"],"additionalProperties":false}"#,
    },
    Tool {
        name: "todos_archive_finished",
        description: "Move every finished top-level item in a group into its archive. \
                      An item whose subtree still holds open work is left alone and reported.",
        schema: r#"{"type":"object","properties":{"group":{"type":"string"}},"required":["group"],"additionalProperties":false}"#,
    },
    Tool {
        name: "todos_create_group",
        description: "Create a new group: a directory with a TODO.md seeded with the same \
                      section headings the existing groups use.",
        schema: r#"{"type":"object","properties":{"name":{"type":"string"}},"required":["name"],"additionalProperties":false}"#,
    },
    Tool {
        name: "todos_sync",
        description: "Run the workspace's configured git sync commands and return their output.",
        schema: r#"{"type":"object","properties":{},"additionalProperties":false}"#,
    },
];

/// The catalogue as `tools/list` returns it.
pub fn schemas() -> Vec<serde_json::Value> {
    TOOLS
        .iter()
        .map(|tool| {
            serde_json::json!({
                "name": tool.name,
                "description": tool.description,
                "inputSchema": serde_json::from_str::<serde_json::Value>(tool.schema)
                    .expect("tool schemas are literals, checked by tests"),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_tool_has_a_name_a_description_and_a_valid_schema() {
        for tool in &TOOLS {
            assert!(!tool.name.is_empty());
            assert!(
                tool.description.len() > 20,
                "{} needs a description an agent can act on",
                tool.name
            );
            let schema: serde_json::Value = serde_json::from_str(tool.schema)
                .unwrap_or_else(|e| panic!("{} has invalid schema JSON: {e}", tool.name));
            assert_eq!(
                schema["type"], "object",
                "{} schema is not an object",
                tool.name
            );
            assert!(
                schema.get("properties").is_some(),
                "{} has no properties",
                tool.name
            );
        }
    }

    #[test]
    fn tool_names_are_unique_and_prefixed() {
        let mut names: Vec<&str> = TOOLS.iter().map(|t| t.name).collect();
        let count = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), count, "two tools share a name");
        assert!(TOOLS.iter().all(|t| t.name.starts_with("todos_")));
    }

    #[test]
    fn the_catalogue_covers_the_specified_surface() {
        let names: Vec<&str> = TOOLS.iter().map(|t| t.name).collect();
        for expected in [
            "todos_list",
            "todos_get_item",
            "todos_get_file",
            "todos_list_groups",
            "todos_create_item",
            "todos_add_child",
            "todos_update_item",
            "todos_set_notes",
            "todos_delete_item",
            "todos_archive_item",
            "todos_archive_finished",
            "todos_create_group",
            "todos_sync",
        ] {
            assert!(names.contains(&expected), "missing {expected}");
        }
    }
}
