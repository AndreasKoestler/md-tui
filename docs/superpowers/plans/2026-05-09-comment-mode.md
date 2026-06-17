# Comment Mode Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add an in-memory "comment mode" reachable from caret mode, allowing a user to attach textual comments to character ranges of the rendered markdown and to cycle between them with `n`/`N` while a sidebar shows them on the right.

**Architecture:** Comment data and the comment-mode state machine live in a new `src/comments.rs` module. `App` (in `src/util.rs`) gains a few new fields and thin helpers. A new `CommentSideBar` widget under `src/boxes/` renders the floating sidebar. The event handler routes keys based on `comment_state`. Render integration is a post-pass on the buffer in `src/main.rs::render_markdown` (mirrors the existing caret post-pass).

**Tech Stack:** Rust 2024, ratatui 0.30, crossterm 0.29 (already in use). No new dependencies.

## Discovery

**Similar implementations:**
- `SearchBox` in `src/boxes/searchbox.rs` is the closest analogue for an interactive textbox: it owns `text: String` + `cursor: usize`, exposes `insert`, `delete`, `clear`, `consume`, and a `Widget` impl. The new comment-edit input mirrors this pattern.
- `BookmarkStore` in `src/bookmarks.rs` and the `bookmarks: BTreeMap<char, Caret>` field on `App` is the closest analogue for a per-document collection of caret-anchored items. Comments are like bookmarks but with a range and text instead of a single coord and char.
- The pending-input pattern (`PendingInput` enum + `app.pending_input` field, consumed early in `keyboard_mode_view` at `src/event_handler.rs:277-292`) is the closest analogue for a sub-state that intercepts the next key press; the comment state machine generalises this idea.

**File conventions:**
- One pub mod per concern at `src/<name>.rs`. Bookmarks (`src/bookmarks.rs`) and search (`src/search.rs`) are dedicated modules; large state files like `src/util.rs` host `App` and a few enums but keep most logic in their own modules. The new comment module follows this pattern: `src/comments.rs`.
- `src/lib.rs` declares modules with `pub mod <name>;`. The new module needs a line there.
- Box/overlay widgets live under `src/boxes/<name>.rs` and are listed in `src/boxes/mod.rs`. Each is a small file (≤110 lines) that owns its state and impls `ratatui::widgets::Widget`. The new sidebar follows: `src/boxes/comment_sidebar.rs`.

**Testing patterns:**
- Tests live inline in `#[cfg(test)] mod tests { ... }` at the bottom of the same file. Examples: `src/util.rs:233-328`, `src/bookmarks.rs:124+`, `src/search.rs:334+`.
- No external test fixtures, no mocking framework — plain `#[test]` functions, `assert_eq!`/`assert!`. Helpers like `app_with_width(w)` are defined inside the test module when shared.
- Render and event-handler logic is tested by exercising the `App` API directly (no terminal harness). Follow the same approach for the comment state machine.

**Integration points:**
- Key dispatch: `src/event_handler.rs::keyboard_mode_view` (line 217) is the central handler for `Mode::View`. New keys plug in here, gated on `app.comment_state` / `app.caret_mode`.
- Render: `src/main.rs::render_markdown` (line 276) renders the document, then runs a caret post-pass at line 349. The new anchor-highlight post-pass and sidebar render slot in immediately after the caret post-pass, before the help/footer rendering.
- Reset: `App::reset` at `src/util.rs:66-75` is the central place that clears per-document state (called when entering a new file). Comment state and the comments vec must reset there.

**Project conventions:**
- `CLAUDE.md` (workspace root) says "Always run the pre-commit hook before declaring a change is complete." There is no active hook in `.git/hooks/`, so in practice this means: run `cargo fmt --all`, `cargo clippy --all-targets -- -D warnings` (best-effort), and `cargo test` before each commit.
- Commit messages in this repo are short imperatives (`Update version`, `Update mermaid dependency`, `Let codeblocks be minimum config width`). Match that style.
- The user has uncommitted in-flight changes on `main` (caret mode, bookmarks, footer). Each task in this plan only stages files that task touches — never `git add -A`.

**Context loaded:** none — no `.superpowers/context/` root in this project; ad-hoc discovery only.

**Spec deviation note:** The spec's "Files to touch" section lists `src/util.rs` for `Comment` and `CommentState`. Following the discovered convention (dedicated module per concern, mirroring `bookmarks.rs` and `search.rs`), this plan puts them in a new `src/comments.rs` module instead. App fields and reset logic stay in `src/util.rs` as the spec called out.

---

## File Structure

**New files:**
- `src/comments.rs` — `Comment` struct, `CommentState` enum, helpers for range normalization and next/prev cycling. Owns its `#[cfg(test)] mod tests`.
- `src/boxes/comment_sidebar.rs` — `CommentSideBar` widget. Stateless renderer: receives a `&[Comment]`, the `active_comment`, the `sidebar_scroll`, and the optional `Editing` draft, and draws cards on the buffer.

**Modified files:**
- `src/lib.rs` — add `pub mod comments;`.
- `src/boxes/mod.rs` — add `pub mod comment_sidebar;`.
- `src/util.rs` — add `comments`, `comment_state`, `active_comment`, `sidebar_scroll` fields to `App`; update `reset`; thin helpers (`enter_comment_mode`, `exit_comment_mode`, `start_selecting`, `commit_selection_to_editing`, `save_draft`, `cycle_comment`).
- `src/util/keys.rs` — add `Action::EnterCommentMode`, `Action::StartCommentSelect` variants and a `KeyConfig` field for each (defaults `c` and `a`); wire them in `key_to_action`.
- `src/event_handler.rs` — comment-state-aware dispatch in `keyboard_mode_view`.
- `src/main.rs` — anchor-highlight post-pass and sidebar render after the caret post-pass in `render_markdown`.

