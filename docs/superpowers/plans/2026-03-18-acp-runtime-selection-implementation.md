# ACP Runtime Selection Implementation Plan

I'm using the writing-plans skill to create the implementation plan.

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add explicit ACP selection, optional ACP model selection, and polished auth-required runtime UX without changing OrbitShell's core ACP transport architecture.

**Architecture:** Introduce a small runtime-preferences layer for default model persistence, a protocol-adjacent model-discovery helper, and view-local session override state in `AgentView` and `TabView`. Keep ACP resolution anchored on `agent_key`, hide model UI when the ACP does not expose models, and surface auth-required failures as readable runtime state.

**Tech Stack:** Rust 2024, GPUI, serde/serde_json, existing ACP transport/client stack, tempfile-backed integration tests.

---

## File Structure

### Existing files to modify

- `src/acp/client.rs`
  - Accept an optional selected model in `session/new`.
- `src/acp/mod.rs`
  - Export runtime preferences and model-discovery modules.
- `src/acp/manager.rs`
  - Reuse existing auth metadata from `AgentSpec` and keep fallback Codex auth behavior aligned.
- `src/ui/views/agent_view.rs`
  - Replace arrow-based ACP switching with explicit selection UI and conditional model UI.
- `src/ui/views/tab_view.rs`
  - Replace cycling-based ACP selection, add session model override state, and improve auth-required UX.
- `tests/acp_runtime_mcp.rs`
  - Extend runtime payload tests to cover model selection.

### New files to create

- `src/acp/runtime_prefs.rs`
  - Persist default model preferences keyed by `agent_key`.
- `src/acp/model_discovery.rs`
  - Normalize model discovery from ACP metadata/capabilities into a shared runtime shape.
- `tests/acp_runtime_selection.rs`
  - Preferences, model resolution, and auth-classification tests.

---

## Chunk 1: Runtime Preferences And Model Discovery Foundation

### Task 1: Add Runtime Preferences Persistence For Default Model Selection

**Files:**
- Create: `src/acp/runtime_prefs.rs`
- Modify: `src/acp/mod.rs`
- Test: `tests/acp_runtime_selection.rs`

- [ ] **Step 1: Write failing preference round-trip tests**

Create `tests/acp_runtime_selection.rs` with:

```rust
use orbitshell::acp::resolve::{AgentKey, AgentSourceKind};

#[test]
fn runtime_preferences_round_trip_default_model_by_agent_key() {
    let key = AgentKey {
        source_type: AgentSourceKind::Registry,
        source_id: "managed".into(),
        acp_id: "codex-acp".into(),
    };
    // save default model, reload file, assert it survives
}
```

- [ ] **Step 2: Run the targeted test and verify it fails**

Run: `cargo test --test acp_runtime_selection preferences -- --nocapture`

Expected: FAIL because `runtime_prefs` does not exist yet.

- [ ] **Step 3: Implement runtime preference storage**

Create `src/acp/runtime_prefs.rs` with:

- `RuntimeAgentPreference`
- `RuntimePreferencesFile`
- `load_default()`
- `save_default()`
- `default_model_for(agent_key)`
- `set_default_model(agent_key, model_id)`
- `clear_default_model(agent_key)`

Back it with `%APPDATA%/orbitshell/acp-runtime-preferences.json`.

- [ ] **Step 4: Re-run targeted tests and format-check the file**

Run:

```bash
cargo test --test acp_runtime_selection preferences -- --nocapture
cargo fmt --check
```

Expected: targeted preference test PASS.

- [ ] **Step 5: Commit the runtime preference layer**

```bash
git add src/acp/runtime_prefs.rs src/acp/mod.rs tests/acp_runtime_selection.rs
git commit -m "feat: persist ACP runtime model preferences"
```

### Task 2: Add ACP Model Discovery Helpers

**Files:**
- Create: `src/acp/model_discovery.rs`
- Modify: `src/acp/client.rs`
- Modify: `src/acp/mod.rs`
- Test: `tests/acp_runtime_selection.rs`

- [ ] **Step 1: Write failing model-normalization tests**

Extend `tests/acp_runtime_selection.rs` with:

```rust
#[test]
fn discovered_model_ids_normalize_into_dropdown_options() {
    // feed representative ACP metadata/capabilities payload
    // assert normalized ids/labels/default flag
}
```

Add a second test ensuring malformed model metadata returns `None` instead of panicking.

Add a third test that pretends a default model has been persisted, runs discovery that omits that model ID, and asserts the persisted preference is cleared (the next resolution should fall back to the ACP default or blank state).

- [ ] **Step 2: Run the targeted test and confirm failure**

Run: `cargo test --test acp_runtime_selection models -- --nocapture`

Expected: FAIL because discovery helpers are missing.

- [ ] **Step 3: Implement model discovery and normalization**

Create `src/acp/model_discovery.rs` with:

- `AcpModelOption`
- pure normalization helpers for capability/metadata payloads
- `discover_models_from_client(client)` that inspects `agent_capabilities` first and `agent_info` second

The discovery contract for this task is fixed:

1. inspect `agent_capabilities`
2. if inconclusive, inspect `agent_info`
3. if both are inconclusive, return `None`

The helper should stop after these two metadata surfaces and must immediately revalidate the persisted default model for the current `agent_key` before returning, clearing it if it is no longer present in the newly discovered catalog.

Extend the tests in this task with one case proving that a saved default model is cleared when discovery succeeds but the saved model is no longer present in the discovered catalog.

Modify `src/acp/client.rs` only as needed to expose already-fetched metadata safely to the discovery layer.

- [ ] **Step 4: Re-run model tests and the lib test suite**

Run:

```bash
cargo test --test acp_runtime_selection models -- --nocapture
cargo test --lib -- --nocapture
```

Expected: model tests PASS; no regressions in lib tests.

- [ ] **Step 5: Commit model discovery helpers**

```bash
git add src/acp/model_discovery.rs src/acp/client.rs src/acp/mod.rs tests/acp_runtime_selection.rs
git commit -m "feat: add ACP model discovery helpers"
```

## Chunk 2: Runtime Payload And Auth State

### Task 3: Extend `session/new` With Effective Selected Model

**Files:**
- Modify: `src/acp/client.rs`
- Modify: `tests/acp_runtime_mcp.rs`
- Test: `tests/acp_runtime_selection.rs`

- [ ] **Step 1: Write failing payload tests for selected model resolution**

Extend `tests/acp_runtime_mcp.rs` with:

```rust
#[test]
fn session_new_includes_selected_model_when_override_exists() {
    // build params with runtime model override
    // assert selected model field is present
}
```

Extend `tests/acp_runtime_selection.rs` with a pure resolution-order test:

- session override wins over persisted default
- persisted default wins over ACP-declared default
- stale persisted default is cleared and no longer wins once discovery runs
- no model field when nothing exists

- [ ] **Step 2: Run the targeted tests and verify failure**

Run:

```bash
cargo test --test acp_runtime_mcp model -- --nocapture
cargo test --test acp_runtime_selection resolution -- --nocapture
```

Expected: FAIL because `session/new` does not yet accept model selection.

- [ ] **Step 3: Implement effective model resolution in the client payload**

Modify `src/acp/client.rs` to:

- accept `selected_model: Option<&str>` in `ensure_session`
- include the model field in `session/new` only when present
- keep the payload unchanged when no model is selected

Keep MCP payload behavior intact.

- [ ] **Step 4: Re-run runtime payload tests and the focused runtime suite**

Run:

```bash
cargo test --test acp_runtime_mcp -- --nocapture
cargo test --test acp_runtime_selection -- --nocapture
```

Expected: selected-model tests PASS and existing MCP runtime tests remain green.

- [ ] **Step 5: Commit runtime payload updates**

```bash
git add src/acp/client.rs tests/acp_runtime_mcp.rs tests/acp_runtime_selection.rs
git commit -m "feat: send selected ACP model in session creation"
```

### Task 4: Strengthen Auth-Required Detection And Messaging

**Files:**
- Modify: `src/ui/views/tab_view.rs`
- Modify: `src/ui/views/agent_view.rs`
- Test: `tests/acp_runtime_selection.rs`

- [ ] **Step 1: Write failing auth-classification tests**

Extend `tests/acp_runtime_selection.rs` with:

```rust
#[test]
fn auth_classification_flags_login_required_errors() {
    // assert classification prefers structured/runtime-known sources before text heuristics
}
```

Add a second test that a generic transport failure does not classify as auth-related.
 
Add a third test that provides both a structured auth error payload and known stderr lines to prove the higher-priority source wins and heuristics never flip the state when structured data is present.

- [ ] **Step 2: Run the targeted tests and verify failure**

Run: `cargo test --test acp_runtime_selection auth -- --nocapture`

Expected: FAIL or partially fail on current heuristics.

- [ ] **Step 3: Implement polished auth-required messaging**

Modify `src/ui/views/tab_view.rs` and `src/ui/views/agent_view.rs` so that:

- auth classification checks sources in fixed priority order:
  - structured request/runtime error, when present
  - known stderr pattern
  - final textual heuristic
- auth-related failures append a plain-language recovery line
- `Authenticate` remains visible when auth is required
- successful auth-triggered retry paths clear the auth-required state

Do not add logout state.

- [ ] **Step 4: Re-run auth tests and a debug build**

Run:

```bash
cargo test --test acp_runtime_selection auth -- --nocapture
cargo build
```

Expected: auth tests PASS and debug build succeeds.

- [ ] **Step 5: Commit auth UX polish**

