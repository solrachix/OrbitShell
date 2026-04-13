# Base Terminal Launch Design

## Summary

OrbitShell should support a base terminal flow that does not require opening a project first. The welcome screen should offer a command input; submitting a command starts a full terminal session immediately. Folder context menu integrations on Linux and Windows should open the selected folder in the same base terminal mode, not as a project/workspace.

The base terminal reuses the existing terminal surface: command input, block output, history, path picker, Terminal/Agent mode switching, agent/model controls, git status, and file preview behavior. The difference is workspace context: base terminal tabs start with the sidebar hidden, do not add the folder to recent projects, and do not imply that a project was opened.

## Goals

- Add a command input to the welcome screen that can start OrbitShell as a terminal without opening a project.
- Allow the command submitted from welcome to run as the initial command in the newly started terminal session.
- Add a base terminal tab state that is distinct from both welcome tabs and project/workspace tabs.
- Keep the existing terminal and Agent UX intact inside base terminal mode.
- Hide the sidebar by default in base terminal mode, while keeping the sidebar toggle available.
- When the sidebar is expanded in base terminal mode, point it at the terminal's current directory and keep it synced as the terminal detects cwd changes.
- Add Linux and Windows folder context menu support that launches OrbitShell in base terminal mode at the selected folder.
- Do not add base terminal folders to recent projects unless the user explicitly opens them as a repository/project.

## Non-Goals

- Redesigning the terminal input bar, Agent mode, or block output UI.
- Replacing the existing project/open repository flow.
- Implementing clone/create project behavior beyond the existing placeholder flow.
- Adding custom scroll behavior or changing list virtualization strategy.
- Building a separate reduced terminal UI for base mode.

## Product Decisions

- Opening a folder from the OS context menu uses base terminal mode.
- Base terminal mode starts with the sidebar hidden.
- Base terminal mode starts in the explicit folder when launched from a folder.
- Base terminal mode starts in the user's home directory when there is no explicit folder; on Windows this means `USERPROFILE` such as `C:\Users\<user>`, with `C:\` only as a last-resort fallback.
- The sidebar can still be expanded manually from base terminal mode.
- The sidebar root follows the terminal cwd in base terminal mode.
- The welcome command input should start a full terminal, not execute in a special welcome-only shell.
- Base terminal mode keeps Agent features available.
- Base terminal tabs should not be saved to recent projects.
- Welcome `Open repository` keeps opening a selected folder as a project.
- Welcome `Create new project` asks for a project prompt, asks for a destination folder, creates a safe project folder name from the prompt, opens it as a project, and starts the Agent prompt there.
- Welcome `Clone repository` asks for the repository URL, asks for a destination folder, creates a safe destination folder name from the repository URL, opens it as a project, and runs `git clone <url> .` there.

## User Flows

### Start From Welcome

1. User launches OrbitShell normally.
2. Welcome screen shows the existing action buttons and recent projects.
3. Welcome screen also shows a command input.
4. User enters a command and presses Enter.
5. The active tab becomes a base terminal tab.
6. OrbitShell starts a PTY in the process working directory, or another configured default if the app already has one.
7. OrbitShell runs the submitted command as the first terminal block.
8. Sidebar remains hidden until the user toggles it.

### Open Project From Welcome

1. User clicks Open repository or a recent item.
2. OrbitShell keeps the existing project/workspace behavior.
3. The tab becomes a project tab.
4. The folder is added to recent projects.
5. Sidebar behavior remains the current project behavior.

### Open Folder From OS Context Menu

1. User right-clicks a folder in the OS file manager.
2. User selects Open in OrbitShell.
3. OrbitShell launches with that folder path as an argument.
4. The app opens a base terminal tab rooted at the selected folder.
5. Sidebar starts hidden, but can be expanded and will show that folder.
6. The folder is not added to recent projects.

## Architecture

### Launch Options

Introduce a small launch options model near app startup:

- no path: open normal welcome tab
- path argument: open base terminal tab with that path as cwd

The CLI/parser should be conservative:

- treat the first existing directory argument as the base terminal cwd
- ignore unsupported arguments for now instead of failing startup
- keep default app launch behavior unchanged when no path is provided

### Workspace Tab Kind

Replace the current boolean `tab_is_welcome` tracking with a tab kind enum:

- `Welcome`
- `BaseTerminal`
- `Project`
- `Utility`

Settings and Agent tabs can use `Utility`, preserving their existing no-sidebar behavior. Project tabs show the sidebar when `sidebar_visible` is true. Welcome and Utility tabs hide it. BaseTerminal tabs hide it initially by setting the global/sidebar visible state to false when created, but if the user toggles the sidebar back on, the sidebar should be allowed to render for BaseTerminal tabs.

This avoids overloading "welcome" to mean "not a project" and gives the sidebar rules a clear place to live.

### Welcome Command Event

Add a new welcome event that carries the command text:

- `StartBaseTerminal { command: String }`

`TabView::new_welcome` should subscribe to the event and emit a corresponding `TabViewEvent` to `Workspace`. `Workspace` should then convert the current welcome tab to base terminal mode and invoke a `TabView` method that starts the terminal and optionally runs the command.

### Terminal Start API

Extend the existing terminal start path with an initial command parameter rather than duplicating terminal startup:

- `start_terminal_with_path(cx, cwd)`
- new helper or overload-like method: `start_terminal(cx, cwd, initial_command, terminal_kind)`

The implementation should:

- initialize the PTY exactly as the current terminal flow does
- clear input/suggestions/history menu as the current flow does
- set `current_path` and git status from the cwd
- switch mode to `Terminal`
- after PTY initialization, run the initial command through the same `run_command` path used by normal input

Using the existing `run_command` path preserves block output, history, pending echo handling, and input visibility behavior.

### Sidebar Sync

For base terminal tabs:

- when the terminal detects a cwd change and emits `CwdChanged`, update `tab_paths`
- if the base terminal tab is active and the sidebar is visible, sync the sidebar root to the updated path
- do not add the path to recent projects

For project tabs:

- keep existing recent project and sidebar behavior

### Packaging Integration

Linux:

- update the desktop entry `Exec` to support a path argument, using desktop-file field codes where appropriate
- add a folder context action such as `Open in OrbitShell`
- update the local install script so the generated desktop file includes the same action

Windows:

- update the NSIS installer to register a per-user Directory shell action that launches `orbitshell.exe "%1"`
- remove the shell action during uninstall
- keep the existing shortcuts unchanged

## Error Handling

- If the provided path argument is missing or is not a directory, fall back to normal welcome launch.
- If PTY startup fails, keep behavior consistent with the current terminal path for now; if a graceful error surface already exists by implementation time, use it.
- If an initial command is empty after trimming, start the base terminal without running a block.
- Folder paths with spaces must be quoted correctly in Windows registry commands and desktop entry handling.

## Testing And Validation

Automated tests should cover the pure state decisions where possible:

- base terminal tabs are not considered project tabs
- base terminal tabs allow sidebar rendering only after sidebar visibility is enabled
- welcome command events map to base terminal startup
- path launch options parse an existing directory and ignore invalid paths
- base terminal cwd changes update sidebar root state without adding recent projects

Manual validation should cover:

- launching `cargo run` still opens welcome normally
- entering a command on welcome starts a full terminal and runs it
- Terminal/Agent mode switching still works inside base terminal mode
- opening a repository from welcome still adds recent project and shows project sidebar behavior
- launching with a folder argument starts base terminal mode in that folder
- generated Linux desktop action and Windows registry command include the selected folder path

## Files In Scope

- `src/main.rs`
- `src/ui/mod.rs`
- `src/ui/views/tab_view.rs`
- `src/ui/views/welcome_view.rs`
- `packaging/linux/dev.carlosmiguel.orbitshell.desktop`
- `installer/linux/install.sh`
- `installer/windows/orbitshell.nsi`
- focused tests under existing unit test modules or `tests/`
