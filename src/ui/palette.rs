//! Every action in the app, as data.
//!
//! One table drives the command palette and the `?` help screen, so a new
//! binding cannot appear in one and be forgotten in the other. An entry carries
//! the key it stands for rather than a behaviour: running it presses that key.

// The table and the scorer land before the palette that reads them.
#![allow(dead_code)]

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    Navigation,
    Items,
    Query,
    Agent,
    Groups,
    View,
    Session,
}

impl Category {
    pub const ALL: [Category; 7] = [
        Category::Navigation,
        Category::Items,
        Category::Query,
        Category::Agent,
        Category::Groups,
        Category::View,
        Category::Session,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Category::Navigation => "Navigation",
            Category::Items => "Items",
            Category::Query => "Query",
            Category::Agent => "Agent",
            Category::Groups => "Groups",
            Category::View => "View",
            Category::Session => "Session",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Action {
    pub label: &'static str,
    /// What the palette and the help screen print for this binding.
    pub keys: &'static str,
    /// The event dispatch presses. Uppercase letters are matched with `_`
    /// modifiers by the key handler, so SHIFT here is cosmetic.
    pub key: KeyEvent,
    pub category: Category,
}

const fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
    KeyEvent::new(code, modifiers)
}

const NONE: KeyModifiers = KeyModifiers::NONE;
const SHIFT: KeyModifiers = KeyModifiers::SHIFT;

pub const ACTIONS: [Action; 41] = [
    Action {
        label: "move down",
        keys: "↓",
        key: key(KeyCode::Down, NONE),
        category: Category::Navigation,
    },
    Action {
        label: "move up",
        keys: "↑",
        key: key(KeyCode::Up, NONE),
        category: Category::Navigation,
    },
    Action {
        label: "open a node and step into it",
        keys: "→",
        key: key(KeyCode::Right, NONE),
        category: Category::Navigation,
    },
    Action {
        label: "close a node and step out",
        keys: "←",
        key: key(KeyCode::Left, NONE),
        category: Category::Navigation,
    },
    Action {
        label: "jump to first item",
        keys: "g",
        key: key(KeyCode::Char('g'), NONE),
        category: Category::Navigation,
    },
    Action {
        label: "jump to last item",
        keys: "G",
        key: key(KeyCode::Char('G'), SHIFT),
        category: Category::Navigation,
    },
    Action {
        label: "move focus left, to the groups pane",
        keys: "h",
        key: key(KeyCode::Char('h'), NONE),
        category: Category::Navigation,
    },
    Action {
        label: "move focus right, back to the item list",
        keys: "l",
        key: key(KeyCode::Char('l'), NONE),
        category: Category::Navigation,
    },
    Action {
        label: "move focus down, to the detail pane",
        keys: "j",
        key: key(KeyCode::Char('j'), NONE),
        category: Category::Navigation,
    },
    Action {
        label: "move focus up, to the item list",
        keys: "k",
        key: key(KeyCode::Char('k'), NONE),
        category: Category::Navigation,
    },
    Action {
        label: "jump focus to the item list",
        keys: "tab",
        key: key(KeyCode::Tab, NONE),
        category: Category::Navigation,
    },
    Action {
        label: "jump focus to the groups list",
        keys: "shift-tab",
        key: key(KeyCode::BackTab, SHIFT),
        category: Category::Navigation,
    },
    Action {
        label: "fold this node",
        keys: "z",
        key: key(KeyCode::Char('z'), NONE),
        category: Category::Navigation,
    },
    Action {
        label: "fold or unfold everything",
        keys: "Z",
        key: key(KeyCode::Char('Z'), SHIFT),
        category: Category::Navigation,
    },
    Action {
        label: "toggle done",
        keys: "space / x",
        key: key(KeyCode::Char(' '), NONE),
        category: Category::Items,
    },
    Action {
        label: "new item (dialog)",
        keys: "a",
        key: key(KeyCode::Char('a'), NONE),
        category: Category::Items,
    },
    Action {
        label: "quick add sibling",
        keys: "o",
        key: key(KeyCode::Char('o'), NONE),
        category: Category::Items,
    },
    Action {
        label: "add child item",
        keys: "A",
        key: key(KeyCode::Char('A'), SHIFT),
        category: Category::Items,
    },
    Action {
        label: "edit item text",
        keys: "e",
        key: key(KeyCode::Char('e'), NONE),
        category: Category::Items,
    },
    Action {
        label: "edit notes in the detail pane",
        keys: "i",
        key: key(KeyCode::Char('i'), NONE),
        category: Category::Items,
    },
    Action {
        label: "delete item (asks first)",
        keys: "d",
        key: key(KeyCode::Char('d'), NONE),
        category: Category::Items,
    },
    Action {
        label: "edit the query",
        keys: "/",
        key: key(KeyCode::Char('/'), NONE),
        category: Category::Query,
    },
    Action {
        label: "clear the query, or cancel a running agent",
        keys: "esc",
        key: key(KeyCode::Esc, NONE),
        category: Category::Query,
    },
    Action {
        label: "hide or show done items",
        keys: "H",
        key: key(KeyCode::Char('H'), SHIFT),
        category: Category::Query,
    },
    Action {
        label: "describe a filter in words, get a query",
        keys: "n",
        key: key(KeyCode::Char('n'), NONE),
        category: Category::Agent,
    },
    Action {
        label: "summarise what's on screen",
        keys: "S",
        key: key(KeyCode::Char('S'), SHIFT),
        category: Category::Agent,
    },
    Action {
        label: "explain this item",
        keys: "E",
        key: key(KeyCode::Char('E'), SHIFT),
        category: Category::Agent,
    },
    Action {
        label: "break this item into sub-items",
        keys: "b",
        key: key(KeyCode::Char('b'), NONE),
        category: Category::Agent,
    },
    Action {
        label: "act on this item with an agent",
        keys: "!",
        key: key(KeyCode::Char('!'), SHIFT),
        category: Category::Agent,
    },
    Action {
        label: "scan the workspace for changes",
        keys: "R",
        key: key(KeyCode::Char('R'), SHIFT),
        category: Category::Agent,
    },
    Action {
        label: "manage items with the agent",
        keys: "M",
        key: key(KeyCode::Char('M'), SHIFT),
        category: Category::Agent,
    },
    Action {
        label: "pick the model service",
        keys: "m",
        key: key(KeyCode::Char('m'), NONE),
        category: Category::Agent,
    },
    Action {
        label: "read this group's notes",
        keys: "N",
        key: key(KeyCode::Char('N'), SHIFT),
        category: Category::Groups,
    },
    Action {
        label: "archive finished items",
        keys: "X",
        key: key(KeyCode::Char('X'), SHIFT),
        category: Category::Groups,
    },
    Action {
        label: "view settings menu",
        keys: "v",
        key: key(KeyCode::Char('v'), NONE),
        category: Category::View,
    },
    Action {
        label: "scrolling ticker on or off",
        keys: "c",
        key: key(KeyCode::Char('c'), NONE),
        category: Category::View,
    },
    Action {
        label: "pause the ticker",
        keys: "p",
        key: key(KeyCode::Char('p'), NONE),
        category: Category::View,
    },
    Action {
        label: "ticker faster",
        keys: "+",
        key: key(KeyCode::Char('+'), SHIFT),
        category: Category::View,
    },
    Action {
        label: "ticker slower",
        keys: "-",
        key: key(KeyCode::Char('-'), NONE),
        category: Category::View,
    },
    Action {
        label: "keyboard help",
        keys: "?",
        key: key(KeyCode::Char('?'), SHIFT),
        category: Category::Session,
    },
    Action {
        label: "quit",
        keys: "q",
        key: key(KeyCode::Char('q'), NONE),
        category: Category::Session,
    },
];

/// The `?` screen, grouped by category.
///
/// Generated from `ACTIONS` so a new binding cannot be added to the palette and
/// forgotten here.
pub fn help_lines() -> Vec<String> {
    let mut lines = vec![
        "  :  or  ctrl-k   command palette — type to filter, enter to run".to_string(),
        String::new(),
    ];
    for category in Category::ALL {
        lines.push(category.label().to_string());
        for action in ACTIONS.iter().filter(|a| a.category == category) {
            lines.push(format!("  {:<10} {}", action.keys, action.label));
        }
        lines.push(String::new());
    }
    lines
}

/// One row of the palette.
///
/// A service is not a keypress — picking one calls into the service list by
/// index — so the two cases stay distinct rather than being forced into a key.
#[derive(Debug, Clone)]
pub enum Entry {
    Key(&'static Action),
    Service { name: String, index: usize },
}

impl Entry {
    pub fn label(&self) -> String {
        match self {
            Entry::Key(action) => action.label.to_string(),
            Entry::Service { name, .. } => format!("use model service: {name}"),
        }
    }

