use crate::git::get_git_branches;
use crate::git::get_git_status;
use crate::terminal::TerminalPty;
use futures::StreamExt;
use futures::channel::mpsc;
use gpui::StatefulInteractiveElement;
use gpui::*;
use lucide_icons::Icon;
use std::collections::{HashSet, VecDeque};
use std::io::Read;
use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant};

use crate::ui::icons::lucide_icon;
use crate::ui::recent::RecentEntry;
use crate::ui::views::settings_view::SettingsView;
use crate::ui::views::welcome_view::{OpenRepositoryEvent, WelcomeView};

pub struct TabView {
    blocks: Vec<Block>,
    pty: Option<TerminalPty>,
    focus_handle: FocusHandle,
    input: String,
    cursor: usize,
    history: VecDeque<String>,
    history_file: Option<PathBuf>,
    history_open: bool,
    history_index: usize,
    history_items: Vec<SuggestionItem>,
    suggestions: Vec<SuggestionItem>,
    suggest_index: usize,
    selection: Option<(usize, usize)>,
    selection_anchor: Option<usize>,
    path_commands: Vec<String>,
    last_path_scan: Instant,
    last_path_var: String,
    current_path: String,
    git_status: Option<crate::git::GitStatus>,
    auto_focus: bool,
    pending_echo: Option<String>,
    scroll_handle: ScrollHandle,
    input_visible: bool,
    overlay: Option<Overlay>,
    needs_git_refresh: bool,
    mode: TabViewMode,
}

#[derive(Clone)]
struct Block {
    command: String,
    output_lines: Vec<String>,
    has_error: bool,
    context: Option<BlockContext>,
}

#[derive(Clone)]
struct BlockContext {
    cwd: String,
    git_branch: Option<String>,
    git_files: Option<usize>,
    git_added: Option<usize>,
    git_deleted: Option<usize>,
    git_modified: Option<usize>,
}

struct PathPickerState {
    cwd: PathBuf,
    query: String,
    entries: Vec<PathEntry>,
    selected: usize,
}

struct PathEntry {
    name: String,
    path: PathBuf,
    is_dir: bool,
}

struct BranchPickerState {
    query: String,
    all_branches: Vec<String>,
    branches: Vec<String>,
    selected: usize,
}

