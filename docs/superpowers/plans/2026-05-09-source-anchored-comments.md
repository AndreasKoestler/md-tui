# Source-Anchored Comments Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Re-anchor comments to markdown source spans while continuing to render and navigate them using current rendered-view spans.

**Architecture:** Make source span the canonical identity for comments and treat rendered ranges as a derived projection from the transformed document tree. Add source span metadata at parse time, carry it through `ParseNode -> Word -> TextComponent`, and build rendered-range projections from the post-transform component tree. Comment editing, matching, and persistence use source spans; sidebar labels, jumps, and highlights use projected rendered spans.

**Tech Stack:** Rust, pest parser spans, ratatui, existing `ComponentRoot` / `TextComponent` transform pipeline

## Discovery

**Similar implementations:** Existing comments are anchored to rendered `Caret` ranges in `src/comments.rs`, `src/util.rs`, and `src/main.rs`. Search highlighting already performs a post-render buffer pass in `src/main.rs`, which is the closest existing projection consumer.

**File conventions:** Core app state lives in `src/util.rs`; domain types live in dedicated modules like `src/comments.rs`; parser concerns stay in `src/parser.rs`; render-time widgets live under `src/boxes/`.

**Testing patterns:** Behavior is covered primarily with unit tests in `src/util.rs` and widget-level tests in `src/boxes/comment_sidebar.rs`. No TUI harness exists; mapping logic should be testable as pure helpers.

**Integration points:** `parse_markdown` in `src/parser.rs` builds a `ComponentRoot`, then `root.transform(width)` rewrites the content into rendered lines. Comment UI integrates in `src/event_handler.rs` and `src/main.rs`.

**Project conventions:** The codebase prefers small focused helpers and direct unit tests over integration scaffolding. Existing comment logic already uses exact range identity and render-time projection for highlights.

**Context loaded:** none — ad-hoc discovery

---

## Sketch

### Canonical Model

- Add a `SourcePos` and `SourceSpan` type.
- `SourcePos` should include:
  - `line: u32`
  - `column: u32`
  - `byte: usize`
- `SourceSpan` should include:
  - `start: SourcePos`
  - `end: SourcePos`
- Store `SourceSpan` on parsed nodes and on renderable words.
- Change `Comment` to store:
  - `source: SourceSpan`
  - `text: String`

The important choice is that **byte offsets are canonical** for splitting and matching, while line/column are retained for UI/debugging. Line/column alone is not enough because wrapping and word splitting operate on substrings.

### Projection Model

- Add a render-projection type, for example:
  - `RenderedRange { start: Caret, end: Caret }`
  - `ProjectedCommentAnchor { source: SourceSpan, rendered: Vec<RenderedRange> }`
- Build projection from the post-transform `ComponentRoot`, not from parser output.
- Each renderable word contributes one or more `(source subspan -> rendered subrange)` fragments.
- Synthetic rendered content gets no source span:
  - inserted wrap hyphens
  - quote/list padding
  - synthetic line-break components
- Comment highlighting and `n`/`N` jumping consume the rendered projection, not the source span directly.

### Matching Rules

- Selecting a rendered anchor produces a **source span** by unioning the source fragments touched by the selected rendered cells.
- Reopening an existing comment matches by exact `SourceSpan`.
- `n`/`N` chooses the comment’s first projected rendered fragment as the caret jump target.
- Sidebar labels should display the **current rendered span** for the active projection, not the source span.
- If a source-backed comment has no current rendered projection, keep the comment but mark it as hidden/unprojectable in the sidebar.

### Why This Is The Right Shape

- Width changes alter rendered rows and columns, but not source spans.
- The current transform stage mutates layout heavily, so source and rendered coordinates are not compatible.
- Source-anchored comments become stable across reflow, while rendered spans remain available for user-facing navigation and highlighting.

## File Structure

- Modify: `src/parser.rs`
  - add source span extraction from `pest::Pair`
  - keep parse-tree span metadata
- Modify: `src/nodes/word.rs`
  - add optional source span metadata to `Word`
  - add split helpers that preserve or subdivide spans
- Modify: `src/nodes/textcomponent.rs`
  - preserve source spans through wrapping/splitting
  - expose helpers to iterate rendered fragments with source spans
- Modify: `src/nodes/root.rs`
  - expose a projection builder over transformed content
- Modify: `src/comments.rs`
  - replace rendered-range comment identity with source span identity
  - add projection structs if they belong in the comment domain
- Modify: `src/util.rs`
  - selection/edit helpers convert rendered carets to source spans
  - jumping/edit matching uses source span identity
- Modify: `src/main.rs`
  - highlight projected rendered ranges instead of stored rendered ranges
  - sidebar render uses projected rendered span labels
- Modify: `src/boxes/comment_sidebar.rs`
  - show rendered span labels for projected comments
