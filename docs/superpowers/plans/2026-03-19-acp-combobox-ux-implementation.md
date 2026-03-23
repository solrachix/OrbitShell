# ACP Combobox UX Implementation Plan

I'm using the writing-plans skill to create the implementation plan.

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Unify ACP, model, directory, and branch pickers under one combobox interaction contract and improve bottom-bar affordance, feedback, and legibility without changing OrbitShell's macro layout.

**Architecture:** Keep the current overlay system in `TabView`, but introduce a shared combobox rendering/state contract that supports two modes: `immediate-search` and `conditional-search`. Drive consistent cursor, hover, focus, selected, disabled, loading, and search-threshold behavior through shared helpers and testable state markers, then apply the same visual language to the settings ACP list where it overlaps.

Even if the implementation remains inside `src/ui/views/tab_view.rs`, it must introduce clearly reusable local helpers/types for combobox behavior and rendering decisions. Manual one-off alignment inside each picker is not sufficient for this plan.

**Tech Stack:** Rust 2024, GPUI, existing `TabView` overlay state, Lucide icons, existing ACP runtime model-loading path, Cargo tests/check.

---

## File Structure

### Existing files to modify

- `src/ui/views/tab_view.rs`
  - Shared combobox state helpers, picker rendering, keyboard behavior, cursor/interaction markers, bottom-bar styling, prompt input, and AI icon swap.
- `src/ui/views/settings_view.rs`
  - Align ACP cards/list affordance with the shared picker visual language where applicable.
- `src/ui/icons.rs`
  - Add or adapt the assistant-facing icon helper and keep ACP avatar helpers consistent.

### Existing tests to extend

- `src/ui/views/tab_view.rs`
  - Extend current unit tests for picker-state selection, threshold logic, and explicit helper states.

### New tests or helper coverage to add inside existing modules

- `src/ui/views/tab_view.rs`
  - Pure tests for:
    - `PickerMode`
    - `TriggerState`
    - `RowState`
    - `InitialFocusTarget`
    - `shows_search_input`
    - `typeahead_enabled`
    - `header_is_static`
    - `cursor_semantics`

---

## Chunk 1: Shared Combobox Contract In `TabView`

### Task 1: Add Shared Picker Mode And Threshold Helpers

**Files:**
- Modify: `src/ui/views/tab_view.rs`
- Test: `src/ui/views/tab_view.rs`

- [ ] **Step 1: Write the failing picker-mode tests**

Add unit tests near the existing picker tests for:

```rust
#[test]
fn conditional_search_picker_hides_search_for_five_or_fewer_items() {}

#[test]
fn conditional_search_picker_shows_search_for_six_or_more_items() {}

#[test]
fn short_conditional_picker_uses_typeahead_without_search_input() {}
```

- [ ] **Step 2: Run the targeted tests and confirm failure**

Run:

```bash
cargo test conditional_search_picker --lib -- --nocapture
```

Expected: FAIL because the shared threshold/mode helpers do not exist yet.

- [ ] **Step 3: Implement shared combobox mode helpers**

In `src/ui/views/tab_view.rs`:

- add a small shared enum/shape for combobox mode, such as:
  - `ImmediateSearch`
  - `ConditionalSearch { has_search_input: bool }`
- add pure helpers that decide:
  - whether search is visible for a picker
  - whether typeahead is enabled without visible search
  - whether the top area is a static header vs editable input
  - what state markers should be exposed

- [ ] **Step 4: Re-run the targeted tests**

Run:

```bash
cargo test conditional_search_picker --lib -- --nocapture
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/ui/views/tab_view.rs
git commit -m "feat: add shared combobox mode helpers"
```

### Task 2: Add Shared Row/Trigger State Markers

**Files:**
- Modify: `src/ui/views/tab_view.rs`
- Test: `src/ui/views/tab_view.rs`

- [ ] **Step 1: Write failing tests for observable state markers**

Add tests for helpers that expose row/trigger states:

```rust
#[test]
fn selected_row_keeps_selected_state_when_also_highlighted() {}

#[test]
fn trigger_state_reports_loading_and_has_search_input() {}

#[test]
fn clickable_rows_and_triggers_report_pointer_semantics() {}

#[test]
fn editable_search_inputs_report_text_semantics() {}
```

- [ ] **Step 2: Run the targeted tests and verify failure**

Run:

```bash
cargo test picker_state_markers --lib -- --nocapture
```

Expected: FAIL because no shared marker helpers exist yet.

- [ ] **Step 3: Implement marker/state helper logic**

In `src/ui/views/tab_view.rs`:

- centralize row state computation for:
  - `selected`
  - `highlighted`
  - `disabled`
- centralize cursor/interaction semantics for:
  - clickable rows/triggers => pointer
  - editable search inputs => text
- centralize trigger state computation for:
  - `expanded`
  - `disabled`
  - `loading`
  - `has_search_input`

Use these helpers to drive render styling so precedence is shared across ACP/model/path/branch pickers.

- [ ] **Step 4: Re-run the targeted tests**

Run:

```bash
cargo test picker_state_markers --lib -- --nocapture
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/ui/views/tab_view.rs
git commit -m "feat: add combobox state markers"
```

## Chunk 2: Picker Behavior And Keyboard Flow

### Task 3: Unify Overlay Keyboard And Focus Behavior

**Files:**
- Modify: `src/ui/views/tab_view.rs`
- Test: `src/ui/views/tab_view.rs`

- [ ] **Step 1: Write failing tests for picker behavior**

Add tests for pure or narrowly scoped behavior helpers covering:

- conditional picker opens with list focus
- immediate-search picker opens with text focus
- short ACP/model picker uses typeahead rather than opening a hidden input
- `Home`/`End` behavior clamps to first/last visible option

- [ ] **Step 2: Run the targeted tests and confirm failure**

Run:

```bash
cargo test picker_focus_behavior --lib -- --nocapture
```

Expected: FAIL because focus/typeahead behavior is still picker-specific.

- [ ] **Step 3: Implement the shared keyboard/focus rules**

Refactor `TabView` overlay handling so:

- directory/branch always open in immediate-search mode
- ACP/model use conditional-search mode
- `ArrowUp`/`ArrowDown`, `Home`, `End`, `Enter`, `Escape` use shared behavior
- ACP/model typing routes to:
  - visible search field when present
  - typeahead-to-highlight when search is hidden

- [ ] **Step 4: Re-run the targeted tests**

Run:

```bash
cargo test picker_focus_behavior --lib -- --nocapture
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/ui/views/tab_view.rs
git commit -m "feat: unify combobox keyboard behavior"
```

### Task 4: Convert ACP Picker To Shared Combobox Rendering

**Files:**
- Modify: `src/ui/views/tab_view.rs`
- Test: `src/ui/views/tab_view.rs`

- [ ] **Step 1: Write failing tests for ACP picker rendering decisions**

Add tests for:

- ACP picker with `<= 5` items returns `shows_search_input == false`
- ACP picker with `>= 6` items returns `shows_search_input == true`
- ACP short-list mode returns `typeahead_enabled == true`
- ACP short-list mode returns `header_is_static == true`
- clicking the static header does not switch into text-edit mode

- [ ] **Step 2: Run the targeted tests and confirm failure**

Run:

```bash
cargo test --lib conditional_search_picker_hides_search_for_five_or_fewer_items
cargo test --lib conditional_search_picker_shows_search_for_six_or_more_items
cargo test --lib short_conditional_picker_uses_typeahead_without_search_input
```

Expected: at least one new test FAILS because ACP rendering decisions are not shared/complete yet.

- [ ] **Step 3: Refactor ACP picker rendering**

Update `render_agent_picker`, ACP trigger rendering, and related helpers so:

- short lists use a static non-editable header
- long lists show a real search input
- the static header does not look editable and does not move focus into text-edit mode when clicked
- mouse hover and selected states are visually distinct
- row/trigger state markers are wired through the shared helpers

- [ ] **Step 4: Re-run the targeted tests**

