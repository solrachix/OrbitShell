# ACP Registry Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a Zed-style ACP Registry and global MCP configuration flow in OrbitShell without breaking existing local `agents.json` support or the current ACP runtime.

**Architecture:** Add a library crate root plus four focused subsystems: registry catalog/cache, managed install state, effective agent resolution, and global MCP configuration. Keep `AcpClient` and `AcpTransport` as the protocol runtime, move business logic out of `SettingsView`, and wire the UI to a merged `agent_key`-based model.

**Tech Stack:** Rust 2024, GPUI, serde/serde_json, futures, blocking HTTP fetch in worker threads, semver parsing, SHA-256 verification, zip/tar extraction for binary installs, tempfile-backed integration tests.

---

## File Structure

### Existing files to modify

- `Cargo.toml`
  - Add runtime crates for registry fetch, version comparison, checksum verification, and archive extraction.
- `src/main.rs`
  - Switch from declaring all modules directly to using the library crate exports.
- `src/acp/mod.rs`
  - Export new ACP submodules.
- `src/acp/manager.rs`
  - Narrow to compatibility loading for custom agents and helpers shared with the resolver.
- `src/acp/client.rs`
  - Accept resolved global MCP runtime config in `session/new`.
- `src/ui/mod.rs`
  - Preserve Settings tab workspace context so workspace-local agents resolve deterministically.
- `src/ui/views/settings_view.rs`
  - Convert the current ACP/MCP settings sections into real state-backed UI.
- `src/ui/views/agent_view.rs`
  - Read effective agents instead of raw `agents.json`.
- `src/ui/views/tab_view.rs`
  - Read effective agents instead of raw `agents.json`, and pass global MCP servers into ACP sessions.

### New files to create

- `src/lib.rs`
  - Library crate root so integration tests can import OrbitShell modules.
- `src/acp/storage.rs`
  - `%APPDATA%` path resolution and shared JSON load/save helpers.
- `src/acp/registry/mod.rs`
  - Registry module exports.
- `src/acp/registry/model.rs`
  - Normalized catalog structs and cache metadata structs.
- `src/acp/registry/cache.rs`
  - Load/save helpers for `registry-index.json`, `registry-meta.json`, and cached manifests.
- `src/acp/registry/fetch.rs`
  - Cached-first registry refresh, manifest refresh, and update detection.
- `src/acp/install/mod.rs`
  - Managed install module exports.
- `src/acp/install/state.rs`
  - `managed-agents.json` model, active version state, and persistence helpers.
- `src/acp/install/runner.rs`
  - Managed wrapper generation and command resolution for `npx` and `uvx`.
- `src/acp/install/binary.rs`
  - Download, SHA-256 verification, extraction, and activation for `binary`.
- `src/acp/resolve.rs`
  - `agent_key`, effective rows, duplicate handling, and conflict policy resolution.
- `src/mcp/mod.rs`
  - MCP module exports.
- `src/mcp/config.rs`
  - MCP config model and persistence.
- `src/mcp/probe.rs`
  - MCP connection-test helpers and status diagnostics.

### Test files to create

- `tests/acp_storage.rs`
- `tests/acp_registry_cache.rs`
- `tests/acp_resolve.rs`
- `tests/acp_installers.rs`
- `tests/acp_runtime_mcp.rs`
- `tests/mcp_config.rs`

---

## Chunk 1: Foundation And Persistence

### Task 1: Extract A Library Crate And Add Module Scaffolding

**Files:**
- Create: `src/lib.rs`
- Modify: `src/main.rs`
- Create: `src/mcp/mod.rs`
- Test: `tests/acp_storage.rs`

- [ ] **Step 1: Write the failing integration test import**

Create `tests/acp_storage.rs` with a compile-time smoke test that expects to import the library crate:

```rust
use orbitshell::ui::Workspace;

#[test]
fn library_exports_workspace_type() {
    let _ = std::any::type_name::<Workspace>();
}
```

- [ ] **Step 2: Run the test to verify the crate is not yet importable**

Run: `cargo test --test acp_storage -- --nocapture`

Expected: FAIL with an unresolved `orbitshell` import or missing exported module.

- [ ] **Step 3: Create the library root and re-export modules**

Create `src/lib.rs` with:

```rust
pub mod acp;
pub mod git;
pub mod mcp;
pub mod terminal;
pub mod ui;
```

