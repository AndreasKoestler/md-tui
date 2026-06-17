# mdt-review Hook — Design

Date: 2026-06-17
Status: Approved — not yet implemented.

## Goal

Let a human review markdown that Claude Code writes, in-place, using md-tui's
comment mode — and feed those comments back to Claude as required revisions.

The integration is a Claude Code `PostToolUse` hook. When Claude finishes
writing a markdown file that matches a configured allow-list, the hook opens
`mdt` in a tmux popup so the user can attach comments to character ranges of the
rendered doc. On close, any comments are returned to Claude as a blocking
revision request; an empty review passes cleanly.

This builds on the existing `scripts/mdt-popup.sh` and the binary's
`MDT_DUMP_PATH` dump routing (`src/main.rs`).

## Non-goals

- No persistence of comments beyond a single review round.
- No support for incremental `Edit`s — only full `Write`s trigger review.
- No automatic install into the user's global Claude Code config; the design
  ships a script plus a documented `settings.json` snippet.

## Decisions (from brainstorming)

| Question | Decision |
|---|---|
| Which writes trigger review? | Opt-in via **path glob**, **`Write` tool only** (final write, not incremental `Edit`). |
| How are comments returned? | **Block + require revision** (`decision:block` + `reason`). No comments → clean pass. |
| How is opt-in configured? | **Path globs passed as args** to the hook command in `settings.json`. |

## Why a file dump, not popup stdout

A `tmux popup` runs as a separate client against the tmux server; its stdout is
not wired to the calling process, and `mdt` draws its ratatui TUI to stdout
anyway. So comments cannot travel back over the popup's stdout. The binary
already solves this: `MDT_DUMP_PATH=<file> mdt <doc>` writes the Sidemark YAML
dump to `<file>` on clean exit. The hook creates the temp file, bakes the env
var into the popup command string (the tmux server scrubs the environment), runs
the popup, and reads the file after it closes. This is the same mechanism as
`scripts/mdt-popup.sh`.

## Configuration

Registered as a `PostToolUse` hook matching the `Write` tool. The glob
allow-list is passed as command arguments, so the Claude config is the opt-in:

```json
{
  "hooks": {
    "PostToolUse": [{
      "matcher": "Write",
      "hooks": [{
        "type": "command",
        "command": "/abs/path/scripts/mdt-review-hook.sh 'docs/**/*.md' 'specs/**/*.md'",
        "timeout": 1800
      }]
    }]
  }
}
```

- `matcher: "Write"` — incremental `Edit`s never trigger review.
- Each positional arg is an allow-list glob, evaluated relative to the project
  root (`cwd` from the hook payload).
- `timeout: 1800` (30 min) so a long review is not killed. If Claude Code caps
  the per-hook timeout below this, the implementation documents the ceiling.

## Data flow

1. Claude finishes a `Write`. The hook fires and reads the tool-call JSON on
   stdin, extracting `tool_input.file_path` and `cwd`.
2. **Glob gate.** Expand each arg glob against the project dir (bash
   `globstar`); check whether the written `file_path` is in the expansion. No
   match → `exit 0`, silent, write untouched.
3. **Environment gate.** If `$TMUX` is unset, or `mdt` is not on `PATH` → one
   line to stderr and `exit 0`. Review is skipped; the hook never blocks here.
4. **Review.** Open `mdt` in a `tmux popup` over the Claude session with
   `MDT_DUMP_PATH` pointing at a fresh temp file. The user reviews and comments,
   then closes the popup.
5. **Result.**
   - Comments present → emit `{"decision":"block","reason":"<formatted
     comments>"}` on stdout. Claude is re-prompted with the comments as required
     revisions.
   - No comments → `exit 0`. Clean pass.

## Components

Each is independently testable.

- **`scripts/mdt-review-hook.sh`** — orchestration: parse stdin, run the two
  gates, invoke the popup, emit the decision JSON. The only new file.
- **glob-match** — pure shell function: `(file_path, project_root, globs...) →
  match | no-match`. Uses `shopt -s globstar nullglob` and compares the absolute
  written path against each glob's filesystem expansion (the file already exists
  because `Write` has completed).
- **dump→reason formatter** — pure shell function: Sidemark YAML dump → block
  reason text, or empty (= pass). Reads the comment entries and renders them as
  a concise list keyed by anchor/range.
- **popup invocation** — shared with `scripts/mdt-popup.sh`. Factor the common
  "run mdt in a popup with `MDT_DUMP_PATH`, return the dump path" step into a
  helper both scripts source, rather than duplicating it.

## Error handling

Fail **open**, never closed. Any failure in the gates or the popup — no tmux,
`mdt` missing, malformed glob, `mdt` non-zero exit, unparseable dump — results
in `exit 0` and an optional stderr breadcrumb, so the write proceeds. The hook
can only ever block on *real comments*, never on its own malfunction.

## Testing

- **glob-match** (bats/shell): in allow-list, sibling directory, wrong
  extension, `**` spanning directories, no globs given.
- **formatter**: fixture YAML dumps — zero comments (pass), one comment, many
  comments; malformed dump (treated as pass, not a block).
- **gates**: `$TMUX` unset → exit 0, no stdout; non-matching path → exit 0;
  `mdt` missing → exit 0.
- **manual end-to-end** inside tmux: write a matching doc, confirm popup opens,
  add a comment, confirm Claude receives the block reason. Documented, not
  automated.

## Assumptions to verify during implementation

1. **`PostToolUse` + `decision:block` re-prompts Claude with `reason`.** The
   `Write` has already happened — `block` does not undo it; it surfaces the
   comments as feedback for the next turn. This matches the intent. Verify
   against the live hook contract before finalizing the JSON shape (fall back to
   `hookSpecificOutput.additionalContext` if `block` is not honored for
   `PostToolUse`).
2. **Hook `timeout` ceiling.** Confirm Claude Code accepts a 1800s per-hook
   timeout; document the actual cap if lower.