---

## Task 1: Comment data types and state machine

**Files:**
- Create: `src/comments.rs`
- Modify: `src/lib.rs`

- [ ] **Step 1.1: Wire the new module into the crate**

Edit `src/lib.rs` to add the module:

```rust
pub mod bookmarks;
pub mod boxes;
pub mod comments;
pub mod event_handler;
pub mod nodes;
pub mod pages;
pub mod parser;
pub mod search;
pub mod util;

pub mod highlight;
```

- [ ] **Step 1.2: Write the failing tests**

Create `src/comments.rs` with the test module first. Tests cover range normalization and next/prev cycling:

```rust
use crate::util::Caret;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Comment {
    pub start: Caret,
    pub end: Caret,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum CommentState {
    #[default]
    Off,
    Browsing,
    Selecting { anchor: Caret },
    Editing { range: (Caret, Caret), draft: String, cursor: usize },
}

/// Returns `(start, end)` ordered so that `start <= end` in document order
/// (line first, then col). `end` is the exclusive end of the selection.
#[must_use]
pub fn normalize_range(a: Caret, b: Caret) -> (Caret, Caret) {
    unimplemented!()
}

/// Given the current active index and the number of comments, return the
/// next index, wrapping around. Returns `None` if `len == 0`.
#[must_use]
pub fn next_index(active: Option<usize>, len: usize) -> Option<usize> {
    unimplemented!()
}

/// Given the current active index and the number of comments, return the
/// previous index, wrapping around. Returns `None` if `len == 0`.
#[must_use]
pub fn prev_index(active: Option<usize>, len: usize) -> Option<usize> {
    unimplemented!()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(line: u16, col: u16) -> Caret {
        Caret { line, col }
    }

    #[test]
    fn normalize_orders_by_line_then_col() {
        assert_eq!(
            normalize_range(c(3, 10), c(1, 5)),
            (c(1, 5), c(3, 10))
        );
        assert_eq!(
            normalize_range(c(2, 8), c(2, 4)),
            (c(2, 4), c(2, 8))
        );
        assert_eq!(
            normalize_range(c(0, 0), c(0, 0)),
            (c(0, 0), c(0, 0))
        );
    }

    #[test]
    fn next_index_wraps() {
        assert_eq!(next_index(None, 0), None);
        assert_eq!(next_index(None, 3), Some(0));
        assert_eq!(next_index(Some(0), 3), Some(1));
        assert_eq!(next_index(Some(2), 3), Some(0));
    }

    #[test]
    fn prev_index_wraps() {
        assert_eq!(prev_index(None, 0), None);
        assert_eq!(prev_index(None, 3), Some(2));
        assert_eq!(prev_index(Some(0), 3), Some(2));
        assert_eq!(prev_index(Some(2), 3), Some(1));
    }

    #[test]
    fn comment_state_default_is_off() {
        assert_eq!(CommentState::default(), CommentState::Off);
    }
}
```

- [ ] **Step 1.3: Run tests to verify they fail**

Run: `cargo test -p md-tui --lib comments`
Expected: FAIL — the three helpers panic with `unimplemented!()` and the equality assertions never reach.

- [ ] **Step 1.4: Implement the helpers**

Replace the three `unimplemented!()` bodies in `src/comments.rs`:

```rust
#[must_use]
pub fn normalize_range(a: Caret, b: Caret) -> (Caret, Caret) {
    if (a.line, a.col) <= (b.line, b.col) {
        (a, b)
    } else {
        (b, a)
    }
}

#[must_use]
pub fn next_index(active: Option<usize>, len: usize) -> Option<usize> {
    if len == 0 {
        return None;
    }
    Some(match active {
        None => 0,
        Some(i) => (i + 1) % len,
    })
}

#[must_use]
pub fn prev_index(active: Option<usize>, len: usize) -> Option<usize> {
    if len == 0 {
        return None;
    }
    Some(match active {
        None => len - 1,
        Some(i) => (i + len - 1) % len,
    })
}
```

- [ ] **Step 1.5: Run tests to verify they pass**

Run: `cargo test -p md-tui --lib comments`
Expected: PASS — 4 tests in `comments::tests`.

- [ ] **Step 1.6: Format and lint**

Run: `cargo fmt --all && cargo clippy --all-targets -- -D warnings`
Expected: clean exit. Fix any warnings before proceeding.

- [ ] **Step 1.7: Commit**

```bash
git add src/comments.rs src/lib.rs
git commit -m "Add Comment types and state machine helpers"
```

---

## Task 2: App state — fields and reset

**Files:**
- Modify: `src/util.rs`

- [ ] **Step 2.1: Write the failing tests**

Add these tests inside the existing `#[cfg(test)] mod tests` in `src/util.rs` (alongside the existing caret tests):

```rust
    #[test]
    fn reset_clears_comment_state() {
        use crate::comments::{Comment, CommentState};
        let mut app = app_with_width(40);
        app.comments.push(Comment {
            start: Caret { line: 1, col: 0 },
            end: Caret { line: 1, col: 4 },
            text: "x".into(),
        });
        app.comment_state = CommentState::Browsing;
        app.active_comment = Some(0);
        app.sidebar_scroll = 7;
        app.reset();
        assert!(app.comments.is_empty());
        assert_eq!(app.comment_state, CommentState::Off);
        assert_eq!(app.active_comment, None);
        assert_eq!(app.sidebar_scroll, 0);
    }

    #[test]
    fn default_app_has_off_comment_state() {
        use crate::comments::CommentState;
        let app = App::default();
        assert!(app.comments.is_empty());
        assert_eq!(app.comment_state, CommentState::Off);
        assert_eq!(app.active_comment, None);
        assert_eq!(app.sidebar_scroll, 0);
    }
```

