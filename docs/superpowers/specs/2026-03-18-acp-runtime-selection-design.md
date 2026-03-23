# ACP Runtime Selection And Auth UX Design

## Summary

This spec extends the existing ACP Registry work with three runtime-facing improvements:

- replace arrow-based ACP switching with an explicit dropdown-style selector
- add optional model discovery and selection for ACPs that expose model metadata
- improve unauthenticated runtime UX without introducing logout management

The goal is to make ACP usage in OrbitShell feel intentional and inspectable. Agent choice should be explicit, model choice should exist when the ACP supports it, and auth failures should produce a clear recovery path instead of raw transport noise.

## Goals

- Replace previous/next ACP navigation with a real picker in the agent-facing UI.
- Persist a default model per ACP while allowing per-session override.
- Show a model dropdown only when the active ACP exposes available models.
- Surface auth-required failures as a clear UI state with actionable guidance.
- Preserve the current ACP transport/runtime architecture.

## Non-Goals

- Implementing logout flows.
- Forcing a manual model list when the ACP exposes no model metadata.
- Building a generic settings system for arbitrary per-agent runtime parameters.
- Changing the ACP protocol itself.

## Approved Product Decisions

- ACP selection should use a dropdown/select control instead of arrow stepping.
- Model selection uses a hybrid policy:
  - default model saved per `agent_key`
  - per-session override allowed in each tab/session
- Model UI is conditional:
  - if the ACP exposes models, show a dropdown
  - if not, hide model controls entirely
- When an unauthenticated user tries to talk to an ACP, OrbitShell should show a friendly auth-required state and keep `Authenticate` visible.
- Logout is intentionally out of scope for this iteration.

## Current Baseline

OrbitShell already supports:

- effective ACP resolution via `agent_key`
- runtime `initialize`, `session/new`, and `session/prompt`
- `Authenticate` actions for agents that define an auth command
- heuristic auth-error detection in `TabView`

Current gaps:

- `AgentView` still uses `<` / `>` buttons to switch ACPs
- there is no model discovery or model persistence layer
- `session/new` does not carry a selected model
- auth failures can still surface as low-level stderr or transport text before the UI becomes helpful

## User Experience

### ACP Picker

Both `AgentView` and `TabView` should show a compact ACP selector near the top of the UI.

Requirements:

- the current ACP is shown explicitly by name
- opening the selector shows every effective ACP row available to the surface
- duplicate IDs remain distinguishable by source label
- changing the selected ACP resets runtime state tied to the previous ACP

The picker should bind to `agent_key`, not plain ACP `id`.

### Model Picker

The model picker is attached to the currently selected ACP.

Rules:

- if the ACP exposes one or more models, show a dropdown
- if the ACP exposes no models, render nothing
- if a saved default model exists for the active `agent_key`, preselect it
- if the current session has overridden the model, the session value wins until the session ends or the user changes it again

The model picker should not appear disabled with placeholder text when no models are available. Hidden is the correct UX for this release.

### Auth UX

When a prompt or session creation fails due to missing authentication:

- OrbitShell marks the active ACP runtime state as `auth required`
- the output/log shows a plain-language message explaining that the ACP needs login before it can answer
- the `Authenticate` action remains visible
- retrying after successful auth should clear the auth-required state

OrbitShell should prefer a readable UI message over raw ANSI-heavy stderr when it can confidently classify the failure as auth-related.

Auth classification priority should be fixed for this iteration:

1. structured ACP error payloads or well-typed request failures, when available
2. known stderr patterns emitted by specific ACP runtimes
3. textual heuristics over the final user-visible error string

OrbitShell should only fall through to the next source when the higher-priority source is absent or inconclusive.

Structured classification covers typed responses such as `AcpError` objects, `AuthRequired` codes returned from `session/new`, or deterministic handshake failures reported by `AcpTransport`. Known stderr patterns come from a curated list of trusted ACP runtimes (for example Dex-coded `Auth(TokenRefused)` messages). Textual heuristics only run when both structured payloads and known stderr patterns are missing; they focus on long-standing marker strings (`"auth required"`, HTTP 401/403, `TokenExpired`) and must include a confidence threshold before changing the runtime state.

## Data Model

### Agent Runtime Preferences

OrbitShell needs persisted runtime preferences keyed by `agent_key`.

Minimum shape:

- `agent_key`
  - `source_type`
  - `source_id`
  - `acp_id`
- `default_model: Option<String>`

Suggested storage:

- `%APPDATA%/orbitshell/acp-runtime-preferences.json`

### Runtime Session State

Each live UI surface that can talk to an ACP should track:

- selected `agent_key`
- discovered models for the selected agent, if any
- selected session model override, if any
- whether auth is currently required
- timestamp of the most recent model discovery for display purposes (this remains view-local and is not persisted)

This state is view-local and should not be persisted except for the default model preference.

### Discovered Model Metadata

OrbitShell should normalize ACP-provided model data into an internal shape:

- `id`
- `label`
- `description: Option<String>`
- `is_default: bool`

If the ACP only exposes plain model IDs, OrbitShell can use the ID as the label.

## Discovery Strategy

