# mitodo

A terminal todo tracker over plain markdown checklists.

`mitodo` reads a directory of `TODO.md` files, shows them in a three-pane TUI
with vim keybindings and a query language, and writes changes back to the source
markdown **without reformatting it**. Your files stay yours: no database, no
lock-in, no proprietary format. Edit them in your editor, from a script, or with
`mitodo` — all at the same time.

The name is Spanish — *mi todo*, "my everything" — and happens to contain the
word "todo".

```
  mitodo   all · 41 open / 96 shown  /pri:P0 !done
┌groups──────────────┐┌items 3/41─────────────────────────────────────────────┐
│  all         41    ││  [x] P0  File the 83(b) election                      │
│▸ work        12    ││    [ ] P0  pull the signature page                    │
│  home         9    ││▸ [ ] P0  Respond to opposing counsel re: discovery    │
│  side         3    │└───────────────────────────────────────────────────────┘
│                    │┌detail─────────────────────────────────────────────────┐
│                    ││Respond to opposing counsel re: discovery              │
│                    ││P0 · P0 — Critical · Discovery                         │
│                    ││                                                       │
│                    ││due Friday. Need the exhibit list from Sam first.      │
└────────────────────┘└───────────────────────────────────────────────────────┘
 space toggle · a add · e edit · d del · / query · s sync · ? keys · q quit
```

## Why

Markdown checklists are the most portable todo format there is, and every editor
and agent can already read them. What they lack is a fast way to *work* them:
filtering, jumping between projects, ticking things off without hunting for the
line. `mitodo` is that layer, and nothing more.

## Install

```sh
cargo install --path .
```

Requires Rust 1.85+ (edition 2024).

## Getting started

Point it at a directory of todo files. It works out the layout and writes a
config you can read and edit:

```sh
$ mitodo init ~/notes/todo
scanning /home/you/notes/todo...
  ✓ 4 group directories, pattern */TODO.md
  ✓ priorities from "## " headings (12 matched P0–P3)
  ✓ _archive/ directories
  ✓ git repository, sync enabled
wrote /home/you/.config/mitodo/config.toml

$ mitodo          # open the TUI
$ mitodo list     # or just print everything
```

Both accept filters, so the TUI can open straight into a view and `list` is
scriptable:

```sh
$ mitodo -q 'pri:P0 !done'            # open the TUI filtered
$ mitodo -p P0 -a work list           # -p/-a are shorthand, ANDed with -q
$ mitodo -q 'sort:pri,text' list      # ordering works too
```

Detection understands two shapes out of the box:

| layout | what you get |
|---|---|
| `<group>/TODO.md` per subdirectory | one group per directory |
| a single `TODO.md` at the root | one group per `## ` heading |

Priorities are read from `## ` headings matching `P0`–`P3` if you use them, and
disabled if you don't. If you tag items inline instead — todo.txt style — point
the config at that:

```toml
[priority]
source  = "tag"
pattern = "\\(([A-D])\\)"     # (A) urgent … (D) whenever
```

The captured marker maps to a band: `0`–`3` and `A`–`D` both work, so the same
setting covers either convention.

A group can also carry a `notes.md` beside its `TODO.md`; `N` reads it.

## Keys

| | | | |
|---|---|---|---|
| `j` `k` | down / up | `space` `x` | toggle done |
| `g` `G` | first / last | `a` `A` | add sibling / child |
| `tab` | focus items | `e` | edit text |
| `shift-tab` | focus groups | `i` | edit description |
| `/` | edit query | `d` | delete (asks first) |
| `esc` | clear query | `h` | hide done |
| `s` | git sync | `c` | scrolling ticker |
| `N` | read group notes | `p` | pause the ticker |
| `?` | help | `q` | quit |

## Queries

```
pri:P0 acct:work !done          urgent, mine, not finished
sec:"High Priority" has:desc    by section, with notes attached
(pri:P0 OR pri:P1) AND !done    parentheses and explicit operators
onehouse                        bare words match text and descriptions
sort:pri,text                   order by priority, then alphabetically
```

| field | matches |
|---|---|
| `acct:` `account:` `group:` | the group name |
| `pri:` `priority:` | `P0`–`P3`, optionally `<=` `>=` `<` `>` |
| `done` / `!done` | completion |
| `sec:` `section:` | the `## ` heading, substring |
| `has:desc` | items with a description block |
| `text:` | text and description, substring |
| `sort:` | ordering: `pri` `text` `group` `section` `done`, comma-separated |

Adjacent terms are ANDed. `NOT` and a leading `!` both negate. `sort:` is not a
filter — it orders whatever survives the rest of the query, applying its keys in
turn, and ties keep the order they had in the file.

## The file format

Anything markdown already does:

```markdown
## P0 — Critical

### Discovery

- [ ] Respond to opposing counsel
  > due Friday. Need the exhibit list from Sam first.
  - [ ] pull the exhibit list
  - [x] confirm the deadline
```

Checkbox lines are items, indented ones are children, and indented `>` lines
beneath an item are its description. Everything else in the file — frontmatter,
prose, headings, horizontal rules — is left exactly as it was.

## Editing safely alongside other tools

`mitodo` assumes it is not the only writer. Before changing a line it re-reads
the file and checks the line still holds what it parsed; if another program got
there first the write is refused, the workspace reloads, and you see what
happened. Writes go to a temporary file and are renamed into place, so a crash
cannot leave a half-written todo list.

It also watches the directory, so edits made in your editor or by a script show
up without a restart.

The guarantee this rests on: **after any change, every line you did not edit is
byte-identical to what it was** — including line endings and whether the file
ends in a newline. That is the property the test suite spends most of its effort
on.

## Optional: an agent

`mitodo` can call an external command to help. It is off unless you configure
one, and it is not tied to any provider — anything that takes a prompt and
prints JSON works.

```toml
[agent]
command     = ["claude", "--print"]
schema_flag = "--json-schema"

[agent.prompts]
scan = "~/.config/mitodo/prompts/scan.md"   # your own prompt, kept local
```

| key | verb | writes? |
|---|---|---|
| `n` | describe a filter in words; it builds the query and shows you what it built | no |
| `S` | summarise what's on screen | no |
| `b` | break the selected item into sub-items | after review |
| `R` | scan for changes across the workspace | after review |

Anything that writes shows you a diff first and applies through the same
conflict-aware writer, so a stale proposal is refused rather than forced.

## Configuration

```toml
[workspace]
root      = "~/notes/todo"
group_by  = "directory"        # or "heading"
todo_glob = "*/TODO.md"

[priority]
source  = "heading"            # "heading" | "tag" | "none"
pattern = "^P([0-3])"

[git]
enabled = true
sync    = [["add", "-A"], ["commit", "-m", "mitodo: sync"], ["pull", "--rebase"], ["push"]]
```

`s` runs the `git.sync` command list in the workspace and shows you the output.
The list is yours to change; set `enabled = false` to remove the key.

## Credits

mitodo is a fork of [eilmeldung](https://github.com/christo-auer/eilmeldung) by
christo-auer — a TUI RSS reader whose terminal foundations, theme system and
overall shape this project is built on. The RSS layer was replaced with a
markdown todo store; the debt for everything underneath it is real. See
[NOTICE](NOTICE).

## Licence

GPL-3.0-or-later, inherited from eilmeldung. See [LICENSE](LICENSE).
