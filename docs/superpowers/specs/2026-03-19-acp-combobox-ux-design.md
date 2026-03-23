# ACP Combobox UX And Picker Affordance Design

## Summary

This spec refines OrbitShell's ACP-facing picker UX across the terminal and settings surfaces. The terminal runtime surface is the primary scope: ACP, model, directory, branch, bottom bar balance, and prompt input legibility. The settings surface is secondary scope: ACP presentation should align visually with the shared picker language where it overlaps, but this spec does not redesign unrelated settings layouts.

OrbitShell should move from a collection of picker-specific overlays toward a shared combobox interaction contract with predictable cursor behavior, hover states, focus rules, search behavior, and selected-item emphasis.

## Goals

- Standardize ACP, model, directory, and branch selection around one shared combobox overlay pattern.
- Fix misleading affordances where non-editable controls look like text inputs.
- Improve cursor feedback so clickable items use pointer semantics and editable inputs use text semantics.
- Make hover, focus, active, and selected states visually distinct and consistent.
- Improve the visual hierarchy and legibility of the bottom bar, including the agent prompt input.
- Keep the UI compact, dark, and aligned with the current OrbitShell aesthetic.

## Non-Goals

- Redesigning the overall OrbitShell shell layout.
- Adding new ACP runtime capabilities beyond picker UX.
- Introducing remote asset fetching for registry icons.
- Reworking unrelated settings panels outside the picker surfaces affected here.

## Approved Product Decisions

- All four selectors become searchable comboboxes: ACP, model, directory, and branch.
- Directory and branch comboboxes open with the search input focused immediately.
- ACP and model comboboxes only show a search input when the list has more than five items.
- ACP and model comboboxes use a strict threshold:
  - `<= 5` items: no visible search input
  - `>= 6` items: visible search input
- ACP and model comboboxes do not auto-focus the search field when opened.
- If ACP or model lists have five items or fewer, the combobox top area should look like a static header, not an editable field.
- The model combobox remains visible in disabled/loading states when OrbitShell is still discovering models from the ACP runtime.
- The AI mode icon in the bottom bar should be replaced with a more assistant-oriented symbol such as sparkles/stars.

## Current Problems

The current picker experience has several mismatches between appearance and behavior:

- elements that act like buttons do not consistently present as clickable
- overlays that look like text inputs do not always allow typing
- hover and selected states are too similar, so active choice is unclear
- the bottom bar distributes visual weight unevenly across mode, ACP, model, directory, and branch
- the agent prompt input has weak contrast and a low-emphasis placeholder
- long overlay text can still feel visually cramped or ambiguous inside the selection list

## Shared Combobox Architecture

OrbitShell should introduce a shared `ComboboxOverlay` base pattern used by:

- ACP picker in `TabView`
- model picker in `TabView`
- directory picker
- branch picker

This is one base component with two operation modes, not two unrelated picker implementations:

- `immediate-search`
- `conditional-search`

The component can still accept contextual data and callbacks, but the interaction contract should be shared rather than reimplemented per picker.

Common responsibilities:

- render optional header or search input
- maintain a highlighted row for keyboard navigation
- apply consistent pointer/text cursor semantics
- render item title, description, metadata badges, and selected state
- expose empty, disabled, and loading sub-states
- expose stable visual-state hooks on rows and triggers so automated tests can assert `selected`, `highlighted`, `disabled`, `loading`, and `search-visible` states without relying only on screenshots

The implementation does not need to fully extract every picker into a separate file if that would fight current code structure, but it should centralize rendering rules and state transitions enough to prevent divergence.

Even when ACP/model open without a visible search field, the control should still be implemented as a combobox/listbox-style selector internally so keyboard interaction and accessibility semantics remain consistent.

## Interaction Model

### Mode 1: Immediate Search

Used for:

- directory
- branch

Behavior:

- opening the combobox focuses the search input immediately
- the input shows a text cursor and visible focus ring
- typing filters the list in real time
- arrow keys move through filtered results
- home/end move to the first/last visible option
- enter confirms the active option
- escape closes the overlay
- tab and shift+tab move focus out of the combobox normally
- clicking outside closes the overlay

### Mode 2: Conditional Search

Used for:

- ACP
- model

Behavior:

- if the list has six items or more, show a real search input at the top
- if the list has five items or fewer, render a static header such as `Select ACP` or `Select model`
- the static header must not look focusable or editable
- clicking the static header should not enter text-edit mode; focus remains on the overlay/list container
- opening the combobox does not auto-focus the search field
- initial focus lands on the overlay container/listbox, with the current selection highlighted
- arrow keys navigate immediately without requiring a click
- home/end move to the first/last visible option
- enter confirms the highlighted option
- escape closes the overlay
- tab moves focus into the search field when it is visible; otherwise it moves out of the overlay
- shift+tab moves focus back to the trigger or previous focusable element
- if a search field is visible, clicking or tabbing into it gives it text focus normally
- if a search field is visible, typing alphanumeric input while the list owns focus should move focus into the search field and seed the typed query there
- if no search field is visible, alphanumeric typing should use lightweight typeahead-to-highlight behavior against the visible option labels, without rendering a new input
- if a search field is shown, it must behave like a real input when clicked or focused
- clicking outside closes the overlay
- reopening the overlay restores the same mode decision for the current item count and re-highlights the current selection

This keeps ACP/model selection compact and calm for short lists while still scaling when the catalog grows.

