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

## Deadlines

Write a date into the item itself and mitodo picks it up:

```markdown
- [ ] File the 83(b) election due:2026-08-01
```

The list shows how long you have — `today`, `tmrw`, `5d`, `3d ago` — with missed
deadlines flagged, and the ticker leads with overdue work. The detail pane spells
the date out in full.

```
▸ [ ] P0  3d ago  File 83(b) election due:2026-07-25
  [ ] P0  today   Respond to opposing counsel due:2026-07-28
  [ ] P0  5d      Draft the LLC agreement due:2026-08-02
  [ ] P0  Someday: reorganise the shelf
```

The marker stays in the text, because the markdown file is the source of truth —
editing an item keeps its deadline with it. If you already write dates a
different way, point the config at your convention:

```toml
[due]
enabled = true
pattern = '\(due (\d{4}-\d{2}-\d{2})\)'   # matches "(due 2026-08-01)"
```

Capture group 1 must yield an ISO date. Set `enabled = false` to ignore dates
entirely.

## Keys

Arrows work the list and the tree; `hjkl` moves between panes.

| | | | |
|---|---|---|---|
| `↑` `↓` | move / scroll | `space` `x` | toggle done |
| `→` | open a node, then step into it | `a` | new item (dialog) |
| | | `o` `A` | quick add sibling / child |
| `←` | close a node, then step out to its parent | `e` | edit text |
| `g` `G` | first / last | `i` | edit notes in the pane |
| `h` `j` `k` `l` | move between panes | `d` | delete (asks first) |
| `z` `Z` | fold / fold all | `H` | hide done |
| `/` `esc` | edit / clear query | `s` | git sync |
| `v` | view settings | `c` | scrolling ticker |
| `N` | read group notes | `X` | archive finished |
| `?` | help | `q` | quit |

The panes sit groups-left, items above detail on the right, and `hjkl` follows
that layout: `h` from either right-hand pane goes to the groups list, `l` goes
back, and `j`/`k` move between the item list and the detail pane. Focus the
detail pane and the arrows scroll it.

Mouse works too: click to select a row, wheel to scroll the pane under the
pointer, and drag any divider to resize the panes — the one beside the groups
list, and the one between the item list and the detail pane. Clicking the
`▾`/`▸` marker of the selected node folds it, and clicking the detail pane
edits that item's notes. Set `ui.mouse = false` to hand selection and
scrollback back to your terminal.

The wheel scrolls the *view* and leaves your selection where it is, the way
scrolling works everywhere else.

## Adding an item

`a` opens a dialog: title, priority, notes and sub-items in one place.

```
┌new item — tab moves between fields ─────────────────────┐
│▸ title                                                  │
│    Draft the LLC agreement                              │
│                                                         │
│  priority                                               │
│    ○ P0  ◉ P1  ○ P2  ○ P3  ○ none                       │
│                                                         │
│  notes                                                  │
│    needs Dylan sign-off                                 │
│                                                         │
│  sub-items (one per line)                               │
│    pull the template                                    │
│    send for review                                      │
│                                                         │
│  Add      Cancel      ctrl-s adds · esc cancels         │
└─────────────────────────────────────────────────────────┘
```

`tab` moves between fields, arrows or `0`–`3` pick the priority, `ctrl-s` adds
and `esc` cancels. `enter` submits from the title, and makes a new line in the
notes and sub-item boxes.

The whole dialog takes the mouse: click a field to focus it and put the caret
where you clicked, click a priority to pick it, and click **Add** or
**Cancel**.

The priority decides where the item goes: choosing P1 files it under that
group's `## P1` heading, after whatever is already there. Notes become the
item's blockquote and sub-items become its children, all in one write.

`o` still adds a plain sibling below the selection without the dialog, and `A`
adds a child.

## Queries

```
pri:P0 acct:work !done          urgent, mine, not finished
sec:"High Priority" has:desc    by section, with notes attached
(pri:P0 OR pri:P1) AND !done    parentheses and explicit operators
onehouse                        bare words match text and descriptions
overdue                         past its deadline and not finished
due:<=7d !done sort:due         this week's work, soonest first
```