enum Overlay {
    Path(PathPickerState),
    Branch(BranchPickerState),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SuggestSource {
    History,
    Command,
    Path,
}

#[derive(Clone, Debug)]
struct SuggestionItem {
    display: String,
    insert: String,
    source: SuggestSource,
}

pub enum TabViewEvent {
    CwdChanged(PathBuf),
    OpenRepository(PathBuf),
}

enum TabViewMode {
    Terminal,
    Welcome(Entity<WelcomeView>),
    Settings(Entity<SettingsView>),
}

impl TabView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self::new_with_path(cx, None)
    }

    fn format_path(path: &PathBuf) -> String {
        let home = std::env::var("USERPROFILE")
            .or_else(|_| std::env::var("HOME"))
            .ok()
            .map(PathBuf::from);

        if let Some(home_path) = home {
            if let Ok(stripped) = path.strip_prefix(&home_path) {
                let stripped = stripped.to_string_lossy();
                if stripped.is_empty() {
                    return "~".to_string();
                }
                let sep = if cfg!(windows) { "\\" } else { "/" };
                return format!("~{}{}", sep, stripped);
            }
        }

        let mut out = path.to_string_lossy().to_string();
        if !cfg!(windows) {
            out = out.replace('\\', "/");
        }
        out
    }

    pub fn new_with_path(cx: &mut Context<Self>, path: Option<PathBuf>) -> Self {
        let mut view = Self::new_base(cx);
        view.start_terminal_with_path(cx, path);
        view
    }

    pub fn new_welcome(cx: &mut Context<Self>, recent: Vec<RecentEntry>) -> Self {
        let mut view = Self::new_base(cx);
        let welcome = cx.new(|cx| WelcomeView::with_recent(cx, recent));
        cx.subscribe(&welcome, |_, _welcome, event: &OpenRepositoryEvent, cx| {
            cx.emit(TabViewEvent::OpenRepository(event.path.clone()));
        })
        .detach();
        view.mode = TabViewMode::Welcome(welcome);
        view
    }

    pub fn new_settings(cx: &mut Context<Self>) -> Self {
        let mut view = Self::new_base(cx);
        let settings = cx.new(|cx| SettingsView::new(cx));
        view.mode = TabViewMode::Settings(settings);
        view
    }

    fn new_base(cx: &mut Context<Self>) -> Self {
        let (history, history_file) = Self::load_initial_history();
        let last_path_var = std::env::var("PATH").unwrap_or_default();
        Self {
            blocks: Vec::new(),
            pty: None,
            focus_handle: cx.focus_handle(),
            input: String::new(),
            cursor: 0,
            history,
            history_file,
            history_open: false,
            history_index: 0,
            history_items: Vec::new(),
            suggestions: Vec::new(),
            suggest_index: 0,
            selection: None,
            selection_anchor: None,
            path_commands: Self::load_path_commands(),
            last_path_scan: Instant::now(),
            last_path_var,
            current_path: "~".to_string(),
            git_status: None,
            auto_focus: true,
            pending_echo: None,
            scroll_handle: ScrollHandle::new(),
            input_visible: true,
            overlay: None,
            needs_git_refresh: false,
            mode: TabViewMode::Terminal,
        }
    }

    pub fn set_recent(&mut self, recent: Vec<RecentEntry>, cx: &mut Context<Self>) {
        if let TabViewMode::Welcome(ref welcome) = self.mode {
            let _ = welcome.update(cx, |view, cx| {
                view.set_recent(recent, cx);
            });
        }
    }

    pub fn is_welcome(&self) -> bool {
        matches!(self.mode, TabViewMode::Welcome(_))
    }

    pub fn set_settings_section(&mut self, section: &str, cx: &mut Context<Self>) {
        if let TabViewMode::Settings(ref settings) = self.mode {
            let _ = settings.update(cx, |view, cx| {
                view.set_active_section(section, cx);
            });
        }
    }

    pub fn start_terminal_with_path(&mut self, cx: &mut Context<Self>, path: Option<PathBuf>) {
        if self.pty.is_some() {
            return;
        }

        let cwd = path.or_else(|| std::env::current_dir().ok());
        let (pty, reader) =
            TerminalPty::new_in_path(80, 24, cwd.as_deref()).expect("failed to create PTY");
        self.pty = Some(pty);
        self.current_path = cwd
            .as_ref()
            .map(Self::format_path)
            .unwrap_or_else(|| "~".to_string());
        self.git_status = cwd.as_ref().and_then(|path| get_git_status(path));
        self.blocks.clear();
        self.input.clear();
        self.cursor = 0;
        self.history_open = false;
        self.history_items.clear();
        self.suggestions.clear();
        self.suggest_index = 0;
        self.selection = None;
        self.selection_anchor = None;
        self.pending_echo = None;
        self.auto_focus = true;
        self.input_visible = true;
        self.overlay = None;
        self.needs_git_refresh = false;
        self.mode = TabViewMode::Terminal;

        let (tx, mut rx) = mpsc::unbounded::<String>();
        thread::spawn(move || {
            let mut reader = reader;
            let mut buf = [0u8; 2048];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        let data = String::from_utf8_lossy(&buf[..n]).to_string();
                        if tx.unbounded_send(data).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        });

        cx.spawn(|view: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                while let Some(data) = rx.next().await {
                    if view
                        .update(&mut cx, |view, cx| {
                            view.append_output(&data, cx);
                            cx.notify();
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            }
        })
        .detach();
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        if self.handle_overlay_key(event, cx) {
            cx.stop_propagation();
            return;
        }
        let ctrl = event.keystroke.modifiers.control;
        let shift = event.keystroke.modifiers.shift;
        if ctrl && event.keystroke.key.eq_ignore_ascii_case("a") {
            self.select_all_input();
            cx.notify();
            cx.stop_propagation();
            return;
        }

        if ctrl && event.keystroke.key.len() == 1 {
            if let Some(ref mut pty) = self.pty {
                let key = event.keystroke.key.as_bytes()[0];
                if key.is_ascii_alphabetic() {
                    let code = key.to_ascii_lowercase() - b'a' + 1;
                    let _ = pty.write(&[code]);
                    cx.stop_propagation();
                    return;
                }
            }
        }

        if !self.input_visible && event.keystroke.key.as_str() != "escape" {
            return;
        }

        match event.keystroke.key.as_str() {
            "enter" | "return" | "numpadenter" => {
                if self.history_open {
                    self.accept_history_item(cx);
                    cx.stop_propagation();
                    return;
                }
                self.commit_input(cx);
                cx.stop_propagation();
            }
            "backspace" => {
                if self.delete_selection_if_any() {
                    self.refresh_suggestions();
                    self.refresh_history_menu();
                    cx.notify();
                    cx.stop_propagation();
                    return;
                }
                if !self.input.is_empty() {
                    self.pop_char_before_cursor();
                    self.refresh_suggestions();
                    self.refresh_history_menu();
                    cx.notify();
                }
                cx.stop_propagation();
            }
            "space" => {
                self.insert_text(" ");
                self.refresh_suggestions();
                self.refresh_history_menu();
                cx.notify();
                cx.stop_propagation();
            }
            "tab" => {
                if self.has_suggestion() {
                    self.accept_suggestion();
                    cx.notify();
                }
                cx.stop_propagation();
            }
            "left" | "arrowleft" => {
                if shift {
                    let new_cursor = if ctrl {
                        self.move_word_left(self.cursor)
                    } else {
                        self.cursor.saturating_sub(1)
                    };
                    let anchor = self.selection_anchor.unwrap_or(self.cursor);
                    self.cursor = new_cursor;
                    self.set_selection_from_anchor(anchor, self.cursor);
                } else {
                    if self.has_selection() {
                        if let Some((a, b)) = self.normalized_selection() {
                            self.cursor = a.min(b);
                        }
                    } else if ctrl {
                        self.cursor = self.move_word_left(self.cursor);
                    } else {
                        self.move_cursor_left();
                    }
                    self.clear_selection();
                }
                self.history_open = false;
                cx.notify();
                cx.stop_propagation();
            }
            "right" | "arrowright" => {
                if shift {
                    let new_cursor = if ctrl {
                        self.move_word_right(self.cursor)
                    } else {
                        let max = self.input.chars().count();
                        (self.cursor + 1).min(max)
                    };
                    let anchor = self.selection_anchor.unwrap_or(self.cursor);
                    self.cursor = new_cursor;
                    self.set_selection_from_anchor(anchor, self.cursor);
                } else if ctrl {
                    self.cursor = self.move_word_right(self.cursor);
                    self.clear_selection();
                } else if self.has_selection() {
                    if let Some((a, b)) = self.normalized_selection() {
                        self.cursor = a.max(b);
                    }
                    self.clear_selection();
                } else if self.has_suggestion() {
                    self.accept_suggestion();
                } else {
                    self.move_cursor_right();
                }
                self.history_open = false;
                cx.notify();
                cx.stop_propagation();
            }
            "home" => {
                self.clear_selection();
                self.cursor = 0;
                self.history_open = false;
                cx.notify();
                cx.stop_propagation();
            }
            "end" => {
                self.clear_selection();
                self.cursor = self.input.chars().count();
                self.history_open = false;
                cx.notify();
                cx.stop_propagation();
            }
            "up" | "arrowup" => {
                self.open_or_step_history(true);
                cx.notify();
            }
            "down" | "arrowdown" => {
                self.open_or_step_history(false);
                cx.notify();
            }
            "escape" => {
                if !self.input_visible {
                    if let Some(ref mut pty) = self.pty {
                        let _ = pty.write(&[3]);
                    }
                    cx.stop_propagation();
                }
            }
            _ => {
                if let Some(text) = event.keystroke.key_char.as_deref() {
                    if !text.is_empty() {
                        self.insert_text(text);
                        self.refresh_suggestions();
                        self.refresh_history_menu();
                        cx.notify();
                        cx.stop_propagation();
                    }
                } else if event.keystroke.key.len() == 1 {
                    let key = event.keystroke.key.clone();
                    self.insert_text(&key);
                    self.refresh_suggestions();
                    self.refresh_history_menu();
                    cx.notify();
                    cx.stop_propagation();
                }
            }
        }
    }

    fn on_focus_input(
        &mut self,
        _event: &MouseDownEvent,
        window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
        window.focus(&self.focus_handle);
    }

    fn on_open_path_picker(
        &mut self,
        _event: &MouseDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.input_visible {
            return;
        }
        let cwd = Self::expand_tilde(&self.current_path);
        let mut picker = PathPickerState {
            cwd,
            query: String::new(),
            entries: Vec::new(),
            selected: 0,
        };
        Self::populate_path_picker(&mut picker);
        self.overlay = Some(Overlay::Path(picker));
        cx.notify();
    }

    fn on_open_branch_picker(
        &mut self,
        _event: &MouseDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.input_visible {
            return;
        }
        let cwd = Self::expand_tilde(&self.current_path);
        let all = get_git_branches(&cwd);
        if all.is_empty() {
            return;
        }
        let mut picker = BranchPickerState {
            query: String::new(),
            all_branches: all.clone(),
            branches: all,
            selected: 0,
        };
        Self::filter_branch_picker(&mut picker);
        self.overlay = Some(Overlay::Branch(picker));
        cx.notify();
    }

    fn handle_overlay_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) -> bool {
        let Some(ref mut overlay) = self.overlay else {
            return false;
        };

        match event.keystroke.key.as_str() {
            "escape" => {
                self.overlay = None;
                cx.notify();
                return true;
            }
            "backspace" => {
                match overlay {
                    Overlay::Path(picker) => {
                        picker.query.pop();
                        picker.selected = 0;
                        Self::populate_path_picker(picker);
                    }
                    Overlay::Branch(picker) => {
                        picker.query.pop();
                        picker.selected = 0;
                        Self::filter_branch_picker(picker);
                    }
                }
                cx.notify();
                return true;
            }
            "enter" | "return" | "numpadenter" => {
                self.accept_overlay_selection(cx);
                return true;
            }
            "up" | "arrowup" => {
                match overlay {
                    Overlay::Path(picker) => {
                        if picker.selected > 0 {
                            picker.selected -= 1;
                        }
                    }
                    Overlay::Branch(picker) => {
                        if picker.selected > 0 {
                            picker.selected -= 1;
                        }
                    }
                }
                cx.notify();
                return true;
            }
            "down" | "arrowdown" => {
                match overlay {
                    Overlay::Path(picker) => {
                        if picker.selected + 1 < picker.entries.len() {
                            picker.selected += 1;
                        }
                    }
                    Overlay::Branch(picker) => {
                        if picker.selected + 1 < picker.branches.len() {
                            picker.selected += 1;
                        }
                    }
                }
                cx.notify();
                return true;
            }
            _ => {}
        }

        if let Some(text) = event.keystroke.key_char.as_deref() {
            if !text.is_empty() && !event.keystroke.modifiers.control {
                match overlay {
                    Overlay::Path(picker) => {
                        picker.query.push_str(text);
                        picker.selected = 0;
                        Self::populate_path_picker(picker);
                    }
                    Overlay::Branch(picker) => {
                        picker.query.push_str(text);
                        picker.selected = 0;
                        Self::filter_branch_picker(picker);
                    }
                }
                cx.notify();
                return true;
            }
        }

        true
    }

    fn populate_path_picker(picker: &mut PathPickerState) {
        let query = picker.query.to_lowercase();
        let mut entries = Vec::new();
        if let Some(parent) = picker.cwd.parent() {
            entries.push(PathEntry {
                name: ".. (Parent Directory)".to_string(),
                path: parent.to_path_buf(),
                is_dir: true,
            });
        }
        let mut list: Vec<PathEntry> = std::fs::read_dir(&picker.cwd)
            .map(|read_dir| {
                read_dir
                    .filter_map(|entry| entry.ok())
                    .filter_map(|entry| {
                        let name = entry.file_name().to_string_lossy().to_string();
                        if !query.is_empty() && !name.to_lowercase().contains(&query) {
                            return None;
                        }
                        let path = entry.path();
                        let is_dir = path.is_dir();
                        if !is_dir {
                            return None;
                        }
                        Some(PathEntry { name, path, is_dir })
                    })
                    .collect()
            })
            .unwrap_or_default();

        list.sort_by(|a, b| match (a.is_dir, b.is_dir) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
        });

        entries.extend(list);
        picker.entries = entries;
        if picker.selected >= picker.entries.len() {
            picker.selected = picker.entries.len().saturating_sub(1);
        }
    }

    fn filter_branch_picker(picker: &mut BranchPickerState) {
        let query = picker.query.to_lowercase();
        if query.is_empty() {
            picker.branches = picker.all_branches.clone();
        } else {
            picker.branches = picker
                .all_branches
                .iter()
                .filter(|b| b.to_lowercase().contains(&query))
                .cloned()
                .collect();
        }
        if picker.selected >= picker.branches.len() {
            picker.selected = picker.branches.len().saturating_sub(1);
        }
    }

    fn accept_overlay_selection(&mut self, cx: &mut Context<Self>) {
        let Some(overlay) = self.overlay.take() else {
            return;
        };
        match overlay {
            Overlay::Path(picker) => {
                if let Some(entry) = picker.entries.get(picker.selected) {
                    if entry.is_dir {
                        let cmd = format!("cd \"{}\"", entry.path.to_string_lossy());
                        self.run_command(cmd, cx);
                    } else {
                        self.overlay = None;
                        cx.notify();
                    }
                }
            }
            Overlay::Branch(picker) => {
                if let Some(branch) = picker.branches.get(picker.selected) {
                    let cmd = format!("git checkout {}", branch);
                    self.run_command(cmd, cx);
                }
            }
        }
    }

    fn render_input_bar(&self, window: &Window, cx: &Context<Self>) -> Div {
        let is_focused = self.focus_handle.is_focused(window);
        let action_button = |icon: Icon| {
            div()
                .flex()
                .items_center()
                .justify_center()
                .w(px(24.0))
                .h(px(24.0))
                .rounded(px(6.0))
                .bg(rgb(0x141414))
                .border_1()
                .border_color(rgb(0x2a2a2a))
                .child(lucide_icon(icon, 12.0, 0x8a8a8a))
        };

        div()
            .flex()
            .flex_col()
            .gap(px(8.0))
            .px(px(16.0))
            .py(px(10.0))
            .h(px(84.0))
            .bg(rgb(0x1a1a1a))
            .border_t_1()
            .border_color(rgb(0x2a2a2a))
            .on_mouse_down(gpui::MouseButton::Left, cx.listener(Self::on_focus_input))
            .child(
                // Meta row (path + git)
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.0))
                            .px(px(10.0))
                            .py(px(6.0))
                            .rounded(px(6.0))
                            .bg(rgb(0x141414))
                            .border_1()
                            .border_color(rgb(0x2a2a2a))
                            .on_mouse_down(
                                gpui::MouseButton::Left,
                                cx.listener(Self::on_open_path_picker),
                            )
                            .child(lucide_icon(Icon::Folder, 12.0, 0x6b9eff))
                            .child(
                                div()
                                    .text_size(px(12.0))
                                    .text_color(rgb(0xcfcfcf))
                                    .child(self.current_path.clone()),
                            ),
                    )
                    .child(if let Some(ref status) = self.git_status {
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.0))
                            .px(px(10.0))
                            .py(px(6.0))
                            .rounded(px(6.0))
                            .bg(rgb(0x141414))
                            .border_1()
                            .border_color(rgb(0x2a2a2a))
                            .on_mouse_down(
                                gpui::MouseButton::Left,
                                cx.listener(Self::on_open_branch_picker),
                            )
                            .child(lucide_icon(Icon::GitBranch, 12.0, 0x6b9eff))
                            .child(
                                div()
                                    .text_size(px(12.0))
                                    .text_color(rgb(0xd7f7c7))
                                    .child(status.branch.clone()),
                            )
                            .child(if status.files_changed > 0 {
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(4.0))
                                    .child(lucide_icon(Icon::FileText, 12.0, 0x9a9a9a))
                                    .child(
                                        div()
                                            .text_size(px(11.0))
                                            .text_color(rgb(0xcfcfcf))
                                            .child(format!("{}", status.files_changed)),
                                    )
                            } else {
                                div()
                            })
                            .child(if status.added > 0 {
                                div()
                                    .text_size(px(11.0))
                                    .text_color(rgb(0x8bd06f))
                                    .child(format!("+{}", status.added))
                            } else {
                                div()
                            })
                            .child(if status.deleted > 0 {
                                div()
                                    .text_size(px(11.0))
                                    .text_color(rgb(0xff7b72))
                                    .child(format!("-{}", status.deleted))
                            } else {
                                div()
                            })
                    } else {
                        div()
                    }),
            )
            .child(
                // Input row
                div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.0))
                            .flex_1()
                            .child(lucide_icon(Icon::ChevronRight, 16.0, 0x6b9eff))
                            .child(self.render_input_text(is_focused)),
                    )
                    .child(
                        // Action icons
                        div()
                            .flex()
                            .items_center()
                            .gap(px(10.0))
                            .child(action_button(Icon::Bot))
                            .child(action_button(Icon::Clipboard))
                            .child(action_button(Icon::Check))
                            .child(action_button(Icon::AtSign))
                            .child(action_button(Icon::Settings)),
                    ),
            )
    }

    fn render_input_text(&self, is_focused: bool) -> Div {
        let show_placeholder = self.input.is_empty();
        let ghost = self.inline_ghost_text();
        let has_selection = self.has_selection();
        let show_ghost = !ghost.is_empty() && !show_placeholder && !has_selection;

        let caret = div()
            .w(px(2.0))
            .h(px(16.0))
            .rounded(px(1.0))
            .bg(if is_focused {
                rgb(0x6b9eff)
            } else {
                rgb(0x2a2a2a)
            });

        let text_normal = |text: String| {
            div()
                .text_size(px(15.0))
                .text_color(rgb(0xdddddd))
                .font_family("Cascadia Code")
                .child(text)
        };

        let text_selected = |text: String| {
            div()
                .px(px(2.0))
                .py(px(1.0))
                .rounded(px(3.0))
                .bg(rgb(0x2d4a7a))
                .child(
                    div()
                        .text_size(px(15.0))
                        .text_color(rgb(0xf0f0f0))
                        .font_family("Cascadia Code")
                        .child(text),
                )
        };

        let placeholder = div()
            .text_size(px(15.0))
            .text_color(rgb(0x666666))
            .child("Type a command...");

        let ghost_div = div()
            .text_size(px(15.0))
            .text_color(rgb(0x555555))
            .font_family("Cascadia Code")
            .child(ghost);

        let mut row = div().flex().gap(px(0.0));

        if show_placeholder {
            row = row.child(caret).child(placeholder);
            return row;
        }

        if let Some((a, b)) = self.normalized_selection().filter(|(a, b)| a != b) {
            let (pre, rest) = Self::split_at_index(&self.input, a);
            let (sel, post) = Self::split_at_index(&rest, b.saturating_sub(a));

            if self.cursor <= a {
                let (pre_left, pre_right) = Self::split_at_index(&pre, self.cursor.min(a));
                row = row
                    .child(text_normal(pre_left))
                    .child(caret)
                    .child(text_normal(pre_right))
                    .child(text_selected(sel))
                    .child(text_normal(post));
            } else if self.cursor >= b {
                let post_cursor = self.cursor.saturating_sub(b);
                let (post_left, post_right) = Self::split_at_index(&post, post_cursor);
                row = row
                    .child(text_normal(pre))
                    .child(text_selected(sel))
                    .child(text_normal(post_left))
                    .child(caret)
                    .child(text_normal(post_right));
            } else {
                let sel_cursor = self.cursor.saturating_sub(a);
                let (sel_left, sel_right) = Self::split_at_index(&sel, sel_cursor);
                row = row
                    .child(text_normal(pre))
                    .child(text_selected(sel_left))
                    .child(caret)
                    .child(text_selected(sel_right))
                    .child(text_normal(post));
            }
        } else {
            let (left, right) = self.split_at_cursor();
            row = row
                .child(text_normal(left))
                .child(caret)
                .child(text_normal(right));
            if show_ghost {
                row = row.child(ghost_div);
            }
        }

        row
    }

    fn commit_input(&mut self, cx: &mut Context<Self>) {
        let command = self.input.trim().to_string();
        if command.is_empty() {
            if let Some(ref mut pty) = self.pty {
                let _ = pty.write(b"\r\n");
            }
            self.input.clear();
            self.cursor = 0;
            self.clear_selection();
            cx.notify();
            return;
        }

        self.run_command(command, cx);
    }

    fn run_command(&mut self, command: String, cx: &mut Context<Self>) {
        let command = command.trim().to_string();
        if command.is_empty() {
            return;
        }

        let lower = command.to_ascii_lowercase();
        self.needs_git_refresh =
            lower.starts_with("git checkout") || lower.starts_with("git switch");

        self.push_history(&command);
        self.pending_echo = Some(command.clone());
        self.blocks.push(Block {
            command: command.clone(),
            output_lines: Vec::new(),
            has_error: false,
            context: Some(BlockContext {
                cwd: self.current_path.clone(),
                git_branch: self.git_status.as_ref().map(|g| g.branch.clone()),
                git_files: self.git_status.as_ref().map(|g| g.files_changed),
                git_added: self.git_status.as_ref().map(|g| g.added),
                git_deleted: self.git_status.as_ref().map(|g| g.deleted),
                git_modified: self.git_status.as_ref().map(|g| g.modified),
            }),
        });

        if let Some(ref mut pty) = self.pty {
            let _ = pty.write(format!("{command}\r\n").as_bytes());
        }

        self.input.clear();
        self.cursor = 0;
        self.history_open = false;
        self.history_items.clear();
        self.suggestions.clear();
        self.suggest_index = 0;
        self.clear_selection();
        self.input_visible = false;
        self.overlay = None;
        self.scroll_handle.scroll_to_bottom();
        cx.notify();
    }

    fn render_history_menu(&self) -> Div {
        if !self.history_open || self.history_items.is_empty() {
            return div();
        }

        let max_items = 8usize.min(self.history_items.len());
        let items = self
            .history_items
            .iter()
            .take(max_items)
            .enumerate()
            .map(|(i, item)| {
                let is_active = i == self.history_index;
                div()
                    .flex()
                    .items_center()
                    .px(px(10.0))
                    .py(px(6.0))
                    .rounded(px(6.0))
                    .bg(if is_active {
                        rgb(0x1f1f1f)
                    } else {
                        rgb(0x111111)
                    })
                    .border_1()
                    .border_color(if is_active {
                        rgb(0x2d2d2d)
                    } else {
                        rgb(0x1a1a1a)
                    })
                    .child(
                        div()
                            .text_size(px(12.0))
                            .text_color(rgb(0xcccccc))
                            .font_family("Cascadia Code")
                            .child(item.display.clone()),
                    )
            });

        div()
            .flex()
            .flex_col()
            .gap(px(6.0))
            .p(px(8.0))
            .rounded(px(8.0))
            .bg(rgb(0x0f0f0f))
            .border_1()
            .border_color(rgb(0x1f1f1f))
            .child(div().h(px(2.0)))
            .children(items)
    }

    fn render_history_menu_container(&self) -> Div {
        if !self.history_open || self.history_items.is_empty() {
            return div().h(px(0.0));
        }

        div()
            .px(px(16.0))
            .pb(px(8.0))
            .child(self.render_history_menu())
    }

    fn render_overlay(&self, cx: &Context<Self>) -> Div {
        let Some(ref overlay) = self.overlay else {
            return div().h(px(0.0));
        };

        let panel = match overlay {
            Overlay::Path(picker) => self.render_path_picker(picker, cx),
            Overlay::Branch(picker) => self.render_branch_picker(picker, cx),
        };

        div()
            .size_full()
            .absolute()
            .top_0()
            .left_0()
            .child(div().size_full().bg(opaque_grey(0.0, 0.25)).on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(Self::on_overlay_dismiss),
            ))
            .child(panel)
    }

    fn render_path_picker(&self, picker: &PathPickerState, cx: &Context<Self>) -> Div {
        let handle = cx.entity().downgrade();
        let query_text = if picker.query.is_empty() {
            "Search directories...".to_string()
        } else {
            picker.query.clone()
        };

        let items = picker.entries.iter().enumerate().map(|(i, entry)| {
            let is_active = i == picker.selected;
            let icon = Icon::Folder;
            div()
                .flex()
                .items_center()
                .gap(px(10.0))
                .px(px(12.0))
                .py(px(8.0))
                .rounded(px(6.0))
                .bg(if is_active {
                    rgb(0x1f2a2f)
                } else {
                    rgb(0x1a1a1a)
                })
                .border_1()
                .border_color(if is_active {
                    rgb(0x27404a)
                } else {
                    rgb(0x1f1f1f)
                })
                .child(lucide_icon(icon, 14.0, 0x9a9a9a))
                .child(
                    div()
                        .text_size(px(13.0))
                        .text_color(rgb(0xeeeeee))
                        .child(entry.name.clone()),
                )
                .on_mouse_down(gpui::MouseButton::Left, {
                    let handle = handle.clone();
                    move |_event, _window, cx| {
                        let _ = handle.update(cx, |view, cx| {
                            view.on_path_picker_select(i, cx);
                        });
                    }
                })
        });

        div()
            .absolute()
            .left(px(24.0))
            .bottom(px(120.0))
            .w(px(520.0))
            .rounded(px(10.0))
            .bg(rgb(0x171717))
            .border_1()
            .border_color(rgb(0x2a2a2a))
            .p(px(10.0))
            .flex()
            .flex_col()
            .gap(px(8.0))
            .child(
                div()
                    .px(px(10.0))
                    .py(px(8.0))
                    .rounded(px(6.0))
                    .bg(rgb(0x111111))
                    .border_1()
                    .border_color(rgb(0x252525))
                    .text_size(px(12.0))
                    .text_color(if picker.query.is_empty() {
                        rgb(0x666666)
                    } else {
                        rgb(0xcccccc)
                    })
                    .child(query_text),
            )
            .child(
                div()
                    .id("path_picker_list")
                    .flex_col()
                    .gap(px(6.0))
                    .max_h(px(260.0))
                    .overflow_y_scroll()
                    .children(items),
            )
    }

    fn render_branch_picker(&self, picker: &BranchPickerState, cx: &Context<Self>) -> Div {
        let handle = cx.entity().downgrade();
        let query_text = if picker.query.is_empty() {
            "Search branches...".to_string()
        } else {
            picker.query.clone()
        };
        let current = self.git_status.as_ref().map(|g| g.branch.clone());

        let items = picker.branches.iter().enumerate().map(|(i, branch)| {
            let is_active = i == picker.selected;
            let is_current = current.as_ref().map(|b| b == branch).unwrap_or(false);
            div()
                .flex()
                .items_center()
                .gap(px(10.0))
                .px(px(12.0))
                .py(px(8.0))
                .rounded(px(6.0))
                .bg(if is_active {
                    rgb(0x1f2a2f)
                } else {
                    rgb(0x1a1a1a)
                })
                .border_1()
                .border_color(if is_active {
                    rgb(0x27404a)
                } else {
                    rgb(0x1f1f1f)
                })
                .child(lucide_icon(Icon::GitBranch, 14.0, 0x9a9a9a))
                .child(
                    div()
                        .text_size(px(13.0))
                        .text_color(if is_current {
                            rgb(0xaad4ff)
                        } else {
                            rgb(0xeeeeee)
                        })
                        .child(branch.clone()),
                )
                .on_mouse_down(gpui::MouseButton::Left, {
                    let handle = handle.clone();
                    move |_event, _window, cx| {
                        let _ = handle.update(cx, |view, cx| {
                            view.on_branch_picker_select(i, cx);
                        });
                    }
                })
        });

        div()
            .absolute()
            .left(px(220.0))
            .bottom(px(120.0))
            .w(px(420.0))
            .rounded(px(10.0))
            .bg(rgb(0x171717))
            .border_1()
            .border_color(rgb(0x2a2a2a))
            .p(px(10.0))
            .flex()
            .flex_col()
            .gap(px(8.0))
            .child(
                div()
                    .px(px(10.0))
                    .py(px(8.0))
                    .rounded(px(6.0))
                    .bg(rgb(0x111111))
                    .border_1()
                    .border_color(rgb(0x252525))
                    .text_size(px(12.0))
                    .text_color(if picker.query.is_empty() {
                        rgb(0x666666)
                    } else {
                        rgb(0xcccccc)
                    })
                    .child(query_text),
            )
            .child(
                div()
                    .id("branch_picker_list")
                    .flex_col()
                    .gap(px(6.0))
                    .max_h(px(260.0))
                    .overflow_y_scroll()
                    .children(items),
            )
    }

    fn on_overlay_dismiss(
        &mut self,
        _event: &MouseDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.overlay = None;
        cx.notify();
    }

    fn on_path_picker_select(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(Overlay::Path(ref mut picker)) = self.overlay {
            picker.selected = index;
        }
        self.accept_overlay_selection(cx);
    }

    fn on_branch_picker_select(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(Overlay::Branch(ref mut picker)) = self.overlay {
            picker.selected = index;
        }
        self.accept_overlay_selection(cx);
    }

    fn push_history(&mut self, command: &str) {
        if self.history.front().map(|c| c == command).unwrap_or(false) {
            return;
        }
        self.history.push_front(command.to_string());
        if self.history.len() > 2000 {
            self.history.pop_back();
        }
        if let Some(path) = self.history_file.clone() {
            let _ = Self::append_history_line(&path, command);
        }
    }

    fn inline_ghost_text(&self) -> String {
        self.inline_ghost_insert().unwrap_or_default()
    }

    fn has_suggestion(&self) -> bool {
        if self.has_selection() {
            return false;
        }
        if self.inline_ghost_insert().is_some() {
            return true;
        }
        if let Some(item) = self.suggestions.get(self.suggest_index) {
            return item.insert != self.input;
        }
        false
    }

    fn cycle_suggestion(&mut self) {
        if self.suggestions.is_empty() {
            return;
        }
        self.suggest_index = (self.suggest_index + 1) % self.suggestions.len();
    }

    fn accept_suggestion(&mut self) {
        if let Some(insert) = self.inline_ghost_insert() {
            if !insert.is_empty() {
                self.insert_text(&insert);
                self.clear_selection();
                self.refresh_suggestions();
                self.refresh_history_menu();
                return;
            }
        }
        if let Some(item) = self.suggestions.get(self.suggest_index) {
            if item.insert != self.input {
                self.input = item.insert.clone();
                self.cursor = self.input.chars().count();
                self.clear_selection();
            }
        } else {
            return;
        }
        self.refresh_suggestions();
        self.refresh_history_menu();
    }

    fn accept_history_item(&mut self, cx: &mut Context<Self>) {
        if self.history_items.is_empty() {
            self.history_open = false;
            return;
        }
        let item = self.history_items[self.history_index.min(self.history_items.len() - 1)].clone();
        self.input = item.insert;
        self.cursor = self.input.chars().count();
        self.clear_selection();
        self.history_open = false;
        self.history_items.clear();
        self.refresh_suggestions();
        cx.notify();
    }

    fn open_or_step_history(&mut self, up: bool) {
        if !self.history_open {
            self.history_open = true;
            self.refresh_history_menu();
            self.history_index = 0;
            return;
        }

        if self.history_items.is_empty() {
            self.history_open = false;
            return;
        }

        if up {
            self.history_index = (self.history_index + 1).min(self.history_items.len() - 1);
        } else if self.history_index == 0 {
            self.history_open = false;
        } else {
            self.history_index -= 1;
        }
    }

    fn refresh_history_menu(&mut self) {
        if !self.history_open {
            return;
        }
        let prefix = self.prefix_at_cursor();
        self.history_items = self
            .history
            .iter()
            .filter(|cmd| cmd.starts_with(&prefix) && cmd.as_str() != prefix)
            .take(8)
            .map(|cmd| SuggestionItem {
                display: cmd.clone(),
                insert: cmd.clone(),
                source: SuggestSource::History,
            })
            .collect();
        if self.history_items.is_empty() {
            self.history_open = false;
        } else {
            self.history_index = self.history_index.min(self.history_items.len() - 1);
        }
    }

    fn refresh_suggestions(&mut self) {
        let prefix = self.prefix_at_cursor();
        if prefix.is_empty() {
            self.suggestions.clear();
            self.suggest_index = 0;
            return;
        }

        let mut history_items = Vec::new();
        for cmd in self.history.iter() {
            if cmd.starts_with(&prefix) && cmd.as_str() != prefix {
                history_items.push(SuggestionItem {
                    display: cmd.clone(),
                    insert: cmd.clone(),
                    source: SuggestSource::History,
                });
            }
        }

        let token = self.current_token();
        let mut path_items = Vec::new();
        let mut command_items = Vec::new();
        if Self::is_path_token(&token) {
            self.append_path_suggestions(&mut path_items);
            path_items.sort_by(|a, b| a.display.cmp(&b.display));
        } else if self.is_command_context() {
            self.maybe_refresh_path_commands();
            for cmd in &self.path_commands {
                if cmd.starts_with(&prefix) && cmd.as_str() != prefix {
                    command_items.push(SuggestionItem {
                        display: cmd.clone(),
                        insert: cmd.clone(),
                        source: SuggestSource::Command,
                    });
                }
            }
            command_items.sort_by(|a, b| a.display.cmp(&b.display));
        }

        self.suggestions = Self::dedupe_suggestions(history_items, path_items, command_items);
        self.suggest_index = 0;
    }

    fn is_command_context(&self) -> bool {
        let token = self.current_token();
        if token.is_empty() {
            return false;
        }
        if Self::is_path_token(&token) {
            return false;
        }
        self.prefix_at_cursor().trim_start() == token
    }

    fn split_at_cursor(&self) -> (String, String) {
        let mut left = String::new();
        let mut right = String::new();
        for (i, ch) in self.input.chars().enumerate() {
            if i < self.cursor {
                left.push(ch);
            } else {
                right.push(ch);
            }
        }
        (left, right)
    }

    fn split_at_index(input: &str, index: usize) -> (String, String) {
        let mut left = String::new();
        let mut right = String::new();
        for (i, ch) in input.chars().enumerate() {
            if i < index {
                left.push(ch);
            } else {
                right.push(ch);
            }
        }
        (left, right)
    }

    fn has_selection(&self) -> bool {
        matches!(self.selection, Some((a, b)) if a != b)
    }

    fn normalized_selection(&self) -> Option<(usize, usize)> {
        self.selection
            .map(|(a, b)| if a <= b { (a, b) } else { (b, a) })
    }

    fn clear_selection(&mut self) {
        self.selection = None;
        self.selection_anchor = None;
    }

    fn set_selection_from_anchor(&mut self, anchor: usize, cursor: usize) {
        self.selection_anchor = Some(anchor);
        self.selection = Some((anchor, cursor));
    }

    fn delete_selection_if_any(&mut self) -> bool {
        let Some((a, b)) = self.normalized_selection() else {
            return false;
        };
        if a == b {
            return false;
        }

        let mut out = String::new();
        for (i, ch) in self.input.chars().enumerate() {
            if i < a || i >= b {
                out.push(ch);
            }
        }
        self.input = out;
        self.cursor = a;
        self.clear_selection();
        true
    }

    fn prefix_at_cursor(&self) -> String {
        let (left, _) = self.split_at_cursor();
        left
    }

    fn inline_ghost_insert(&self) -> Option<String> {
        if self.has_selection() || self.input.is_empty() || self.suggestions.is_empty() {
            return None;
        }
        let suggestion = &self.suggestions[self.suggest_index.min(self.suggestions.len() - 1)];
        let (left, right) = self.split_at_cursor();
        let candidate = &suggestion.insert;

        if right.is_empty() {
            if candidate.starts_with(&self.input) && candidate.len() > self.input.len() {
                return Some(candidate[self.input.len()..].to_string());
            }
            return None;
        }

        if candidate.starts_with(&left) && candidate.ends_with(&right) {
            let start = left.len();
            let end = candidate.len().saturating_sub(right.len());
            if end > start {
                return Some(candidate[start..end].to_string());
            }
        }

        None
    }

    fn current_token(&self) -> String {
        self.prefix_at_cursor()
            .split_whitespace()
            .last()
            .unwrap_or("")
            .to_string()
    }

    fn is_path_token(token: &str) -> bool {
        if token.is_empty() {
            return false;
        }
        let t = token;
        t.starts_with("./")
            || t.starts_with("../")
            || t.starts_with("~")
            || t.starts_with(".\\")
            || t.starts_with("..\\")
            || t.contains('/')
            || t.contains('\\')
            || (t.len() >= 3
                && t.as_bytes()[1] == b':'
                && (t.as_bytes()[2] == b'\\' || t.as_bytes()[2] == b'/'))
            || t.starts_with("\\\\")
    }

    fn is_word_char(ch: char) -> bool {
        ch.is_alphanumeric() || ch == '_' || ch == '-' || ch == '.'
    }

    fn move_word_left(&self, from: usize) -> usize {
        let chars: Vec<char> = self.input.chars().collect();
        if from == 0 {
            return 0;
        }
        let mut i = from;
        while i > 0 && chars[i - 1].is_whitespace() {
            i -= 1;
        }
        while i > 0 && Self::is_word_char(chars[i - 1]) {
            i -= 1;
        }
        i
    }

    fn move_word_right(&self, from: usize) -> usize {
        let chars: Vec<char> = self.input.chars().collect();
        let n = chars.len();
        if from >= n {
            return n;
        }
        let mut i = from;
        while i < n && chars[i].is_whitespace() {
            i += 1;
        }
        while i < n && Self::is_word_char(chars[i]) {
            i += 1;
        }
        i
    }

    fn insert_text(&mut self, text: &str) {
        let _ = self.delete_selection_if_any();
        let mut left = String::new();
        let mut right = String::new();
        for (i, ch) in self.input.chars().enumerate() {
            if i < self.cursor {
                left.push(ch);
            } else {
                right.push(ch);
            }
        }
        left.push_str(text);
        left.push_str(&right);
        self.input = left;
        self.cursor = (self.cursor + text.chars().count()).min(self.input.chars().count());
        self.selection_anchor = None;
    }

    fn pop_char_before_cursor(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let mut out = String::new();
        let mut removed = false;
        for (i, ch) in self.input.chars().enumerate() {
            if i + 1 == self.cursor && !removed {
                removed = true;
                continue;
            }
            out.push(ch);
        }
        self.input = out;
        self.cursor = self.cursor.saturating_sub(1);
        self.selection_anchor = None;
    }

    fn move_cursor_left(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
        }
    }

    fn move_cursor_right(&mut self) {
        let max = self.input.chars().count();
        if self.cursor < max {
            self.cursor += 1;
        }
    }

    fn select_all_input(&mut self) {
        if self.input.is_empty() {
            return;
        }
        let end = self.input.chars().count();
        self.selection = Some((0, end));
        self.selection_anchor = Some(0);
        self.cursor = end;
        self.history_open = false;
    }

    fn load_path_commands() -> Vec<String> {
        let mut set = HashSet::new();
        let mut out = Vec::new();
        let path_var = std::env::var("PATH").unwrap_or_default();
        let exts = if cfg!(windows) {
            std::env::var("PATHEXT").unwrap_or_else(|_| ".EXE;.CMD;.BAT;.COM".to_string())
        } else {
            String::new()
        };
        let ext_list: Vec<String> = exts
            .split(';')
            .filter(|s| !s.is_empty())
            .map(|s| s.to_ascii_lowercase())
            .collect();

        for dir in path_var.split(';') {
            if dir.trim().is_empty() {
                continue;
            }
            let entries = std::fs::read_dir(dir);
            if entries.is_err() {
                continue;
            }
            for entry in entries.unwrap().flatten() {
                let path = entry.path();
                let file_name = match path.file_name().and_then(|s| s.to_str()) {
                    Some(name) => name.to_string(),
                    None => continue,
                };
                if cfg!(windows) {
                    let lower = file_name.to_ascii_lowercase();
                    if let Some(ext) = PathBuf::from(&lower).extension().and_then(|e| e.to_str()) {
                        let ext = format!(".{}", ext);
                        if ext_list.iter().any(|e| e == &ext) {
                            let stem = PathBuf::from(&file_name)
                                .file_stem()
                                .and_then(|s| s.to_str())
                                .unwrap_or(&file_name)
                                .to_string();
                            if set.insert(stem.clone()) {
                                out.push(stem);
                            }
                        }
                    }
                } else if path.is_file() {
                    if set.insert(file_name.clone()) {
                        out.push(file_name);
                    }
                }
            }
        }

        out.sort();
        out
    }

    fn maybe_refresh_path_commands(&mut self) {
        let ttl = Duration::from_secs(60);
        if self.last_path_scan.elapsed() < ttl {
            return;
        }
        self.last_path_scan = Instant::now();

        let current = std::env::var("PATH").unwrap_or_default();
        if current == self.last_path_var {
            return;
        }
        self.last_path_var = current;
        self.path_commands = Self::load_path_commands();
    }

    fn load_initial_history() -> (VecDeque<String>, Option<PathBuf>) {
        let mut history = VecDeque::new();
        let mut seen = HashSet::new();
        let mut app_history_path = None;

        if let Some(app_dir) = Self::app_data_dir() {
            let app_dir = app_dir.join("orbitshell");
            let _ = std::fs::create_dir_all(&app_dir);
            let app_path = app_dir.join("history.txt");
            app_history_path = Some(app_path.clone());
            Self::load_history_from_file(&app_path, &mut history, &mut seen);
        }

        if cfg!(windows) {
            if let Ok(appdata) = std::env::var("APPDATA") {
                let windows_ps = PathBuf::from(&appdata)
                    .join("Microsoft")
                    .join("Windows")
                    .join("PowerShell")
                    .join("PSReadLine")
                    .join("ConsoleHost_history.txt");
                Self::load_history_from_file(&windows_ps, &mut history, &mut seen);

                let pwsh_ps = PathBuf::from(&appdata)
                    .join("Microsoft")
                    .join("PowerShell")
                    .join("PSReadLine")
                    .join("ConsoleHost_history.txt");
                Self::load_history_from_file(&pwsh_ps, &mut history, &mut seen);
            }

            #[cfg(windows)]
            Self::load_cmd_doskey_history(&mut history, &mut seen);
        } else {
            Self::load_unix_shell_history(&mut history, &mut seen);
        }

        (history, app_history_path)
    }

    fn app_data_dir() -> Option<PathBuf> {
        if let Ok(appdata) = std::env::var("APPDATA") {
            return Some(PathBuf::from(appdata));
        }
        if let Ok(xdg) = std::env::var("XDG_DATA_HOME") {
            return Some(PathBuf::from(xdg));
        }
        if let Ok(home) = std::env::var("HOME") {
            return Some(PathBuf::from(home).join(".local").join("share"));
        }
        None
    }

    fn load_unix_shell_history(history: &mut VecDeque<String>, seen: &mut HashSet<String>) {
        let Some(home) = std::env::var("HOME").ok().map(PathBuf::from) else {
            return;
        };

        let bash = home.join(".bash_history");
        Self::load_history_from_file(&bash, history, seen);

        let zsh = home.join(".zsh_history");
        Self::load_zsh_history(&zsh, history, seen);

        let fish = home.join(".config").join("fish").join("fish_history");
        Self::load_fish_history(&fish, history, seen);
    }

    fn load_zsh_history(
        path: &PathBuf,
        history: &mut VecDeque<String>,
        seen: &mut HashSet<String>,
    ) {
        let Ok(contents) = std::fs::read_to_string(path) else {
            return;
        };
        for line in contents.lines().rev() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let cmd = if let Some(pos) = line.find(';') {
                &line[pos + 1..]
            } else {
                line
            };
            let cmd = cmd.trim();
            if cmd.is_empty() {
                continue;
            }
            if seen.insert(cmd.to_string()) {
                history.push_front(cmd.to_string());
            }
        }
    }

    fn load_fish_history(
        path: &PathBuf,
        history: &mut VecDeque<String>,
        seen: &mut HashSet<String>,
    ) {
        let Ok(contents) = std::fs::read_to_string(path) else {
            return;
        };
        for line in contents.lines().rev() {
            let line = line.trim();
            if let Some(cmd) = line.strip_prefix("- cmd:") {
                let cmd = cmd.trim();
                if !cmd.is_empty() && seen.insert(cmd.to_string()) {
                    history.push_front(cmd.to_string());
                }
            } else if let Some(cmd) = line.strip_prefix("cmd:") {
                let cmd = cmd.trim();
                if !cmd.is_empty() && seen.insert(cmd.to_string()) {
                    history.push_front(cmd.to_string());
                }
            }
        }
    }

    #[cfg(windows)]
    fn load_cmd_doskey_history(history: &mut VecDeque<String>, seen: &mut HashSet<String>) {
        use std::process::Command;
        let out = Command::new("cmd")
            .args(["/c", "doskey", "/history"])
            .output();
        let Ok(out) = out else {
            return;
        };
        if !out.status.success() {
            return;
        }
        let text = String::from_utf8_lossy(&out.stdout);
        for line in text.lines().rev() {
            let l = line.trim();
            if l.is_empty() {
                continue;
            }
            if seen.insert(l.to_string()) {
                history.push_front(l.to_string());
            }
        }
    }

    fn load_history_from_file(
        path: &PathBuf,
        history: &mut VecDeque<String>,
        seen: &mut HashSet<String>,
    ) {
        let Ok(contents) = std::fs::read_to_string(path) else {
            return;
        };
        let lines: Vec<&str> = contents.lines().filter(|l| !l.trim().is_empty()).collect();
        for line in lines.into_iter().rev() {
            if seen.insert(line.to_string()) {
                history.push_front(line.to_string());
            }
        }
    }

    fn append_history_line(path: &PathBuf, command: &str) -> std::io::Result<()> {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
        writeln!(file, "{command}")?;
        Ok(())
    }

    fn dedupe_suggestions(
        history_items: Vec<SuggestionItem>,
        path_items: Vec<SuggestionItem>,
        command_items: Vec<SuggestionItem>,
    ) -> Vec<SuggestionItem> {
        let mut seen = HashSet::new();
        let mut out = Vec::new();

        for item in history_items.into_iter() {
            if seen.insert(item.insert.clone()) {
                out.push(item);
            }
        }
        for item in path_items.into_iter() {
            if seen.insert(item.insert.clone()) {
                out.push(item);
            }
        }
        for item in command_items.into_iter() {
            if seen.insert(item.insert.clone()) {
                out.push(item);
            }
        }

        out
    }

    fn append_path_suggestions(&self, items: &mut Vec<SuggestionItem>) {
        let (left, right) = self.split_at_cursor();
        let token = self.current_token();
        if token.is_empty() {
            return;
        }

        let (base, partial, sep) = Self::split_path_token(&token);
        let base_dir = if base.is_empty() {
            PathBuf::from(".")
        } else {
            Self::expand_tilde(&base)
        };

        let Ok(entries) = std::fs::read_dir(&base_dir) else {
            return;
        };

        let left_prefix = left.strip_suffix(&token).unwrap_or(&left).to_string();

        for entry in entries.flatten() {
            let path = entry.path();
            let name = match path.file_name().and_then(|s| s.to_str()) {
                Some(name) => name.to_string(),
                None => continue,
            };
            if !name.starts_with(&partial) {
                continue;
            }
            let mut completed = if base.is_empty() {
                name.clone()
            } else {
                format!("{base}{sep}{name}")
            };
            if path.is_dir() {
                completed.push(sep);
            }
            let insert = format!("{left_prefix}{completed}{right}");
            items.push(SuggestionItem {
                display: completed,
                insert,
                source: SuggestSource::Path,
            });
        }
    }

    fn split_path_token(token: &str) -> (String, String, char) {
        let sep = if token.contains('\\') { '\\' } else { '/' };
        if let Some(pos) = token.rfind(sep) {
            let base = token[..pos].to_string();
            let partial = token[pos + 1..].to_string();
            (base, partial, sep)
        } else {
            (String::new(), token.to_string(), sep)
        }
    }

    fn expand_tilde(path: &str) -> PathBuf {
        if let Some(rest) = path.strip_prefix('~') {
            let home = std::env::var("USERPROFILE")
                .or_else(|_| std::env::var("HOME"))
                .ok()
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("~"));
            if rest.is_empty() {
                return home;
            }
            let rest = rest.trim_start_matches(['\\', '/']);
            return home.join(rest);
        }
        PathBuf::from(path)
    }

    fn ensure_output_block(&mut self) -> &mut Block {
        if self.blocks.is_empty() {
            self.blocks.push(Block {
                command: String::new(),
                output_lines: Vec::new(),
                has_error: false,
                context: None,
            });
        }
        self.blocks.last_mut().expect("blocks is not empty")
    }

    fn append_output(&mut self, chunk: &str, cx: &mut Context<Self>) {
        let normalized = strip_ansi(chunk).replace("\r\n", "\n").replace('\r', "\n");
        let mut lines: Vec<&str> = normalized.split('\n').collect();
        if normalized.ends_with('\n') {
            if matches!(lines.last(), Some(&"")) {
                lines.pop();
            }
        }
        for line in &lines {
            self.maybe_update_prompt_path(line, cx);
            if self.needs_git_refresh && Self::is_git_branch_change_line(line) {
                self.refresh_git_status();
                self.needs_git_refresh = false;
            }
        }

        let mut iter = lines.into_iter();
        if let Some(first) = iter.next() {
            self.append_output_first_line(first);
        }
        for part in iter {
            self.append_output_new_line(part);
        }
    }

    fn maybe_update_prompt_path(&mut self, line: &str, cx: &mut Context<Self>) {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("PS ") {
            if let Some(path) = rest.strip_suffix('>') {
                let path = path.trim();
                if !path.is_empty() && self.current_path != path {
                    self.current_path = path.to_string();
                    self.refresh_git_status();
                    self.needs_git_refresh = false;
                    cx.emit(TabViewEvent::CwdChanged(PathBuf::from(path)));
                }
            }
        }
    }

    fn is_git_branch_change_line(line: &str) -> bool {
        let s = line.trim().to_ascii_lowercase();
        s.contains("switched to branch")
            || s.contains("switched to a new branch")
            || s.contains("already on")
            || s.contains("head is now at")
            || s.contains("your branch is up to date")
    }

    fn is_prompt_line(line: &str) -> bool {
        let trimmed = line.trim();
        trimmed.starts_with("PS ") && trimmed.ends_with('>')
    }

    fn should_skip_output_line(&mut self, line: &str) -> bool {
        let trimmed = line.trim();

        if let Some(expected) = self.pending_echo.as_ref() {
            if trimmed == expected {
                self.pending_echo = None;
                return true;
            }
        }

        if trimmed == ">>" {
            self.input_visible = true;
            return true;
        }

        if Self::is_prompt_line(trimmed) {
            self.input_visible = true;
            self.refresh_git_status();
            self.needs_git_refresh = false;
            return true;
        }
        false
    }

    fn refresh_git_status(&mut self) {
        let cwd = Self::expand_tilde(&self.current_path);
        self.git_status = get_git_status(&cwd);
    }

    fn append_output_first_line(&mut self, line: &str) {
        if self.should_skip_output_line(line) {
            return;
        }

        let block = self.ensure_output_block();
        if Self::is_error_line(line) {
            block.has_error = true;
        }
        if block.output_lines.is_empty() {
            block.output_lines.push(line.to_string());
        } else if let Some(last) = block.output_lines.last_mut() {
            last.push_str(line);
        }
        self.scroll_handle.scroll_to_bottom();
    }

    fn append_output_new_line(&mut self, line: &str) {
        if self.should_skip_output_line(line) {
            return;
        }
        let block = self.ensure_output_block();
        if Self::is_error_line(line) {
            block.has_error = true;
        }
        block.output_lines.push(line.to_string());
        self.scroll_handle.scroll_to_bottom();
    }

    fn is_error_line(line: &str) -> bool {
        let s = line.trim().to_ascii_lowercase();
        s.contains("not recognized as")
            || s.contains("is not recognized")
            || s.contains("cannot find path")
            || s.contains("categoryinfo")
            || s.contains("fullyqualifiederrorid")
            || s.starts_with("error:")
            || s.contains("exception")
            || s.contains("at line:")
    }

    fn is_dir_header_line(line: &str) -> bool {
        let trimmed = line.trim();
        trimmed.starts_with("Directory:")
            || trimmed.starts_with("Mode")
            || trimmed.starts_with("----")
            || trimmed.contains("LastWriteTime")
                && trimmed.contains("Length")
                && trimmed.contains("Name")
    }

    fn render_output_line(&self, line: &str, has_error: bool) -> Div {
        let color = if has_error && Self::is_error_line(line) {
            rgb(0xff7b72)
        } else if Self::is_dir_header_line(line) {
            rgb(0x8bd06f)
        } else {
            rgb(0xdddddd)
        };

        div().text_color(color).child(line.to_string())
    }

    fn render_block(&self, block: &Block, index: usize, active_index: usize) -> Div {
        let has_command = !block.command.is_empty();
        let is_active = index == active_index && has_command;
        let block_bg = if block.has_error {
            rgb(0x2a1515)
        } else if is_active {
            rgb(0x0e2a33)
        } else {
            rgb(0x0a0a0a)
        };
        let divider_color = if index > 0 {
            rgb(0x1a1a1a)
        } else {
            rgb(0x0a0a0a)
        };
        let accent_color = if is_active {
            rgb(0x2b7a8f)
        } else {
            rgb(0x0a0a0a)
        };

        let context_line = if let Some(ref ctx) = block.context {
            let mut line = ctx.cwd.clone();
            if let Some(ref branch) = ctx.git_branch {
                if let Some(files) = ctx.git_files {
                    line = format!("{line}  git:({branch})  {files} files");
                    if let Some(added) = ctx.git_added {
                        if added > 0 {
                            line = format!("{line}  +{added}");
                        }
                    }
                    if let Some(deleted) = ctx.git_deleted {
                        if deleted > 0 {
                            line = format!("{line}  -{deleted}");
                        }
                    }
                    if let Some(modified) = ctx.git_modified {
                        if modified > 0 {
                            line = format!("{line}  ~{modified}");
                        }
                    }
                } else {
                    line = format!("{line}  git:({branch})");
                }
            }
            div()
                .text_size(px(11.0))
                .text_color(rgb(0x7a7a7a))
                .child(line)
        } else {
            div()
        };

        let header = if has_command {
            div()
                .text_size(px(13.0))
                .text_color(if block.has_error {
                    rgb(0xffa3a3)
                } else {
                    rgb(0xffe29a)
                })
                .font_weight(FontWeight::BOLD)
                .child(block.command.clone())
        } else {
            div()
        };

        let output = if block.output_lines.is_empty() {
            div()
        } else {
            div().flex_col().gap(px(2.0)).text_size(px(12.0)).children(
                block
                    .output_lines
                    .iter()
                    .map(|line| self.render_output_line(line, block.has_error)),
            )
        };

        div()
            .flex()
            .gap(px(12.0))
            .px(px(12.0))
            .py(px(10.0))
            .border_t_1()
            .border_color(divider_color)
            .bg(block_bg)
            .child(div().w(px(3.0)).bg(accent_color))
            .child(
                div()
                    .flex_1()
                    .flex_col()
                    .gap(px(6.0))
                    .child(context_line)
                    .child(header)
                    .child(output),
            )
    }
}