- [ ] **Step 2.2: Run tests to verify they fail**

Run: `cargo test -p md-tui --lib util::tests::reset_clears_comment_state util::tests::default_app_has_off_comment_state`
Expected: FAIL — the new fields don't exist on `App`, compilation error.

- [ ] **Step 2.3: Add fields to `App`**

In `src/util.rs`, update the imports at the top and the `App` struct:

```rust
use crate::boxes::{errorbox::ErrorBox, help_box::HelpBox, linkbox::LinkBox, searchbox::SearchBox};
use crate::comments::{Comment, CommentState};

// ... (existing items unchanged) ...

#[derive(Default, Clone)]
pub struct App {
    pub vertical_scroll: u16,
    width: u16,
    pub selected: bool,
    pub select_index: usize,
    pub mode: Mode,
    pub boxes: Boxes,
    pub history: JumpHistory,
    pub search_box: SearchBox,
    pub message_box: ErrorBox,
    pub help_box: HelpBox,
    pub link_box: LinkBox,
    pub caret_mode: bool,
    pub caret: Caret,
    pub bookmarks: BTreeMap<char, Caret>,
    pub bookmark_origin_width: u16,
    pub pending_input: Option<PendingInput>,
    pub comments: Vec<Comment>,
    pub comment_state: CommentState,
    pub active_comment: Option<usize>,
    pub sidebar_scroll: u16,
}
```

- [ ] **Step 2.4: Update `App::reset`**

Replace the body of `App::reset` (`src/util.rs:66-75`) with:

```rust
    pub fn reset(&mut self) {
        self.vertical_scroll = 0;
        self.selected = false;
        self.select_index = 0;
        self.boxes = Boxes::None;
        self.help_box.close();
        self.caret = Caret::default();
        self.caret_mode = false;
        self.pending_input = None;
        self.comments.clear();
        self.comment_state = CommentState::Off;
        self.active_comment = None;
        self.sidebar_scroll = 0;
    }
```

- [ ] **Step 2.5: Run tests to verify they pass**

Run: `cargo test -p md-tui --lib util::tests`
Expected: PASS — both new tests plus existing caret/bookmark tests.

- [ ] **Step 2.6: Format and lint**

Run: `cargo fmt --all && cargo clippy --all-targets -- -D warnings`
Expected: clean exit.

- [ ] **Step 2.7: Commit**

```bash
git add src/util.rs
git commit -m "Add comment state fields to App"
```

---

## Task 3: App helpers for comment-mode transitions

**Files:**
- Modify: `src/util.rs`

These helpers are called from the event handler. Keeping them on `App` lets us unit-test the state machine without going through key dispatch.

- [ ] **Step 3.1: Write the failing tests**

Append to the `#[cfg(test)] mod tests` in `src/util.rs`:

```rust
    #[test]
    fn enter_comment_mode_requires_caret_mode() {
        use crate::comments::CommentState;
        let mut app = app_with_width(40);
        // not in caret mode
        assert!(!app.enter_comment_mode());
        assert_eq!(app.comment_state, CommentState::Off);
        // now turn caret mode on
        app.caret_mode = true;
        assert!(app.enter_comment_mode());
        assert_eq!(app.comment_state, CommentState::Browsing);
    }

    #[test]
    fn exit_comment_mode_clears_state_keeps_comments() {
        use crate::comments::{Comment, CommentState};
        let mut app = app_with_width(40);
        app.comments.push(Comment {
            start: Caret { line: 0, col: 0 },
            end: Caret { line: 0, col: 1 },
            text: "y".into(),
        });
        app.comment_state = CommentState::Browsing;
        app.active_comment = Some(0);
        app.exit_comment_mode();
        assert_eq!(app.comment_state, CommentState::Off);
        assert_eq!(app.active_comment, None);
        assert_eq!(app.comments.len(), 1);
    }

    #[test]
    fn start_selecting_anchors_at_current_caret() {
        use crate::comments::CommentState;
        let mut app = app_with_width(40);
        app.caret_mode = true;
        app.caret = Caret { line: 5, col: 12 };
        app.comment_state = CommentState::Browsing;
        app.start_selecting();
        assert_eq!(
            app.comment_state,
            CommentState::Selecting { anchor: Caret { line: 5, col: 12 } }
        );
    }

    #[test]
    fn start_selecting_only_works_in_browsing() {
        use crate::comments::CommentState;
        let mut app = app_with_width(40);
        app.comment_state = CommentState::Off;
        app.start_selecting();
        assert_eq!(app.comment_state, CommentState::Off);
    }

    #[test]
    fn commit_selection_to_editing_normalizes_range() {
        use crate::comments::CommentState;
        let mut app = app_with_width(40);
        app.caret_mode = true;
        app.caret = Caret { line: 1, col: 4 };
        app.comment_state = CommentState::Selecting {
            anchor: Caret { line: 3, col: 10 },
        };
        app.commit_selection_to_editing();
        match app.comment_state {
            CommentState::Editing { range, draft, cursor } => {
                assert_eq!(range, (Caret { line: 1, col: 4 }, Caret { line: 3, col: 10 }));
                assert!(draft.is_empty());
                assert_eq!(cursor, 0);
            }
            other => panic!("expected Editing, got {other:?}"),
        }
    }

    #[test]
    fn save_draft_pushes_comment_and_browses() {
        use crate::comments::CommentState;
        let mut app = app_with_width(40);
        let range = (Caret { line: 0, col: 0 }, Caret { line: 0, col: 5 });
        app.comment_state = CommentState::Editing {
            range,
            draft: "hello".into(),
            cursor: 5,
        };
        app.save_draft();
        assert_eq!(app.comments.len(), 1);
        assert_eq!(app.comments[0].start, range.0);
        assert_eq!(app.comments[0].end, range.1);
        assert_eq!(app.comments[0].text, "hello");
        assert_eq!(app.active_comment, Some(0));
        assert_eq!(app.comment_state, CommentState::Browsing);
    }

    #[test]
    fn cycle_comment_next_wraps_and_moves_caret_to_start() {
        use crate::comments::{Comment, CommentState};
        let mut app = app_with_width(40);
        app.caret_mode = true;
        app.comment_state = CommentState::Browsing;
        app.comments.push(Comment {
            start: Caret { line: 5, col: 0 },
            end: Caret { line: 5, col: 3 },
            text: "a".into(),
        });
        app.comments.push(Comment {
            start: Caret { line: 20, col: 2 },
            end: Caret { line: 20, col: 5 },
            text: "b".into(),
        });
        // First call from None goes to index 0
        app.cycle_comment(true, 100, 20);
        assert_eq!(app.active_comment, Some(0));
        assert_eq!(app.caret, Caret { line: 5, col: 0 });
        // Next goes to 1
        app.cycle_comment(true, 100, 20);
        assert_eq!(app.active_comment, Some(1));
        assert_eq!(app.caret, Caret { line: 20, col: 2 });
        // Wraps back to 0
        app.cycle_comment(true, 100, 20);
        assert_eq!(app.active_comment, Some(0));
    }

    #[test]
    fn cycle_comment_no_op_when_empty() {
        use crate::comments::CommentState;
        let mut app = app_with_width(40);
        app.caret_mode = true;
        app.comment_state = CommentState::Browsing;
        app.cycle_comment(true, 100, 20);
        assert_eq!(app.active_comment, None);
    }

    #[test]
    fn cycle_comment_centers_main_pane() {
        use crate::comments::{Comment, CommentState};
        let mut app = app_with_width(40);
        app.caret_mode = true;
        app.comment_state = CommentState::Browsing;
        app.comments.push(Comment {
            start: Caret { line: 50, col: 0 },
            end: Caret { line: 50, col: 1 },
            text: "x".into(),
        });
        app.cycle_comment(true, 200, 20);
        assert_eq!(app.vertical_scroll, 50u16.saturating_sub(10));
    }
```

