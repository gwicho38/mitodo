# `mitodo self mcp` — Design

**Date:** 2026-08-17
**Status:** Approved for planning
**Builds on:** [2026-08-12-mcp-server-design.md](2026-08-12-mcp-server-design.md)

---

## 1. What this is

Three subcommands that register mitodo's MCP server with the clients on the
machine, so nobody hand-writes `--mcp-config` JSON or edits a client's config
file:

```sh
mitodo self mcp setup     # register with every supported client found
mitodo self mcp status    # where mitodo is registered, and whether it still resolves
mitodo self mcp remove    # unregister
```

`setup` is idempotent and doubles as the repair: run it after any install or
rebuild and it either reports "already registered" or re-points a stale path.

---

## 2. What the clients actually offer

Surveyed on this machine, not assumed:

| Client | Registration path | Verdict |
|---|---|---|
| `claude` | `mcp add/get/remove`, `--scope local\|user\|project` | **delegate** |
| `codex` | `mcp add/get/remove` | **delegate** |
| Claude Desktop | `~/Library/Application Support/Claude/claude_desktop_config.json` — plain JSON, `mcpServers` map, no CLI | **merge one key** |
| `opencode` | `mcp add` takes **no arguments** (interactive), and there is no `remove` or `get` | unsupported |
| Zed | `~/.config/zed/settings.json` — **JSONC, with comments** | unsupported |

Two exclusions, each for a concrete reason rather than effort:

- **`opencode`** cannot be driven non-interactively — `opencode mcp add --help`
  lists only `--help`, `--version`, `--print-logs`, `--log-level`, `--pure`. With
  no arguments there is nothing to script, and with no `remove` there is nothing
  to undo.
- **Zed** stores settings as JSONC. A strict read-modify-write deletes the user's
  comments, which is precisely the damage `store::write` exists to prevent in
  markdown. Comment-preserving editing is real work for one client.

`status` reports both as *detected, not supported*, with the reason. Silence
would read as "not installed".

---

## 3. What gets registered

The same triple everywhere:

```
name:     mitodo
command:  <absolute path to the running binary>     std::env::current_exe()
args:     ["mcp-server"]
```

**Absolute, never bare `mitodo`.** The existing entries on this machine prove why:

| Where | How existing servers name their command |
|---|---|
| Claude **Desktop** | `/Users/lefv/.local/share/uv/tools/repowise/bin/repowise` — absolute |
| `claude` **CLI** (`~/.claude.json`) | `uv`, `write-like-me-mcp` — bare |

The desktop app is launched by the GUI and does not inherit a shell `PATH`, so a
bare name fails there. `~/.cargo/bin` is not on the default `PATH` either. One
absolute path works in both places, so both get one.

**No config directory is pinned.** `mcp-server` resolves
`$XDG_CONFIG_HOME/mitodo/config.toml`, else `~/.config/mitodo/config.toml`, at run
time — the same file the TUI reads. One registration therefore follows the config
wherever it points, which matters when the same config is kept byte-identical
across machines. A second registration pinned to a
different workspace would need `--config-dir` and `--name`; §8 records those as
unbuilt, since one registration that follows the config covers both machines.

**`--scope user` for `claude`.** Its default is `local`, which registers only for
the current directory; a todo server is not directory-scoped.

---

## 4. Architecture

```
  mitodo self mcp {setup,status,remove}
  ┌──────────────────────────────────────────────────────────────────────┐
  │  src/selfcfg/mod.rs      the three verbs, and one reporter           │
  │  src/selfcfg/target.rs   what a registerable client is (pure data)   │
  │  src/selfcfg/desktop.rs  the single JSON merge, quarantined          │
  └──────────────────────────────────────────────────────────────────────┘
                │                                    │
      ┌─────────┴──────────┐              ┌──────────┴───────────┐
      │  Delegated         │              │  File-merged         │
      │  claude · codex    │              │  Claude Desktop      │
      │  run their own     │              │  read → merge one    │
      │  mcp add/get/      │              │  key → temp file →   │
      │  remove            │              │  atomic rename       │
      └────────────────────┘              └──────────────────────┘
```

**The boundary that matters: mitodo never parses a config a client CLI can write
for it.** Where a CLI exists we shell out, so the client owns its own format and a
future format change costs us nothing. Only Claude Desktop, which has no CLI, gets
a file merge — and that merge touches exactly one key, preserving every other byte
through read → modify → temp file → atomic rename. Same discipline `store::write`
already applies to markdown, for the same reason: it is someone else's file.

Units:

- **`target.rs`** — `Target { name, kind }` where `kind` is
  `Delegated { cli }` or `DesktopJson { path }`; plus `detect() -> Vec<Target>`
  and `unsupported() -> Vec<(&str, &str)>` naming what was seen and why it is
  skipped. Pure apart from existence checks.
- **`mod.rs`** — `setup`, `status`, `remove`, each returning a
  `Vec<Outcome>` that one reporter renders, so the three verbs cannot drift in
  wording.
