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

/// How well `needle` matches `haystack`, or `None` if it does not.
///
/// A greedy left-to-right subsequence walk: not the optimal alignment fzf finds
/// by dynamic programming, but indistinguishable across 41 short labels.
pub fn score(needle: &str, haystack: &str) -> Option<u32> {
    if needle.trim().is_empty() {
        return Some(0);
    }
    let needle: Vec<char> = needle.to_lowercase().chars().collect();
    let hay: Vec<char> = haystack.to_lowercase().chars().collect();

    let mut points: u32 = 0;
    let mut next = 0usize;
    let mut previous_match: Option<usize> = None;

    for wanted in needle {
        let found = hay[next..].iter().position(|c| *c == wanted)? + next;
        let at_word_start =
            found == 0 || matches!(hay.get(found - 1), Some(' ') | Some('-') | Some(':'));
        points += 1;
        if at_word_start {
            points += 8;
        }
        // Above the word-start bonus, so a contiguous run beats a needle whose
        // characters happen to land on several word starts.
        if previous_match == Some(found.saturating_sub(1)) {
            points += 10;
        }
        if previous_match.is_none() {
            points = points.saturating_sub(found as u32);
        }
        previous_match = Some(found);
        next = found + 1;
    }
    Some(points)
}

#[cfg(test)]
mod tests {
    use super::*;

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