- [ ] **Step 3.2: Run tests to verify they fail**

Run: `cargo test -p md-tui --lib util::tests`
Expected: FAIL — methods don't exist; compilation error.

- [ ] **Step 3.3: Implement the helpers**

Add to the `impl App { ... }` block in `src/util.rs` (immediately after the existing `jump_bookmark` method):

```rust
    pub fn enter_comment_mode(&mut self) -> bool {
        if !self.caret_mode {
            return false;
        }
        self.comment_state = CommentState::Browsing;
        true
    }

    pub fn exit_comment_mode(&mut self) {
        self.comment_state = CommentState::Off;
        self.active_comment = None;
        self.sidebar_scroll = 0;
    }

    pub fn start_selecting(&mut self) {
        if matches!(self.comment_state, CommentState::Browsing) {
            self.comment_state = CommentState::Selecting { anchor: self.caret };
        }
    }

    pub fn commit_selection_to_editing(&mut self) {
        if let CommentState::Selecting { anchor } = self.comment_state {
            let range = crate::comments::normalize_range(anchor, self.caret);
            self.comment_state = CommentState::Editing {
                range,
                draft: String::new(),
                cursor: 0,
            };
        }
    }

    pub fn save_draft(&mut self) {
        if let CommentState::Editing { range, draft, .. } =
            std::mem::replace(&mut self.comment_state, CommentState::Browsing)
        {
            let comment = Comment {
                start: range.0,
                end: range.1,
                text: draft,
            };
            self.comments.push(comment);
            self.active_comment = Some(self.comments.len() - 1);
        }
    }

    pub fn cancel_editing(&mut self) {
        if matches!(self.comment_state, CommentState::Editing { .. } | CommentState::Selecting { .. }) {
            self.comment_state = CommentState::Browsing;
        }
    }

    pub fn cycle_comment(&mut self, forward: bool, max_line: u16, viewport_height: u16) {
        let len = self.comments.len();
        if len == 0 {
            return;
        }
        let next = if forward {
            crate::comments::next_index(self.active_comment, len)
        } else {
            crate::comments::prev_index(self.active_comment, len)
        };
        self.active_comment = next;
        if let Some(idx) = next {
            let target = self.comments[idx].start;
            self.caret = target;
            // Center main pane on the anchor's line.
            let _ = max_line; // currently unused; reserved for clamping if needed
            let center = viewport_height / 2;
            self.vertical_scroll = self.caret.line.saturating_sub(center);
        }
    }
```

Add the new imports near the top of `src/util.rs` (extend the existing `use crate::comments::...` line if necessary):

```rust
use crate::comments::{Comment, CommentState};
```

- [ ] **Step 3.4: Run tests to verify they pass**

Run: `cargo test -p md-tui --lib util::tests`
Expected: PASS — all new and existing tests in `util::tests`.

- [ ] **Step 3.5: Format and lint**

Run: `cargo fmt --all && cargo clippy --all-targets -- -D warnings`
Expected: clean exit.

- [ ] **Step 3.6: Commit**

```bash
git add src/util.rs
git commit -m "Add App helpers for comment mode transitions"
```

---

## Task 4: New actions and key bindings

**Files:**
- Modify: `src/util/keys.rs`

- [ ] **Step 4.1: Add `Action` variants and `KeyConfig` fields**

In `src/util/keys.rs`, extend the `Action` enum:

```rust
pub enum Action {
    Up,
    Down,
    PageUp,
    PageDown,
    HalfPageUp,
    HalfPageDown,
    Search,
    SelectLink,
    SelectLinkAlt,
    SearchNext,
    SearchPrevious,
    Edit,
    Hover,
    Enter,
    Escape,
    ToTop,
    ToBottom,
    Help,
    Back,
    ToFileTree,
    Sort,
    ToggleCaretMode,
    CaretLineStart,
    CaretLineEnd,
    BookmarkSetPending,
    BookmarkJumpPending,
    EnterCommentMode,
    StartCommentSelect,
    None,
}
```

Extend the `KeyConfig` struct:

```rust
pub struct KeyConfig {
    pub up: char,
    pub down: char,
    pub page_up: char,
    pub page_down: char,
    pub half_page_up: char,
    pub half_page_down: char,
    pub search: char,
    pub search_next: char,
    pub search_previous: char,
    pub select_link: char,
    pub select_link_alt: char,
    pub edit: char,
    pub hover: char,
    pub top: char,
    pub bottom: char,
    pub back: char,
    pub file_tree: char,
    pub sort: char,
    pub toggle_caret: char,
    pub bookmark_set: char,
    pub bookmark_jump: char,
    pub comment: char,
    pub comment_select: char,
}
```

- [ ] **Step 4.2: Wire `key_to_action` and the config defaults**

In `key_to_action`, add the two new bindings just below the existing `bookmark_jump` check (right before `if c == '0'`):

```rust
            if c == KEY_CONFIG.comment {
                return Action::EnterCommentMode;
            }

            if c == KEY_CONFIG.comment_select {
                return Action::StartCommentSelect;
            }
```

In the `KEY_CONFIG` `LazyLock` builder, add the two new defaults at the bottom of the `KeyConfig { ... }` literal:

```rust
        comment: settings.get::<char>("comment").unwrap_or('c'),
        comment_select: settings.get::<char>("comment_select").unwrap_or('a'),
```

- [ ] **Step 4.3: Verify it compiles**

Run: `cargo build`
Expected: clean build.

- [ ] **Step 4.4: Format and lint**

Run: `cargo fmt --all && cargo clippy --all-targets -- -D warnings`
Expected: clean exit.

- [ ] **Step 4.5: Commit**

```bash
git add src/util/keys.rs
git commit -m "Add comment-mode actions and key bindings"
```

---

## Task 5: CommentSideBar widget

**Files:**
- Create: `src/boxes/comment_sidebar.rs`
- Modify: `src/boxes/mod.rs`

This widget is stateless (it does not own `comments` or scroll). The caller passes in everything via a small builder so the sidebar can be rendered without holding a borrow into `App`.

- [ ] **Step 5.1: Wire the new module**

Edit `src/boxes/mod.rs`:

```rust
pub mod comment_sidebar;
pub mod errorbox;
pub mod help_box;
pub mod linkbox;
pub mod searchbox;
```

- [ ] **Step 5.2: Create the widget file**

Create `src/boxes/comment_sidebar.rs`:

```rust
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, Paragraph, Widget, Wrap},
};

use crate::comments::Comment;

pub const SIDEBAR_WIDTH: u16 = 32;
const CARD_GAP: u16 = 1; // blank line between cards

/// What the sidebar should show in the active card.
#[derive(Debug)]
pub enum ActiveDisplay<'a> {
    /// Show the saved comment text.
    Saved,
    /// Show a draft being edited, with a visible cursor cell.
    Editing { draft: &'a str, cursor: usize },
}

#[derive(Debug)]
pub struct CommentSideBar<'a> {
    pub comments: &'a [Comment],
    pub active: Option<usize>,
    pub scroll: u16,
    pub active_display: ActiveDisplay<'a>,
}

impl<'a> Widget for CommentSideBar<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        // Outer frame: left border separating it from the markdown.
        let block = Block::default()
            .borders(Borders::LEFT)
            .style(Style::default().bg(Color::Black));
        let inner = block.inner(area);
        block.render(area, buf);

        if self.comments.is_empty() {
            let hint = Paragraph::new("(no comments yet — press `a` to add)")
                .style(Style::default().add_modifier(Modifier::DIM))
                .wrap(Wrap { trim: true });
            hint.render(inner, buf);
            return;
        }

        // Render cards in a vertical stack starting at -scroll.
        let mut y = inner.y as i32 - self.scroll as i32;
        for (i, comment) in self.comments.iter().enumerate() {
            let active = self.active == Some(i);
            let card_h = card_height(comment, &self.active_display, active, inner.width);
            let card_area = clip_card_area(inner, y, card_h);
            if let Some(card_area) = card_area {
                render_card(card_area, buf, comment, active, &self.active_display);
            }
            y += card_h as i32 + CARD_GAP as i32;
            if y >= (inner.y + inner.height) as i32 {
                break;
            }
        }
    }
}

fn card_height(
    comment: &Comment,
    active_display: &ActiveDisplay,
    active: bool,
    width: u16,
) -> u16 {
    // 1 line excerpt + 1 blank + N lines of body + 1 line bottom border
    let body_lines = if active {
        match active_display {
            ActiveDisplay::Editing { .. } => 1,
            ActiveDisplay::Saved => wrap_lines(&comment.text, width),
        }
    } else {
        wrap_lines(&comment.text, width)
    };
    1 + 1 + body_lines + 1
}

fn wrap_lines(text: &str, width: u16) -> u16 {
    if width == 0 {
        return 1;
    }
    let w = width as usize;
    let mut lines: u16 = 0;
    for paragraph in text.split('\n') {
        if paragraph.is_empty() {
            lines += 1;
            continue;
        }
        let n = paragraph.chars().count();
        lines += ((n + w - 1) / w).max(1) as u16;
    }
    lines.max(1)
}

fn clip_card_area(inner: Rect, top: i32, height: u16) -> Option<Rect> {
    let bottom = top + height as i32;
    let inner_top = inner.y as i32;
    let inner_bottom = (inner.y + inner.height) as i32;
    if bottom <= inner_top || top >= inner_bottom {
        return None;
    }
    let visible_top = top.max(inner_top);
    let visible_bottom = bottom.min(inner_bottom);
    let h = (visible_bottom - visible_top) as u16;
    Some(Rect {
        x: inner.x,
        y: visible_top as u16,
        width: inner.width,
        height: h,
    })
}

fn render_card(
    area: Rect,
    buf: &mut Buffer,
    comment: &Comment,
    active: bool,
    active_display: &ActiveDisplay,
) {
    // Excerpt line: brackets around what's anchored.
    let excerpt = format_excerpt(comment, area.width);
    let excerpt_para = Paragraph::new(excerpt).style(card_style(active));
    let excerpt_area = Rect { height: 1.min(area.height), ..area };
    excerpt_para.render(excerpt_area, buf);

    if area.height <= 2 {
        return;
    }

    // Blank gap line
    // (no widget needed; the bg colour is filled by the outer block)

    // Body
    let body_y = area.y + 2;
    let body_h = area.height.saturating_sub(3);
    let body_area = Rect {
        x: area.x,
        y: body_y,
        width: area.width,
        height: body_h,
    };

    if active {
        match active_display {
            ActiveDisplay::Editing { draft, cursor } => {
                let para = Paragraph::new(*draft).style(card_style(true));
                para.render(body_area, buf);
                // Cursor cell: position cursor at column min(*cursor, width-1)
                if body_h > 0 {
                    let cx = body_area.x + (*cursor as u16).min(body_area.width.saturating_sub(1));
                    let cy = body_area.y;
                    if let Some(cell) = buf.cell_mut((cx, cy)) {
                        cell.set_style(Style::default().add_modifier(Modifier::REVERSED));
                    }
                }
            }
            ActiveDisplay::Saved => {
                let para = Paragraph::new(comment.text.clone())
                    .style(card_style(true))
                    .wrap(Wrap { trim: true });
                para.render(body_area, buf);
            }
        }
    } else {
        let para = Paragraph::new(comment.text.clone())
            .style(card_style(false))
            .wrap(Wrap { trim: true });
        para.render(body_area, buf);
    }

    // Bottom border line: a row of '─' characters at the last visible row.
    let border_y = area.y + area.height.saturating_sub(1);
    if border_y >= area.y {
        for x in area.x..area.x + area.width {
            if let Some(cell) = buf.cell_mut((x, border_y)) {
                cell.set_char('─');
            }
        }
    }
}

fn card_style(active: bool) -> Style {
    if active {
        Style::default().add_modifier(Modifier::BOLD)
    } else {
        Style::default().add_modifier(Modifier::DIM)
    }
}

fn format_excerpt(comment: &Comment, width: u16) -> String {
    let mut s = String::new();
    s.push('[');
    s.push_str(&format!(
        "L{}c{}..L{}c{}",
        comment.start.line, comment.start.col, comment.end.line, comment.end.col
    ));
    s.push(']');
    if (s.chars().count() as u16) > width.saturating_sub(1) {
        // Truncate to fit
        let max = width.saturating_sub(2) as usize;
        s = s.chars().take(max).collect::<String>();
        s.push('…');
    }
    s
}
```

> Note: the excerpt currently shows the `(line, col)` range. We don't have access to the rendered character grid here without plumbing it through, and since persistence is out of scope for this iteration, a positional excerpt is sufficient. A future iteration can sample the rendered text via `ComponentRoot`.

- [ ] **Step 5.3: Verify it compiles**

Run: `cargo build`
Expected: clean build.

- [ ] **Step 5.4: Format and lint**

Run: `cargo fmt --all && cargo clippy --all-targets -- -D warnings`
Expected: clean exit. (You may have to add `#[allow(dead_code)]` to `wrap_lines` etc. if clippy complains; only do so if it does.)

- [ ] **Step 5.5: Commit**

```bash
git add src/boxes/comment_sidebar.rs src/boxes/mod.rs
git commit -m "Add CommentSideBar widget"
```

---

## Task 6: Event handler — entry, selection, editing

**Files:**
- Modify: `src/event_handler.rs`

This is the core wiring task. We add a comment-state-aware dispatch that runs *before* the existing caret-mode dispatch. When `comment_state == Off`, behavior is unchanged. When set, comment-mode keys are intercepted.

- [ ] **Step 6.1: Import the new types**

At the top of `src/event_handler.rs`, extend the imports:

```rust
use crate::comments::CommentState;
```

- [ ] **Step 6.2: Add a comment-mode dispatch helper**

In `src/event_handler.rs`, just below `viewport_height` and before the existing `persist_marks`, add a new helper that returns `true` if the key was consumed by comment mode:

```rust
fn handle_comment_mode_key(
    key: KeyCode,
    app: &mut App,
    markdown: &ComponentRoot,
    vh: u16,
) -> bool {
    use Action::*;
    match app.comment_state.clone() {
        CommentState::Off => false,
        CommentState::Browsing => match key_to_action(key) {
            EnterCommentMode | Escape => {
                app.exit_comment_mode();
                true
            }
            StartCommentSelect => {
                app.start_selecting();
                true
            }
            SearchNext => {
                let max = markdown.height();
                app.cycle_comment(true, max, vh);
                true
            }
            SearchPrevious => {
                let max = markdown.height();
                app.cycle_comment(false, max, vh);
                true
            }
            // Caret motion still works while browsing — fall through.
            _ => false,
        },
        CommentState::Selecting { .. } => match key_to_action(key) {
            Enter => {
                app.commit_selection_to_editing();
                true
            }
            Escape => {
                app.cancel_editing();
                true
            }
            // Caret motion in Selecting falls through to caret-mode handler so
            // the caret moves and the live highlight follows.
            _ => false,
        },
        CommentState::Editing { .. } => {
            match key {
                KeyCode::Esc => {
                    app.cancel_editing();
                    return true;
                }
                KeyCode::Enter => {
                    app.save_draft();
                    return true;
                }
                KeyCode::Backspace => {
                    if let CommentState::Editing { draft, cursor, .. } = &mut app.comment_state
                        && *cursor > 0
                    {
                        draft.remove(*cursor - 1);
                        *cursor -= 1;
                    }
                    return true;
                }
                KeyCode::Char(c) => {
                    if let CommentState::Editing { draft, cursor, .. } = &mut app.comment_state {
                        draft.insert(*cursor, c);
                        *cursor += 1;
                    }
                    return true;
                }
                _ => {}
            }
            // Anything else (arrow keys etc.) is a no-op while editing.
            true
        }
    }
}
```

- [ ] **Step 6.3: Hook the helper into `keyboard_mode_view`**

In `src/event_handler.rs::keyboard_mode_view`, just after `let vh = viewport_height(height);` (currently line 274 inside the `Boxes::None` arm), but before the pending-input block, insert:

```rust
            // Comment mode dispatch. Runs first so it can intercept keys before
            // pending-input or caret-mode logic. Returns true when the key was
            // consumed, false to fall through to the rest of the handler.
            if handle_comment_mode_key(key, app, markdown, vh) {
                return KeyBoardAction::Continue;
            }
```

- [ ] **Step 6.4: Make the global `c` key enter comment mode**

Below the existing `Action::ToggleCaretMode` arm in the "Universal new actions" match (around `src/event_handler.rs:316`), add:

```rust
                Action::EnterCommentMode => {
                    if app.caret_mode {
                        app.enter_comment_mode();
                    }
                    return KeyBoardAction::Continue;
                }
```

This runs only when `comment_state == Off` (because the helper above returned `false` in that case). When already in comment mode, `c` is consumed by the helper and never reaches this arm.

- [ ] **Step 6.5: Make caret-mode toggle off also exit comment mode**

In `src/util.rs::toggle_caret_mode`, add a guard that clears comment state when leaving caret mode. Replace the method with:

```rust
    pub fn toggle_caret_mode(&mut self, viewport_height: u16) {
        self.caret_mode = !self.caret_mode;
        if self.caret_mode {
            if viewport_height == 0
                || self.caret.line < self.vertical_scroll
                || self.caret.line >= self.vertical_scroll + viewport_height
            {
                self.caret.line = self.vertical_scroll;
                self.caret.col = 0;
            }
        } else {
            self.exit_comment_mode();
        }
    }
```

- [ ] **Step 6.6: Build and run all tests**

Run: `cargo test -p md-tui`
Expected: PASS — all existing tests plus the comment-state-machine tests from Tasks 1-3.

- [ ] **Step 6.7: Format and lint**

Run: `cargo fmt --all && cargo clippy --all-targets -- -D warnings`
Expected: clean exit.

- [ ] **Step 6.8: Commit**

```bash
git add src/event_handler.rs src/util.rs
git commit -m "Wire comment-mode dispatch into event handler"
```

---

## Task 7: Render integration — anchor highlight + sidebar

**Files:**
- Modify: `src/main.rs`

- [ ] **Step 7.1: Add an anchor-highlight post-pass**

In `src/main.rs`, just after the caret post-pass at line 355 (`let buf = f.buffer_mut(); ... cell.set_style(...)`), add a new function call and its definition. Inside `render_markdown`, immediately after the caret post-pass `if let Some((cx, cy)) = caret_screen_pos(app, &area) { ... }` block:

```rust
    apply_comment_highlights(f, app, &area);
```

Then add this function near the bottom of `src/main.rs`, alongside `caret_screen_pos`:

```rust
fn apply_comment_highlights(f: &mut Frame, app: &App, area: &Rect) {
    use md_tui::comments::CommentState;
    use ratatui::style::Color;

    let buf = f.buffer_mut();

    // Helper: paint cells in a (start, end) range using the given style.
    let mut paint = |start: md_tui::util::Caret, end: md_tui::util::Caret, style: Style| {
        if start == end {
            return;
        }
        let line_min = app.vertical_scroll;
        let line_max = app.vertical_scroll + area.height;
        for line in start.line..=end.line {
            if line < line_min || line >= line_max {
                continue;
            }
            let row = area.y + (line - line_min);
            let col_start = if line == start.line { start.col } else { 0 };
            let col_end = if line == end.line {
                end.col
            } else {
                area.width
            };
            let x0 = area.x + col_start.min(area.width.saturating_sub(1));
            let x1 = area.x + col_end.min(area.width);
            for x in x0..x1 {
                if let Some(cell) = buf.cell_mut((x, row)) {
                    let prev = cell.style();
                    cell.set_style(prev.patch(style));
                }
            }
        }
    };

    // Saved comments first (dim background).
    let dim_style = Style::default().bg(Color::DarkGray);
    let active_style = Style::default().bg(Color::Blue);
    for (i, comment) in app.comments.iter().enumerate() {
        let style = if app.active_comment == Some(i) {
            active_style
        } else {
            dim_style
        };
        paint(comment.start, comment.end, style);
    }

    // In-progress selection (Selecting state) uses the active style.
    if let CommentState::Selecting { anchor } = app.comment_state {
        let (s, e) = md_tui::comments::normalize_range(anchor, app.caret);
        paint(s, e, active_style);
    }
}
```