- **`desktop.rs`** — `read_servers`, `merge_server`, `remove_server`, over a
  path passed in rather than resolved internally, so tests never touch the real
  file.

---

## 5. Behaviour

```
 setup, per target
 ─────────────────────────────────────────────────────────────────────────
   not registered            → register              "registered with claude"
   registered, same path     → nothing               "claude: already current"
   registered, other path    → re-register           "claude: re-pointed from
                                                      /old/path"
   client absent             → skip                  (not reported as failure)
   registration failed       → report, keep going    "claude: failed — <stderr>"

 status, per target
 ─────────────────────────────────────────────────────────────────────────
   registered, path exists   → "claude    ✓ /abs/path mcp-server"
   registered, path missing  → "claude    ! /old/path (no such file) — run setup"
   not registered            → "claude    – not registered"
   detected, unsupported     → "opencode  – unsupported: mcp add is interactive"

 remove, per target
 ─────────────────────────────────────────────────────────────────────────
   registered                → unregister            "removed from claude"
   not registered            → nothing               "claude: nothing to remove"
```

Rules that follow:

- **One failing target never aborts the others.** A missing `codex` must not stop
  the `claude` registration. The exit code is non-zero only if a target that was
  *found* failed to register.
- **`--dry-run` prints the plan and writes nothing**, including no subprocess
  call. Worth having because `setup` mutates files outside mitodo's own tree.
- **Stale-path repair is the reason `setup` is re-runnable**, and the reason it
  compares paths rather than just presence.
- **Exit codes:** `0` all good, `1` at least one found target failed, `2` no
  supported client found at all — that last one is worth distinguishing, since
  "nothing to do" and "everything worked" should not look alike.

---

## 6. Errors

| Condition | Handling |
|---|---|
| `current_exe()` fails | abort before touching anything: without a path there is nothing correct to register |
| a client CLI exits non-zero | report its stderr's first line against that target, continue with the rest |
| Claude Desktop's JSON is unparseable | refuse to write, report the parse error and the path; never overwrite a file we could not read |
| its `mcpServers` key is missing | create it; that is a valid empty config |
| its `mcpServers` is not an object | refuse, same as unparseable |
| the config directory does not exist | skip the target — the app is not installed |
| a temp-file rename fails | report; the original is untouched because the rename is the only mutation |

---

## 7. Testing

No client CLI and no real config file in the suite.

```
 target       detect() finds a Delegated target for a CLI on PATH and skips one absent
              unsupported() names opencode and zed with their reasons

 desktop      merging into a file with other servers leaves them byte-identical
              merging twice is idempotent — the second is a no-op
              a stale command path is replaced, args included
              remove_server drops only mitodo, leaving siblings intact
              unparseable JSON is refused and the file is left untouched
              a missing mcpServers key is created
              a non-object mcpServers is refused
              the write is atomic: the original survives a failure to rename

 verbs        setup on an empty target set exits 2, not 0
              one failing target does not prevent another from registering
              dry-run makes no filesystem change and issues no command
              status distinguishes registered-and-resolves from registered-but-missing
```

The delegated path is exercised by pointing `Delegated { cli }` at a stub script
in a temp dir, so the argv mitodo builds is asserted without needing `claude`
installed.

**Not in CI:** actually registering with the real clients, which mutates files
outside the repo. That is a documented manual step, and this session performs it
on both machines after merge.

---

## 8. Out of scope

- Zed, and any other JSONC-configured client
- `opencode`, until `mcp add` accepts arguments
- Cursor, Windsurf, Claude Desktop on Windows or Linux paths — detection is
  macOS-first because that is what both machines run; adding a path per platform
  is a later, mechanical change
- HTTP/SSE transports: mitodo's server is stdio only
- Registering anything other than mitodo itself
- `setup --config-dir <dir>` and `--name`, for a second registration pinned to a
  different workspace. §3 describes them as the escape hatch, but nothing needs
  one yet: the default registration follows the config file wherever it points.
- `self` verbs beyond `mcp` (`self update`, `self doctor`) — the namespace exists
  so they have somewhere to go, not because they are planned here

---

## 9. Files touched

| File | Change |
|---|---|
| `src/selfcfg/mod.rs` | New: the three verbs and their reporter |
| `src/selfcfg/target.rs` | New: `Target`, `detect`, `unsupported` |
| `src/selfcfg/desktop.rs` | New: the Claude Desktop JSON merge |
| `src/cli.rs` | `Command::Selfie { action: SelfAction }` — `Self` is a reserved word, so the variant is renamed while clap still spells the subcommand `self` via `#[command(name = "self")]` |
| `src/main.rs` | dispatch, before any terminal setup |
| `README.md` | replace the hand-written `--mcp-config` incantation with `mitodo self mcp setup` |

No new dependencies: `serde_json` and `std::process` are already present.
