# mdt-review hook

A Claude Code `PostToolUse` hook that opens md-tui's comment mode on markdown
files Claude writes (matching a glob allow-list), and feeds your comments back
to Claude as context it must address. No comments -> the write passes silently.

## Requirements

- Claude Code is launched **inside tmux** (the review uses `tmux popup`). Outside
  tmux the hook silently no-ops and the write proceeds.
- `mdt` and `jq` are on `PATH`.

## Install

Add to `.claude/settings.json` (project) or `~/.claude/settings.json` (global),
using the **real absolute path** to the script (see the symlink note below) and
one or more globs (relative to the project root):

```json
{
  "hooks": {
    "PostToolUse": [
      {
        "matcher": "Write",
        "hooks": [
          {
            "type": "command",
            "command": "/ABSOLUTE/PATH/TO/md-tui/scripts/mdt-review-hook.sh 'docs/**/*.md' 'specs/**/*.md'",
            "timeout": 1800
          }
        ]
      }
    ]
  }
}
```

- `matcher: "Write"` — only full writes trigger review; incremental `Edit`s do not.
- `timeout` is in **seconds**. The default for command hooks is 600 (10 min);
  set it generously (e.g. 1800) so a long review is not killed. There is no
  documented maximum, but confirm a long value is honored in your Claude Code
  version. The hook blocks Claude's turn until you close the popup.
- The comment author defaults to `$USER`; override with `MDT_REVIEW_AUTHOR`.

### Symlink note

The hook finds its helper (`scripts/lib/mdt-popup-lib.sh`) relative to its own
location via `${BASH_SOURCE[0]}`, which does **not** resolve symlinks. Point the
`command` at the script's real path; do not symlink `mdt-review-hook.sh` onto
`PATH` and expect `lib/` to resolve from the symlink's directory.

## Behavior

| Situation | Result |
|---|---|
| Written path not in any glob | Hook exits 0, nothing happens |
| Not in tmux / `mdt` or `jq` missing | Hook exits 0, note on stderr, write proceeds |
| Popup closed with no comments | Hook exits 0, clean pass |
| Popup closed with comments | Hook emits `additionalContext` listing the comments; Claude is told it must address each before continuing |

The hook fails **open**: any internal error lets the write proceed rather than
blocking it. It can only ever surface review comments, never halt on its own
malfunction.

## How it works

On a matching `Write`, the hook opens `mdt` in a `tmux popup` with
`MDT_DUMP_PATH` set to a temp file. You enter comment mode, attach comments to
ranges of the rendered markdown, and quit. md-tui writes a Sidemark YAML dump to
that file (nothing is written if there are no comments). The hook parses the
dump and emits the comments as `hookSpecificOutput.additionalContext` so Claude
picks them up on its next turn.