Update `src/main.rs` to import from the library crate instead of declaring all modules locally.

Create an empty `src/mcp/mod.rs`:

```rust
```

- [ ] **Step 4: Re-run the same test and fix compile errors until the library shape is stable**

Run: `cargo test --test acp_storage -- --nocapture`

Expected: PASS once the test imports a real exported item.

- [ ] **Step 5: Commit the crate extraction**

Run:

```bash
git add src/main.rs src/lib.rs src/mcp/mod.rs tests/acp_storage.rs
git commit -m "refactor: expose orbitshell library modules"
```

### Task 2: Add App-Data Storage, Preferences, And MCP Config Models

**Files:**
- Create: `src/acp/storage.rs`
- Create: `src/mcp/config.rs`
- Modify: `src/mcp/mod.rs`
- Test: `tests/acp_storage.rs`
- Test: `tests/mcp_config.rs`

- [ ] **Step 1: Write failing persistence tests**

Expand `tests/acp_storage.rs` to verify path helpers and JSON round-trip:

```rust
use std::path::PathBuf;

#[test]
fn registry_paths_live_under_appdata() {
    let root = orbitshell::acp::storage::app_root_from(PathBuf::from("C:/tmp/appdata"));
    assert!(root.ends_with("orbitshell"));
}
```

Create `tests/mcp_config.rs` with a round-trip test for the minimum MCP schema:

```rust
#[test]
fn mcp_server_round_trips_with_required_fields() {
    let server = orbitshell::mcp::config::McpServerConfig {
        id: "fs".into(),
        name: "Filesystem".into(),
        transport: "stdio".into(),
        command: Some("mcp-server-fs".into()),
        url: None,
        args: vec![".".into()],
        env: Default::default(),
        enabled: true,
        last_tested_at: None,
        last_status: None,
        last_error: None,
    };
    let json = serde_json::to_string(&server).unwrap();
    let decoded: orbitshell::mcp::config::McpServerConfig = serde_json::from_str(&json).unwrap();
    assert_eq!(decoded.id, "fs");
}
```

- [ ] **Step 2: Run the targeted tests to capture the missing models**

Run:

```bash
cargo test --test acp_storage -- --nocapture
cargo test --test mcp_config -- --nocapture
```

Expected: FAIL with missing `storage` and `config` items.

- [ ] **Step 3: Implement the storage helpers and MCP config model**

Create `src/acp/storage.rs` with helpers for:

- resolving `%APPDATA%/orbitshell` and test-injectable app roots
- loading/saving JSON files
- ensuring parent directories exist

Create `src/mcp/config.rs` with:

```rust
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct McpServerConfig {
    pub id: String,
    pub name: String,
    pub transport: String,
    pub command: Option<String>,
    pub url: Option<String>,
    pub args: Vec<String>,
    pub env: std::collections::BTreeMap<String, String>,
    pub enabled: bool,
    pub last_tested_at: Option<i64>,
    pub last_status: Option<String>,
    pub last_error: Option<String>,
}
```

Add a `GlobalMcpConfig` container and `load/save` helpers backed by `mcp-servers.json`.

- [ ] **Step 4: Re-run the targeted tests and then the full test suite**

Run:

```bash
cargo test --test acp_storage -- --nocapture
cargo test --test mcp_config -- --nocapture
cargo test -- --nocapture
```

Expected:

- targeted tests PASS
- full suite PASS, or only unrelated failures already present in the branch

- [ ] **Step 5: Commit the persistence foundation**

Run:

```bash
git add src/acp/storage.rs src/mcp/config.rs src/mcp/mod.rs tests/acp_storage.rs tests/mcp_config.rs
git commit -m "feat: add app-data storage and MCP config models"
```

### Task 3: Add Registry And Managed-State Core Models

**Files:**
- Create: `src/acp/registry/model.rs`
- Create: `src/acp/install/state.rs`
- Create: `src/acp/resolve.rs`
- Modify: `src/acp/mod.rs`
- Test: `tests/acp_resolve.rs`

- [ ] **Step 1: Write failing model and conflict-policy tests**

Create `tests/acp_resolve.rs` with serialization and conflict-policy tests:

```rust
#[test]
fn show_both_keeps_two_rows_with_distinct_agent_keys() {
    let rows = orbitshell::acp::resolve::resolve_effective_agents(
        /* registry row */, /* custom row */, orbitshell::acp::resolve::ConflictPolicy::ShowBoth
    );
    assert_eq!(rows.len(), 2);
    assert_ne!(rows[0].agent_key, rows[1].agent_key);
}
```

