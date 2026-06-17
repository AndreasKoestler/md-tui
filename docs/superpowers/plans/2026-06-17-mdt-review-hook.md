# mdt-review Hook Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a Claude Code `PostToolUse` hook that opens md-tui's comment mode on
allow-listed markdown files Claude writes, and feeds the resulting comments back
as a blocking revision request.

**Architecture:** One hook script (`scripts/mdt-review-hook.sh`) registered against
the `Write` tool. It reads the tool-call JSON on stdin, gates on a path-glob
allow-list (passed as command args) and on the environment (`$TMUX` + `mdt`), opens
`mdt` in a `tmux popup` with `MDT_DUMP_PATH` pointing at a temp file, then parses the
Sidemark YAML dump. Comments → `{"decision":"block","reason":...}`; no comments →
clean `exit 0`. All failures fail **open** (exit 0). The popup invocation is factored
into a shared lib also used by the existing `scripts/mdt-popup.sh`.

**Tech Stack:** Bash, `jq` (stdin parse + scalar decode), `awk` (dump parse), `tmux popup`, the existing `mdt` binary's `MDT_DUMP_PATH` routing.

## Discovery

**Similar implementations:** `scripts/mdt-popup.sh` already runs `mdt` in a `tmux popup` with `MDT_DUMP_PATH` baked into the command string and reads the dump file after the popup closes. The new hook reuses this exact pattern via an extracted lib.
**File conventions:** Scripts live in `scripts/` (single existing file, `mdt-popup.sh`). No `scripts/lib/` or `scripts/test/` yet — created here. POSIX-ish bash with `set -euo pipefail`, `%q`-quoted command strings, `mktemp -t` + `trap ... EXIT` cleanup.
**Testing patterns:** Rust uses `cargo test` (`src/sidemark.rs`, `src/main.rs` have `#[cfg(test)]`). No shell tests exist. New shell logic is tested with a dependency-free bash runner (`scripts/test/mdt-review-hook.test.sh`) — no `bats`/`yq` (neither installed).
**Integration points:** The `mdt` binary reads `MDT_DUMP_PATH` (`src/main.rs:95-135`, `resolve_dump_target`/`dump_comments`). `sidemark::render` (`src/sidemark.rs:49`) returns `None` when there are no comments, so the dump file is left untouched — "no comments" == "file absent/empty". Dump scalars are double-quoted with `\\ \" \n \r \t \xNN` escapes (`yaml_quoted`, `src/sidemark.rs:146`).
**Project conventions:** No repo-local `CONTRIBUTING.md`/pre-commit hook/`shellcheck` config found. `jq` present at `/usr/bin/jq`.
**Context loaded:** none — ad-hoc discovery.

---

## File Structure

- **Create `scripts/lib/mdt-popup-lib.sh`** — shared, sourced helper. One function `run_mdt_popup` that opens `mdt` in a tmux popup writing to a caller-provided dump path. No side effects on source (defines a function only).
- **Modify `scripts/mdt-popup.sh`** — source the lib and call `run_mdt_popup` instead of inlining the `tmux popup` command. Behavior unchanged.
- **Create `scripts/mdt-review-hook.sh`** — the hook. Pure functions (`extract_file_path`, `extract_cwd`, `glob_match`, `format_reason`, `emit_block`) plus `main`. Guarded so sourcing it (for tests) does not run `main`.
- **Create `scripts/test/mdt-review-hook.test.sh`** — dependency-free bash test runner. Sources the hook script and exercises the pure functions against fixtures.
- **Create `docs/mdt-review-hook.md`** — install instructions: the `settings.json` snippet and the manual end-to-end checklist.

Each file has one responsibility; the pure functions are unit-tested in isolation, and the only untestable-by-unit piece (the actual `tmux popup`) lives alone in the lib and is covered by the documented manual check.

---

## Task 1: Test runner scaffold + `glob_match`

