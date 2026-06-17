# Comment Mode — Design

Date: 2026-05-09
Status: Approved — implemented. The shipped design diverged in places: comment
mode is decoupled from caret mode, comments are anchored to source spans (not
raw caret coordinates), and the sidebar tracks scroll via the markdown scroll
position rather than a dedicated `sidebar_scroll` field on `App`. Treat the
sections below as the original intent, not the current state.

## Goal

Let a user, while in caret mode, attach textual comments to character ranges of
the rendered markdown view. Comments are visible in a floating sidebar on the
right and are navigable with `n`/`N`. Comments are in-memory only for this
iteration — no persistence.

## Glossary

- **Caret mode** — existing mode toggled with `v` where a visible caret moves
  through the rendered markdown grid.
- **Comment mode** — new state, only valid while caret mode is on. Sub-states
  defined below.
- **Anchor / range** — `(start, end)` pair of `Caret` coordinates (line, col).
  `start` inclusive, `end` exclusive. May span multiple rendered lines.

## Data model

```rust
pub struct Comment {
    pub start: Caret,   // (line, col), inclusive
    pub end:   Caret,   // (line, col), exclusive
    pub text:  String,
}
```

New fields on `App`:

```rust
pub comments:        Vec<Comment>,    // insertion order; no persistence
pub comment_state:   CommentState,
pub active_comment:  Option<usize>,   // index into `comments`
pub sidebar_scroll:  u16,             // sidebar's own vertical scroll
```

`CommentState` enum (new):

- `Off` — comment mode is not active.
- `Browsing` — sidebar open, caret moves freely, `n`/`N` cycles comments.
- `Selecting { anchor: Caret }` — `a` was pressed in `Browsing`; caret motion
  extends the selection from `anchor` to the live caret.
- `Editing { range: (Caret, Caret), draft: String, cursor: usize }` — `Enter`
  was pressed in `Selecting`; textbox is focused.

Constraints:

- Comment mode requires caret mode. Toggling caret mode off (`v`) while in any
  comment state forces `comment_state = Off`.
- `App::reset()` resets `comments`, `comment_state`, `active_comment`,
  `sidebar_scroll`.

## User flows

**Entry / exit**

| From | Key | To | Side effect |
|---|---|---|---|
| Caret mode + `Off` | `c` | `Browsing` | Sidebar opens. Caret retained. |
| `Browsing` | `c` or `Esc` | `Off` | Sidebar closes. Comments retained. |
| `Selecting` | `Esc` | `Browsing` | Selection discarded. |
| `Editing` | `Esc` | `Browsing` | Draft discarded; no comment created. |
| Caret mode toggled off (`v`) | — | `Off` | Comment state is forcibly cleared. |

`c` is consumed only in `Browsing`. While in `Selecting` it is a no-op (its
behavior already covered there). While in `Editing` it is treated as a normal
character and inserted into the draft.

**`Browsing` → `Selecting`**

- `a` → `Selecting { anchor: caret }`. The cell at the caret becomes the
  selection's anchor.

**`Selecting`**

- All caret motion keys (`j`/`k`/`h`/`l`, arrows, `g`/`G`, `0`/`$`, page
  motions) move the caret normally. The live selection is
  `min(anchor, caret)..max(anchor, caret)` in document order.
- `Enter` → `Editing { range, draft: "", cursor: 0 }`. Textbox in sidebar gains
  focus.
- `Esc` → `Browsing`. Selection discarded.
- `a` while already selecting → no-op.

**`Editing`**

- Char keys → insert into `draft` at `cursor`; advance `cursor`.
- Backspace → delete from `draft`; decrement `cursor`.
- `Enter` → push `Comment { range, text: draft }` onto `comments`. Set
  `active_comment = Some(<new index>)`. Center on it. Transition to `Browsing`.
- `Esc` → `Browsing`. Draft discarded. No comment created.

**`Browsing` — navigation between comments**

- `n` → `active_comment = (active.unwrap_or(usize::MAX).wrapping_add(1)) % comments.len()`.
  Caret jumps to that comment's `start`. Both panes scroll so anchor and the
  matching sidebar card sit at mid-screen.
- `N` → previous (wrap around).
- Caret motion keys still work and move the caret freely. Moving the caret does
  **not** change `active_comment`.
- `n`/`N` are no-ops if `comments.is_empty()`.

**Conflicts and key-routing notes**

- `n`/`N` are search-next/search-prev globally. They are reinterpreted only
  while `comment_state != Off`.