Run:

```bash
cargo test --lib conditional_search_picker_hides_search_for_five_or_fewer_items
cargo test --lib conditional_search_picker_shows_search_for_six_or_more_items
cargo test --lib short_conditional_picker_uses_typeahead_without_search_input
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/ui/views/tab_view.rs
git commit -m "feat: polish ACP combobox"
```

### Task 5: Convert Model Picker To Shared Combobox Rendering

**Files:**
- Modify: `src/ui/views/tab_view.rs`
- Test: `src/ui/views/tab_view.rs`

- [ ] **Step 1: Write failing tests for model trigger and overlay state**

Add tests for:

- model trigger `loading` reports `loading == true` and disabled state
- model trigger `unavailable` reports explicit unavailable label state
- model picker with `<= 5` options returns `shows_search_input == false`
- model picker with `>= 6` options returns `shows_search_input == true`

- [ ] **Step 2: Run the targeted tests and confirm failure**

Run:

```bash
cargo test --lib model_picker_uses_selected_model_when_present
```

Expected: at least one new assertion FAILS because model states are not fully wired to shared helpers yet.

- [ ] **Step 3: Refactor model picker rendering**

Update `render_model_picker`, model trigger rendering, and related helpers so:

- short lists use a static non-editable header
- long lists show a real search input
- trigger labels communicate `Loading models...` and `No models available`
- row/trigger state markers are wired through the shared helpers

- [ ] **Step 4: Re-run the targeted tests**

Run:

```bash
cargo test --lib model_picker_uses_selected_model_when_present
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/ui/views/tab_view.rs
git commit -m "feat: polish model combobox"
```

### Task 6: Convert Directory Picker To Shared Immediate-Search Combobox

**Files:**
- Modify: `src/ui/views/tab_view.rs`
- Test: `src/ui/views/tab_view.rs`

- [ ] **Step 1: Write failing tests for directory picker state helpers**

Add tests for:

- directory picker returns `InitialFocusTarget::SearchInput`
- directory picker returns `shows_search_input == true`
- directory picker rows expose `pointer` semantics and shared `RowState`

- [ ] **Step 2: Run the targeted tests and confirm failure**

Run:

```bash
cargo test --lib picker_focus_behavior
```

Expected: at least one new assertion FAILS because directory picker state is not fully standardized.

- [ ] **Step 3: Refactor directory picker rendering**

Update `render_path_picker` and related helpers so:

- search input is always present
- opening focuses the search input
- shared row states drive hover/selected/highlight precedence
- long path labels use defensive text treatment and remain scannable, preferring path-emphasis/truncation rules that preserve the most useful segment instead of arbitrary clipping

- [ ] **Step 4: Re-run the targeted tests**

Run:

```bash
cargo test --lib picker_focus_behavior
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/ui/views/tab_view.rs
git commit -m "feat: unify directory combobox"
```

### Task 7: Convert Branch Picker To Shared Immediate-Search Combobox

**Files:**
- Modify: `src/ui/views/tab_view.rs`
- Test: `src/ui/views/tab_view.rs`

- [ ] **Step 1: Write failing tests for branch picker state helpers**

Add tests for:

- branch picker returns `InitialFocusTarget::SearchInput`
- branch picker returns `shows_search_input == true`
- branch rows use the shared `RowState` precedence markers

- [ ] **Step 2: Run the targeted tests and confirm failure**

Run:

```bash
cargo test --lib picker_focus_behavior
```

Expected: at least one new assertion FAILS because branch picker state is not fully standardized.

- [ ] **Step 3: Refactor branch picker rendering**

Update `render_branch_picker` and related helpers so:

- search input is always present
- opening focuses the search input
- shared row states drive hover/selected/highlight precedence
- long branch labels remain scannable via defensive truncation/emphasis treatment instead of arbitrary horizontal clipping

- [ ] **Step 4: Re-run the targeted tests**

Run:

```bash
cargo test --lib picker_focus_behavior
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/ui/views/tab_view.rs
git commit -m "feat: unify branch combobox"
```