**Files:**
- Create: `scripts/mdt-review-hook.sh`
- Create: `scripts/test/mdt-review-hook.test.sh`

- [ ] **Step 1: Write the failing test**

Create `scripts/test/mdt-review-hook.test.sh`:

```bash
#!/usr/bin/env bash
# Dependency-free test runner for mdt-review-hook.sh. Sources the hook (which
# does NOT run main when sourced) and exercises its pure functions.
set -uo pipefail

HERE=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
ROOT=$(cd -- "$HERE/../.." && pwd)
# shellcheck source=../mdt-review-hook.sh
. "$ROOT/scripts/mdt-review-hook.sh"

pass=0 fail=0
ok() { printf 'ok   - %s\n' "$1"; pass=$((pass + 1)); }
no() { printf 'FAIL - %s\n' "$1"; fail=$((fail + 1)); }
# check <desc> <expected> <actual>
check() {
    if [[ "$2" == "$3" ]]; then
        ok "$1"
    else
        no "$1"
        printf '       expected: %q\n       actual:   %q\n' "$2" "$3"
    fi
}

# --- glob_match ---------------------------------------------------------
glob_fixture=$(mktemp -d -t mdt-glob.XXXXXX)
trap 'rm -rf -- "$glob_fixture"' EXIT
mkdir -p "$glob_fixture/docs/sub" "$glob_fixture/src"
touch "$glob_fixture/docs/a.md" "$glob_fixture/docs/sub/b.md" \
    "$glob_fixture/src/c.md" "$glob_fixture/docs/d.txt"

glob_match "$glob_fixture/docs/a.md" "$glob_fixture" 'docs/**/*.md'
check "glob: top-level docs .md matches" 0 $?

glob_match "$glob_fixture/docs/sub/b.md" "$glob_fixture" 'docs/**/*.md'
check "glob: nested docs .md matches (globstar)" 0 $?

glob_match "$glob_fixture/src/c.md" "$glob_fixture" 'docs/**/*.md'
check "glob: .md outside allow-list does not match" 1 $?

glob_match "$glob_fixture/docs/d.txt" "$glob_fixture" 'docs/**/*.md'
check "glob: wrong extension does not match" 1 $?

glob_match "$glob_fixture/docs/a.md" "$glob_fixture"
check "glob: no globs given does not match" 1 $?

printf '\n%d passed, %d failed\n' "$pass" "$fail"
[[ $fail -eq 0 ]]
```

- [ ] **Step 2: Run test to verify it fails**

Run: `bash scripts/test/mdt-review-hook.test.sh`
Expected: FAIL — `scripts/mdt-review-hook.sh` does not exist (source error: "No such file or directory").

- [ ] **Step 3: Write minimal implementation**

Create `scripts/mdt-review-hook.sh`:

```bash
#!/usr/bin/env bash
# Claude Code PostToolUse hook (matcher: Write). On an allow-listed markdown
# write, open mdt's comment mode in a tmux popup and return any comments as a
# blocking revision request. Fails OPEN: any problem -> exit 0, write proceeds.
#
# Usage (from settings.json hook command):
#   mdt-review-hook.sh '<glob>' ['<glob>' ...]
# Globs are evaluated relative to the project root (cwd from the hook payload).
set -uo pipefail

# match_file <file> <project_root> <glob>...
# Returns 0 if <file> is in any glob's filesystem expansion under root, else 1.
glob_match() {
    local file=$1 root=$2
    shift 2
    local g f rc=1
    shopt -s globstar nullglob
    for g in "$@"; do
        for f in "$root"/$g; do
            if [[ $f == "$file" ]]; then
                rc=0
                break 2
            fi
        done
    done
    shopt -u globstar nullglob
    return $rc
}

main() {
    : # filled in by later tasks
}

# Only run main when executed directly, not when sourced by the test runner.
if [[ "${BASH_SOURCE[0]}" == "${0}" ]]; then
    main "$@"
fi
```