Add a second test that ensures `RegistryWins` does not make the losing source unreachable in the returned structure.

- [ ] **Step 2: Run the resolver test to verify the missing models**

Run: `cargo test --test acp_resolve -- --nocapture`

Expected: FAIL with missing resolver/model types.

- [ ] **Step 3: Implement the serializable core models**

In `src/acp/registry/model.rs`, add:

- `RegistryCacheMeta`
- `RegistryCatalogEntry`
- lightweight manifest structs needed by fetch/cache

In `src/acp/install/state.rs`, add:

- `ManagedAgentState`
- `ManagedAgentsStateFile`

In `src/acp/resolve.rs`, add:

- `ConflictPolicy`
- `AgentKey { source_type, source_id, acp_id }`
- `EffectiveAgentRow`
- pure merge/resolution helpers

- [ ] **Step 4: Re-run the resolver test and full test suite**

Run:

```bash
cargo test --test acp_resolve -- --nocapture
cargo test -- --nocapture
```

Expected: targeted resolver test PASS; full suite PASS or only unrelated known failures.

- [ ] **Step 5: Commit the model layer**

Run:

```bash
git add src/acp/registry/model.rs src/acp/install/state.rs src/acp/resolve.rs src/acp/mod.rs tests/acp_resolve.rs
git commit -m "feat: add ACP registry and resolver core models"
```

## Chunk 2: Registry Catalog And Effective Resolution

### Task 4: Implement Registry Cache Files And Cached-First Loading

**Files:**
- Create: `src/acp/registry/cache.rs`
- Modify: `src/acp/registry/mod.rs`
- Modify: `src/acp/storage.rs`
- Test: `tests/acp_registry_cache.rs`

- [ ] **Step 1: Write failing cache round-trip tests**

Create `tests/acp_registry_cache.rs` with tests for:

- writing `registry-index.json`
- writing `registry-meta.json`
- writing a manifest cache
- loading cached data when all files exist

Example:

```rust
#[test]
fn registry_cache_round_trips_index_meta_and_manifest() {
    // use tempfile dir
    // save index, meta, manifest
    // load them back
    // assert etag and ttl_seconds are preserved
}
```

- [ ] **Step 2: Run the cache tests to verify the cache module is missing**

Run: `cargo test --test acp_registry_cache -- --nocapture`

Expected: FAIL with unresolved cache helpers.

- [ ] **Step 3: Implement cache helpers**

Create `src/acp/registry/cache.rs` with load/save helpers for:

- `registry-index.json`
- `registry-meta.json`
- per-agent manifest files

Export the module in `src/acp/registry/mod.rs`:

```rust
pub mod cache;
pub mod fetch;
pub mod model;
```

- [ ] **Step 4: Re-run cache tests and ensure all cache files land in the expected directory layout**

Run:

```bash
cargo test --test acp_registry_cache -- --nocapture
cargo test -- --nocapture
```

Expected: targeted cache test PASS.

- [ ] **Step 5: Commit the cache layer**

Run:

```bash
git add src/acp/registry/cache.rs src/acp/registry/mod.rs src/acp/storage.rs tests/acp_registry_cache.rs
git commit -m "feat: add ACP registry cache persistence"
```

### Task 5: Implement Cached-First Refresh And Update Detection

**Files:**
- Create: `src/acp/registry/fetch.rs`
- Modify: `Cargo.toml`
- Modify: `src/acp/registry/mod.rs`
- Modify: `src/acp/registry/model.rs`
- Modify: `src/acp/install/state.rs`
- Test: `tests/acp_registry_cache.rs`

- [ ] **Step 1: Add failing refresh tests with a fake HTTP source**

Extend `tests/acp_registry_cache.rs` with:

- one test that loads cached data immediately and then applies fresher remote data
- one test that keeps cached data when remote fetch fails
- one test that marks `update_available` when `latest_registry_version > installed_version`

Use a tiny local HTTP handler or a deterministic fake fetch trait instead of real network.

- [ ] **Step 2: Run the refresh-focused tests to capture missing fetch behavior**

Run: `cargo test --test acp_registry_cache refresh -- --nocapture`

Expected: FAIL with missing fetch/update logic.

- [ ] **Step 3: Add the new dependencies and implement fetch logic**