```bash
git add src/ui/views/tab_view.rs src/ui/views/agent_view.rs tests/acp_runtime_selection.rs
git commit -m "feat: improve ACP auth-required runtime feedback"
```

## Chunk 3: Explicit ACP Picker And Conditional Model UI

### Task 5: Replace Arrow-Based ACP Switching In `AgentView`

**Files:**
- Modify: `src/ui/views/agent_view.rs`
- Test: `tests/acp_runtime_selection.rs`

- [ ] **Step 1: Write failing view-model tests for explicit selection state**

Add pure tests in `tests/acp_runtime_selection.rs` for helper logic that:

- maps effective agents to picker options
- preserves selection by `agent_key`
- resets client state when a different `agent_key` is selected
- clears discovered models and any session-local model override when a different `agent_key` is selected

- [ ] **Step 2: Run the targeted test and confirm failure**

Run: `cargo test --test acp_runtime_selection picker -- --nocapture`

Expected: FAIL on missing helper/state behavior.

- [ ] **Step 3: Rework `AgentView` top bar selection UX**

Modify `src/ui/views/agent_view.rs` to:

- replace `<` / `>` controls with a real dropdown-style selector
- keep source-distinguishing labels for duplicate IDs
- render a conditional model dropdown when discovered models exist
- persist default model updates when the user changes the dropdown in the agent surface
- clear client state, discovered model state, and session-local override when the selected ACP changes, re-running discovery so the picker and session payload respect the newly validated catalog

- [ ] **Step 4: Re-run picker tests and a focused build**

Run:

```bash
cargo test --test acp_runtime_selection picker -- --nocapture
cargo build
```

Expected: picker tests PASS and build succeeds.

- [ ] **Step 5: Commit `AgentView` selection UX**

```bash
git add src/ui/views/agent_view.rs tests/acp_runtime_selection.rs
git commit -m "feat: add explicit ACP picker to agent view"
```

### Task 6: Add ACP Picker And Session Model Override To `TabView`

**Files:**
- Modify: `src/ui/views/tab_view.rs`
- Test: `tests/acp_runtime_selection.rs`
- Test: `tests/acp_runtime_mcp.rs`

- [ ] **Step 1: Write failing tests for session override semantics**

Extend `tests/acp_runtime_selection.rs` with:

- one test that session model override wins over persisted default
- one test that switching ACP clears the session override for the old agent

Extend `tests/acp_runtime_mcp.rs` if needed to verify `TabView`-driven session creation uses the resolved model.

- [ ] **Step 2: Run the targeted tests and verify failure**

Run:

```bash
cargo test --test acp_runtime_selection session -- --nocapture
cargo test --test acp_runtime_mcp model -- --nocapture
```

Expected: FAIL until `TabView` tracks the override explicitly.

- [ ] **Step 3: Implement explicit ACP selection and session override handling in `TabView`**

Modify `src/ui/views/tab_view.rs` to:

- replace cycling-style ACP switching affordances with a dropdown/select control
- store session model override separately from persisted default
- run model discovery when the active ACP changes
- pass the effective selected model into `ensure_session`
- hide the model picker entirely when no models are available

- [ ] **Step 4: Re-run targeted tests and the full runtime suite**

Run:

```bash
cargo test --test acp_runtime_selection -- --nocapture
cargo test --test acp_runtime_mcp -- --nocapture
```

Expected: override semantics PASS; runtime suite remains green.

- [ ] **Step 5: Commit `TabView` runtime selection UX**

```bash
git add src/ui/views/tab_view.rs tests/acp_runtime_selection.rs tests/acp_runtime_mcp.rs
git commit -m "feat: add ACP and model selection to tab runtime"
```

## Chunk 4: Final Verification

### Task 7: Final Verification Pass

**Files:**
- Modify: none expected
- Test: `tests/acp_runtime_selection.rs`
- Test: `tests/acp_runtime_mcp.rs`

- [ ] **Step 1: Run focused runtime selection tests**

Run:

```bash
cargo test --test acp_runtime_selection -- --nocapture
cargo test --test acp_runtime_mcp -- --nocapture
```

Expected: all runtime selection and payload tests PASS.

- [ ] **Step 2: Run the full suite**

Run: `cargo test -- --nocapture`

Expected: PASS with zero test failures.

- [ ] **Step 3: Run formatting and both builds**

Run:

```bash
cargo fmt --check
cargo build
cargo build --release
```

Expected: format check clean, debug build PASS, release build PASS.

- [ ] **Step 4: Launch the app and manually sanity-check the runtime UX**

Run: `cargo run`

Expected:

- explicit ACP picker visible in agent and tab surfaces
- model dropdown visible only for ACPs exposing models
- auth-required errors show friendly guidance and `Authenticate`

- [ ] **Step 5: Commit the final verified state**

```bash
git add src tests
git commit -m "feat: improve ACP runtime selection and auth UX"
```