## Chunk 3: Bottom Bar Visual Polish And Settings Alignment

### Task 8: Rebalance Bottom-Bar Affordance And Prompt Input

**Files:**
- Modify: `src/ui/views/tab_view.rs`
- Modify: `src/ui/icons.rs`

- [ ] **Step 1: Add a narrow regression test if practical**

If there is a pure helper for labels/icons/states, add a small test for the new assistant icon helper or prompt-label state. If no useful pure test exists, note that this task is primarily visual and will rely on `cargo check` + manual validation.

- [ ] **Step 2: Implement bottom-bar polish**

Update `render_input_bar` and supporting icon helpers so:

- clickable chips/buttons use pointer semantics consistently
- the AI mode icon becomes a clearer assistant-oriented icon
- the prompt input gains stronger contrast and legibility
- visual weight between mode, ACP, model, directory, branch, and prompt input feels more balanced without changing macro layout

- [ ] **Step 3: Run `cargo check`**

Run:

```bash
cargo check
```

Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add src/ui/views/tab_view.rs src/ui/icons.rs
git commit -m "feat: polish bottom bar affordance"
```

### Task 9: Align Settings ACP Presentation

**Files:**
- Modify: `src/ui/views/settings_view.rs`
- Modify: `src/ui/icons.rs`

- [ ] **Step 1: Add or extend a narrow helper test if practical**

If shared ACP icon/label helpers gain pure logic, add a unit test next to existing `registry_avatar` coverage.

- [ ] **Step 2: Implement settings-surface alignment**

Update the ACP Registry list in `settings_view.rs` so the overlap with the combobox language is consistent:

- clearer pointer affordance on clickable controls
- stronger hierarchy between title, description, badges, and actions
- consistent state styling overlap where settings shares ACP identity treatment with terminal pickers
- preserve current card structure; do not redesign unrelated settings layout

- [ ] **Step 3: Run targeted checks**

Run:

```bash
cargo test registry_avatar --lib -- --nocapture
cargo check
```

Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add src/ui/views/settings_view.rs src/ui/icons.rs
git commit -m "feat: align settings ACP affordance"
```

## Chunk 4: Full Validation

### Task 10: Run Automated Validation

**Files:**
- No code changes required unless failures are found

- [ ] **Step 1: Run focused picker tests**

Run:

```bash
cargo test --lib conditional_search_picker_hides_search_for_five_or_fewer_items
cargo test --lib conditional_search_picker_shows_search_for_six_or_more_items
cargo test --lib short_conditional_picker_uses_typeahead_without_search_input
cargo test --lib picker_focus_behavior
cargo test --lib model_picker_uses_selected_model_when_present
```

Expected: PASS.

- [ ] **Step 2: Run broad verification**

Run:

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --lib -- --nocapture
cargo check
```

Expected: PASS.

- [ ] **Step 3: Fix any failures before proceeding**

If anything fails, make the smallest corrective edit and rerun the affected command before moving on.

### Task 11: Run Manual UI Validation

**Files:**
- No code changes required unless validation finds a bug

- [ ] **Step 1: Launch the app**

Run:

```bash
cargo run
```

- [ ] **Step 2: Validate the final flow manually**

Confirm:

- ACP picker with 3 items opens without search input and supports typeahead highlight
- ACP picker with 3 items keeps a static non-editable header that does not behave like a text field
- ACP picker with 6 items opens with visible search input
- model trigger shows `Loading models...`, ready label, and unavailable label at the right times
- directory picker opens with search focused, including a long nested path label
- branch picker opens with search focused, including a long branch label
- settings ACP list preserves hierarchy and clickable affordance
- pointer vs text cursor semantics are correct
- selected rows are visually distinct from hover-only rows
- prompt input is more legible
- long path/branch labels remain scannable and do not degrade into arbitrary clipping
- app console/log output does not show a new error related to the updated UI flow

- [ ] **Step 3: Capture any issue and fix it before closing**

If manual validation finds a bug, fix it and rerun the relevant automated checks plus the affected manual flow.