Add runtime dependencies in `Cargo.toml`:

```toml
[dependencies]
semver = "1"
sha2 = "0.10"
ureq = "2"
zip = "2"
flate2 = "1"
tar = "0.4"

[dev-dependencies]
tempfile = "3"
```

Create `src/acp/registry/fetch.rs` with:

- blocking fetch inside worker-friendly functions
- ETag-aware refresh using `registry-meta.json`
- cached-first `load_then_refresh` API
- update detection comparing `installed_version` and `latest_registry_version`

- [ ] **Step 4: Re-run refresh tests and full suite**

Run:

```bash
cargo test --test acp_registry_cache -- --nocapture
cargo test -- --nocapture
```

Expected: refresh tests PASS and update detection behaves deterministically.

- [ ] **Step 5: Commit refresh and update detection**

Run:

```bash
git add Cargo.toml src/acp/registry/fetch.rs src/acp/registry/mod.rs src/acp/registry/model.rs src/acp/install/state.rs tests/acp_registry_cache.rs
git commit -m "feat: add cached-first ACP registry refresh"
```

### Task 6: Implement Effective Resolution Across Managed, Global, And Workspace Sources

**Files:**
- Modify: `src/acp/manager.rs`
- Modify: `src/acp/resolve.rs`
- Test: `tests/acp_resolve.rs`

- [ ] **Step 1: Add failing tests for all three source combinations**

Extend `tests/acp_resolve.rs` with cases for:

- managed + global custom duplicate `id`
- managed + workspace custom duplicate `id`
- `ShowBoth` with two rows and two `agent_key` values
- `RegistryWins` and `LocalWins` preserving alternate-source actions

- [ ] **Step 2: Run the resolver tests and confirm at least one new case fails**

Run: `cargo test --test acp_resolve -- --nocapture`

Expected: FAIL on the newly added merge cases.

- [ ] **Step 3: Narrow `AgentRegistry` to custom-agent compatibility and wire the resolver**

Modify `src/acp/manager.rs` so it stays responsible for loading custom `AgentSpec` sources, but no longer acts as the final merged catalog. Add helpers that:

- load global custom `agents.json`
- load workspace-local `agents.json`
- return source-tagged entries ready for `src/acp/resolve.rs`

Extend `src/acp/resolve.rs` so `resolve_agent(agent_key)` returns the concrete runtime `AgentSpec`.

- [ ] **Step 4: Re-run resolver tests and a focused full suite**

Run:

```bash
cargo test --test acp_resolve -- --nocapture
cargo test -- --nocapture
```

Expected: all conflict-policy tests PASS.

- [ ] **Step 5: Commit the merged-resolution path**

Run:

```bash
git add src/acp/manager.rs src/acp/resolve.rs tests/acp_resolve.rs
git commit -m "feat: merge managed and custom ACP sources"
```

## Chunk 3: Managed Installs And Runtime Integration

### Task 7: Implement Managed Install State And `npx` / `uvx` Wrappers

**Files:**
- Create: `src/acp/install/runner.rs`
- Modify: `src/acp/install/mod.rs`
- Modify: `src/acp/install/state.rs`
- Test: `tests/acp_installers.rs`

- [ ] **Step 1: Write failing wrapper-generation tests**

Create `tests/acp_installers.rs` with tests for:

- generating a pinned `npx` wrapper
- generating a pinned `uvx` wrapper
- removing wrapper files without touching external caches

Example:

```rust
#[test]
fn npx_wrapper_pins_package_version() {
    let launch = orbitshell::acp::install::runner::build_npx_launch(
        "@zed-industries/codex-acp",
        "0.10.0"
    );
    assert!(launch.command.contains("npx"));
    assert!(launch.args.iter().any(|arg| arg.contains("@0.10.0")));
}
```

- [ ] **Step 2: Run installer tests to capture missing runner behavior**

Run: `cargo test --test acp_installers -- --nocapture`

Expected: FAIL with unresolved runner helpers.

- [ ] **Step 3: Implement wrapper generation and managed-state updates**

Create `src/acp/install/runner.rs` with:

- wrapper-generation helpers for `npx`
- wrapper-generation helpers for `uvx`
- command-resolution structs reused by runtime

Update `src/acp/install/state.rs` so installs are versioned and the active version is state-driven.

- [ ] **Step 4: Re-run installer tests and full suite**