OrbitShell should use separate model discovery, not infer models from normal prompt traffic. The discovery helper confines itself to the metadata surfaces that ACP runtimes officially expose so the runtime state remains predictable.

Model discovery surfaces:

- Primary: the `agentCapabilities` response, especially the `modelCatalog`, `modelSelection`, or equivalent metadata fields that enumerate each candidate model along with labels, descriptions, and default flags.
- Secondary: the `agentInfo` response (or any capability-adjacent field such as `modelCatalog` nested inside it) if `agentCapabilities` does not include a usable catalog.
- Stop: the helper stops as soon as one surface yields a catalog and does not infer models from prompt traffic.

Preferred sequence:

1. connect to the selected ACP
2. initialize
3. inspect `agentCapabilities` for a model catalog or model-selection metadata
4. if step 3 is inconclusive, inspect `agentInfo` for model catalog metadata
5. normalize models if present
6. validate any saved default model against the newly discovered catalog and clear it immediately if it no longer exists
7. cache the result in memory for the current surface
8. persist only the user-selected default model, not the full model catalog

If no model catalog is exposed, OrbitShell stops there and keeps the picker hidden. The discovery path must be defensive because ACP support will vary by agent.

Every time discovery runs—when the runtime surface opens, when the user switches ACPs, or right before session creation—we revalidate the persisted default model for the current `agent_key`. Clearing happens before the picker is rendered so the UI can fall back to the ACP-declared default or a blank state without ever sending an invalid model to the runtime.

## Runtime Contract Changes

`session/new` needs to carry the effective selected model when one exists.

Effective selected model resolution:

1. session override, if present
2. persisted default model for the active `agent_key`, if present
3. ACP-declared default model, if one exists
4. omit the field entirely

OrbitShell must not send an empty string or fake placeholder model.

## Error Handling

OrbitShell should handle the following cases cleanly:

- ACP exposes no models
  - hide model UI
- ACP exposes malformed model metadata
  - ignore the catalog, hide the picker, log a diagnostic line
- saved default model no longer exists
  - clear the saved model during model discovery for that ACP and fall back to the ACP default or no model
- auth failure before prompt
  - show auth-required message and `Authenticate`
- auth failure during prompt
  - keep partial output if any, then append auth-required message
- model rejected by ACP
  - surface a readable message and allow the user to pick a different model

## Architecture

### 1. Runtime Preferences Layer

Responsibility:

- load and save per-agent runtime preferences
- resolve persisted default model by `agent_key`

Suggested interface:

- `load_runtime_preferences()`
- `save_runtime_preferences()`
- `default_model_for(agent_key)`
- `set_default_model(agent_key, model_id)`
- `clear_default_model(agent_key)`

### 2. Model Discovery Layer

Responsibility:

- discover model options from an ACP after initialization
- normalize agent-specific metadata into a common runtime list

Suggested interface:

- `discover_models(client) -> Result<Option<Vec<AcpModelOption>>>`

Discovery order is fixed:

1. inspect `agentCapabilities`
2. inspect `agentInfo`
3. stop and return `None`

This layer should remain ACP-protocol-adjacent and not live directly in the views.

### 3. View Runtime State

Responsibility:

- own the selected ACP in the current surface
- own the session-level model override
- react to auth-required and model-discovery updates

This state belongs in `AgentView` and `TabView`, not in global settings.

## UI Touch Points

### `AgentView`

- replace arrow controls with a dropdown-like ACP selector
- add a conditional model dropdown
- preserve `Authenticate`
- display readable auth-required feedback in output
- reset client state, discovered models, and session override state when the selected ACP changes

### `TabView`

- replace ACP cycling affordance with explicit selection UI
- add conditional model dropdown in agent mode
- pass selected model into ACP session creation
- keep auth-required call-to-action visible in the tab header
- reset client state, discovered models, and session override state when the selected ACP changes

### Settings

No large new Settings surface is required for this iteration. The default model can be managed from runtime surfaces and persisted automatically.

## Testing Strategy

### Unit Tests

- runtime preferences round-trip by `agent_key`
- effective model resolution order
- stale saved model is ignored when absent from discovered catalog
- auth-related error classification stays deterministic

### Integration Tests

- `session/new` includes selected model when a session override exists
- `session/new` includes saved default model when no session override exists
- model field is omitted when no models are available
- auth failure flips runtime state into auth-required

### UI-Oriented Smoke Checks

- ACP selector shows explicit current selection
- model dropdown appears only for ACPs with discovered models
- unauthenticated prompt attempt shows readable recovery guidance

## Open Constraints For Planning

- preserve existing `agent_key`-based resolution
- avoid pushing protocol/discovery logic directly into view rendering
- keep model discovery optional and defensive
- avoid introducing manual model configuration UX in this iteration

## Recommendation For Implementation Order

1. add runtime preferences persistence for default model by `agent_key`
2. add ACP model discovery and normalized runtime model metadata
3. extend `session/new` payload generation with selected model resolution
4. replace ACP arrow pickers with explicit selectors in `AgentView` and `TabView`
5. wire auth-required UX polish around runtime failures