- [ ] **Step 4: Run test to verify it passes**

Run: `bash scripts/test/mdt-review-hook.test.sh`
Expected: PASS — `5 passed, 0 failed`.

- [ ] **Step 5: Commit**

```bash
git add scripts/mdt-review-hook.sh scripts/test/mdt-review-hook.test.sh
git commit -m "feat(hook): add mdt-review glob_match + test runner"
```

---

## Task 2: Parse the hook payload (`extract_file_path`, `extract_cwd`)

**Files:**
- Modify: `scripts/mdt-review-hook.sh`
- Test: `scripts/test/mdt-review-hook.test.sh`

- [ ] **Step 1: Write the failing test**

Append to `scripts/test/mdt-review-hook.test.sh` immediately before the final
`printf '\n%d passed...` summary line:

```bash
# --- payload parsing ----------------------------------------------------
payload='{"tool_name":"Write","cwd":"/proj","tool_input":{"file_path":"/proj/docs/a.md","content":"hi"}}'

check "payload: extract_file_path" "/proj/docs/a.md" \
    "$(printf '%s' "$payload" | extract_file_path)"
check "payload: extract_cwd" "/proj" \
    "$(printf '%s' "$payload" | extract_cwd)"
check "payload: missing file_path -> empty" "" \
    "$(printf '%s' '{"tool_input":{}}' | extract_file_path)"
```

- [ ] **Step 2: Run test to verify it fails**

Run: `bash scripts/test/mdt-review-hook.test.sh`
Expected: FAIL — `extract_file_path: command not found` (3 new failures).

- [ ] **Step 3: Write minimal implementation**

In `scripts/mdt-review-hook.sh`, add these functions immediately after the
`glob_match` function:

```bash
# Read the hook JSON payload from stdin and echo the written file path.
extract_file_path() {
    jq -r '.tool_input.file_path // empty'
}

# Read the hook JSON payload from stdin and echo the project root (cwd).
extract_cwd() {
    jq -r '.cwd // empty'
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `bash scripts/test/mdt-review-hook.test.sh`
Expected: PASS — `8 passed, 0 failed`.

- [ ] **Step 5: Commit**

```bash
git add scripts/mdt-review-hook.sh scripts/test/mdt-review-hook.test.sh
git commit -m "feat(hook): parse Write payload file_path and cwd"
```

---

## Task 3: Render the dump into a block reason (`format_reason`)

**Files:**
- Modify: `scripts/mdt-review-hook.sh`
- Test: `scripts/test/mdt-review-hook.test.sh`

`format_reason <dump_file>` prints a human-readable revision request and exits
0, or prints nothing (and the caller treats empty output as "pass"). It parses
the Sidemark YAML with `awk` into `line<TAB>text<TAB>selected_text` rows, then
decodes each quoted scalar with `jq -r .` (the scalars use JSON-compatible
escapes; on the rare `\xNN` case `jq` fails and we fall back to the raw token).

- [ ] **Step 1: Write the failing test**

Append to `scripts/test/mdt-review-hook.test.sh` before the summary line:

```bash
# --- format_reason ------------------------------------------------------
empty_dump=$(mktemp -t mdt-empty.XXXXXX)
check "format_reason: empty/absent dump -> empty" "" "$(format_reason "$empty_dump")"
check "format_reason: nonexistent dump -> empty" "" "$(format_reason /no/such/file)"
rm -f -- "$empty_dump"

one_dump=$(mktemp -t mdt-one.XXXXXX)
cat >"$one_dump" <<'YAML'
mrsf_version: "1.0"
document: "docs/a.md"
comments:
  - id: 11111111-1111-1111-1111-111111111111
    author: "me"
    timestamp: '2026-06-17T00:00:00+00:00'
    text: "tighten this sentence"
    resolved: false
    line: 3
    end_line: 3
    start_column: 0
    end_column: 5
    selected_text: "Hello world"