impl Render for TabView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.auto_focus {
            window.focus(&self.focus_handle);
            self.auto_focus = false;
        }

        let mut root = div()
            .id("terminal_root")
            .track_focus(&self.focus_handle)
            .flex()
            .flex_col()
            .size_full()
            .min_h(px(0.0))
            .bg(rgb(0x0a0a0a));

        if matches!(self.mode, TabViewMode::Terminal) {
            root = root
                .focusable()
                .on_key_down(cx.listener(Self::on_key_down))
                .on_mouse_down(gpui::MouseButton::Left, cx.listener(Self::on_focus_input))
                .child(
                    // Terminal output area
                    div()
                        .flex_1()
                        .min_h(px(0.0))
                        .p(px(16.0))
                        .id("terminal_output")
                        .track_scroll(&self.scroll_handle)
                        .overflow_scroll()
                        .font_family("Cascadia Code")
                        .text_size(px(13.0))
                        .text_color(rgb(0xcccccc))
                        .child({
                            let active_index = self.blocks.len().saturating_sub(1);
                            let blocks: Vec<Div> = self
                                .blocks
                                .iter()
                                .enumerate()
                                .map(|(i, block)| self.render_block(block, i, active_index))
                                .collect();
                            div()
                                .flex_col()
                                .gap(px(0.0))
                                .min_h(px(0.0))
                                .children(blocks)
                        }),
                )
                .child(self.render_overlay(cx))
                .child(if self.input_visible {
                    div()
                        .flex_none()
                        .child(self.render_history_menu_container())
                        .child(
                            div()
                                .px(px(16.0))
                                .pb(px(12.0))
                                .child(self.render_input_bar(window, cx)),
                        )
                } else {
                    div().h(px(0.0))
                });
        } else if let TabViewMode::Welcome(ref welcome) = self.mode {
            root = root.child(div().flex_1().min_h(px(0.0)).child(welcome.clone()));
        } else if let TabViewMode::Settings(ref settings) = self.mode {
            root = root.child(div().flex_1().min_h(px(0.0)).child(settings.clone()));
        }

        root
    }
}

fn strip_ansi(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\x1b' {
            if let Some(next) = chars.peek() {
                match *next {
                    '[' => {
                        chars.next();
                        while let Some(c) = chars.next() {
                            if ('@'..='~').contains(&c) {
                                break;
                            }
                        }
                    }
                    ']' => {
                        chars.next();
                        let mut prev = '\0';
                        while let Some(c) = chars.next() {
                            if c == '\x07' || (prev == '\x1b' && c == '\\') {
                                break;
                            }
                            prev = c;
                        }
                    }
                    _ => {
                        continue;
                    }
                }
            }
        } else {
            out.push(ch);
        }
    }
    out
}

impl Focusable for TabView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl EventEmitter<TabViewEvent> for TabView {}
