# ACP Approval And Terminal Performance Implementation Plan

I'm using the writing-plans skill to create the implementation plan.

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add full inline ACP permission approval support and remove the most visible UI freezes in OrbitShell's agent terminal flow, especially when sending prompts, rendering large blocks, and scrolling long outputs.

**Architecture:** Extend the ACP transport/client stack so OrbitShell can handle agent-originated JSON-RPC requests such as `session/request_permission`, surface them inline inside the active terminal block, and return the user's choice back to the agent. In parallel, reduce synchronous work on the UI thread and cap/render output more efficiently so prompt submission, chunk streaming, and long-block scrolling remain responsive.

**Tech Stack:** Rust 2024, GPUI, existing ACP transport/client/runtime stack, `TabView`, Cargo unit tests, local UI validation with `cargo run`.

**UX Priorities:**
1. Show immediate visual feedback on `Enter` and eliminate the most obvious submit freeze.
2. Support inline ACP approval requests inside the active conversation block.
3. Reduce render/scroll degradation for large streamed blocks and stderr-heavy output.
4. Render ACP text responses as Markdown where the protocol provides formatted text.

---

## File Structure

### Existing files to modify

- `src/acp/transport.rs`
  - Support inbound JSON-RPC requests from the ACP process and allow the client to respond with success or error.
- `src/acp/client.rs`
  - Add permission-request handling, broaden update parsing, and expose a callback/event surface suitable for UI approval prompts.
- `src/ui/views/tab_view.rs`
  - Render inline approval cards inside the active block, remove synchronous prompt-submission jank, and optimize block update/scroll behavior.
- `src/ui/views/settings_view.rs`
  - Only if needed for shared ACP state presentation or approval-related status copy; otherwise leave untouched.
- `src/acp/manager.rs`
  - Only if the permission flow needs explicit spec metadata for capabilities or auth copy.

### Existing tests to extend

- `src/acp/client.rs`
  - Add tests for ACP request parsing, permission payload extraction, and richer streaming payload shapes.
- `src/ui/views/tab_view.rs`
  - Add tests for inline approval state, optimistic prompt block creation, and placeholder/status transitions.

### New focused validation targets

- `cargo test --lib`
- `cargo check`
- Manual validation via `cargo run`:
  - send prompt in agent mode
  - trigger permission request
  - approve and reject flows
  - long-response scrolling
  - large stderr/output rendering
  - markdown-heavy ACP responses with lists, emphasis, and code fences

---

## Chunk 1: ACP Approval Protocol Support

### Task 1: Teach The Transport Layer To Handle Inbound ACP Requests

**Files:**
- Modify: `src/acp/transport.rs`
- Test: `src/acp/client.rs`

- [ ] **Step 1: Write failing tests for inbound ACP requests**

Add unit tests that model inbound JSON-RPC messages with:

```rust
{
  "jsonrpc": "2.0",
  "id": 99,
  "method": "session/request_permission",
  "params": { ... }
}
```

Test expectations:
- request-shaped messages are distinguished from notifications
- the handler can return a JSON result payload
- the transport can send back either success or error for the inbound request

- [ ] **Step 2: Run the targeted tests and confirm failure**

Run:

```bash
cargo test request_permission --lib -- --nocapture
```

Expected: FAIL because the transport currently treats all `method` messages as fire-and-forget notifications.

- [ ] **Step 3: Implement request-aware transport handling**

In `src/acp/transport.rs`:

- detect when an inbound JSON message has both `id` and `method`
- route it through a dedicated request callback, not the existing notification callback
- add helpers to send:
  - JSON-RPC result
  - JSON-RPC error
- preserve current notification behavior for `session/update` and `stderr`

Treat `session/request_permission` as the expected first method to support, but do not hardcode the transport around that single name. Unknown inbound request methods must be capturable/loggable so OrbitShell can surface protocol mismatches instead of silently failing.

Keep the API small and explicit so `AcpClient` can decide how to map request methods to UI events.

- [ ] **Step 4: Re-run the targeted tests**

Run:

```bash
cargo test request_permission --lib -- --nocapture
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/acp/transport.rs src/acp/client.rs
git commit -m "feat: support inbound acp requests"
```

### Task 2: Add ACP Permission Request Modeling In The Client

**Files:**
- Modify: `src/acp/client.rs`
- Test: `src/acp/client.rs`

- [ ] **Step 1: Write failing client tests for permission extraction**

Add tests covering:
- extract approval request metadata from `session/request_permission`
- preserve tool/action/title/description if present
- map approval response choices to ACP result payloads

Representative cases:
- allow once
- allow always
- reject
- malformed/unknown payload

- [ ] **Step 2: Run targeted tests and confirm failure**

Run:

```bash
cargo test permission_request --lib -- --nocapture
```

Expected: FAIL because the client has no permission model yet.

- [ ] **Step 3: Implement permission request data model and callback flow**

In `src/acp/client.rs`:

- add a small typed struct for permission requests, for example:
  - `session_id`
  - `request_id`
  - `tool_name`
  - `title`
  - `description`
  - `risk_level` or equivalent if present
  - raw params for forward compatibility if needed
- extend `prompt`/request handling so the UI can receive a permission request event while the ACP call is in flight
- add a method to resolve a permission request with:
  - allow once
  - allow always
  - reject

`Allow always` in this plan means "return the persistent/permissive choice the ACP protocol expects for that request." It does **not** imply OrbitShell must persist its own separate approval database unless a later spec adds that requirement.

Do not hardcode the UI here; keep the client focused on protocol mapping.

- [ ] **Step 4: Re-run targeted tests**

Run:

```bash
cargo test permission_request --lib -- --nocapture
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/acp/client.rs
git commit -m "feat: model acp permission requests"
```

## Chunk 2: Inline Approval UX In The Terminal

### Task 3: Add Inline Approval State To Agent Blocks

**Files:**
- Modify: `src/ui/views/tab_view.rs`
- Test: `src/ui/views/tab_view.rs`

- [ ] **Step 1: Write failing tests for approval block state**

Add tests for:
- active block can enter `awaiting_permission`
- permission prompt renders inside the current agent block rather than a modal
- prompt submission keeps the block visible while waiting for user choice
- resolving approval updates block state without creating duplicate blocks
- prompt -> approval -> approval resolved -> streamed response all stay in the same block/history entry

- [ ] **Step 2: Run targeted tests and confirm failure**

Run:

```bash
cargo test approval_block --lib -- --nocapture
```

Expected: FAIL because the terminal block model has no permission UI state.

- [ ] **Step 3: Implement inline approval block state**

In `src/ui/views/tab_view.rs`:

- extend block/runtime state to hold pending approval data
- render an inline approval card inside the active block with:
  - concise tool/action summary
  - optional description/details
  - `Permitir uma vez`
  - `Sempre permitir`
  - `Negar`
- ensure this card is part of the conversation history and scrolls naturally with the block
- disable repeated prompt submission while an approval is unresolved for that block
- keep approval UI and subsequent streamed answer in the same block instead of splitting history into synthetic extra blocks

Keep the visual language compact and consistent with the existing dark terminal UI.

- [ ] **Step 4: Re-run targeted tests**

Run:

```bash
cargo test approval_block --lib -- --nocapture
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/ui/views/tab_view.rs
git commit -m "feat: add inline acp approval ui"
```

### Task 4: Wire Approval Decisions Back To The ACP Request

**Files:**
- Modify: `src/ui/views/tab_view.rs`
- Modify: `src/acp/client.rs`
- Test: `src/ui/views/tab_view.rs`

- [ ] **Step 1: Write failing integration-style tests for approval decisions**

Add tests for:
- clicking allow once resolves the pending permission request
- clicking always allow resolves it with the persistent choice payload expected by ACP
- clicking reject resolves it as denial and the block remains coherent
- after resolution, streamed updates can continue inside the same block

- [ ] **Step 2: Run targeted tests and confirm failure**

Run:

```bash
cargo test approval_resolution --lib -- --nocapture
```

Expected: FAIL because the UI does not yet bind actions back to the in-flight ACP request.

