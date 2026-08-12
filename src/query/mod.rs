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
//! due:2026-08-01     on that day; also <=, >=, <, >
//! due:today  due:7d   relative: today, tomorrow, Nd days from now
//! due:none            has no deadline
//! overdue             due before today and not finished
//! sort:pri,text      ordering; pri, text, group, section, done, due
//! AND OR NOT ( )     combinators; adjacency implies AND
//! ```

use chrono::{Duration, Local, NaiveDate};

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
    #[error("{0:?} is not a date (expected YYYY-MM-DD, today, tomorrow or Nd)")]
    BadDate(String),
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
    /// Comparison against a deadline, resolved relative to the current day.
    Due(Cmp, DueTarget),
    /// True when the item has no deadline.
    NoDue,
    /// Unfinished and past its deadline.
    Overdue,
    Priority(Cmp, Priority),
    Done(bool),
    Section(String),
    HasDescription,
    Text(String),
}

/// A deadline to compare against. Relative forms are resolved at match time,
/// so a session left open overnight still means "today" tomorrow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DueTarget {
    On(NaiveDate),
    DaysFromToday(i64),
}

impl DueTarget {
    fn resolve(self, today: NaiveDate) -> NaiveDate {
        match self {
            DueTarget::On(date) => date,
            DueTarget::DaysFromToday(days) => today + Duration::days(days),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Clause {
    And(Box<Clause>, Box<Clause>),
    Or(Box<Clause>, Box<Clause>),
    Not(Box<Clause>),
    Atom(Atom),
}

/// A field the result list can be ordered by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortKey {
    Priority,
    Text,
    Group,
    Section,
    Done,
    Due,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Query {
    clause: Option<Clause>,
    sort: Vec<SortKey>,
}

impl Query {
    pub fn parse(source: &str) -> Result<Option<Query>, QueryError> {
        let all = tokenize(source)?;
        // `sort:` is an instruction about the result list rather than a
        // predicate, so it is lifted out before the clause is parsed.
        let mut sort = Vec::new();
        let mut tokens = Vec::new();
        for token in all {
            match token
                .split_once(':')
                .filter(|(field, _)| field.eq_ignore_ascii_case("sort"))
            {
                Some((_, spec)) => sort.extend(parse_sort(&unquote(spec))?),
                None => tokens.push(token),
            }
        }

        if tokens.is_empty() {
            return match sort.is_empty() {
                true => Ok(None),
                false => Ok(Some(Query { clause: None, sort })),
            };
        }

        let mut parser = Parser { tokens, pos: 0 };
        let clause = parser.parse_or()?;
        if parser.pos < parser.tokens.len() {
            return Err(QueryError::Unexpected(parser.tokens[parser.pos].clone()));
        }
        Ok(Some(Query {
            clause: Some(clause),
            sort,
        }))
    }

    /// `group` is the name of the group the item belongs to, needed by `acct:`.
    ///
    /// Relative deadlines resolve against the current day on every call, so a
    /// long-running session does not go stale at midnight.
    pub fn matches(&self, item: &Item, group: Option<&str>) -> bool {
        self.matches_on(item, group, Local::now().date_naive())
    }

    /// Match against an explicit "today", for deterministic tests.
    pub fn matches_on(&self, item: &Item, group: Option<&str>, today: NaiveDate) -> bool {
        match &self.clause {
            Some(clause) => eval(clause, item, group, today),
            None => true,
        }
    }

    /// Order items in place. A stable sort, so items compare equal on every
    /// key keep the order they had in the file.
    pub fn sort_items<'a>(&self, items: &mut [(&'a Item, Option<&'a str>)]) {
        if self.sort.is_empty() {
            return;
        }
        items.sort_by(|(a, a_group), (b, b_group)| {
            for key in &self.sort {
                let ordering = match key {
                    SortKey::Priority => a.priority.cmp(&b.priority),
                    SortKey::Text => a.text.to_lowercase().cmp(&b.text.to_lowercase()),
                    SortKey::Group => a_group.unwrap_or("").cmp(b_group.unwrap_or("")),
                    SortKey::Section => a.section.cmp(&b.section),
                    SortKey::Done => a.done.cmp(&b.done),
                    // Items with no deadline sort last rather than first.
                    SortKey::Due => match (a.due, b.due) {
                        (Some(x), Some(y)) => x.cmp(&y),
                        (Some(_), None) => std::cmp::Ordering::Less,
                        (None, Some(_)) => std::cmp::Ordering::Greater,
                        (None, None) => std::cmp::Ordering::Equal,
                    },
                };
                if ordering != std::cmp::Ordering::Equal {
                    return ordering;
                }
            }
            std::cmp::Ordering::Equal
        });
    }
}

fn parse_sort(spec: &str) -> Result<Vec<SortKey>, QueryError> {
    spec.split(',')
        .map(|part| part.trim())
        .filter(|part| !part.is_empty())
        .map(|part| match part.to_lowercase().as_str() {
            "pri" | "priority" => Ok(SortKey::Priority),
            "text" => Ok(SortKey::Text),
            "acct" | "account" | "group" => Ok(SortKey::Group),
            "sec" | "section" => Ok(SortKey::Section),
            "done" => Ok(SortKey::Done),
            "due" => Ok(SortKey::Due),
            other => Err(QueryError::UnknownField(format!("sort:{other}"))),
        })
        .collect()
}

fn eval(clause: &Clause, item: &Item, group: Option<&str>, today: NaiveDate) -> bool {
    match clause {
        Clause::And(a, b) => eval(a, item, group, today) && eval(b, item, group, today),
        Clause::Or(a, b) => eval(a, item, group, today) || eval(b, item, group, today),
        Clause::Not(inner) => !eval(inner, item, group, today),
        Clause::Atom(atom) => eval_atom(atom, item, group, today),
    }
}

fn eval_atom(atom: &Atom, item: &Item, group: Option<&str>, today: NaiveDate) -> bool {
    match atom {
        Atom::Group(name) => group.is_some_and(|g| g.eq_ignore_ascii_case(name)),
        Atom::NoDue => item.due.is_none(),
        Atom::Overdue => item.due.is_some_and(|d| d < today) && !item.done,
        Atom::Due(cmp, target) => match item.due {
            None => false,
            Some(due) => {
                let against = target.resolve(today);
                match cmp {
                    Cmp::Eq => due == against,
                    Cmp::Le => due <= against,
                    Cmp::Ge => due >= against,
                    Cmp::Lt => due < against,
                    Cmp::Gt => due > against,
                }
            }
        },
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

/// Parse the value side of `due:`, e.g. `2026-08-01`, `today`, `7d`, `none`.
fn parse_due(raw: &str) -> Result<Atom, QueryError> {
    let (cmp, rest) = if let Some(r) = raw.strip_prefix("<=") {
        (Cmp::Le, r)
    } else if let Some(r) = raw.strip_prefix(">=") {
        (Cmp::Ge, r)
    } else if let Some(r) = raw.strip_prefix('<') {
        (Cmp::Lt, r)
    } else if let Some(r) = raw.strip_prefix('>') {
        (Cmp::Gt, r)
    } else {
        (Cmp::Eq, raw)
    };

    let rest = rest.trim();
    if rest.eq_ignore_ascii_case("none") {
        return Ok(Atom::NoDue);
    }
    let target = match rest.to_lowercase().as_str() {
        "today" => DueTarget::DaysFromToday(0),
        "tomorrow" => DueTarget::DaysFromToday(1),
        "yesterday" => DueTarget::DaysFromToday(-1),
        other => match other.strip_suffix('d').and_then(|n| n.parse::<i64>().ok()) {
            Some(days) => DueTarget::DaysFromToday(days),
            None => NaiveDate::parse_from_str(other, "%Y-%m-%d")
                .map(DueTarget::On)
                .map_err(|_| QueryError::BadDate(rest.to_string()))?,
        },
    };
    Ok(Atom::Due(cmp, target))
}

fn parse_atom(token: &str) -> Result<Atom, QueryError> {
    if token.eq_ignore_ascii_case("done") {
        return Ok(Atom::Done(true));
    }
    if token.eq_ignore_ascii_case("overdue") {
        return Ok(Atom::Overdue);
    }

    let Some((field, value)) = token.split_once(':') else {
        return Ok(Atom::Text(unquote(token)));
    };

    match field.to_lowercase().as_str() {
        "acct" | "account" | "group" => Ok(Atom::Group(unquote(value))),
        "sec" | "section" => Ok(Atom::Section(unquote(value))),
        "text" => Ok(Atom::Text(unquote(value))),
        "due" => parse_due(&unquote(value)),
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
            id: ItemId::compute("f", "s", "h", 0, text, 0),
            file: PathBuf::from("f"),
            line: 0,
            indent: 0,
            done,
            text: text.to_string(),
            raw: format!("- [{}] {}", if done { "x" } else { " " }, text),
            description: String::new(),
            section: "P1 — High Priority".to_string(),
            heading: "H".to_string(),
            priority,
            due: None,
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

    fn sorted(query: &str, items: &[Item]) -> Vec<String> {
        let parsed = Query::parse(query).unwrap().unwrap();
        let mut pairs: Vec<(&Item, Option<&str>)> = items.iter().map(|i| (i, Some("g"))).collect();
        parsed.sort_items(&mut pairs);
        pairs.into_iter().map(|(i, _)| i.text.clone()).collect()
    }

    #[test]
    fn sort_orders_by_priority() {
        let items = [
            item("c", false, Priority::P2),
            item("a", false, Priority::P0),
            item("b", false, Priority::P1),
        ];
        assert_eq!(sorted("sort:pri", &items), vec!["a", "b", "c"]);
    }

    #[test]
    fn sort_orders_by_text() {
        let items = [
            item("charlie", false, Priority::P0),
            item("alpha", false, Priority::P0),
        ];
        assert_eq!(sorted("sort:text", &items), vec!["alpha", "charlie"]);
    }

    #[test]
    fn sort_keys_apply_in_order() {
        let items = [
            item("zeta", false, Priority::P1),
            item("alpha", false, Priority::P1),
            item("beta", false, Priority::P0),
        ];
        assert_eq!(
            sorted("sort:pri,text", &items),
            vec!["beta", "alpha", "zeta"],
            "priority first, then text within a band"
        );
    }

    #[test]
    fn sort_puts_open_items_before_done_ones() {
        let items = [
            item("done", true, Priority::P0),
            item("open", false, Priority::P0),
        ];
        assert_eq!(sorted("sort:done", &items), vec!["open", "done"]);
    }

    #[test]
    fn sort_composes_with_a_filter() {
        let parsed = Query::parse("!done sort:pri").unwrap().unwrap();
        let done = item("x", true, Priority::P0);
        let open = item("y", false, Priority::P1);
        assert!(!parsed.matches(&done, None), "filter still applies");
        assert!(parsed.matches(&open, None));

        let items = [
            item("b", false, Priority::P1),
            item("a", false, Priority::P0),
        ];
        assert_eq!(
            sorted("!done sort:pri", &items),
            vec!["a", "b"],
            "and the sort still applies"
        );
    }

    #[test]
    fn a_query_that_is_only_a_sort_matches_everything() {
        let parsed = Query::parse("sort:pri").unwrap().unwrap();
        assert!(parsed.matches(&item("anything", true, Priority::None), None));
    }

    #[test]
    fn sorting_is_stable_within_equal_keys() {
        let items = [
            item("first", false, Priority::P0),
            item("second", false, Priority::P0),
        ];
        assert_eq!(
            sorted("sort:pri", &items),
            vec!["first", "second"],
            "file order preserved when the key ties"
        );
    }

    #[test]
    fn an_unknown_sort_key_is_rejected() {
        assert_eq!(
            Query::parse("sort:bogus"),
            Err(QueryError::UnknownField("sort:bogus".to_string()))
        );
    }

    // --- deadlines ---

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 7, 28).unwrap()
    }

    fn due_item(text: &str, done: bool, due: Option<&str>) -> Item {
        let mut it = item(text, done, Priority::None);
        it.due = due.map(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").unwrap());
        it
    }

    fn matches_today(query: &str, item: &Item) -> bool {
        Query::parse(query)
            .expect("query parses")
            .expect("not empty")
            .matches_on(item, None, today())
    }

    #[test]
    fn due_matches_an_exact_date() {
        let it = due_item("a", false, Some("2026-08-01"));
        assert!(matches_today("due:2026-08-01", &it));
        assert!(!matches_today("due:2026-08-02", &it));
    }

    #[test]
    fn due_today_and_tomorrow_resolve_relative_to_now() {
        let today_item = due_item("a", false, Some("2026-07-28"));
        let tomorrow_item = due_item("b", false, Some("2026-07-29"));
        assert!(matches_today("due:today", &today_item));
        assert!(!matches_today("due:today", &tomorrow_item));
        assert!(matches_today("due:tomorrow", &tomorrow_item));
    }

    #[test]
    fn a_day_offset_resolves_relative_to_now() {
        let it = due_item("a", false, Some("2026-08-04"));
        assert!(matches_today("due:7d", &it), "7 days after 2026-07-28");
        assert!(matches_today("due:<=7d", &it));
        assert!(!matches_today("due:<=3d", &it));
    }

    #[test]
    fn due_comparisons_work() {
        let it = due_item("a", false, Some("2026-08-01"));
        assert!(matches_today("due:<2026-08-02", &it));
        assert!(matches_today("due:>2026-07-31", &it));
        assert!(matches_today("due:>=2026-08-01", &it));
        assert!(!matches_today("due:<2026-08-01", &it));
    }

    #[test]
    fn an_item_without_a_deadline_never_matches_a_date_comparison() {
        let it = due_item("a", false, None);
        assert!(!matches_today("due:<=7d", &it));
        assert!(!matches_today("due:today", &it));
    }

    #[test]
    fn due_none_finds_items_without_a_deadline() {
        assert!(matches_today("due:none", &due_item("a", false, None)));
        assert!(!matches_today(
            "due:none",
            &due_item("a", false, Some("2026-08-01"))
        ));
    }

    #[test]
    fn overdue_is_past_and_unfinished() {
        let late = due_item("a", false, Some("2026-07-27"));
        let late_but_done = due_item("b", true, Some("2026-07-27"));
        let due_today = due_item("c", false, Some("2026-07-28"));

        assert!(matches_today("overdue", &late));
        assert!(
            !matches_today("overdue", &late_but_done),
            "finished work is not overdue"
        );
        assert!(
            !matches_today("overdue", &due_today),
            "due today is not yet overdue"
        );
    }

    #[test]
    fn deadlines_compose_with_other_terms() {
        let it = due_item("brief", false, Some("2026-07-29"));
        assert!(matches_today("!done due:<=7d brief", &it));
        assert!(!matches_today("done due:<=7d", &it));
    }

    #[test]
    fn sort_by_due_puts_the_soonest_first_and_undated_last() {
        let items = [
            due_item("later", false, Some("2026-09-01")),
            due_item("undated", false, None),
            due_item("sooner", false, Some("2026-08-01")),
        ];
        assert_eq!(
            sorted("sort:due", &items),
            vec!["sooner", "later", "undated"]
        );
    }

    #[test]
    fn a_malformed_date_is_rejected() {
        assert_eq!(
            Query::parse("due:not-a-date"),
            Err(QueryError::BadDate("not-a-date".to_string()))
        );
        assert!(Query::parse("due:2026-13-45").is_err(), "month 13");
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
