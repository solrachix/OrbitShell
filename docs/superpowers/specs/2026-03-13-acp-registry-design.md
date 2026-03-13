# ACP Registry And Global MCP Design

## Summary

This spec defines the first implementation of an ACP Registry experience in OrbitShell modeled after Zed's current flow, while preserving OrbitShell's existing local ACP support.

The feature adds:

- A unified `ACP Registry` catalog in Settings that combines official registry agents and local/custom agents.
- Managed installs stored under `%APPDATA%/orbitshell/...`.
- A conflict policy preference controlling how registry and local agents with the same `id` are resolved.
- Global MCP server configuration shared by all ACPs.
- Manual ACP updates with `Update available` state.
- Transparent confirmation before any install or update command runs.

This spec explicitly targets the first usable release. It does not try to replicate every Zed detail or every future ACP registry capability.

## Goals

- Let users discover ACPs from an official remote registry inside OrbitShell.
- Keep local/custom ACPs usable without breaking the current `agents.json` flow.
- Support managed installation for `npx`, `uvx`, and platform `binary` distributions.
- Keep MCP configuration global and reusable across all ACPs.
- Provide install, authenticate, test, remove, and update actions from Settings.
- Preserve clear separation between registry data, managed install state, and user-authored local config.

## Non-Goals

- Per-ACP MCP configuration.
- Silent background auto-update of ACPs.
- Full parity with every future Zed registry feature.
- Rich marketplace features such as ratings, screenshots, or reviews.
- Remote account sync of ACP and MCP settings.
- Replacing the current ACP runtime protocol layer. `AcpClient` and `AcpTransport` remain the protocol execution path.

## Current OrbitShell Baseline

OrbitShell already contains an early local ACP implementation:

- `src/acp/manager.rs` loads `agents.json`, defines `AgentSpec`, and supports `install` and `auth` commands.
- `src/acp/client.rs` and `src/acp/transport.rs` already speak ACP via `initialize`, `session/new`, and `session/prompt`.
- `src/ui/views/settings_view.rs` already has an `ACP Registry` section backed by local `AgentRegistry` data and a placeholder `MCP servers` section.
- `src/ui/views/agent_view.rs` and `src/ui/views/tab_view.rs` already let the user select and run ACPs.

The current limitation is that OrbitShell only knows about local `agents.json` entries. There is no remote registry fetch, managed install state, cache, merged catalog, or MCP configuration flow.

## Product Decisions Approved For This Design

- The UI is a single unified ACP list in Settings.
- Global MCP configuration is shared by all ACPs.
- Managed installs do not overwrite `agents.json`.
- The first cut supports `npx`, `uvx`, and `binary` distributions.
- The registry is fetched automatically when opening `Settings > ACP Registry`, with cached fallback.
- Install and update actions always show a confirmation UI with the exact command or artifact source before execution.
- Duplicate ACP `id` resolution is user-configurable:
  - `Local wins`
  - `Registry wins`
  - `Show both`
- ACP updates are manual, surfaced as `Update available`.
- Managed installs and registry cache live under `%APPDATA%/orbitshell/...`.
- MCP management supports CRUD plus connection/status testing.

## External References

The design is based on the current public ACP and Zed material:

- Zed ACP Registry announcement: `https://zed.dev/blog/acp-registry`
- Zed external agents docs: `https://zed.dev/docs/ai/external-agents`
- ACP Agent Registry RFD: `https://agentclientprotocol.com/rfds/acp-agent-registry`
- ACP protocol schema: `https://agentclientprotocol.com/protocol/schema`
- ACP registry repository: `https://github.com/agentclientprotocol/registry`

These sources establish the shape of the registry, supported distribution types, and the separation between ACP registry management and MCP support.

## User Experience

### Settings > ACP Registry

The ACP Registry page becomes a unified catalog with the following controls:

- Search input.
- Filters:
  - `All`
  - `Installed`
  - `Not Installed`
  - `Update Available`
- Conflict policy preference:
  - `Local wins`
  - `Registry wins`
  - `Show both`
- Status line showing:
  - whether the registry is live or cached
  - last refresh timestamp
  - errors when refresh fails

Settings is a global surface, but workspace-local ACP definitions are scoped to the workspace context of the current Settings tab. For the first cut:

- A Settings tab opened from a workspace tab uses that workspace path as its local ACP source.
- If the Settings tab has no workspace context, workspace-local `agents.json` entries are omitted.
- OrbitShell must not implicitly switch workspace-local ACP sources based on unrelated tab focus changes after the Settings tab is opened.

Each ACP row or card shows:

- Name
- Version
- Short description
- Agent `id`
- Source badge:
  - `Registry`
  - `Custom`
  - `Workspace`
- Status badge:
  - `Installed`
  - `Not Installed`
  - `Update available`
  - `Broken`
  - `Auth required`

Available actions per ACP:

- `Install`
- `Update`
- `Authenticate`
- `Test`
- `Remove`

Action visibility rules:

- `Install` is shown for registry ACPs not yet installed.
- `Update` is shown when a newer registry version exists than the managed install version.
- `Authenticate` is shown when the agent defines an auth command or the last runtime/test result indicates auth is required.
- `Test` is shown for any resolved effective ACP.
- `Remove` is shown for managed installs. For local-only custom agents, the UI does not delete the user-authored config automatically.

Duplicate source handling:

- Under `Show both`, both sources are rendered as separate rows with distinct source badges and internal identity.
- Under `Local wins` or `Registry wins`, the winning source is shown as the primary row.
- When a losing source still exists, the primary row exposes an `Other sources` disclosure so the user can still inspect, test, update, or remove the non-winning source.
- A hidden losing source must never become operationally unreachable.

### Confirmation Before Install Or Update

Before installation or update, OrbitShell opens a confirmation surface that includes:

- ACP name and version
- Distribution type
- Exact command to run for `npx` or `uvx`
- Exact download URL and expected executable path for `binary`
- Install destination under `%APPDATA%/orbitshell/...`

The user must confirm before execution begins.

### Settings > MCP Servers

The MCP Servers section becomes a real configuration surface for global MCP state.

Capabilities:

- List configured MCP servers
- Add a new MCP server
- Edit an existing MCP server
- Delete an MCP server
- Test connection
- Show status badge:
  - `Online`
  - `Offline`
  - `Misconfigured`

This section is global. It is not bound per ACP in the first cut.

## Storage Layout

OrbitShell stores registry and MCP state under `%APPDATA%/orbitshell/`.

### Files And Directories

- `%APPDATA%/orbitshell/registry/cache/registry-index.json`
  - Cached registry index payload.
- `%APPDATA%/orbitshell/registry/cache/registry-meta.json`
  - Cache metadata including `last_fetch`, `etag`, and `ttl_seconds`.
- `%APPDATA%/orbitshell/registry/cache/manifests/<id>.json`
  - Cached, normalized manifest for each registry ACP.
- `%APPDATA%/orbitshell/registry/state/managed-agents.json`
  - Managed install state and derived status.
- `%APPDATA%/orbitshell/registry/installs/<id>/<version>/...`
  - Files created by managed install operations.
- `%APPDATA%/orbitshell/mcp-servers.json`
  - Global MCP server configuration.
- `%APPDATA%/orbitshell/preferences.json`
  - User preferences for registry behavior.
- `%APPDATA%/orbitshell/agents.json`
  - Optional global custom ACP definitions.

### Existing Local Files

The current project-local `agents.json` remains supported as an additional source.

OrbitShell discovers three sources:

1. Managed registry installs
2. Global custom agents from `%APPDATA%/orbitshell/agents.json`
3. Workspace-local custom agents from the current working directory `agents.json`

This discovery order does not decide conflict winners. Conflict winners are chosen only by the user-configured conflict policy.

This keeps existing behavior while introducing a real global settings model.

## Data Model

### Registry Catalog Entry

A normalized catalog entry should include:

- `id`
- `name`
- `version`
- `description`
- `homepage`
- `repository`
- `distribution`
- `auth_methods`
- `platform_support`
- `source = registry`
- `manifest_cache_path`

OrbitShell should normalize the remote registry into an internal model rather than pass raw JSON through the UI.

### Managed Install State

Each installed registry ACP should track:

- `id`
- `installed_version`
- `latest_registry_version`
- `distribution_kind`
- `install_root`
- `resolved_command`
- `resolved_args`
- `active_version`
- `last_install_at`
- `last_checked_at`
- `status`
- `auth_required`
- `install_error`