- [ ] **Step 3: Implement approval action wiring**

In `src/ui/views/tab_view.rs` and `src/acp/client.rs`:

- map the three inline buttons to client-side approval resolution
- show a small transient status in the same block after click, e.g.:
  - `[agent] permission granted`
  - `[agent] permission denied`
- ensure the in-flight prompt resumes or fails cleanly
- ensure repeated clicks are blocked once a choice is made

- [ ] **Step 4: Re-run targeted tests**

Run:

```bash
cargo test approval_resolution --lib -- --nocapture
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/acp/client.rs src/ui/views/tab_view.rs
git commit -m "feat: wire inline acp approval actions"
```

## Chunk 3: Prompt Submission Latency And Streaming Responsiveness

### Task 5: Remove Remaining UI-Thread Prompt Submission Jank

**Files:**
- Modify: `src/ui/views/tab_view.rs`
- Test: `src/ui/views/tab_view.rs`

- [ ] **Step 1: Write failing tests for optimistic prompt behavior**

Add tests for:
- prompt block is created before any client initialization or lock attempt
- an `ensure_agent_client` failure mutates the existing block instead of delaying or creating a second block
- placeholder states update in place
- feedback is scheduled before ACP initialization / heavy client work begins

- [ ] **Step 2: Run targeted tests and confirm failure if coverage is incomplete**

Run:

```bash
cargo test optimistic_prompt --lib -- --nocapture
```

Expected: FAIL or incomplete coverage before the final refactor lands.

- [ ] **Step 3: Finish isolating slow work from the visible UI path**

In `src/ui/views/tab_view.rs`:

- keep block creation, input clearing, and immediate UI feedback strictly synchronous and minimal
- move any remaining expensive preparation out of the visible input-submit path
- avoid extra full-list/block scans on every prompt submit
- ensure `cx.notify()` happens early enough that the prompt appears before ACP initialization work starts

Acceptance intent for this task:
- pressing `Enter` must create visible feedback before ACP init / heavy setup work
- the UI should not appear hung while the client is connecting or preparing the session

- [ ] **Step 4: Re-run targeted tests**

Run:

```bash
cargo test optimistic_prompt --lib -- --nocapture
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/ui/views/tab_view.rs
git commit -m "perf: make agent prompt submission optimistic"
```

### Task 6: Make ACP Streaming Parsing More Robust To Real Payload Shapes

**Files:**
- Modify: `src/acp/client.rs`
- Test: `src/acp/client.rs`

- [ ] **Step 1: Write failing tests for real-world ACP update shapes**

Cover:
- `content` arrays
- nested `message.content`
- nested `message.text`
- `delta`
- chunk vs non-chunk update kinds
- payloads with leading/trailing whitespace fragments

- [ ] **Step 2: Run targeted tests and confirm failure**

Run:

```bash
cargo test extract_update_text --lib -- --nocapture
```

Expected: FAIL for unsupported payloads before the parser is fully widened.

- [ ] **Step 3: Implement tolerant text extraction**

In `src/acp/client.rs`:

- centralize text extraction from strings, arrays, and nested content objects
- preserve chunk spacing correctly
- keep append-to-last behavior only for real incremental chunk events
- preserve compatibility with the current `stderr` side-channel

Out of scope for this plan unless needed to unblock approval UX:
- rendering rich `tool_call` / `tool_call_update` timeline events as first-class conversation items

If those events are encountered during debugging, log them clearly behind the ACP debug flag and document the gap rather than silently discarding them.

- [ ] **Step 4: Re-run targeted tests**

Run:

```bash
cargo test extract_update_text --lib -- --nocapture
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/acp/client.rs
git commit -m "fix: broaden acp streaming payload parsing"
```

## Chunk 4: Terminal Rendering And Scroll Performance

This chunk is specifically about render/reflow cost once the state already exists. It is separate from submit/update jank above, which is caused by synchronous work and event timing.

### Task 7: Reduce Reflow Cost For Large Blocks

**Files:**
- Modify: `src/ui/views/tab_view.rs`
- Test: `src/ui/views/tab_view.rs`