YAML
out=$(format_reason "$one_dump")
rm -f -- "$one_dump"
case $out in
    *"Address each before continuing"*) ok "format_reason: header present" ;;
    *) no "format_reason: header present" ;;
esac
case $out in
    *'- L3: tighten this sentence  (on: Hello world)'*) ok "format_reason: comment line rendered + decoded" ;;
    *) no "format_reason: comment line rendered + decoded"; printf '       got: %q\n' "$out" ;;
esac

two_dump=$(mktemp -t mdt-two.XXXXXX)
cat >"$two_dump" <<'YAML'
mrsf_version: "1.0"
document: "docs/a.md"
comments:
  - id: 11111111-1111-1111-1111-111111111111
    author: "me"
    timestamp: '2026-06-17T00:00:00+00:00'
    text: "first note with \"quotes\""
    resolved: false
    line: 1
    end_line: 1
    start_column: 0
    end_column: 1
  - id: 22222222-2222-2222-2222-222222222222
    author: "me"
    timestamp: '2026-06-17T00:00:00+00:00'
    text: "second note"
    resolved: false
    line: 9
    end_line: 9
    start_column: 0
    end_column: 2
YAML
out2=$(format_reason "$two_dump")
rm -f -- "$two_dump"
case $out2 in
    *'returned 2 comment(s)'*) ok "format_reason: counts two comments" ;;
    *) no "format_reason: counts two comments"; printf '       got: %q\n' "$out2" ;;
esac
case $out2 in
    *'- L1: first note with "quotes"'*) ok "format_reason: decodes escaped quotes, no selected_text" ;;
    *) no "format_reason: decodes escaped quotes, no selected_text"; printf '       got: %q\n' "$out2" ;;
esac
```

- [ ] **Step 2: Run test to verify it fails**

Run: `bash scripts/test/mdt-review-hook.test.sh`
Expected: FAIL — `format_reason: command not found`.

- [ ] **Step 3: Write minimal implementation**

In `scripts/mdt-review-hook.sh`, add after `extract_cwd`:

```bash
# Decode a Sidemark double-quoted scalar (e.g. "a \"b\"") to plain text.
# The scalars use JSON-compatible escapes for the common cases; jq decodes
# them. On the rare \xNN control escape jq errors -> fall back to the raw
# token so we never lose the comment.
decode_scalar() {
    local tok=$1 dec
    dec=$(printf '%s' "$tok" | jq -r . 2>/dev/null) && {
        printf '%s' "$dec"
        return 0
    }
    printf '%s' "$tok"
}