Run:

```bash
cargo test --test acp_installers -- --nocapture
cargo test -- --nocapture
```

Expected: targeted wrapper tests PASS.

- [ ] **Step 5: Commit managed wrappers**

Run:

```bash
git add src/acp/install/mod.rs src/acp/install/runner.rs src/acp/install/state.rs tests/acp_installers.rs
git commit -m "feat: add managed npx and uvx ACP installs"
```

### Task 8: Implement Verified `binary` Installs

**Files:**
- Create: `src/acp/install/binary.rs`
- Modify: `src/acp/install/mod.rs`
- Modify: `src/acp/install/state.rs`
- Test: `tests/acp_installers.rs`

- [ ] **Step 1: Add failing binary verification tests**

Extend `tests/acp_installers.rs` with cases for:

- SHA-256 mismatch aborts activation
- verified artifact becomes the active version
- unsupported or unverifiable binary metadata is rejected

- [ ] **Step 2: Run the binary-focused tests**

Run: `cargo test --test acp_installers binary -- --nocapture`

Expected: FAIL with missing binary installer behavior.

- [ ] **Step 3: Implement download, verify, extract, and activate**

Create `src/acp/install/binary.rs` with:

- artifact download helper
- SHA-256 verification
- zip/tar extraction
- state-file activation update only after verification succeeds

Expose the module from `src/acp/install/mod.rs`.

- [ ] **Step 4: Re-run binary tests and full suite**

Run:

```bash
cargo test --test acp_installers -- --nocapture
cargo test -- --nocapture
```

Expected: checksum mismatch test PASS by failing safely; verified install test PASS by activating the expected version.

- [ ] **Step 5: Commit binary install support**

Run:

```bash
git add src/acp/install/mod.rs src/acp/install/binary.rs src/acp/install/state.rs tests/acp_installers.rs
git commit -m "feat: add verified binary ACP installs"
```

### Task 9: Inject Effective Agents And Global MCP Servers Into Runtime

**Files:**
- Modify: `src/acp/client.rs`
- Modify: `src/ui/views/agent_view.rs`
- Modify: `src/ui/views/tab_view.rs`
- Modify: `src/ui/mod.rs`
- Create: `src/mcp/probe.rs`
- Test: `tests/acp_runtime_mcp.rs`

- [ ] **Step 1: Write failing runtime payload tests**

Create `tests/acp_runtime_mcp.rs` with a test that asserts `session/new` receives normalized enabled MCP servers:

```rust
#[test]
fn session_new_uses_enabled_global_mcp_servers() {
    // build config with one enabled and one disabled server
    // map to runtime payload
    // assert only enabled server is present
}
```

Add a second test ensuring `resolve_agent(agent_key)` feeds `AgentView` or `TabView` with the correct source-selected `AgentSpec`.

- [ ] **Step 2: Run the runtime-focused tests**

Run: `cargo test --test acp_runtime_mcp -- --nocapture`

Expected: FAIL with missing MCP injection or agent-key-based resolution.

- [ ] **Step 3: Update runtime plumbing**

Modify `src/acp/client.rs` so `ensure_session` accepts a normalized MCP server list instead of hardcoding `mcpServers: []`.

Modify `src/ui/mod.rs`, `src/ui/views/agent_view.rs`, and `src/ui/views/tab_view.rs` so they:

- resolve the effective agent list through the resolver
- keep selection by `agent_key`
- pass enabled global MCP servers into `session/new`

Create `src/mcp/probe.rs` for reusable MCP test/status helpers used by both runtime and Settings.

- [ ] **Step 4: Re-run runtime tests and full suite**

Run:

```bash
cargo test --test acp_runtime_mcp -- --nocapture
cargo test -- --nocapture
```

Expected: MCP payload test PASS; effective-agent selection test PASS.

- [ ] **Step 5: Commit runtime integration**

Run:

```bash
git add src/acp/client.rs src/ui/mod.rs src/ui/views/agent_view.rs src/ui/views/tab_view.rs src/mcp/probe.rs tests/acp_runtime_mcp.rs
git commit -m "feat: resolve ACP runtime from registry and global MCP"
```

## Chunk 4: Settings UI

### Task 10: Replace The Current ACP Registry UI With The Unified Catalog

**Files:**
- Modify: `src/ui/views/settings_view.rs`
- Modify: `src/acp/registry/fetch.rs`
- Modify: `src/acp/resolve.rs`
- Test: `tests/acp_resolve.rs`