- Optional create: `src/source_map.rs`
  - if projection logic gets large, isolate `SourceSpan`, fragment mapping, and union helpers here

## Task 1: Add Canonical Source Span Types

**Files:**
- Modify: `src/parser.rs`
- Modify: `src/nodes/word.rs`
- Optional create: `src/source_map.rs`

- [ ] Define `SourcePos` and `SourceSpan` with both byte offsets and line/column.
- [ ] Extract `Pair::as_span()` metadata in `parse_text` and store it on `ParseNode`.
- [ ] Thread source span into every `Word` created from source text.
- [ ] Leave synthetic words span-less (`None`) so they cannot be mistaken for source-backed content.

## Task 2: Preserve Spans Through Transform

**Files:**
- Modify: `src/nodes/word.rs`
- Modify: `src/nodes/textcomponent.rs`

- [ ] Extend `Word::split_off` so byte-sliced content also splits the source span.
- [ ] Update wrapping and hyphenation paths in `word_wrapping` to preserve spans on real text fragments.
- [ ] Mark inserted hyphen fragments as synthetic/unmapped.
- [ ] Ensure trim/indent logic does not silently claim source ownership for inserted spaces.

## Task 3: Build Rendered Projection From Transformed Content

**Files:**
- Modify: `src/nodes/root.rs`
- Modify: `src/nodes/textcomponent.rs`
- Optional create: `src/source_map.rs`

- [ ] Add a pure helper that walks transformed content and emits `(SourceSpan fragment, RenderedRange)` pairs.
- [ ] Coalesce adjacent fragments that belong to the same source span when possible.
- [ ] Provide lookup helpers:
  - rendered selection -> source span union
  - source span -> rendered fragments
  - source span -> primary rendered jump target

## Task 4: Re-anchor Comments To Source

**Files:**
- Modify: `src/comments.rs`
- Modify: `src/util.rs`
- Modify: `src/event_handler.rs`

- [ ] Change `Comment` to use `source: SourceSpan` as its canonical anchor.
- [ ] Update selection commit to convert current rendered selection into a source span via projection lookup.
- [ ] Update reopen/edit matching to compare exact source spans.
- [ ] Update `n`/`N` to jump using the comment’s primary projected rendered range.

## Task 5: Render Projected Spans

**Files:**
- Modify: `src/main.rs`
- Modify: `src/boxes/comment_sidebar.rs`

- [ ] Highlight all projected rendered fragments for each comment.
- [ ] Continue to use rendered carets for visual jump/selection.
- [ ] Sidebar should show projected rendered labels like `[L12c3..L12c18]`.
- [ ] If a comment has multiple rendered fragments, show either:
  - first-to-last rendered range, or
  - a compact multi-fragment label

Recommended first cut: show first fragment start and last fragment end.

## Task 6: Tests

**Files:**
- Modify: `src/util.rs`
- Modify: `src/boxes/comment_sidebar.rs`
- Add unit tests near projection helpers

- [ ] Parser tests for source span capture on simple paragraphs and multi-line constructs.
- [ ] Word split tests proving byte-range/source-range subdivision survives wrapping.
- [ ] Projection tests for:
  - wrapped paragraph
  - quote indentation
  - code block lines
  - synthetic hyphen insertion
- [ ] Comment tests for:
  - rendered selection resolves to source span
  - exact source-span reopen
  - width change preserves comment identity but changes projected rendered range
- [ ] Sidebar tests for rendered-span labels.

## Complexity Notes

- **Moderate refactor** if done cleanly: parser + word model + transform + comment state.
- Hardest parts:
  - preserving source ownership through wrapping and splitting
  - deciding how synthetic rendered cells participate in selection
  - keeping projection helpers pure and testable

## Recommended Implementation Choices

- Canonical identity: `SourceSpan` by byte offsets, with line/column cached for UI.
- Projection storage: derived on demand from `ComponentRoot`, not persisted in `Comment`.
- Synthetic rendered chars: non-owning; if selected, ignore them unless adjacent source-backed cells extend the union.
- Sidebar label: rendered span only.
- Future persistence format: source spans only.

## Risks

- The current parser normalizes newlines to spaces for most nodes, so source spans must be captured from `Pair::as_span()` before any text normalization.
- If source spans are added only to `Comment` and not propagated through `Word`, the render projection will be too lossy to be trustworthy.
- Table and code-block rendering may need dedicated fragment logic instead of relying on generic paragraph wrapping.

## First Cut Milestone

If you want the smallest correct vertical slice:

1. Add source span metadata to parser output and words.
2. Implement projection only for paragraph/task/quote text.
3. Switch comments to source spans.
4. Leave tables/code blocks on rendered anchors temporarily with a clear `TODO`.

That gets the architecture right without forcing every block type to land in the first patch.