The first cut only requires update decisions against the installed version and latest registry version. It does not require full version history or user-selectable rollback UX, though the storage layout must not block that later.

### Custom Agent Entry

Custom agents continue using the current `AgentSpec` shape:

- `id`
- `name`
- `command`
- `args`
- `env_keys`
- `install`
- `auth`

For merge purposes they also need a source marker:

- `source = global_custom` or `workspace_custom`

### Effective Agent Identity

The merged catalog cannot use raw ACP `id` as its only identity, because duplicate `id` values can remain visible under `Show both` and can remain manageable under the other conflict policies.

OrbitShell therefore needs a first-class composite identity:

- `agent_key`
  - `source_type`
  - `source_id`
  - ACP `id`

The UI, actions, diagnostics, and runtime resolution must use `agent_key`, not just ACP `id`.

### Preferences

Preferences should include:

- `conflict_policy`

The first cut does not expose registry URL or refresh cadence as user-facing preferences. Registry fetch-on-open is fixed behavior for this release.

### MCP Server Entry

MCP entries should contain enough information to render, test, and serialize global server config. The exact fields can follow the OrbitShell runtime needs, but at minimum the model must support:

- `id`
- `name`
- `transport`
- `command` or `url`
- `args`
- `env`
- `enabled`
- `last_tested_at`
- `last_status`
- `last_error`

The UI must not rely on opaque free-form JSON as its only editable representation.

## Architecture

### 1. Registry Catalog Layer

Responsibility:

- Fetch the remote ACP registry index.
- Cache the result locally.
- Fetch or refresh manifests for referenced ACPs.
- Normalize external registry data into OrbitShell catalog entries.

Interface:

- `refresh_registry() -> RegistryRefreshResult`
- `load_cached_registry() -> RegistryCatalog`
- `list_catalog_entries() -> Vec<RegistryCatalogEntry>`

This layer does not decide which ACP is effective for runtime.

### 2. Managed Install Layer

Responsibility:

- Install ACPs from supported distribution types.
- Remove managed installs.
- Detect update availability.
- Persist install state.

Interface:

- `install(entry_id, version)`
- `update(entry_id)`
- `remove(entry_id)`
- `load_managed_state()`
- `save_managed_state()`

The install layer is split by distribution:

- `NpxInstaller`
- `UvxInstaller`
- `BinaryInstaller`

This keeps distribution-specific logic isolated and testable.

Materialization rules:

- `npx`
  - OrbitShell stores managed state plus a small wrapper script or launch definition under `%APPDATA%/orbitshell/registry/installs/<id>/<version>/`.
  - The wrapper must pin the package version explicitly in the invocation.
  - OrbitShell does not vendor the npm package payload into its own install directory for the first cut; package contents remain managed by the external npm cache.
- `uvx`
  - OrbitShell stores managed state plus a small wrapper script or launch definition under `%APPDATA%/orbitshell/registry/installs/<id>/<version>/`.
  - The wrapper must pin the package version explicitly in the invocation.
  - OrbitShell does not vendor the Python package payload into its own install directory for the first cut; package contents remain managed by the external uv cache.
- `binary`
  - OrbitShell downloads the artifact into the managed install directory, verifies it, and activates the resolved executable from that directory.

Activation rules:

- Managed installs are versioned under `installs/<id>/<version>/`.
- The active version is chosen through managed state metadata, not a filesystem symlink.
- Update activation must be atomic at the state-file level so OrbitShell never points at a partially installed version.

This avoids symlink-specific platform friction on Windows while still allowing future rollback support.

Remove semantics:

- Removing an `npx` or `uvx` managed install removes OrbitShell-managed wrappers and state only.
- OrbitShell does not attempt to garbage-collect shared external package-manager caches in the first cut.

### 3. Custom Agent Config Layer

Responsibility:

- Load global custom ACP config.
- Load workspace custom ACP config.
- Normalize both into a common internal format.

Interface:

- `load_global_custom_agents()`
- `load_workspace_custom_agents()`

This layer remains compatible with the existing `AgentRegistry` concepts.

### 4. Effective Agent Resolver

Responsibility:

- Merge registry-managed ACPs and custom/local ACPs.
- Apply conflict policy.
- Produce the effective catalog for the UI.
- Produce the effective `AgentSpec` for runtime execution.

Interface:

- `list_effective_agents() -> Vec<EffectiveAgentRow>`
- `resolve_agent(agent_key) -> Option<ResolvedAgentSpec>`
- `list_alternate_sources(acp_id) -> Vec<EffectiveAgentRow>`

This is the only layer that decides which agent wins under conflict.

### 5. MCP Configuration Layer

Responsibility:

- Persist global MCP server config.
- Validate and test server definitions.
- Produce runtime MCP entries for ACP session creation.

Interface:

- `list_mcp_servers()`
- `save_mcp_server()`
- `delete_mcp_server()`
- `test_mcp_server()`
- `resolve_runtime_mcp_servers()`

### 6. ACP Runtime Layer

Responsibility:

- Connect to the resolved ACP command.
- Run `initialize`, `session/new`, and `session/prompt`.

This remains the job of the existing `AcpClient` and `AcpTransport`. The runtime should change as little as possible.

## Runtime Flow

### Opening ACP Registry

1. User opens `Settings > ACP Registry`.
2. OrbitShell resolves the Settings tab workspace context, if any, for workspace-local custom ACP discovery.
3. OrbitShell loads the cached registry immediately, if present.
4. Managed installs and custom agents are loaded.
5. Effective merge is computed from cached data and rendered immediately.
6. OrbitShell starts an asynchronous registry refresh in the background.
7. If refresh succeeds:
   - cache payload and metadata are updated
   - manifests are refreshed as needed
   - update availability is recomputed
   - the UI refreshes in place
8. If refresh fails:
   - cached content remains visible
   - UI shows cached mode and the refresh error

The page also exposes a manual `Refresh` action.

### Installing An ACP

1. User clicks `Install`.
2. OrbitShell shows confirmation with exact command or artifact source.
3. On confirm, the matching installer runs.
4. Managed install state is persisted.
5. Effective merge is recomputed.
6. UI updates to `Installed` or `Broken`.

### Updating An ACP

1. Registry refresh marks `Update available`.
2. User clicks `Update`.
3. OrbitShell shows the exact update operation.
4. The installer installs the newer version side-by-side or replaces the previous managed state in a controlled way.
5. Managed state is updated.
6. Old install files may be cleaned up after success.

### Removing An ACP

1. User clicks `Remove`.
2. OrbitShell removes only the managed install state and managed install files.
3. Custom/local definitions are left untouched.
4. Effective merge is recomputed.

### Testing An ACP

The test action reuses the current handshake shape:

1. Resolve effective runtime `AgentSpec` from `agent_key`.
2. Spawn the ACP command.
3. Run `initialize`.
4. Run `session/new` with global MCP servers.
5. Report success or precise failure.

### Using MCP At Runtime

When an ACP session is created:

- OrbitShell loads global MCP server config.
- Enabled and valid MCP server entries are mapped into the `mcpServers` payload.
- The same global list is supplied to all ACPs.

The runtime does not need to know which MCP server was created from which UI action. It only consumes a normalized runtime list.

## Conflict Policy Semantics

### Local Wins

If a managed registry ACP and custom ACP share the same `id`, the local or custom entry is the effective runtime entry.

### Registry Wins

If a managed registry ACP and custom ACP share the same `id`, the registry-backed managed entry is the effective runtime entry.

### Show Both

Both entries remain visible and selectable in the UI, but they must be uniquely distinguishable. The UI must show source and enough label detail that the user can tell them apart.

Implementation note:

The resolver must avoid ambiguous row identity even if two visible entries share the same ACP `id`. The effective list therefore needs a stable composite key based on source and source-local identity, not just source plus ACP `id`.

## Error Handling

The first cut must handle the following cases cleanly:

- Registry fetch failure
  - Use cache and show cached mode.
- Invalid registry or manifest data
  - Ignore invalid entries, report parsing failure in diagnostics, do not crash Settings.
- Unsupported distribution
  - Show `Unavailable on this platform`.
- Install command missing
  - Show actionable hint, for example missing Node or Python tooling.
- Binary download or extraction failure
  - Leave previous managed state intact when possible.