For ACP/model, the `<= 5` / `>= 6` threshold is evaluated against the unfiltered option count at the moment the overlay opens. Once open, filtering does not hide or create the search field mid-session. A runtime state change that replaces the option set while the overlay is open may reevaluate the threshold once, as part of the new dataset swap.

## Visual And Affordance Rules

### Cursor Semantics

- clickable rows, trigger chips, and icon buttons must use pointer cursor behavior
- editable inputs must use text cursor behavior
- disabled controls must not present as interactive

These semantics should be exposed through stable component props or role/state markers so tests can verify which elements are interactive vs text-editable without relying purely on pixel inspection.

### Hover, Focus, And Selection

- hover must be visible on every selectable row
- keyboard focus/active row must be distinguishable from passive hover
- the selected item must look persistently active, not merely hovered
- visual precedence is fixed as `selected > active/highlighted > hover`
- when a row is both selected and highlighted, the selected treatment remains primary and the highlighted treatment should layer on top as a lighter emphasis instead of replacing it
- focus rings should be visible but restrained, matching the dark theme

### Item Hierarchy

Each row should clearly separate:

- primary label
- secondary description
- supporting badge or state such as `Default`

The `Default` badge should be smaller and visually subordinate to the model name, not the strongest element in the row.

### Bottom Bar Balance

The lower control strip should feel intentionally weighted:

- mode switch should read as a mode control, not a random icon cluster
- ACP and model chips should feel related but not overpower the directory/branch context
- prompt input should regain visual priority as the main interaction target

This iteration is limited to hierarchy, contrast, padding, iconography, and state styling within the existing bottom bar structure. It does not include macro layout changes or functional redistribution of controls.

## Runtime Model Button States

The model trigger should stay visible and communicate state explicitly:

- `loading`: visible, disabled, communicates that OrbitShell is fetching runtime model options
- `ready`: visible, enabled, opens the combobox
- `unavailable`: visible, disabled, communicates that no model list is exposed

OrbitShell should not hide the model control during runtime discovery. The user should understand that the product is still resolving that information.

The trigger label should also communicate state explicitly rather than reusing a neutral selection label:

- loading example: `Loading models...`
- unavailable example: `No models available`

If the model overlay is open while the runtime state changes:

- `loading -> ready`: keep the overlay open and swap in the discovered results
- `loading -> unavailable`: close the overlay if it no longer has actionable content and update the trigger label
- `ready -> loading` due to ACP switch: close the previous overlay and return the trigger to disabled/loading state

## Iconography

The current AI mode icon should be replaced with an icon more clearly associated with an assistant, such as sparkles/stars. The new icon should harmonize with the rest of the bottom bar and not dominate it.

Registry-provided icon metadata can continue to influence ACP avatar treatment, but this spec does not require rendering the remote icon asset itself.

## Error And Empty States

- empty filtered lists should say that no results match the current query
- unavailable pickers should explain why in the trigger label or supporting text
- loading pickers should avoid looking broken or blank
- overlay content should wrap cleanly instead of overflowing horizontally
- directory and branch rows should handle long labels defensively, using truncation or path-emphasis treatment where needed so the most useful part of the label remains scannable

## Files In Scope

Primary implementation surfaces:

- `src/ui/views/tab_view.rs`
- `src/ui/views/settings_view.rs`
- `src/ui/icons.rs`

Supporting UI primitives or helpers may be introduced if the current picker logic is too fragmented to keep consistent.

## Testing And Validation

### Automated

- add or update unit tests for shared picker state logic
- add tests for search threshold behavior on ACP/model pickers
- add tests for selected-row precedence vs hover/highlight state where practical
- add tests for keyboard behavior in conditional-search comboboxes, including type-to-focus-search
- keep `cargo check` and relevant picker tests green

### Manual

Validate in the running app:

- ACP picker opens, filters, and selects correctly
- model picker shows loading, ready, and unavailable states correctly
- directory and branch pickers focus the search field immediately
- pointer vs text cursor semantics are correct across triggers, rows, and inputs
- selected rows remain visually distinct from hovered rows
- prompt input is more legible and visually prioritized
- no obvious overflow or clipping occurs in overlay content
- combobox/listbox semantics remain coherent for disabled, loading, and unavailable states

### Accessibility And Semantics

- triggers should keep consistent combobox-style semantics with an expanded/collapsed state
- overlays should expose listbox semantics with an observable active option
- disabled and unavailable triggers should remain readable, but not present as actionable
- loading triggers should communicate that state textually
- if the UI toolkit does not map directly to web `aria-*`, OrbitShell should still preserve equivalent semantic state in component props/test hooks

## Observable Acceptance Criteria

- ACP/model pickers with `<= 5` options open without a visible search field
- ACP/model pickers with `>= 6` options open with a visible search field
- filtering does not toggle search-field visibility mid-open
- directory/branch pickers focus the input on open
- ACP/model pickers open with list focus, not text focus
- typing in a conditional-search picker with visible search field moves focus into the field and seeds the query
- row state markers expose at least `selected`, `highlighted`, and `disabled`
- trigger state markers expose at least `expanded`, `disabled`, `loading`, and `has-search-input`
- selected styling remains primary when a row is both selected and highlighted
- clickable triggers and rows expose pointer semantics
- editable search inputs expose text-edit semantics

## Success Criteria

This work is successful when a user can move between ACP, model, directory, and branch selection without relearning each overlay. Every picker should clearly communicate whether it is clickable, searchable, focused, selected, loading, or unavailable.