    pub fn keys(&self) -> &str {
        match self {
            Entry::Key(action) => action.keys,
            Entry::Service { .. } => "",
        }
    }

    pub fn category(&self) -> &'static str {
        match self {
            Entry::Key(action) => action.category.label(),
            Entry::Service { .. } => "Model",
        }
    }
}

/// The entries matching `needle`, best first.
///
/// `services` arrives as names rather than being read from config, so this stays
/// a pure function of its arguments.
pub fn filter(needle: &str, services: &[String]) -> Vec<Entry> {
    let all = ACTIONS
        .iter()
        .map(Entry::Key)
        .chain(
            services
                .iter()
                .enumerate()
                .map(|(index, name)| Entry::Service {
                    name: name.clone(),
                    index,
                }),
        );

    let mut scored: Vec<(u32, Entry)> = all
        .filter_map(|entry| {
            score_entry(needle, &entry.label(), entry.category()).map(|points| (points, entry))
        })
        .collect();

    // Stable, so an empty needle keeps the table's grouping.
    scored.sort_by(|left, right| right.0.cmp(&left.0));
    scored.into_iter().map(|(_, entry)| entry).collect()
}

/// How well `needle` matches `haystack`, or `None` if it does not.
///
/// A greedy left-to-right subsequence walk: not the optimal alignment fzf finds
/// by dynamic programming, but indistinguishable across 41 short labels.
/// How well `needle` matches an entry, or `None` if any of its words match
/// neither the label nor the category.
///
/// Each word is scored against the two fields separately and takes its better
/// result. Scoring against them joined would let one word straddle the boundary,
/// and a word every entry in a category shares would then outweigh the word that
/// actually distinguishes them.
pub fn score_entry(needle: &str, label: &str, category: &str) -> Option<u32> {
    if needle.trim().is_empty() {
        return Some(0);
    }
    let label: Vec<char> = label.to_lowercase().chars().collect();
    let category: Vec<char> = category.to_lowercase().chars().collect();

    let mut total = 0u32;
    for word in needle.split_whitespace() {
        let in_label = score_word(word, &label);
        let in_category = score_word(word, &category);
        total += in_label.max(in_category)?;
    }
    Some(total)
}

/// Each whitespace-separated word is scored on its own, so word order does not
/// matter.
pub fn score(needle: &str, haystack: &str) -> Option<u32> {
    if needle.trim().is_empty() {
        return Some(0);
    }
    let hay: Vec<char> = haystack.to_lowercase().chars().collect();
    let mut total = 0u32;
    for word in needle.split_whitespace() {
        total += score_word(word, &hay)?;
    }
    Some(total)
}

fn score_word(word: &str, hay: &[char]) -> Option<u32> {
    let mut points: i64 = 0;
    let mut next = 0usize;
    let mut previous: Option<usize> = None;

    for wanted in word.to_lowercase().chars() {
        let found = hay[next..].iter().position(|c| *c == wanted)? + next;
        points += 1;
        match previous {
            None => points -= (found as i64).min(3),
            // Above the word-start bonus, so a contiguous run beats characters
            // that happen to land on several word starts.
            Some(prev) if prev + 1 == found => points += 10,
            Some(prev) => points -= ((found - prev - 1) as i64).min(4),
        }
        if found == 0 || matches!(hay.get(found - 1), Some(' ') | Some('-') | Some(':')) {
            points += 8;
        }
        previous = Some(found);
        next = found + 1;
    }
    Some(points.max(0) as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_help_screen_lists_every_action() {
        let text = help_lines().join("\n");
        for action in ACTIONS {
            assert!(text.contains(action.label), "help omits {:?}", action.label);
            assert!(
                text.contains(action.keys),
                "help omits key {:?}",
                action.keys
            );
        }
    }

    #[test]
    fn the_help_screen_is_grouped_by_category() {
        let text = help_lines().join("\n");
        for category in Category::ALL {
            assert!(
                text.contains(category.label()),
                "help omits the {} heading",
                category.label()
            );
        }
    }

    #[test]
    fn the_help_screen_mentions_the_palette_itself() {
        let text = help_lines().join("\n");
        assert!(
            text.contains(':'),
            "the palette key is worth finding: {text}"
        );
    }

    fn labels(entries: &[Entry]) -> Vec<String> {
        entries.iter().map(|e| e.label()).collect()
    }

    #[test]
    fn an_empty_needle_returns_every_entry_in_table_order() {
        let entries = filter("", &[]);
        assert_eq!(entries.len(), ACTIONS.len());
        assert_eq!(entries[0].label(), ACTIONS[0].label);
        assert_eq!(
            entries[ACTIONS.len() - 1].label(),
            ACTIONS[ACTIONS.len() - 1].label
        );
    }

    #[test]
    fn one_entry_is_appended_per_configured_service() {
        let services = vec!["claude".to_string(), "ollama".to_string()];
        let entries = filter("", &services);
        assert_eq!(entries.len(), ACTIONS.len() + 2);
        let found = labels(&entries);
        assert!(found.contains(&"use model service: claude".to_string()));
        assert!(found.contains(&"use model service: ollama".to_string()));
    }

    #[test]
    fn a_service_entry_carries_its_index() {
        let services = vec!["claude".to_string(), "ollama".to_string()];
        let entries = filter("ollama", &services);
        match entries.first() {
            Some(Entry::Service { name, index }) => {
                assert_eq!(name, "ollama");
                assert_eq!(*index, 1, "the index selects it in config order");
            }
            other => panic!("expected a service entry, got {other:?}"),
        }
    }

    #[test]
    fn the_worked_examples_rank_as_designed() {
        for (needle, expected) in [
            ("arf", "archive finished items"),
            ("mod", "pick the model service"),
            ("manage", "manage items with the agent"),
            ("sum", "summarise what's on screen"),
        ] {
            let entries = filter(needle, &[]);
            let top = entries.first().map(|e| e.label()).unwrap_or_default();
            assert_eq!(top, expected, "{needle:?} should rank {expected:?} first");
        }
    }

    #[test]
    fn the_category_is_searchable_too() {
        let entries = filter("agent scan", &[]);
        assert_eq!(
            entries.first().map(|e| e.label()).unwrap_or_default(),
            "scan the workspace for changes"
        );
    }

    #[test]
    fn a_needle_matching_nothing_returns_no_entries() {
        assert!(filter("qqzzxx", &[]).is_empty());
    }

    // Joining label and category into one haystack would let "scan" match across
    // the boundary — "…serviceAgent" contains s, c, a, n in order — so a shared
    // category word would decide instead of the word that distinguishes.
    #[test]
    fn a_word_must_match_one_field_rather_than_straddle_two() {
        assert!(score_entry("agent scan", "scan the workspace for changes", "Agent").is_some());
        assert_eq!(
            score_entry("agent scan", "pick the model service", "Agent"),
            None,
            "\"scan\" is in neither this label nor its category"
        );
    }

    #[test]
    fn a_word_matching_neither_field_rejects_the_entry() {
        assert_eq!(
            score_entry("zebra", "archive finished items", "Groups"),
            None
        );
        assert!(score_entry("groups archive", "archive finished items", "Groups").is_some());
    }

    #[test]
    fn a_key_entry_reports_its_display_keys_and_a_service_entry_does_not() {
        let entries = filter("archive finished", &[]);
        assert_eq!(entries[0].keys(), "X");
        assert_eq!(entries[0].category(), "Groups");

        let services = vec!["codex".to_string()];
        let service = filter("codex", &services);
        assert_eq!(service[0].keys(), "");
        assert_eq!(service[0].category(), "Model");
    }

    #[test]
    fn a_subsequence_matches_and_a_non_subsequence_does_not() {
        assert!(score("arf", "archive finished items").is_some());
        assert!(score("zzz", "archive finished items").is_none());
    }

    #[test]
    fn characters_must_appear_in_order() {
        assert!(score("ab", "abc").is_some());
        assert!(score("ba", "abc").is_none());
    }

    #[test]
    fn matching_ignores_case_in_both_directions() {
        assert!(score("ARF", "archive finished items").is_some());
        assert!(score("arf", "ARCHIVE FINISHED ITEMS").is_some());
    }

    #[test]
    fn an_empty_needle_matches_everything_equally() {
        assert_eq!(score("", "anything"), Some(0));
        assert_eq!(score("", "something else"), Some(0));
    }

    #[test]
    fn a_word_start_outranks_a_mid_word_hit() {
        let at_start = score("f", "finished items").unwrap();
        let mid_word = score("f", "off screen").unwrap();
        assert!(at_start > mid_word, "{at_start} should beat {mid_word}");
    }

    #[test]
    fn a_contiguous_run_outranks_a_scattered_one() {
        let contiguous = score("arch", "archive items").unwrap();
        let scattered = score("arch", "a rather cheap thing").unwrap();
        assert!(
            contiguous > scattered,
            "{contiguous} should beat {scattered}"
        );
    }

    #[test]
    fn an_early_match_outranks_a_late_one() {
        let early = score("item", "items list").unwrap();
        let late = score("item", "delete an item").unwrap();
        assert!(early > late, "{early} should beat {late}");
    }

    #[test]
    fn every_action_has_a_label_and_a_key_display() {
        for action in ACTIONS {
            assert!(!action.label.is_empty(), "a label is missing");
            assert!(
                !action.keys.is_empty(),
                "{} has no key display",
                action.label
            );
        }
    }

    #[test]
    fn action_labels_are_unique() {
        let mut labels: Vec<&str> = ACTIONS.iter().map(|a| a.label).collect();
        labels.sort_unstable();
        let count = labels.len();
        labels.dedup();
        assert_eq!(labels.len(), count, "two actions share a label");
    }
}