- Binary checksum verification failure
  - Abort install or update and do not activate the downloaded artifact.
- Broken managed install
  - Mark `Broken` and surface reinstall or remove.
- ACP auth failure
  - Mark `Auth required` and surface `Authenticate`.
- MCP test failure
  - Keep saved config, mark failing status, show error summary.
- Workspace custom file parse failure
  - Report source-specific error without hiding valid registry entries.

## Security And Trust Model

OrbitShell executes third-party commands and downloads external binaries in this feature. The first cut therefore requires:

- explicit confirmation before install or update
- visible artifact or command source
- install destination shown to the user
- no silent auto-update
- SHA-256 checksum verification for managed `binary` installs before activation

Binary distribution rule for the first cut:

- A binary artifact must provide a stable versioned URL plus `sha256` metadata usable by OrbitShell.
- Artifact metadata should also include byte size when available, but `sha256` is the required integrity check for the first cut.
- If a binary distribution entry cannot be verified, OrbitShell marks it unavailable for managed install rather than downloading and running it blindly.

This is intentionally stricter than a hidden background installer.

## Testing Strategy

### Unit Tests

- Registry index parsing and normalization
- Manifest parsing and normalization
- Conflict resolution for all three policies
- Managed state persistence
- Preferences persistence
- MCP config persistence
- Distribution command resolution for `npx`, `uvx`, and `binary`

### Integration Tests

- Refresh with remote success
- Refresh with remote failure and cached fallback
- Install flow for each supported distribution using fake installers
- Remove flow
- Update available detection
- Global plus workspace agent merge behavior
- MCP runtime payload generation

### Runtime Smoke Tests

- Effective ACP can complete `initialize`
- Effective ACP can complete `session/new`
- Auth errors surface the correct state

The implementation should prefer fake or stub agents and fake installers where possible, so the tests are deterministic and do not rely on public registries or package managers.

## Implementation Shape In The Existing Codebase

The first plan should likely introduce these focused modules:

- `src/acp/registry/`
  - remote catalog fetching
  - cache loading
  - manifest normalization
- `src/acp/install/`
  - install state
  - per-distribution installers
- `src/acp/resolve/`
  - merge and conflict policy
- `src/mcp/`
  - MCP config model
  - persistence
  - connectivity test helpers

Likely UI touch points:

- `src/ui/views/settings_view.rs`
  - real ACP Registry page
  - real MCP servers page
- `src/ui/views/agent_view.rs`
  - use effective resolved agents
- `src/ui/views/tab_view.rs`
  - use effective resolved agents
- `src/acp/manager.rs`
  - evolve from single-source `agents.json` loader into compatibility layer or move responsibilities into the new resolver

The plan should keep individual files and modules focused. The install layer, registry layer, resolver layer, and MCP layer should remain independently understandable and testable.

## Open Design Constraints For Planning

These are not unresolved requirements. They are planning constraints:

- Preserve current ACP protocol runtime behavior unless the registry flow requires a narrow change.
- Do not break existing workspace-local `agents.json`.
- Avoid coupling registry fetch logic directly into view rendering.
- Avoid making the `SettingsView` own business logic that belongs in ACP or MCP state modules.
- Keep distribution-specific install logic out of the generic registry catalog code.

## Known First-Cut Tradeoffs

- `npx` and `uvx` managed installs use pinned wrappers plus external package-manager caches, not vendored package payloads. This is acceptable for the first release, but it is weaker than full artifact vendoring for long-term reproducibility.
- Runtime process isolation is unchanged in the first cut. ACPs still execute as local child processes with the current trust model. Sandboxing is a future hardening topic, not part of this release.

## Recommended First Implementation Order

1. Add persistence models and filesystem layout for registry cache, managed installs, preferences, and MCP config.
2. Add remote registry fetch plus cached fallback.
3. Add effective merge and conflict policy resolution.
4. Switch the existing ACP UI to read the effective catalog instead of raw `agents.json`.
5. Add managed install and remove flows for `npx`, `uvx`, and `binary`.
6. Add update detection and update action.
7. Add real MCP CRUD plus connection testing.
8. Inject global MCP servers into ACP `session/new`.

This order delivers visible value early while keeping the runtime stable.
