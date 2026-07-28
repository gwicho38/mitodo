//! Query language over todo items.
//!
//! eilmeldung's query module is 1326 lines built around article fields. The AST
//! shape is worth keeping, but the vocabulary and the matcher are entirely
//! different, so this is a fresh compact implementation.
//!
//! ```text
//! acct:lefv          group / account
//! pri:P0  pri:<=P1   priority, with optional comparison
//! done   !done       completion
//! sec:"P1 — High"    section heading
//! has:desc           has a description block
//! text:"onehouse"    substring, case-insensitive
//! onehouse           bare word — same as text:
//! AND OR NOT ( )     combinators; adjacency implies AND
//! ```

use crate::store::model::{Item, Priority};

#[derive(thiserror::Error, Debug, PartialEq, Eq)]
pub enum QueryError {
    #[error("unexpected end of query")]
    UnexpectedEnd,
    #[error("unexpected {0:?}")]
    Unexpected(String),
    #[error("unclosed quote")]
    UnclosedQuote,
    #[error("{0:?} is not a priority (expected P0–P3)")]
    BadPriority(String),
    #[error("unknown field {0:?}")]
    UnknownField(String),
    #[error("missing closing parenthesis")]
    UnclosedParen,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cmp {
    Eq,
    Le,
    Ge,
    Lt,
    Gt,
}

impl Cmp {
    fn test(self, left: Priority, right: Priority) -> bool {
        // `Priority::None` sorts last, which makes `pri:<=P1` exclude
        // unprioritised items — the intuitive reading.
        match self {
            Cmp::Eq => left == right,
            Cmp::Le => left <= right,
            Cmp::Ge => left >= right,
            Cmp::Lt => left < right,
            Cmp::Gt => left > right,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Atom {
    Group(String),
    Priority(Cmp, Priority),
    Done(bool),
    Section(String),
    HasDescription,
    Text(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Clause {
    And(Box<Clause>, Box<Clause>),
    Or(Box<Clause>, Box<Clause>),
    Not(Box<Clause>),
    Atom(Atom),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Query {
    clause: Clause,
}

impl Query {
    pub fn parse(source: &str) -> Result<Option<Query>, QueryError> {
        let tokens = tokenize(source)?;
        if tokens.is_empty() {
            return Ok(None);
        }
        let mut parser = Parser { tokens, pos: 0 };
        let clause = parser.parse_or()?;
        if parser.pos < parser.tokens.len() {
            return Err(QueryError::Unexpected(parser.tokens[parser.pos].clone()));
        }
        Ok(Some(Query { clause }))
    }

    /// `group` is the name of the group the item belongs to, needed by `acct:`.
    pub fn matches(&self, item: &Item, group: Option<&str>) -> bool {
        eval(&self.clause, item, group)
    }
}

fn eval(clause: &Clause, item: &Item, group: Option<&str>) -> bool {
    match clause {
        Clause::And(a, b) => eval(a, item, group) && eval(b, item, group),
        Clause::Or(a, b) => eval(a, item, group) || eval(b, item, group),
        Clause::Not(inner) => !eval(inner, item, group),
        Clause::Atom(atom) => eval_atom(atom, item, group),
    }
}

fn eval_atom(atom: &Atom, item: &Item, group: Option<&str>) -> bool {
    match atom {
        Atom::Group(name) => group.is_some_and(|g| g.eq_ignore_ascii_case(name)),
        Atom::Priority(cmp, priority) => cmp.test(item.priority, *priority),
        Atom::Done(want) => item.done == *want,
        Atom::Section(needle) => item.section.to_lowercase().contains(&needle.to_lowercase()),
        Atom::HasDescription => !item.description.is_empty(),
        Atom::Text(needle) => {
            let needle = needle.to_lowercase();
            item.text.to_lowercase().contains(&needle)
                || item.description.to_lowercase().contains(&needle)
        }
    }
}

/// Split on whitespace, keeping quoted runs together and parentheses separate.
fn tokenize(source: &str) -> Result<Vec<String>, QueryError> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut chars = source.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            '"' => {
                in_quotes = !in_quotes;
                // A quote is part of the token so `sec:"a b"` stays one token;
                // the field parser strips them.
                current.push(c);
            }
            '(' | ')' if !in_quotes => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
                tokens.push(c.to_string());
            }
            c if c.is_whitespace() && !in_quotes => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(c),
        }
        // Peek only to keep the borrow checker quiet about `chars` above.
        let _ = chars.peek();
    }

    if in_quotes {
        return Err(QueryError::UnclosedQuote);
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    Ok(tokens)
}

struct Parser {
    tokens: Vec<String>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> Option<&str> {
        self.tokens.get(self.pos).map(|s| s.as_str())
    }

    fn eat_keyword(&mut self, keyword: &str) -> bool {
        if self.peek().is_some_and(|t| t.eq_ignore_ascii_case(keyword)) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn parse_or(&mut self) -> Result<Clause, QueryError> {
        let mut left = self.parse_and()?;
        while self.eat_keyword("OR") {
            let right = self.parse_and()?;
            left = Clause::Or(Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn parse_and(&mut self) -> Result<Clause, QueryError> {
        let mut left = self.parse_unary()?;
        loop {
            // Adjacency implies AND, so an explicit AND is optional.
            if self.eat_keyword("AND") {
                let right = self.parse_unary()?;
                left = Clause::And(Box::new(left), Box::new(right));
                continue;
            }
            match self.peek() {
                None | Some(")") => break,
                Some(t) if t.eq_ignore_ascii_case("OR") => break,
                _ => {
                    let right = self.parse_unary()?;
                    left = Clause::And(Box::new(left), Box::new(right));
                }
            }
        }
        Ok(left)
    }

    fn parse_unary(&mut self) -> Result<Clause, QueryError> {
        if self.eat_keyword("NOT") {
            return Ok(Clause::Not(Box::new(self.parse_unary()?)));
        }
        let Some(token) = self.peek() else {
            return Err(QueryError::UnexpectedEnd);
        };
        if let Some(rest) = token.strip_prefix('!')
            && !rest.is_empty()
        {
            let rest = rest.to_string();
            self.pos += 1;
            return Ok(Clause::Not(Box::new(Clause::Atom(parse_atom(&rest)?))));
        }
        self.parse_primary()
    }

    fn parse_primary(&mut self) -> Result<Clause, QueryError> {
        let Some(token) = self.peek().map(|s| s.to_string()) else {
            return Err(QueryError::UnexpectedEnd);
        };
        if token == "(" {
            self.pos += 1;
            let inner = self.parse_or()?;
            if self.peek() != Some(")") {
                return Err(QueryError::UnclosedParen);
            }
            self.pos += 1;
            return Ok(inner);
        }
        if token == ")" {
            return Err(QueryError::Unexpected(token));
        }
        self.pos += 1;
        Ok(Clause::Atom(parse_atom(&token)?))
    }
}

fn unquote(value: &str) -> String {
    value.trim_matches('"').to_string()
}

fn parse_atom(token: &str) -> Result<Atom, QueryError> {
    if token.eq_ignore_ascii_case("done") {
        return Ok(Atom::Done(true));
    }

    let Some((field, value)) = token.split_once(':') else {
        return Ok(Atom::Text(unquote(token)));
    };

    match field.to_lowercase().as_str() {
        "acct" | "account" | "group" => Ok(Atom::Group(unquote(value))),
        "sec" | "section" => Ok(Atom::Section(unquote(value))),
        "text" => Ok(Atom::Text(unquote(value))),
        "has" => match unquote(value).to_lowercase().as_str() {
            "desc" | "description" => Ok(Atom::HasDescription),
            other => Err(QueryError::UnknownField(format!("has:{other}"))),
        },
        "pri" | "priority" => {
            let raw = unquote(value);
            let (cmp, rest) = if let Some(r) = raw.strip_prefix("<=") {
                (Cmp::Le, r)
            } else if let Some(r) = raw.strip_prefix(">=") {
                (Cmp::Ge, r)
            } else if let Some(r) = raw.strip_prefix('<') {
                (Cmp::Lt, r)
            } else if let Some(r) = raw.strip_prefix('>') {
                (Cmp::Gt, r)
            } else {
                (Cmp::Eq, raw.as_str())
            };
            let priority = match rest.to_uppercase().as_str() {
                "P0" => Priority::P0,
                "P1" => Priority::P1,
                "P2" => Priority::P2,
                "P3" => Priority::P3,
                _ => return Err(QueryError::BadPriority(rest.to_string())),
            };
            Ok(Atom::Priority(cmp, priority))
        }
        other => Err(QueryError::UnknownField(other.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::model::ItemId;
    use std::path::PathBuf;

    fn item(text: &str, done: bool, priority: Priority) -> Item {
        Item {
            id: ItemId::compute("f", "s", "h", 0, text),
            file: PathBuf::from("f"),
            line: 0,
            indent: 0,
            done,
            text: text.to_string(),
            description: String::new(),
            section: "P1 — High Priority".to_string(),
            heading: "H".to_string(),
            priority,
            parent: None,
            children: Vec::new(),
        }
    }

    fn matches(query: &str, item: &Item, group: Option<&str>) -> bool {
        Query::parse(query)
            .expect("query parses")
            .expect("query is not empty")
            .matches(item, group)
    }

    #[test]
    fn an_empty_query_is_none() {
        assert_eq!(Query::parse("").unwrap(), None);
        assert_eq!(Query::parse("   ").unwrap(), None);
    }

    #[test]
    fn bare_words_match_text_case_insensitively() {
        let it = item("File the 83(b) election", false, Priority::P0);
        assert!(matches("83(b)", &it, None) || matches("election", &it, None));
        assert!(matches("ELECTION", &it, None), "case-insensitive");
        assert!(!matches("nonexistent", &it, None));
    }

    #[test]
    fn text_also_searches_the_description() {
        let mut it = item("short", false, Priority::P0);
        it.description = "mentions the CPA".to_string();
        assert!(matches("cpa", &it, None));
    }

    #[test]
    fn done_and_not_done() {
        let open = item("a", false, Priority::P0);
        let done = item("a", true, Priority::P0);
        assert!(matches("done", &done, None));
        assert!(!matches("done", &open, None));
        assert!(matches("!done", &open, None));
        assert!(!matches("!done", &done, None));
    }

    #[test]
    fn priority_equality_and_comparison() {
        let p0 = item("a", false, Priority::P0);
        let p2 = item("a", false, Priority::P2);
        assert!(matches("pri:P0", &p0, None));
        assert!(!matches("pri:P0", &p2, None));
        assert!(matches("pri:<=P1", &p0, None), "P0 is more urgent than P1");
        assert!(!matches("pri:<=P1", &p2, None));
        assert!(matches("pri:>=P1", &p2, None));
    }

    #[test]
    fn priority_is_case_insensitive() {
        let p0 = item("a", false, Priority::P0);
        assert!(matches("pri:p0", &p0, None));
    }

    #[test]
    fn unprioritised_items_are_excluded_by_an_upper_bound() {
        let none = item("a", false, Priority::None);
        assert!(!matches("pri:<=P3", &none, None), "None sorts after P3");
    }

    #[test]
    fn group_matches_the_supplied_group_name() {
        let it = item("a", false, Priority::P0);
        assert!(matches("acct:lefv", &it, Some("lefv")));
        assert!(matches("acct:LEFV", &it, Some("lefv")), "case-insensitive");
        assert!(!matches("acct:lefv", &it, Some("jzlaw")));
        assert!(!matches("acct:lefv", &it, None));
    }

    #[test]
    fn section_matches_a_substring() {
        let it = item("a", false, Priority::P1);
        assert!(matches("sec:High", &it, None));
        assert!(matches(r#"sec:"High Priority""#, &it, None));
        assert!(!matches("sec:Someday", &it, None));
    }

    #[test]
    fn has_desc_tests_for_a_description_block() {
        let mut it = item("a", false, Priority::P0);
        assert!(!matches("has:desc", &it, None));
        it.description = "note".to_string();
        assert!(matches("has:desc", &it, None));
    }

    #[test]
    fn adjacency_implies_and() {
        let it = item("onehouse", false, Priority::P0);
        assert!(matches("pri:P0 !done onehouse", &it, None));
        assert!(!matches("pri:P0 done", &it, None));
    }

    #[test]
    fn explicit_and_or_and_not() {
        let it = item("a", false, Priority::P0);
        assert!(matches("pri:P0 AND !done", &it, None));
        assert!(matches("pri:P3 OR pri:P0", &it, None));
        assert!(matches("NOT pri:P3", &it, None));
        assert!(!matches("NOT pri:P0", &it, None));
    }

    #[test]
    fn parentheses_group_alternatives() {
        let p0_done = item("a", true, Priority::P0);
        assert!(matches("(pri:P0 OR pri:P1) AND done", &p0_done, None));
        assert!(!matches("(pri:P2 OR pri:P3) AND done", &p0_done, None));
    }

    #[test]
    fn quoted_values_may_contain_spaces() {
        let it = item("call the CPA back", false, Priority::P0);
        assert!(matches(r#"text:"the CPA""#, &it, None));
    }

    #[test]
    fn the_daily_driver_query_parses_and_matches() {
        // This is the query that retires `mcli todos act -p P0 -a lefv`.
        let it = item("file 83(b)", false, Priority::P0);
        assert!(matches("pri:P0 acct:lefv !done", &it, Some("lefv")));
        assert!(!matches("pri:P0 acct:lefv !done", &it, Some("jzlaw")));
    }

    #[test]
    fn unknown_fields_are_rejected() {
        assert_eq!(
            Query::parse("bogus:value"),
            Err(QueryError::UnknownField("bogus".to_string()))
        );
    }

    #[test]
    fn bad_priorities_are_rejected() {
        assert_eq!(
            Query::parse("pri:P9"),
            Err(QueryError::BadPriority("P9".to_string()))
        );
    }

    #[test]
    fn unbalanced_syntax_is_rejected() {
        assert_eq!(
            Query::parse(r#"text:"unclosed"#),
            Err(QueryError::UnclosedQuote)
        );
        assert_eq!(Query::parse("(pri:P0"), Err(QueryError::UnclosedParen));
        assert_eq!(Query::parse("NOT"), Err(QueryError::UnexpectedEnd));
        assert!(Query::parse(")").is_err());
    }
}