| field | matches |
|---|---|
| `acct:` `account:` `group:` | the group name |
| `pri:` `priority:` | `P0`–`P3`, optionally `<=` `>=` `<` `>` |
| `done` / `!done` | completion |
| `sec:` `section:` | the `## ` heading, substring |
| `has:desc` | items with a description block |
| `text:` | text and description, substring |
| `due:` | a deadline: `2026-08-01`, `today`, `tomorrow`, `7d`, `none`, with `<=` `>=` `<` `>` |
| `overdue` | past its deadline and still open |
| `sort:` | ordering: `pri` `text` `group` `section` `done` `due`, comma-separated |

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

Nesting goes as deep as you like, and the list behaves like a file tree:

```
▾ [ ] P0  parent one
    ▾ [ ] P0  child a
        [ ] P0  grandchild
      [ ] P0  child b
    [ ] P0  parent two
```

`→` opens a node and `←` closes it, exactly as a file tree behaves: pressing
`→` again on an open node steps into its first child, and `←` on a leaf steps
out to its parent. `z` toggles the selected node, `Z` does every node at once.
Folding hides the whole subtree, however deep. `A` adds a child to the
selected item.

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

Prompt templates get two placeholders: `{items}` is the view as rendered, and
`{files}` is every todo file with its workspace-relative path and contents.
`scan` needs `{files}` — a proposed change names the file it belongs to, so an
agent that never saw the paths cannot produce a usable one. `{input}` is
whatever you typed.

Anything that writes shows you the proposals first, as a list you pick from:

```
┌review — space picks one · a toggles all ─────────────────────────────────┐
│found 4 actionable items across your inboxes — 3 of 4 selected            │
│▸ [x] + add w/TODO.md · Reply to opposing counsel about the discovery…    │
│  [ ] + add w/TODO.md · File the 83(b) election                           │
│  [x] ✓ done w/TODO.md · alpha                                            │
│                                                                          │
│+ add w/TODO.md · Reply to opposing counsel about the discovery deadline  │
│extension request they sent Tuesday                                       │
│why: email from David Ashcraft, 2 days old, asks for a response this week │
│                                                                          │
│  Apply 3      Cancel      enter applies · esc cancels                    │
└──────────────────────────────────────────────────────────────────────────┘
```

Everything starts picked. `j`/`k` moves, `space` picks one, `a` toggles all,
and clicking a row toggles it. **Apply** and **Cancel** are buttons you can
click — Apply counts what is selected, so you can see what you are about to
commit to — and `enter` and `esc` do the same. The highlighted change is shown
in full below the list, wrapped, with the agent's reason for proposing it.

Applying goes through the same conflict-aware writer, so a stale proposal is
refused rather than forced.

While an agent is running the status bar says so, with a spinner and a clock:

```
 ⣻ running scan for 2m 14s · still working · the rest of the app is usable
```

A scan over a real inbox takes minutes, so it says how long it has been going
and keeps moving to show it has not wedged. The rest of the app stays usable
throughout — browse, filter and tick things off while it runs.

If an agent never answers it is killed and reported rather than left waiting:
set `agent.timeout_secs` (default 600).

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

`hide_done` and the ticker are remembered between runs in a `[ui]` section,
written on exit only when they actually changed.

## Archiving

`X` moves finished items out of the working file into `<archive_dir>/TODO.md`
under a dated heading. It is a move, not a delete — the lines are appended
verbatim before being removed, so nothing is lost and anything can be pasted
back.

An item whose sub-items are not all finished is left alone and reported, since
archiving it would hide open work. Descriptions and fully-finished subtrees
travel with their item.

## Credits

mitodo is a fork of [eilmeldung](https://github.com/christo-auer/eilmeldung) by
christo-auer — a TUI RSS reader whose terminal foundations, theme system and
overall shape this project is built on. The RSS layer was replaced with a
markdown todo store; the debt for everything underneath it is real. See
[NOTICE](NOTICE).

## Licence

GPL-3.0-or-later, inherited from eilmeldung. See [LICENSE](LICENSE).