- `c` is currently unbound; bound to `Action::EnterCommentMode` and only
  meaningful while caret mode is on.
- `a` is currently unbound; bound to `Action::StartCommentSelect` and only
  meaningful while `comment_state == Browsing`.

## Rendering

**Sidebar — `CommentSideBar` widget (new)**

- Floating overlay on the right edge of the markdown view area. Default width
  32 cells. Hard-coded for this iteration; can be made configurable later.
- Drawn after the markdown and after the caret post-pass, so it covers the
  rightmost columns of the rendered document.
- Visible iff `comment_state != Off`.
- Contents: a vertical stack of cards, one per `Comment`, in `comments` order:
  - Anchor excerpt: characters from `start..end` clipped to one line and
    truncated to fit (e.g. `"sed do eiusmod…"`).
  - Blank line.
  - Comment text (wrapped to sidebar width).
  - Bottom border separating cards.
- Active card (`active_comment`) highlighted with a different background.
  Reuses the existing `link_selected_bg_color` config value.
- The sidebar tracks its own `sidebar_scroll`. When `active_comment` changes,
  scroll so the active card sits at the sidebar's vertical midpoint.
- In `Editing`, the active card's comment-text region is replaced with a
  one-line input rendering `draft` plus a visible cursor cell — mirrors the
  existing `SearchBox` rendering.

**Anchor highlighting in the main pane (post-pass on the buffer)**

- Performed in `render_markdown` after the caret post-pass at
  `src/main.rs:349`.
- For each saved `Comment`, walk every cell in `start..end`, handling
  multi-line spans:
  - On the start line: from `start.col` to end of line.
  - On full intermediate lines: entire line.
  - On the end line: from line start to `end.col`.
- Apply a dim background highlight to those cells.
- Active comment uses a brighter highlight than the rest.
- During `Selecting`, the live range
  `min(anchor, caret)..max(anchor, caret)` is highlighted with the same style
  as the active comment.
- The existing `Modifier::REVERSED` caret post-pass still runs, so the caret
  remains visible on top of any highlight.

**Centering on `n`/`N`**

- Main pane: `vertical_scroll = active.start.line.saturating_sub(vh / 2)`
  clamped to a valid scroll position, where `vh` is `viewport_height(height)`.
- Sidebar: set `sidebar_scroll` so the active card's vertical midpoint lines
  up with the sidebar's vertical midpoint, clamped so we never scroll past the
  end.

## Files to touch

- `src/util.rs` — `Comment`, `CommentState`, new `App` fields, reset, helpers
  for next/prev navigation and centering math.
- `src/util/keys.rs` — new `Action` variants and `KeyConfig` defaults
  (`comment = 'c'`, `comment_select = 'a'`). Routing for `n`/`N` stays
  unchanged at the keymap level — the event handler interprets them based on
  `comment_state`.
- `src/event_handler.rs` — `comment_state`-aware dispatch in
  `keyboard_mode_view`: routes char/backspace/enter/esc into the textbox
  during `Editing`; intercepts motion to live-update during `Selecting`;
  handles `c`/`a`/`n`/`N` transitions.
- `src/boxes/mod.rs` + new file `src/boxes/comment_sidebar.rs` —
  `CommentSideBar` widget with `scroll_offset`, card rendering, and draft
  rendering when the active card is in `Editing`.
- `src/main.rs` — call the anchor-highlight post-pass and the sidebar render
  after the caret post-pass in `render_markdown`.

## Out of scope (deferred)

- Persistence — comments live only for the current session.
- Multi-line comment input — single-line only; `Enter` saves.
- Editing or deleting existing comments — browse is view-only.
- Comment count badge in footer or any status indicator.
- Configurable sidebar width and colors.
- Reflowing the markdown to make room for the sidebar — floating overlay
  covers the right edge of the document instead.
- File-scoped storage / reacting to file changes.

## Tests

- Unit tests in `src/util.rs`:
  - `comment_state` transitions: `Off` → `Browsing` → `Selecting` → `Editing`
    → `Browsing` and back via `Esc` from each sub-state.
  - `n`/`N` cycling wraps correctly with 0, 1, and N comments.
  - Toggling caret mode off forces `Off` and clears `active_comment`.
  - Multi-line range normalization (anchor below caret produces a `start`
    above `end`).
- No unit test for rendering; verify by running `mdt` against an example file
  and exercising the flow manually.
