# Base Terminal Launch Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a full-featured base terminal flow from the welcome screen and OS folder context menus without treating those folders as projects.

**Architecture:** Reuse the existing `TabView` terminal implementation and add a workspace-level tab kind to distinguish welcome, base terminal, project, and utility tabs. Startup argument parsing selects either normal welcome launch or a base terminal rooted at a directory. Packaging changes pass the selected folder path into the app.

**Tech Stack:** Rust 2024, GPUI, portable-pty, NSIS, Linux desktop entries.

---

## Chunk 1: Workspace State And Launch Options

### Task 1: Launch Options

**Files:**
- Create: `src/ui/launch.rs`
- Modify: `src/ui/mod.rs`
- Modify: `src/main.rs`
- Test: `src/ui/launch.rs`

- [ ] **Step 1: Write failing tests**

Add unit tests for parsing launch arguments:

```rust
#[test]
fn launch_options_select_existing_directory_argument() {
    let temp = tempfile::tempdir().expect("temp dir");
    let path = temp.path().to_path_buf();

    let options = LaunchOptions::from_args([OsString::from("orbitshell"), path.clone().into()]);

    assert_eq!(options.base_terminal_cwd, Some(path));
}

#[test]
fn launch_options_ignore_invalid_directory_argument() {
    let options = LaunchOptions::from_args([
        OsString::from("orbitshell"),
        OsString::from("/definitely/not/a/real/orbitshell/path"),
    ]);

    assert_eq!(options.base_terminal_cwd, None);
}
```

- [ ] **Step 2: Run tests to verify failure**

Run: `cargo test ui::launch --lib`

Expected: compile/test failure because `ui::launch` and `LaunchOptions` do not exist.

- [ ] **Step 3: Implement launch options**

Create `src/ui/launch.rs` with a small public `LaunchOptions` struct and `from_args` parser that selects the first existing directory after argv[0]. Export it from `src/ui/mod.rs`. Update `main.rs` to compute `LaunchOptions` and call `Workspace::new_with_options`.

- [ ] **Step 4: Run tests**

Run: `cargo test ui::launch --lib`

Expected: pass.

### Task 2: Tab Kind State

**Files:**
- Modify: `src/ui/mod.rs`
- Test: `src/ui/mod.rs`

- [ ] **Step 1: Write failing tests**

Add unit tests for tab kind sidebar visibility logic:

```rust
#[test]
fn base_terminal_sidebar_renders_only_when_sidebar_is_visible() {
    assert!(!Workspace::should_show_sidebar_for_tab(TabKind::BaseTerminal, false));
    assert!(Workspace::should_show_sidebar_for_tab(TabKind::BaseTerminal, true));
}

#[test]
fn welcome_and_utility_tabs_never_show_sidebar() {
    assert!(!Workspace::should_show_sidebar_for_tab(TabKind::Welcome, true));
    assert!(!Workspace::should_show_sidebar_for_tab(TabKind::Utility, true));
}

#[test]
fn project_tabs_follow_sidebar_visibility() {
    assert!(!Workspace::should_show_sidebar_for_tab(TabKind::Project, false));
    assert!(Workspace::should_show_sidebar_for_tab(TabKind::Project, true));
}
```

- [ ] **Step 2: Run tests to verify failure**

Run: `cargo test ui::tests::base_terminal_sidebar_renders_only_when_sidebar_is_visible ui::tests::welcome_and_utility_tabs_never_show_sidebar ui::tests::project_tabs_follow_sidebar_visibility --lib`

Expected: compile failure because `TabKind` and helper do not exist.

- [ ] **Step 3: Implement tab kind state**

Replace `tab_is_welcome: Vec<bool>` with `tab_kinds: Vec<TabKind>`. Mark welcome as `Welcome`, settings/agent as `Utility`, opened repositories as `Project`, and future base terminals as `BaseTerminal`. Add `Workspace::should_show_sidebar_for_tab`.

- [ ] **Step 4: Run tests**

Run: `cargo test ui::tests::base_terminal_sidebar_renders_only_when_sidebar_is_visible ui::tests::welcome_and_utility_tabs_never_show_sidebar ui::tests::project_tabs_follow_sidebar_visibility --lib`

Expected: pass.

## Chunk 2: Welcome Command Starts Base Terminal

### Task 3: Welcome Command Event

**Files:**
- Modify: `src/ui/views/welcome_view.rs`
- Modify: `src/ui/views/tab_view.rs`
- Modify: `src/ui/mod.rs`
- Test: `src/ui/views/welcome_view.rs` or `src/ui/views/tab_view.rs`