- [ ] **Step 1: Add failing view-model tests around filtering and duplicate rows**

If `SettingsView` is too UI-heavy to unit-test directly, add pure helper tests in `tests/acp_resolve.rs` for:

- `Installed` filter
- `Not Installed` filter
- `Update Available` filter
- `Other sources` disclosure rows

- [ ] **Step 2: Run the targeted tests**

Run: `cargo test --test acp_resolve filters -- --nocapture`

Expected: FAIL on missing filter or disclosure helpers.

- [ ] **Step 3: Rework `SettingsView` to consume the merged catalog**

Update `src/ui/views/settings_view.rs` to:

- render cached data immediately
- trigger background refresh on open
- expose manual `Refresh`
- render search, filters, badges, `Install`, `Update`, `Authenticate`, `Test`, `Remove`
- render `Other sources` disclosure when duplicates exist under a winning policy
- show `cached` and `refresh error` status text

Keep business logic in ACP modules; `SettingsView` should only orchestrate state and rendering.

- [ ] **Step 4: Re-run targeted tests and a debug build**

Run:

```bash
cargo test --test acp_resolve -- --nocapture
cargo build
```

Expected: targeted tests PASS and debug build succeeds.

- [ ] **Step 5: Commit the ACP Registry UI**

Run:

```bash
git add src/ui/views/settings_view.rs src/acp/registry/fetch.rs src/acp/resolve.rs tests/acp_resolve.rs
git commit -m "feat: add unified ACP registry settings UI"
```

### Task 11: Implement MCP Servers CRUD And Connection Testing UI

**Files:**
- Modify: `src/ui/views/settings_view.rs`
- Modify: `src/mcp/config.rs`
- Modify: `src/mcp/probe.rs`
- Test: `tests/mcp_config.rs`

- [ ] **Step 1: Add failing MCP CRUD and probe tests**

Extend `tests/mcp_config.rs` with:

- add/edit/delete round-trip tests
- status update tests after a probe result
- serialization tests for `command`-backed and `url`-backed servers

- [ ] **Step 2: Run MCP tests to capture missing CRUD and status behavior**

Run: `cargo test --test mcp_config -- --nocapture`

Expected: FAIL on missing save/delete/probe status behavior.

- [ ] **Step 3: Implement MCP settings interactions**

Update `src/mcp/config.rs` and `src/mcp/probe.rs` so the Settings page can:

- create servers
- edit servers
- delete servers
- run connection tests
- persist `last_tested_at`, `last_status`, and `last_error`

Update `src/ui/views/settings_view.rs` to render the MCP list and action controls using those modules.

- [ ] **Step 4: Re-run MCP tests and a release build**

Run:

```bash
cargo test --test mcp_config -- --nocapture
cargo build --release
```

Expected: MCP tests PASS and release build succeeds.

- [ ] **Step 5: Commit MCP settings UI**

Run:

```bash
git add src/ui/views/settings_view.rs src/mcp/config.rs src/mcp/probe.rs tests/mcp_config.rs
git commit -m "feat: add global MCP settings management"
```

### Task 12: Final Verification Pass

**Files:**
- Modify: none expected
- Test: `tests/acp_storage.rs`
- Test: `tests/acp_registry_cache.rs`
- Test: `tests/acp_resolve.rs`
- Test: `tests/acp_installers.rs`
- Test: `tests/acp_runtime_mcp.rs`
- Test: `tests/mcp_config.rs`

- [ ] **Step 1: Run the focused integration suite**

Run:

```bash
cargo test --test acp_storage -- --nocapture
cargo test --test acp_registry_cache -- --nocapture
cargo test --test acp_resolve -- --nocapture
cargo test --test acp_installers -- --nocapture
cargo test --test acp_runtime_mcp -- --nocapture
cargo test --test mcp_config -- --nocapture
```

Expected: all six integration test files PASS.

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

- [ ] **Step 4: Launch the app and sanity-check the new Settings flows**

Run: `cargo run`

Expected:

- OrbitShell window opens
- `Settings > ACP Registry` shows cached-first list and later refreshes
- `Settings > MCP servers` shows CRUD and test actions

- [ ] **Step 5: Commit the final verified state**

Run:

```bash
git add Cargo.toml src tests
git commit -m "feat: ship ACP registry and global MCP management"
```