- [ ] **Step 1: Write focused tests for line-wrapping and block-size safeguards**

Add tests for pure helpers that decide:
- max lines kept per block
- max wrapped lines rendered per frame/block
- when to truncate or summarize excessive stderr/output sections

- [ ] **Step 2: Run targeted tests and confirm failure**

Run:

```bash
cargo test block_render_limits --lib -- --nocapture
```

Expected: FAIL because render limits are either absent or not centralized.

- [ ] **Step 3: Implement output-size safeguards**

In `src/ui/views/tab_view.rs`:

- stop re-wrapping every long line on every render if possible
- introduce cached/precomputed wrapped output for stable lines or a lightweight memo strategy local to the block
- cap pathological block growth with explicit limits and a user-visible truncation marker when needed
- ensure large stderr dumps do not create thousands of wrapped visual rows per frame

The goal is not to hide useful output, but to avoid GPUI doing excessive work each frame.

- [ ] **Step 4: Re-run targeted tests**

Run:

```bash
cargo test block_render_limits --lib -- --nocapture
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/ui/views/tab_view.rs
git commit -m "perf: limit expensive terminal block rendering"
```

### Task 8: Reduce Scroll-To-Bottom And Update Churn

**Files:**
- Modify: `src/ui/views/tab_view.rs`
- Test: `src/ui/views/tab_view.rs`

- [ ] **Step 1: Write failing tests for batched output updates**

Add tests for:
- multiple chunk lines can be appended before a single scroll-to-bottom decision
- placeholder/status updates do not trigger unnecessary total-line churn
- repeated stderr bursts do not cause one scroll action per raw line

- [ ] **Step 2: Run targeted tests and confirm failure**

Run:

```bash
cargo test output_batching --lib -- --nocapture
```

Expected: FAIL because output updates currently scroll and notify too often.

- [ ] **Step 3: Implement batching/reduced churn**

In `src/ui/views/tab_view.rs`:

- batch append operations where safe
- avoid calling `scroll_to_bottom()` for each individual fragment when a larger update is in progress
- avoid repeated `trim_output_lines()` churn when multiple lines from the same ACP event can be processed together
- keep follow-output behavior correct when the user has intentionally scrolled away

- [ ] **Step 4: Re-run targeted tests**

Run:

```bash
cargo test output_batching --lib -- --nocapture
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/ui/views/tab_view.rs
git commit -m "perf: batch terminal output updates"
```

## Chunk 5: ACP Markdown Rendering

The ACP protocol allows text content to be plain text or Markdown. OrbitShell currently treats all ACP output as plain strings. This chunk adds Markdown-aware rendering without regressing plain text, approval UI, or terminal responsiveness.

### Task 9: Add A Markdown-Aware Agent Response Model

**Files:**
- Modify: `src/acp/client.rs`
- Modify: `src/ui/views/tab_view.rs`
- Test: `src/acp/client.rs`
- Test: `src/ui/views/tab_view.rs`

- [ ] **Step 1: Write failing tests for Markdown-carrying ACP text**

Add tests covering:
- text chunks containing Markdown lists/headings/code fences remain intact through extraction
- plain text still renders as plain text content
- multiline code fences are not flattened into unusable output

- [ ] **Step 2: Run targeted tests and confirm failure**

Run:

```bash
cargo test markdown_agent_output --lib -- --nocapture
```

Expected: FAIL because ACP output is still modeled as flat plain-text terminal lines only.

- [ ] **Step 3: Introduce a Markdown-aware response representation**

In `src/acp/client.rs` and `src/ui/views/tab_view.rs`:

- preserve raw ACP text chunks/final text without stripping Markdown structure
- add the minimum response model needed so the UI can distinguish:
  - plain text terminal/status lines
  - ACP assistant content that should be rendered as Markdown
- keep stderr and internal status lines out of Markdown rendering

Do not over-generalize this into a full rich-document model. Keep it narrowly scoped to assistant response rendering.

- [ ] **Step 4: Re-run targeted tests**

Run:

```bash
cargo test markdown_agent_output --lib -- --nocapture
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/acp/client.rs src/ui/views/tab_view.rs
git commit -m "feat: preserve markdown agent output"
```

### Task 10: Render Markdown Safely In The Agent Block UI

**Files:**
- Modify: `src/ui/views/tab_view.rs`
- Test: `src/ui/views/tab_view.rs`

- [ ] **Step 1: Write failing UI tests for Markdown rendering helpers**

Add tests for helper behavior covering:
- headings/lists render through the Markdown path
- fenced code blocks stay visually distinct
- plain terminal stderr/status lines stay on the plain-text path
- links/emphasis do not break layout when shown in block output

- [ ] **Step 2: Run targeted tests and confirm failure**

Run:

```bash
cargo test markdown_render_helpers --lib -- --nocapture
```

Expected: FAIL because the block renderer only knows how to draw wrapped plain text lines.

- [ ] **Step 3: Implement a constrained Markdown renderer for ACP responses**

In `src/ui/views/tab_view.rs`:

- add a Markdown render path for ACP assistant output only
- support the core formatting that materially improves readability in this terminal surface:
  - paragraphs
  - emphasis/strong
  - bullet/numbered lists
  - inline code
  - fenced code blocks
- keep links visually readable even if they are not yet fully interactive
- fall back to plain text safely when parsing fails or content is not marked/rendered as Markdown

Avoid a full browser-like renderer. This should stay compact, fast, and visually aligned with the existing terminal UI.

- [ ] **Step 4: Re-run targeted tests**

Run:

```bash
cargo test markdown_render_helpers --lib -- --nocapture
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/ui/views/tab_view.rs
git commit -m "feat: render markdown in agent responses"
```

## Chunk 6: Diagnostics And Final Validation

### Task 11: Add Debuggable ACP Event Instrumentation Behind A Guard

**Files:**
- Modify: `src/acp/client.rs`
- Modify: `src/acp/transport.rs`

- [ ] **Step 1: Add opt-in ACP debug logging**

Implement a small guarded debug path controlled by an env var such as:

```bash
ORBITSHELL_ACP_DEBUG=1
```

When enabled, log:
- inbound ACP request methods
- `session/update` method names / shapes
- permission request summaries
- whether text extraction fell back to a generic path

Do not spam logs by default.

- [ ] **Step 2: Verify the guarded logging compiles cleanly**

Run:

```bash
cargo check
```

Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add src/acp/client.rs src/acp/transport.rs
git commit -m "chore: add guarded acp debug instrumentation"
```

### Task 12: Full Validation Pass

**Files:**
- Validate the modified ACP/runtime/UI files from this plan

- [ ] **Step 1: Run formatting check**

Run:

```bash
cargo fmt --check
```

Expected: PASS. If it fails, run `cargo fmt`, then re-run `cargo fmt --check`.

- [ ] **Step 2: Run full library tests**

Run:

```bash
cargo test --lib
```

Expected: PASS.

- [ ] **Step 3: Run compile validation**

Run:

```bash
cargo check
```

Expected: PASS.

- [ ] **Step 4: Manual runtime validation**

Run:

```bash
cargo run
```

Then validate manually:

- send a simple prompt in agent mode and confirm the prompt block appears immediately
- confirm visible staged status transitions are at least perceptible on slower prompts
- trigger an ACP permission request and verify:
  - inline approval card appears
  - `Permitir uma vez` resumes execution
  - `Sempre permitir` resumes execution
  - `Negar` fails gracefully in the same block
- send or reproduce a large output/error block and confirm scrolling stays responsive
- send or reproduce a Markdown-rich ACP response and confirm lists/code fences remain readable
- confirm no obvious freeze or Windows “app not responding” dialog on prompt submit

- [ ] **Step 5: Report anything still not validated**

Document exact gaps if:
- permission request could not be reproduced locally
- the Codex ACP emits a different request method than expected
- a remaining freeze still happens but lacks instrumentation to isolate

---

Plan complete and saved to `docs/superpowers/plans/2026-03-19-acp-approval-and-terminal-performance-implementation.md`. Ready to execute?