- [ ] **Step 1: Write failing tests**

Add focused pure tests for welcome command normalization:

```rust
#[test]
fn normalize_welcome_terminal_command_trims_and_ignores_empty_input() {
    assert_eq!(WelcomeView::normalize_terminal_command("  pwd  "), Some("pwd".to_string()));
    assert_eq!(WelcomeView::normalize_terminal_command("   "), None);
}
```

- [ ] **Step 2: Run test to verify failure**

Run: `cargo test welcome_view::tests::normalize_welcome_terminal_command_trims_and_ignores_empty_input --lib`

Expected: compile failure because the helper does not exist.

- [ ] **Step 3: Implement welcome input and event**

Add `StartBaseTerminalEvent { command: String }`, a command input field on the welcome screen, key handling when no overlay is open, and subscription plumbing through `TabViewEvent::StartBaseTerminal`.

- [ ] **Step 4: Run test**

Run: `cargo test welcome_view::tests::normalize_welcome_terminal_command_trims_and_ignores_empty_input --lib`

Expected: pass.

### Task 4: Terminal Start With Initial Command

**Files:**
- Modify: `src/ui/views/tab_view.rs`
- Modify: `src/ui/mod.rs`
- Test: `src/ui/views/tab_view.rs`

- [ ] **Step 1: Write failing tests**

Add a pure test for initial command normalization:

```rust
#[test]
fn normalize_initial_terminal_command_ignores_empty_values() {
    assert_eq!(TabView::normalize_initial_terminal_command(Some("  ls  ")), Some("ls".to_string()));
    assert_eq!(TabView::normalize_initial_terminal_command(Some("   ")), None);
    assert_eq!(TabView::normalize_initial_terminal_command(None), None);
}
```

- [ ] **Step 2: Run test to verify failure**

Run: `cargo test tab_view::tests::normalize_initial_terminal_command_ignores_empty_values --lib`

Expected: compile failure because the helper does not exist.

- [ ] **Step 3: Implement terminal startup API**

Add a method that starts the PTY with a path and optional initial command, reusing existing terminal initialization and `run_command`. Workspace should convert the active welcome tab to `BaseTerminal`, hide the sidebar by setting `sidebar_visible = false`, and avoid recent project updates.

- [ ] **Step 4: Run test**

Run: `cargo test tab_view::tests::normalize_initial_terminal_command_ignores_empty_values --lib`

Expected: pass.

## Chunk 3: OS Folder Context Menu Integration

### Task 5: Linux Desktop Action

**Files:**
- Modify: `packaging/linux/dev.carlosmiguel.orbitshell.desktop`
- Modify: `installer/linux/install.sh`

- [ ] **Step 1: Update static desktop entry**

Set `Exec=orbitshell %F`, add `MimeType=inode/directory;`, and add an `OpenDirectory` desktop action with `Exec=orbitshell %F`.

- [ ] **Step 2: Update local install script**

Generate the same desktop metadata, using the installed binary path and `%F`.

- [ ] **Step 3: Validate shell script syntax**

Run: `bash -n installer/linux/install.sh`

Expected: pass.

### Task 6: Windows NSIS Shell Action

**Files:**
- Modify: `installer/windows/orbitshell.nsi`

- [ ] **Step 1: Add registry entries**

Write per-user registry keys under `Software\Classes\Directory\shell\OrbitShell` and `Software\Classes\Directory\Background\shell\OrbitShell`, with commands that launch `"$INSTDIR\orbitshell.exe" "%1"` and `"$INSTDIR\orbitshell.exe" "%V"` respectively.

- [ ] **Step 2: Remove registry entries during uninstall**

Delete both shell action keys in the uninstall section.

- [ ] **Step 3: Inspect quoting**

Run: `grep -n "Directory.*OrbitShell\\|%1\\|%V" installer/windows/orbitshell.nsi`

Expected: both context menu commands are present and quoted.

## Chunk 4: Integration Verification

### Task 7: Full Test And Build Check

**Files:**
- All modified files

- [ ] **Step 1: Run targeted Rust tests**

Run all new targeted tests from chunks 1 and 2.

Expected: pass.

- [ ] **Step 2: Run full test suite**

Run: `cargo test`

Expected: pass.

- [ ] **Step 3: Run packaging syntax checks**

Run: `bash -n installer/linux/install.sh`

Expected: pass.

- [ ] **Step 4: Review diff**

Run: `git diff --check && git diff --stat && git status --short`

Expected: no whitespace errors; diff contains only spec, plan, launch/base terminal code, and packaging changes.