# format_reason <dump_file>
# Print a blocking revision request, or nothing if there are no comments.
format_reason() {
    local dump=$1
    [[ -s $dump ]] || return 0

    local rows
    rows=$(awk '
        /^  - id:/             { if (have) print line "\t" text "\t" sel; have=1; line=""; text=""; sel="" }
        /^    line: /          { line=$2 }
        /^    text: /          { text=substr($0, index($0, ": ") + 2) }
        /^    selected_text: / { sel=substr($0, index($0, ": ") + 2) }
        END                    { if (have) print line "\t" text "\t" sel }
    ' "$dump")
    [[ -n $rows ]] || return 0

    local count
    count=$(printf '%s\n' "$rows" | grep -c .)
    printf 'Markdown review returned %d comment(s). Address each before continuing:\n\n' "$count"

    local line text sel t s
    while IFS=$'\t' read -r line text sel; do
        t=$(decode_scalar "$text")
        if [[ -n $sel ]]; then
            s=$(decode_scalar "$sel")
            printf -- '- L%s: %s  (on: %s)\n' "$line" "$t" "$s"
        else
            printf -- '- L%s: %s\n' "$line" "$t"
        fi
    done <<<"$rows"
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `bash scripts/test/mdt-review-hook.test.sh`
Expected: PASS — `14 passed, 0 failed`.

- [ ] **Step 5: Commit**

```bash
git add scripts/mdt-review-hook.sh scripts/test/mdt-review-hook.test.sh
git commit -m "feat(hook): render Sidemark dump into a block reason"
```

---

## Task 4: Emit the decision JSON (`emit_block`)

**Files:**
- Modify: `scripts/mdt-review-hook.sh`
- Test: `scripts/test/mdt-review-hook.test.sh`

- [ ] **Step 1: Write the failing test**

Append to `scripts/test/mdt-review-hook.test.sh` before the summary line:

```bash
# --- emit_block ---------------------------------------------------------
emitted=$(emit_block $'do the thing\nand the other')
check "emit_block: decision field" "block" "$(printf '%s' "$emitted" | jq -r .decision)"
check "emit_block: reason preserves newlines" $'do the thing\nand the other' \
    "$(printf '%s' "$emitted" | jq -r .reason)"
```

- [ ] **Step 2: Run test to verify it fails**

Run: `bash scripts/test/mdt-review-hook.test.sh`
Expected: FAIL — `emit_block: command not found`.

- [ ] **Step 3: Write minimal implementation**

In `scripts/mdt-review-hook.sh`, add after `format_reason`:

```bash
# emit_block <reason>
# Print the PostToolUse decision JSON that re-prompts Claude with <reason>.
emit_block() {
    jq -n --arg r "$1" '{decision: "block", reason: $r}'
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `bash scripts/test/mdt-review-hook.test.sh`
Expected: PASS — `16 passed, 0 failed`.

- [ ] **Step 5: Commit**

```bash
git add scripts/mdt-review-hook.sh scripts/test/mdt-review-hook.test.sh
git commit -m "feat(hook): emit PostToolUse block decision JSON"
```

---

## Task 5: Extract shared popup lib; refactor `mdt-popup.sh`

**Files:**
- Create: `scripts/lib/mdt-popup-lib.sh`
- Modify: `scripts/mdt-popup.sh`
- Test: `scripts/test/mdt-review-hook.test.sh`

The actual `tmux popup` call can't be unit-tested (needs a live tmux + a human),
so it lives alone in a sourced lib. The test only asserts the lib is sourceable
and defines the function; the popup itself is covered by the Task 7 manual check.

- [ ] **Step 1: Write the failing test**

Append to `scripts/test/mdt-review-hook.test.sh` before the summary line:

```bash
# --- shared popup lib ---------------------------------------------------
. "$ROOT/scripts/lib/mdt-popup-lib.sh"
if declare -F run_mdt_popup >/dev/null; then
    ok "lib: run_mdt_popup is defined after sourcing"
else
    no "lib: run_mdt_popup is defined after sourcing"
fi
```

- [ ] **Step 2: Run test to verify it fails**

Run: `bash scripts/test/mdt-review-hook.test.sh`
Expected: FAIL — `scripts/lib/mdt-popup-lib.sh: No such file or directory`.

- [ ] **Step 3: Write minimal implementation**

Create `scripts/lib/mdt-popup-lib.sh`:

```bash
# Shared helper for opening md-tui (mdt) inside a tmux popup. Sourced, not
# executed — defines a function and has no side effects on source.

# run_mdt_popup <abs_file> <dump_path> [username]
# Opens mdt on <abs_file> in a tmux popup, with MDT_DUMP_PATH=<dump_path> so the
# Sidemark dump lands in a file (the popup's stdout is the TUI and is not wired
# back to the caller). The env var is baked into the %q-quoted command string
# because the tmux server scrubs the environment. The caller is responsible for
# creating/reading/removing <dump_path> and for checking $TMUX and `mdt`.
run_mdt_popup() {
    local file=$1 dump=$2 username=${3-} cmd
    cmd=$(printf 'MDT_DUMP_PATH=%q mdt %q' "$dump" "$file")
    [[ -n $username ]] && cmd+=$(printf ' -u %q' "$username")
    tmux popup -E -w 90% -h 90% "$cmd"
}
```

- [ ] **Step 4: Refactor `mdt-popup.sh` to use the lib**

Replace the tail of `scripts/mdt-popup.sh` (everything from the
`# Inline env var + %q-quoted args:` comment through the `tmux popup` line) with
a source + call. The final file's executable body becomes:

```bash
abs_file=$(cd -- "$(dirname -- "$file")" && pwd)/$(basename -- "$file")
dump=$(mktemp -t mdt-dump.XXXXXX)
trap 'rm -f -- "$dump"' EXIT

# shellcheck source=lib/mdt-popup-lib.sh
. "$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)/lib/mdt-popup-lib.sh"

run_mdt_popup "$abs_file" "$dump" "$username"

[[ -s $dump ]] && cat -- "$dump"
```

(Leave the `usage`, arg-parsing, and the `$TMUX`/`mdt`/file existence checks at
the top of `mdt-popup.sh` unchanged.)

- [ ] **Step 5: Run tests + smoke-check the refactor**

Run: `bash scripts/test/mdt-review-hook.test.sh`
Expected: PASS — `17 passed, 0 failed`.

Run: `bash -n scripts/mdt-popup.sh && echo SYNTAX-OK`
Expected: `SYNTAX-OK` (the script still parses; `bash -n` does not execute it).

- [ ] **Step 6: Commit**

```bash
git add scripts/lib/mdt-popup-lib.sh scripts/mdt-popup.sh scripts/test/mdt-review-hook.test.sh
git commit -m "refactor(scripts): extract run_mdt_popup into shared lib"
```

---

## Task 6: Wire `main` — gates, popup, decision (fail-open)

**Files:**
- Modify: `scripts/mdt-review-hook.sh`
- Test: `scripts/test/mdt-review-hook.test.sh`

`main` is integration glue around the tested pure functions plus the side-effecting
popup. It is tested at its fail-open boundaries (which exit 0 *before* the popup);
the happy path that opens the popup is the Task 7 manual check.

- [ ] **Step 1: Write the failing test**

Append to `scripts/test/mdt-review-hook.test.sh` before the summary line:

```bash
# --- main fail-open gates (must exit 0 and emit nothing before popup) ---
HOOK="$ROOT/scripts/mdt-review-hook.sh"

# Non-matching path: exits 0, no stdout, regardless of $TMUX.
out=$(printf '%s' '{"cwd":"/proj","tool_input":{"file_path":"/proj/src/c.md"}}' \
    | TMUX="" bash "$HOOK" 'docs/**/*.md'; printf 'rc=%s' "$?")
check "main: non-matching path emits nothing" "rc=0" "$out"

# Matching path but no tmux: exits 0, nothing on stdout (note goes to stderr).
out=$(printf '%s' '{"cwd":"/proj","tool_input":{"file_path":"/proj/docs/a.md"}}' \
    | TMUX="" bash "$HOOK" 'docs/**/*.md' 2>/dev/null; printf 'rc=%s' "$?")
check "main: matching path without tmux skips (exit 0, no stdout)" "rc=0" "$out"

# Empty payload: exits 0.
out=$(printf '%s' '{}' | TMUX="" bash "$HOOK" 'docs/**/*.md' 2>/dev/null; printf 'rc=%s' "$?")
check "main: empty payload exits 0" "rc=0" "$out"
```

Note: these tests use `/proj/...` paths that do not exist on disk. `glob_match`
expands globs against the real filesystem, so a non-existent `/proj` yields no
matches — which is exactly why the first case must short-circuit to exit 0. To
test the *matching* gate deterministically, the second/third cases rely on the
`$TMUX`/payload gates firing first; see Step 3 ordering.

- [ ] **Step 2: Run test to verify it fails**

Run: `bash scripts/test/mdt-review-hook.test.sh`
Expected: FAIL — `main` is still the `:` stub, so it exits 0 with no output and
the *first* check passes by luck, but rerun after Step 3 confirms intent. (If all
three already print `rc=0`, that is acceptable — the stub trivially satisfies
exit-0; Step 3 makes the behavior real rather than accidental.)

- [ ] **Step 3: Write the implementation**

Replace the `main` stub in `scripts/mdt-review-hook.sh` with:

```bash
main() {
    # jq is required to parse the payload; without it, fail open.
    command -v jq >/dev/null 2>&1 || exit 0

    local payload file cwd
    payload=$(cat)
    file=$(printf '%s' "$payload" | extract_file_path)
    cwd=$(printf '%s' "$payload" | extract_cwd)
    [[ -n $file ]] || exit 0
    cwd=${cwd:-$PWD}
    [[ $file == /* ]] || file=$cwd/$file

    # Glob gate: only allow-listed paths get reviewed.
    glob_match "$file" "$cwd" "$@" || exit 0

    # Environment gate: review needs a live tmux and the mdt binary.
    if [[ -z ${TMUX-} ]]; then
        printf 'mdt-review: not inside tmux, skipping review of %s\n' "$file" >&2
        exit 0
    fi
    if ! command -v mdt >/dev/null 2>&1; then
        printf 'mdt-review: mdt not on PATH, skipping review of %s\n' "$file" >&2
        exit 0
    fi

    # shellcheck source=lib/mdt-popup-lib.sh
    . "$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)/lib/mdt-popup-lib.sh"

    local dump
    dump=$(mktemp -t mdt-review.XXXXXX) || exit 0
    trap 'rm -f -- "$dump"' EXIT
    run_mdt_popup "$file" "$dump" "${MDT_REVIEW_AUTHOR:-${USER-}}" || exit 0

    local reason
    reason=$(format_reason "$dump")
    [[ -n $reason ]] || exit 0
    emit_block "$reason"
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `bash scripts/test/mdt-review-hook.test.sh`
Expected: PASS — `20 passed, 0 failed`.

- [ ] **Step 5: Make the hook executable + commit**

```bash
chmod +x scripts/mdt-review-hook.sh
git add scripts/mdt-review-hook.sh scripts/test/mdt-review-hook.test.sh
git commit -m "feat(hook): wire mdt-review main with fail-open gates"
```

---

## Task 7: Install docs + manual end-to-end verification

**Files:**
- Create: `docs/mdt-review-hook.md`
- Modify: `README.md`

- [ ] **Step 1: Write the install doc**

Create `docs/mdt-review-hook.md`:

````markdown
# mdt-review hook

A Claude Code `PostToolUse` hook that opens md-tui's comment mode on markdown
files Claude writes (matching a glob allow-list), and returns your comments as a
blocking revision request. No comments → the write passes silently.

## Requirements

- Claude Code is launched **inside tmux** (the review uses `tmux popup`). Outside
  tmux the hook silently no-ops.
- `mdt` and `jq` are on `PATH`.

## Install

Add to `.claude/settings.json` (project) or `~/.claude/settings.json` (global),
with the absolute path to the script and one or more globs (relative to the
project root):

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
- `timeout` is seconds; set it generously so a long review is not killed.
- The comment author defaults to `$USER`; override with `MDT_REVIEW_AUTHOR`.

## Behavior

| Situation | Result |
|---|---|
| Written path not in any glob | Hook exits 0, nothing happens |
| Not in tmux / `mdt` or `jq` missing | Hook exits 0, note on stderr, write proceeds |
| Popup closed with no comments | Hook exits 0, clean pass |
| Popup closed with comments | Hook returns a `block` decision; Claude must address each comment |

The hook fails **open**: any internal error lets the write proceed rather than
blocking it.
````

- [ ] **Step 2: Link it from the README**

Add a bullet under the README's features/usage area pointing at the new doc:

```markdown
- **Claude Code review hook** — review markdown Claude writes in md-tui's comment
  mode and feed comments back as revisions. See [`docs/mdt-review-hook.md`](docs/mdt-review-hook.md).
```

- [ ] **Step 3: Verify the assumptions from the spec**

Confirm the two spec assumptions against the installed Claude Code build:

1. `PostToolUse` honors `{"decision":"block","reason":...}` by re-prompting with
   the reason. If this build instead requires
   `{"hookSpecificOutput":{"hookEventName":"PostToolUse","additionalContext":...}}`,
   change `emit_block` accordingly (and update its test in Task 4). Run
   `claude --help` / consult the hooks docs to confirm the current contract.
2. A `timeout` of `1800` is accepted. If Claude Code caps it lower, note the cap
   in `docs/mdt-review-hook.md`.

- [ ] **Step 4: Manual end-to-end check (inside tmux)**

```bash
# Build mdt so it is on PATH for the test.
cargo build --release
export PATH="$PWD/target/release:$PATH"

# Dry-run the hook by hand, simulating a Write payload for an allow-listed file.
printf '%s' '{"cwd":"'"$PWD"'","tool_input":{"file_path":"'"$PWD"'/docs/mdt-review-hook.md"}}' \
  | ./scripts/mdt-review-hook.sh 'docs/**/*.md'
```

Expected:
- A tmux popup opens showing `docs/mdt-review-hook.md` rendered in `mdt`.
- Enter comment mode, attach a comment to some text, then quit `mdt`.
- After the popup closes, the hook prints a JSON object with `"decision":"block"`
  and a `reason` listing your comment(s). Validate it:
  `… | jq .` should show well-formed JSON.
- Repeat without adding any comment → the hook prints nothing and exits 0.

- [ ] **Step 5: Run the full shell test suite once more + commit**

Run: `bash scripts/test/mdt-review-hook.test.sh`
Expected: PASS — `20 passed, 0 failed`.

```bash
git add docs/mdt-review-hook.md README.md
git commit -m "docs(hook): install + manual verification guide for mdt-review"
```

---

## Self-Review

**Spec coverage:**
- Opt-in via path glob, `Write`-only → Task 1 (`glob_match`), Task 7 (`matcher: "Write"` in config). ✓
- File-dump mechanism, not popup stdout → Task 5 (`run_mdt_popup` with `MDT_DUMP_PATH`). ✓
- Glob gate / environment gate / fail-open → Task 6 (`main`). ✓
- Comments present → block; none → pass → Task 3 (`format_reason` empty = pass) + Task 4 (`emit_block`) + Task 6 (wiring). ✓
- Components independently testable → Tasks 1–4 unit-test pure functions; popup isolated in Task 5. ✓
- Error handling fails open → Task 6 gates + Task 3/4 fallbacks. ✓
- Testing (glob, formatter, gates, manual e2e) → Tasks 1,3,6,7. ✓
- Assumptions to verify (block contract, timeout cap) → Task 7 Step 3. ✓

**Placeholder scan:** No TODO/TBD/"handle edge cases". The Task 1 `main` is an explicit `:` stub by design, replaced with full code in Task 6. ✓

**Type/name consistency:** Function names used consistently across tasks and tests: `glob_match`, `extract_file_path`, `extract_cwd`, `decode_scalar`, `format_reason`, `emit_block`, `run_mdt_popup`, `main`. Test pass-counts are cumulative (5 → 8 → 14 → 16 → 17 → 20). ✓

**Discovery referenced:** File placement (`scripts/`, new `scripts/lib/`, `scripts/test/`), the `MDT_DUMP_PATH` reuse, the "no comments → no file" fact, and the dependency-free test choice all trace to the Discovery section. ✓