> The closure-borrow pattern keeps `paint` reusable for the in-progress selection path. If clippy or borrow-checker complains, inline the closure into two near-identical loops.

- [ ] **Step 7.2: Render the sidebar**

In `src/main.rs::render_markdown`, just before the help-bar block (around line 357), insert:

```rust
    use md_tui::boxes::comment_sidebar::{CommentSideBar, ActiveDisplay, SIDEBAR_WIDTH};
    use md_tui::comments::CommentState;
    if app.comment_state != CommentState::Off {
        let sb_x = area.x + area.width.saturating_sub(SIDEBAR_WIDTH);
        let sb_area = Rect {
            x: sb_x,
            y: area.y,
            width: area.width.min(SIDEBAR_WIDTH),
            height: area.height,
        };
        let active_display = match &app.comment_state {
            CommentState::Editing { draft, cursor, .. } => ActiveDisplay::Editing {
                draft,
                cursor: *cursor,
            },
            _ => ActiveDisplay::Saved,
        };
        let sidebar = CommentSideBar {
            comments: &app.comments,
            active: app.active_comment,
            scroll: app.sidebar_scroll,
            active_display,
        };
        f.render_widget(Clear, sb_area);
        f.render_widget(sidebar, sb_area);
    }
```

- [ ] **Step 7.3: Build**

Run: `cargo build`
Expected: clean build. Fix any borrow-checker issues by inlining the `paint` closure into two explicit loops if necessary.

- [ ] **Step 7.4: Run all tests**

Run: `cargo test -p md-tui`
Expected: PASS — no test changes here, just confirming nothing broke.

- [ ] **Step 7.5: Format and lint**

Run: `cargo fmt --all && cargo clippy --all-targets -- -D warnings`
Expected: clean exit.

- [ ] **Step 7.6: Commit**

```bash
git add src/main.rs
git commit -m "Render comment-mode sidebar and anchor highlights"
```

---

## Task 8: Manual smoke test

**Files:** none (manual run)

- [ ] **Step 8.1: Build a release-ish binary and run it on the README**

Run: `cargo run -- README.md`

- [ ] **Step 8.2: Walk the happy path**

Execute the following sequence and verify each step:

1. Press `v` — enter caret mode. Caret is visible top-left.
2. Move with `j`/`l` to position the caret. Confirm caret moves.
3. Press `c` — sidebar appears on the right with a hint "(no comments yet — press `a` to add)". Caret still movable.
4. Press `a` — selection starts at caret. Move `l` a few times. Confirm the cells from anchor to caret are highlighted.
5. Press `Enter` — sidebar gains a card with the excerpt and an empty body line with a visible cursor.
6. Type "first comment" — characters appear in the card body in real time.
7. Press `Enter` — comment is saved. Card body now shows "first comment" non-editable. `active_comment` is the new card (highlighted brighter).
8. Repeat steps 2-7 to add a second comment in a different region.
9. Press `n` — caret jumps to the second comment's start. Both panes recenter.
10. Press `N` — caret jumps back to the first.
11. Press `Esc` — sidebar closes. Highlights remain visible? They should be cleared (because we render highlights only when `comment_state != Off`). Actually the spec says highlights are shown while in comment mode — confirm that and adjust `apply_comment_highlights` accordingly if the desired behaviour is "hide when comment_state == Off".
12. Press `c` — sidebar reopens with both comments still present (in-memory only).
13. Press `v` — caret mode toggles off. Sidebar disappears. `comment_state` is `Off`.

- [ ] **Step 8.3: If the highlight-visibility behaviour from step 11 needs adjusting**

If the desired behaviour is "highlights only while in comment mode", wrap the body of `apply_comment_highlights` in an early-return:

```rust
fn apply_comment_highlights(f: &mut Frame, app: &App, area: &Rect) {
    use md_tui::comments::CommentState;
    if app.comment_state == CommentState::Off {
        return;
    }
    // ... rest unchanged
}
```

Re-run `cargo build && cargo test -p md-tui && cargo fmt --all && cargo clippy --all-targets -- -D warnings`.

- [ ] **Step 8.4: Commit any adjustments**

```bash
git add src/main.rs
git commit -m "Hide comment highlights when comment mode is off"
```

(Skip this commit if no adjustment was needed.)

---

## Self-review notes (written during plan authoring)

**Spec coverage:**
- Data model (Comment, CommentState, App fields) → Tasks 1, 2.
- State transitions → Task 3 with TDD.
- Key bindings (`c`, `a`, `n`/`N`, Esc, Enter) → Tasks 4, 6.
- Sidebar rendering and centering → Tasks 5, 7.
- Anchor highlight post-pass → Task 7.
- Caret-mode-off forces comment-state Off → Task 6 step 6.5.
- Manual end-to-end verification → Task 8.

**Placeholder scan:** No TBDs/TODOs in plan steps. The closure note in Task 7 step 7.1 is a fallback hint for borrow-checker issues, not a placeholder.

**Type consistency:** Methods on `App` referenced by Task 6 (`enter_comment_mode`, `exit_comment_mode`, `start_selecting`, `commit_selection_to_editing`, `save_draft`, `cancel_editing`, `cycle_comment`) are all defined in Task 3. `Comment`, `CommentState`, `next_index`, `prev_index`, `normalize_range` defined in Task 1 and used in Tasks 3, 6, 7. `SIDEBAR_WIDTH`, `ActiveDisplay`, `CommentSideBar` defined in Task 5 and used in Task 7.

**Discovery referenced:** File Structure lists `src/comments.rs` (matching `bookmarks.rs` / `search.rs` convention); sidebar at `src/boxes/comment_sidebar.rs` (matching `searchbox.rs` / `linkbox.rs`); inline tests with `#[cfg(test)] mod tests` (matching every other module). Spec deviation noted explicitly.
