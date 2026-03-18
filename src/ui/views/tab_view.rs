use crate::git::get_git_branches;
use crate::git::get_git_status;
use crate::terminal::TerminalPty;
use crate::{
    acp::client::AcpClient,
    acp::manager::AgentCommandSpec,
    acp::resolve::{AgentKey, ConflictPolicy, EffectiveAgentRow, load_effective_agent_rows},
};
use futures::StreamExt;
use futures::channel::mpsc;
use gpui::*;
use lucide_icons::Icon;
use std::collections::{HashSet, VecDeque};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crate::ui::icons::lucide_icon;
use crate::ui::recent::RecentEntry;
use crate::ui::text_edit::TextEditState;
use crate::ui::views::agent_view::AgentView;
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
    last_line_incomplete: bool,
    total_output_lines: usize,
    follow_output: bool,
    input_mode: InputMode,
    agent_rows: Vec<EffectiveAgentRow>,
    agent_selected_key: Option<AgentKey>,
    agent_client: Option<Arc<Mutex<AcpClient>>>,
    agent_client_key: Option<AgentKey>,
    agent_busy: bool,
    agent_needs_auth: bool,
    selected_block: Option<usize>,
    output_selection_anchor: Option<(usize, usize)>,
    output_selection_head: Option<(usize, usize)>,
    output_selecting: bool,
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

struct TooltipView {
    text: String,
}

enum Overlay {
    Path(PathPickerState),
    Branch(BranchPickerState),
}

#[derive(Clone, Debug)]
struct SuggestionItem {
    display: String,
    insert: String,
}

pub enum TabViewEvent {
    CwdChanged(PathBuf),
    OpenRepository(PathBuf),
}

const MAX_OUTPUT_LINES: usize = 5000;

enum TabViewMode {
    Terminal,
    Agent(Entity<AgentView>),
    Welcome(Entity<WelcomeView>),
    Settings(Entity<SettingsView>),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum InputMode {
    Terminal,
    Agent,
}

enum AgentPromptEvent {
    Update { text: String, append: bool },
    Done(Result<Option<String>, String>),
}

impl TabView {
    fn update_follow_output_from_scroll(&mut self) {
        let max_y: f32 = self.scroll_handle.max_offset().height.into();
        if max_y <= 0.0 {
            self.follow_output = true;
            return;
        }
        let offset_y: f32 = self.scroll_handle.offset().y.into();
        let threshold_px = 24.0;
        self.follow_output = (max_y - offset_y) <= threshold_px;
    }

    fn on_output_scroll_wheel(
        &mut self,
        _event: &ScrollWheelEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let handle = cx.entity().downgrade();
        window.defer(cx, move |_window, cx| {
            let _ = handle.update(cx, |view, cx| {
                view.update_follow_output_from_scroll();
                cx.notify();
            });
        });
    }

    fn render_jump_to_bottom(&self, cx: &Context<Self>) -> Div {
        let handle = cx.entity().downgrade();
        let tooltip_text = "Jump to the bottom of this block".to_string();
        let mut button = div()
            .absolute()
            .right(px(16.0))
            .bottom(px(16.0))
            .size(px(36.0))
            .rounded(px(8.0))
            .bg(rgb(0x1a1a1a))
            .border_1()
            .border_color(rgb(0x2a2a2a))
            .flex()
            .items_center()
            .justify_center()
            .child(
                lucide_icon(Icon::ArrowDownToLine, 16.0, 0xdddddd)
                    .cursor(CursorStyle::PointingHand),
            )
            .on_mouse_down(MouseButton::Left, move |_event, _window, cx| {
                cx.stop_propagation();
                let _ = handle.update(cx, |view, cx| {
                    view.follow_output = true;
                    view.scroll_handle.scroll_to_bottom();
                    cx.notify();
                });
            });

        button.interactivity().tooltip(move |_window, cx| {
            let text = tooltip_text.clone();
            cx.new(|_| TooltipView { text }).into()
        });

        button
    }

    fn on_copy_output(
        &mut self,
        _event: &MouseDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.copy_selected_output(cx);
    }

    fn copy_selected_output(&mut self, cx: &mut Context<Self>) {
        if let Some(text) = self.selected_output_text() {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
            return;
        }

        let fallback_index = self
            .selected_block
            .or_else(|| self.blocks.len().checked_sub(1));
        if let Some(index) = fallback_index {
            self.copy_block_at(index, cx);
        }
    }

    fn copy_block_at(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(block) = self.blocks.get(index) else {
            return;
        };
        let text = self.block_to_text(block);
        if !text.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
            self.selected_block = Some(index);
        }
    }

    fn block_context_text(&self, block: &Block) -> Option<String> {
        let ctx = block.context.as_ref()?;
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
        Some(line)
    }

    fn block_to_text(&self, block: &Block) -> String {
        let mut parts = Vec::new();
        if let Some(context_line) = self.block_context_text(block) {
            parts.push(context_line);
        }
        if !block.command.is_empty() {
            parts.push(block.command.clone());
        }
        parts.extend(block.output_lines.clone());
        parts.join("\n")
    }

    fn clear_output_selection(&mut self) {
        self.output_selection_anchor = None;
        self.output_selection_head = None;
        self.output_selecting = false;
    }

    fn shift_output_indices_after_front_block_removal(&mut self) {
        self.selected_block = self.selected_block.and_then(|index| index.checked_sub(1));

        let shift = |point: Option<(usize, usize)>| -> Option<(usize, usize)> {
            match point {
                Some((0, _)) => None,
                Some((block_index, line_index)) => Some((block_index - 1, line_index)),
                None => None,
            }
        };

        self.output_selection_anchor = shift(self.output_selection_anchor);
        self.output_selection_head = shift(self.output_selection_head);
        if self.output_selection_anchor.is_none() || self.output_selection_head.is_none() {
            self.clear_output_selection();
        }
    }

    fn normalize_output_selection(&self) -> Option<((usize, usize), (usize, usize))> {
        let anchor = self.output_selection_anchor?;
        let head = self.output_selection_head?;
        if anchor == head {
            return None;
        }
        if Self::line_position_key(anchor) <= Self::line_position_key(head) {
            Some((anchor, head))
        } else {
            Some((head, anchor))
        }
    }

    fn line_position_key((block_index, line_index): (usize, usize)) -> (usize, usize) {
        (block_index, line_index)
    }

    fn is_output_line_selected(&self, block_index: usize, line_index: usize) -> bool {
        let Some((start, end)) = self.normalize_output_selection() else {
            return false;
        };
        let current = (block_index, line_index);
        Self::line_position_key(current) >= Self::line_position_key(start)
            && Self::line_position_key(current) <= Self::line_position_key(end)
    }

    fn selected_output_text(&self) -> Option<String> {
        let (start, end) = self.normalize_output_selection()?;
        let mut lines = Vec::new();

        for block_index in start.0..=end.0 {
            let Some(block) = self.blocks.get(block_index) else {
                continue;
            };
            if block.output_lines.is_empty() {
                continue;
            }

            let from = if block_index == start.0 { start.1 } else { 0 };
            if from >= block.output_lines.len() {
                continue;
            }
            let to = if block_index == end.0 {
                end.1.min(block.output_lines.len().saturating_sub(1))
            } else {
                block.output_lines.len().saturating_sub(1)
            };
            if from > to {
                continue;
            }
            lines.extend(block.output_lines[from..=to].iter().cloned());
        }

        if lines.is_empty() {
            None
        } else {
            Some(lines.join("\n"))
        }
    }

    fn on_select_block(&mut self, index: usize, cx: &mut Context<Self>) {
        self.selected_block = Some(index);
        self.clear_output_selection();
        cx.notify();
    }

    fn on_output_line_mouse_down_at(
        &mut self,
        block_index: usize,
        line_index: usize,
        event: &MouseDownEvent,
        cx: &mut Context<Self>,
    ) {
        let position = (block_index, line_index);
        if event.modifiers.shift {
            if self.output_selection_anchor.is_none() {
                self.output_selection_anchor = Some(position);
            }
            self.output_selection_head = Some(position);
        } else {
            self.output_selection_anchor = Some(position);
            self.output_selection_head = Some(position);
        }
        self.selected_block = Some(block_index);
        self.output_selecting = true;
        cx.notify();
        cx.stop_propagation();
    }

    fn on_output_line_mouse_move_at(
        &mut self,
        block_index: usize,
        line_index: usize,
        event: &MouseMoveEvent,
        cx: &mut Context<Self>,
    ) {
        if !self.output_selecting || !event.dragging() {
            return;
        }
        self.output_selection_head = Some((block_index, line_index));
        self.selected_block = Some(block_index);
        cx.notify();
    }

    fn on_output_mouse_up(
        &mut self,
        _event: &MouseUpEvent,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
        self.output_selecting = false;
    }

    fn format_path(path: &Path) -> String {
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

    pub fn new_agent(cx: &mut Context<Self>) -> Self {
        let mut view = Self::new_base(cx);
        let agent = cx.new(|cx| AgentView::new(cx));
        view.mode = TabViewMode::Agent(agent);
        view
    }

    fn new_base(cx: &mut Context<Self>) -> Self {
        let (history, history_file) = Self::load_initial_history();
        let last_path_var = std::env::var("PATH").unwrap_or_default();
        let agent_rows = load_effective_agent_rows(ConflictPolicy::LocalWins).unwrap_or_default();
        let agent_selected_key = agent_rows.first().map(|row| row.agent_key.clone());
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
            last_line_incomplete: false,
            total_output_lines: 0,
            follow_output: true,
            input_mode: InputMode::Terminal,
            agent_rows,
            agent_selected_key,
            agent_client: None,
            agent_client_key: None,
            agent_busy: false,
            agent_needs_auth: false,
            selected_block: None,
            output_selection_anchor: None,
            output_selection_head: None,
            output_selecting: false,
        }
    }

    pub fn set_recent(&mut self, recent: Vec<RecentEntry>, cx: &mut Context<Self>) {
        if let TabViewMode::Welcome(ref welcome) = self.mode {
            let _ = welcome.update(cx, |view, cx| {
                view.set_recent(recent, cx);
            });
        }
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
            .map(|path| Self::format_path(path))
            .unwrap_or_else(|| "~".to_string());
        self.git_status = cwd.as_ref().and_then(|path| get_git_status(path));
        self.blocks.clear();
        self.selected_block = None;
        self.clear_output_selection();
        self.total_output_lines = 0;
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
        self.last_line_incomplete = false;
        self.follow_output = true;
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
        if ctrl && shift && event.keystroke.key.eq_ignore_ascii_case("c") {
            self.copy_selected_output(cx);
            cx.stop_propagation();
            return;
        }
        if ctrl && event.keystroke.key.eq_ignore_ascii_case("i") {
            self.toggle_input_mode(cx);
            cx.stop_propagation();
            return;
        }
        if ctrl
            && event.keystroke.key.eq_ignore_ascii_case("c")
            && self.input_mode == InputMode::Agent
            && self.agent_busy
        {
            self.cancel_agent_prompt();
            cx.notify();
            cx.stop_propagation();
            return;
        }
        if ctrl && event.keystroke.key.eq_ignore_ascii_case("a") {
            self.select_all_input();
            cx.notify();
            cx.stop_propagation();
            return;
        }
        if self.input_visible
            && ((ctrl && event.keystroke.key.eq_ignore_ascii_case("v"))
                || (shift && event.keystroke.key.eq_ignore_ascii_case("insert")))
        {
            if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                let paste = text
                    .replace("\r\n", "\n")
                    .replace('\r', "\n")
                    .replace('\n', " ");
                if !paste.is_empty() {
                    self.insert_text(&paste);
                    self.refresh_suggestions();
                    self.refresh_history_menu();
                    cx.notify();
                }
            }
            cx.stop_propagation();
            return;
        }

        if self.input_mode == InputMode::Terminal && ctrl && event.keystroke.key.len() == 1 {
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
        let cwd = expand_tilde(&self.current_path);
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
        let cwd = expand_tilde(&self.current_path);
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

    fn on_set_terminal_mode(
        &mut self,
        _event: &MouseDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.input_mode != InputMode::Terminal {
            self.toggle_input_mode(cx);
        }
    }

    fn on_set_agent_mode(
        &mut self,
        _event: &MouseDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.input_mode != InputMode::Agent {
            self.toggle_input_mode(cx);
        }
    }

    fn on_prev_agent(
        &mut self,
        _event: &MouseDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.agent_rows.is_empty() {
            return;
        }
        let next_index = match self.active_agent_index() {
            Some(0) | None => self.agent_rows.len().saturating_sub(1),
            Some(index) => index.saturating_sub(1),
        };
        self.select_agent_index(next_index);
        cx.notify();
    }

    fn on_next_agent(
        &mut self,
        _event: &MouseDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.agent_rows.is_empty() {
            return;
        }
        let next_index = match self.active_agent_index() {
            Some(index) => (index + 1) % self.agent_rows.len(),
            None => 0,
        };
        self.select_agent_index(next_index);
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

    fn render_input_bar(&mut self, window: &Window, cx: &Context<Self>) -> Div {
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
                .child(lucide_icon(icon, 12.0, 0x8a8a8a).cursor(CursorStyle::PointingHand))
        };
        let agent_name = self
            .active_agent_name()
            .unwrap_or_else(|| "No agent".to_string());
        let agent_mode = self.input_mode == InputMode::Agent;

        div()
            .flex()
            .flex_col()
            .gap(px(8.0))
            .px(px(16.0))
            .py(px(10.0))
            .h(px(84.0))
            .bg(rgb(0x1a1a1a))
            .border_1()
            .border_color(rgb(0x2a2a2a))
            .rounded(px(10.0))
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
                            .gap(px(4.0))
                            .px(px(4.0))
                            .py(px(4.0))
                            .rounded(px(8.0))
                            .bg(rgb(0x141414))
                            .border_1()
                            .border_color(rgb(0x2a2a2a))
                            .child(
                                div()
                                    .w(px(28.0))
                                    .h(px(24.0))
                                    .rounded(px(6.0))
                                    .bg(if agent_mode {
                                        rgb(0x141414)
                                    } else {
                                        rgb(0x1b283a)
                                    })
                                    .border_1()
                                    .border_color(if agent_mode {
                                        rgb(0x2a2a2a)
                                    } else {
                                        rgb(0x3f669c)
                                    })
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .child(
                                        lucide_icon(
                                            Icon::Terminal,
                                            13.0,
                                            if agent_mode { 0x7f7f7f } else { 0x8eb8ff },
                                        )
                                        .cursor(CursorStyle::PointingHand),
                                    )
                                    .on_mouse_down(
                                        gpui::MouseButton::Left,
                                        cx.listener(Self::on_set_terminal_mode),
                                    ),
                            )
                            .child(
                                div()
                                    .w(px(28.0))
                                    .h(px(24.0))
                                    .rounded(px(6.0))
                                    .bg(if agent_mode {
                                        rgb(0x1b283a)
                                    } else {
                                        rgb(0x141414)
                                    })
                                    .border_1()
                                    .border_color(if agent_mode {
                                        rgb(0x3f669c)
                                    } else {
                                        rgb(0x2a2a2a)
                                    })
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .child(
                                        lucide_icon(
                                            Icon::Bot,
                                            13.0,
                                            if agent_mode { 0x8eb8ff } else { 0x7f7f7f },
                                        )
                                        .cursor(CursorStyle::PointingHand),
                                    )
                                    .on_mouse_down(
                                        gpui::MouseButton::Left,
                                        cx.listener(Self::on_set_agent_mode),
                                    ),
                            ),
                    )
                    .child(if agent_mode {
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.0))
                            .px(px(8.0))
                            .py(px(5.0))
                            .rounded(px(6.0))
                            .bg(rgb(0x141414))
                            .border_1()
                            .border_color(rgb(0x2a2a2a))
                            .child(
                                div()
                                    .w(px(18.0))
                                    .h(px(18.0))
                                    .rounded(px(4.0))
                                    .bg(rgb(0x191919))
                                    .border_1()
                                    .border_color(rgb(0x2a2a2a))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .text_size(px(11.0))
                                    .text_color(rgb(0xbdbdbd))
                                    .child("<")
                                    .on_mouse_down(
                                        gpui::MouseButton::Left,
                                        cx.listener(Self::on_prev_agent),
                                    ),
                            )
                            .child(
                                div()
                                    .text_size(px(11.0))
                                    .text_color(rgb(0xcfcfcf))
                                    .child(agent_name),
                            )
                            .child(if self.agent_needs_auth {
                                div()
                                    .px(px(8.0))
                                    .py(px(2.0))
                                    .rounded(px(5.0))
                                    .bg(rgb(0x2a1a14))
                                    .border_1()
                                    .border_color(rgb(0x8b4a2b))
                                    .text_size(px(10.0))
                                    .text_color(rgb(0xffc18f))
                                    .child("Authenticate")
                                    .on_mouse_down(
                                        gpui::MouseButton::Left,
                                        cx.listener(Self::on_authenticate_agent),
                                    )
                            } else {
                                div()
                            })
                            .child(
                                div()
                                    .w(px(18.0))
                                    .h(px(18.0))
                                    .rounded(px(4.0))
                                    .bg(rgb(0x191919))
                                    .border_1()
                                    .border_color(rgb(0x2a2a2a))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .text_size(px(11.0))
                                    .text_color(rgb(0xbdbdbd))
                                    .child(">")
                                    .on_mouse_down(
                                        gpui::MouseButton::Left,
                                        cx.listener(Self::on_next_agent),
                                    ),
                            )
                    } else {
                        div()
                    })
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
                            .child(
                                lucide_icon(Icon::Folder, 12.0, 0x6b9eff)
                                    .cursor(CursorStyle::PointingHand),
                            )
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
                            .child(
                                lucide_icon(Icon::GitBranch, 12.0, 0x6b9eff)
                                    .cursor(CursorStyle::PointingHand),
                            )
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
                                    .child(
                                        lucide_icon(Icon::FileText, 12.0, 0x9a9a9a)
                                            .cursor(CursorStyle::PointingHand),
                                    )
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
                            .child(
                                lucide_icon(Icon::ChevronRight, 16.0, 0x6b9eff)
                                    .cursor(CursorStyle::PointingHand),
                            )
                            .child(self.render_input_text(is_focused)),
                    )
                    .child(
                        // Action icons
                        div()
                            .flex()
                            .items_center()
                            .gap(px(10.0))
                            .child(action_button(Icon::Clipboard).on_mouse_down(
                                gpui::MouseButton::Left,
                                cx.listener(Self::on_copy_output),
                            ))
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

        let placeholder_text = if self.input_mode == InputMode::Agent {
            "Ask the agent..."
        } else {
            "Type a command..."
        };
        let placeholder = div()
            .text_size(px(15.0))
            .text_color(rgb(0x666666))
            .child(placeholder_text);

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

    fn toggle_input_mode(&mut self, cx: &mut Context<Self>) {
        if self.input_mode == InputMode::Terminal {
            self.input_mode = InputMode::Agent;
            self.follow_output = true;
        } else {
            self.input_mode = InputMode::Terminal;
        }
        cx.notify();
    }

    fn active_agent_name(&self) -> Option<String> {
        self.active_agent_row().map(|row| row.name.clone())
    }

    fn active_agent_spec(&self) -> Option<crate::acp::manager::AgentSpec> {
        self.active_agent_row().map(|row| row.spec.clone())
    }

    fn active_agent_row(&self) -> Option<&EffectiveAgentRow> {
        let selected_key = self.agent_selected_key.as_ref()?;
        self.agent_rows
            .iter()
            .find(|row| row.agent_key == *selected_key)
            .or_else(|| self.agent_rows.first())
    }

    fn active_agent_index(&self) -> Option<usize> {
        let selected_key = self.agent_selected_key.as_ref()?;
        self.agent_rows
            .iter()
            .position(|row| row.agent_key == *selected_key)
            .or(if self.agent_rows.is_empty() {
                None
            } else {
                Some(0)
            })
    }

    fn select_agent_index(&mut self, index: usize) {
        self.agent_selected_key = self.agent_rows.get(index).map(|row| row.agent_key.clone());
        self.agent_client = None;
        self.agent_client_key = None;
        self.agent_needs_auth = false;
    }

    fn is_auth_related_error(message: &str) -> bool {
        let s = message.to_ascii_lowercase();
        s.contains("login")
            || s.contains("authenticate")
            || s.contains("not logged in")
            || s.contains("unauthorized")
            || s.contains("401")
            || s.contains("auth required")
    }

    fn quote_shell_token(token: &str) -> String {
        if token.is_empty() {
            return "\"\"".to_string();
        }
        let needs_quotes = token
            .chars()
            .any(|c| c.is_whitespace() || c == '"' || c == '\'');
        if !needs_quotes {
            return token.to_string();
        }
        format!("\"{}\"", token.replace('"', "`\""))
    }

    fn format_shell_command(cmd: &AgentCommandSpec) -> String {
        let mut parts = Vec::with_capacity(cmd.args.len() + 1);
        parts.push(Self::quote_shell_token(&cmd.command));
        for arg in &cmd.args {
            parts.push(Self::quote_shell_token(arg));
        }
        parts.join(" ")
    }

    fn run_agent_auth_flow(&mut self, cx: &mut Context<Self>) {
        let Some(spec) = self.active_agent_spec() else {
            self.append_agent_update_line("[agent] no selected agent.");
            cx.notify();
            return;
        };
        let Some(auth_cmd) = spec.auth else {
            self.append_agent_update_line("[agent] this agent has no auth command configured.");
            cx.notify();
            return;
        };

        if self.pty.is_none() {
            self.start_terminal_with_path(cx, None);
        }
        self.input_mode = InputMode::Terminal;
        self.agent_needs_auth = false;
        let command = Self::format_shell_command(&auth_cmd);
        self.run_command(command, cx);
    }

    fn on_authenticate_agent(
        &mut self,
        _event: &MouseDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.run_agent_auth_flow(cx);
    }

    fn ensure_agent_client(&mut self) -> Result<Arc<Mutex<AcpClient>>, String> {
        let row = self
            .active_agent_row()
            .cloned()
            .ok_or_else(|| "no effective ACP agent is available".to_string())?;
        let spec = row.spec.clone();
        if !spec.is_available() {
            return Err(format!(
                "agent '{}' command '{}' not found in PATH. Open Settings > ACP Registry and click Install.",
                spec.name, spec.command
            ));
        }

        let recreate = self.agent_client.is_none()
            || self
                .agent_client_key
                .as_ref()
                .map(|id| id != &row.agent_key)
                .unwrap_or(true);
        if recreate {
            let client = AcpClient::connect(&spec).map_err(|err| {
                format!(
                    "failed to spawn agent command '{}'. Check agents.json (Windows often needs `.cmd` shim like `codex.cmd`). Details: {err}",
                    spec.command
                )
            })?;
            self.agent_client = Some(Arc::new(Mutex::new(client)));
            self.agent_client_key = Some(row.agent_key.clone());
        }
        self.agent_client
            .as_ref()
            .cloned()
            .ok_or_else(|| "failed to initialize agent client".to_string())
    }

    fn append_agent_update_line(&mut self, text: &str) {
        self.append_agent_update(text, false);
    }

    fn append_agent_update(&mut self, text: &str, append_to_last: bool) {
        let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
        if self.blocks.is_empty() {
            self.blocks.push(Block {
                command: String::new(),
                output_lines: Vec::new(),
                has_error: false,
                context: None,
            });
        }
        if let Some(block) = self.blocks.last_mut() {
            if append_to_last && !normalized.contains('\n') {
                if let Some(last) = block.output_lines.last_mut() {
                    last.push_str(&normalized);
                } else if !normalized.is_empty() {
                    if is_error_line(normalized.trim()) {
                        block.has_error = true;
                    }
                    block.output_lines.push(normalized.clone());
                    self.total_output_lines += 1;
                }
                self.trim_output_lines();
                if self.follow_output {
                    self.scroll_handle.scroll_to_bottom();
                }
                return;
            }

            for line in normalized.lines() {
                let trimmed = line.trim_end();
                if trimmed.is_empty() {
                    continue;
                }
                if is_error_line(trimmed) {
                    block.has_error = true;
                }
                block.output_lines.push(trimmed.to_string());
                self.total_output_lines += 1;
            }
        }
        self.trim_output_lines();
        if self.follow_output {
            self.scroll_handle.scroll_to_bottom();
        }
    }

    fn run_agent_prompt(&mut self, prompt: String, cx: &mut Context<Self>) {
        if self.agent_busy {
            return;
        }
        let client = match self.ensure_agent_client() {
            Ok(client) => client,
            Err(err) => {
                self.agent_needs_auth = Self::is_auth_related_error(&err);
                self.blocks.push(Block {
                    command: format!("agent> {prompt}"),
                    output_lines: vec![format!("[agent] {err}")],
                    has_error: true,
                    context: Some(BlockContext {
                        cwd: self.current_path.clone(),
                        git_branch: self.git_status.as_ref().map(|g| g.branch.clone()),
                        git_files: self.git_status.as_ref().map(|g| g.files_changed),
                        git_added: self.git_status.as_ref().map(|g| g.added),
                        git_deleted: self.git_status.as_ref().map(|g| g.deleted),
                        git_modified: self.git_status.as_ref().map(|g| g.modified),
                    }),
                });
                self.selected_block = self.blocks.len().checked_sub(1);
                self.clear_output_selection();
                self.total_output_lines += 1;
                self.trim_output_lines();
                cx.notify();
                return;
            }
        };

        let agent_label = self
            .active_agent_name()
            .unwrap_or_else(|| "Agent".to_string());
        self.push_history(&prompt);
        self.blocks.push(Block {
            command: format!("{agent_label}> {prompt}"),
            output_lines: vec![],
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
        self.selected_block = self.blocks.len().checked_sub(1);
        self.clear_output_selection();
        self.follow_output = true;
        self.agent_busy = true;
        self.agent_needs_auth = false;
        self.input.clear();
        self.cursor = 0;
        self.history_open = false;
        self.history_items.clear();
        self.suggestions.clear();
        self.suggest_index = 0;
        self.clear_selection();
        self.overlay = None;
        self.scroll_handle.scroll_to_bottom();

        let (tx, mut rx) = mpsc::unbounded::<AgentPromptEvent>();
        thread::spawn(move || {
            let result = (|| -> Result<Option<String>, String> {
                let mut guard = client
                    .lock()
                    .map_err(|_| "agent lock poisoned".to_string())?;
                if guard.protocol_version.is_none() {
                    guard.initialize().map_err(|err| err.to_string())?;
                }
                let cwd = std::env::current_dir()
                    .map_err(|err| err.to_string())?
                    .to_string_lossy()
                    .to_string();
                let runtime_mcp = crate::mcp::probe::load_enabled_runtime_mcp_servers();
                let session_id = guard
                    .ensure_session(&cwd, &runtime_mcp)
                    .map_err(|err| err.to_string())?;
                let mut on_update = |text: String, append: bool| {
                    let _ = tx.unbounded_send(AgentPromptEvent::Update { text, append });
                };
                guard
                    .prompt(&session_id, &prompt, &mut on_update)
                    .map_err(|err| err.to_string())
            })();
            let _ = tx.unbounded_send(AgentPromptEvent::Done(result));
        });

        cx.spawn(|view: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                while let Some(event) = rx.next().await {
                    if view
                        .update(&mut cx, |view, cx| {
                            match event {
                                AgentPromptEvent::Update { text, append } => {
                                    view.append_agent_update(&text, append)
                                }
                                AgentPromptEvent::Done(result) => {
                                    view.agent_busy = false;
                                    match result {
                                        Ok(Some(final_text)) => {
                                            view.agent_needs_auth =
                                                Self::is_auth_related_error(&final_text);
                                            view.append_agent_update_line(&final_text);
                                        }
                                        Ok(None) => {}
                                        Err(err) => {
                                            view.agent_needs_auth =
                                                Self::is_auth_related_error(&err);
                                            view.append_agent_update_line(&format!(
                                                "[agent] {err}"
                                            ));
                                        }
                                    }
                                }
                            }
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

    fn cancel_agent_prompt(&mut self) {
        if !self.agent_busy {
            return;
        }
        self.agent_busy = false;
        self.append_agent_update_line("[agent] cancel requested");
        let Some(client) = self.agent_client.as_ref().cloned() else {
            return;
        };
        thread::spawn(move || {
            let Ok(client) = client.lock() else {
                return;
            };
            if let Some(session_id) = client.session_id.as_ref() {
                let _ = client.cancel(session_id);
            }
        });
    }

    fn commit_input(&mut self, cx: &mut Context<Self>) {
        let command = self.input.trim().to_string();
        if command.is_empty() {
            if self.input_mode == InputMode::Terminal {
                if let Some(ref mut pty) = self.pty {
                    let _ = pty.write(b"\r\n");
                }
            }
            self.input.clear();
            self.cursor = 0;
            self.clear_selection();
            cx.notify();
            return;
        }

        if self.input_mode == InputMode::Agent {
            self.run_agent_prompt(command, cx);
        } else {
            self.run_command(command, cx);
        }
    }

    fn run_command(&mut self, command: String, cx: &mut Context<Self>) {
        let command = command.trim().to_string();
        if command.is_empty() {
            return;
        }

        self.follow_output = true;
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
        self.selected_block = self.blocks.len().checked_sub(1);
        self.clear_output_selection();
        self.last_line_incomplete = false;

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
                .child(lucide_icon(icon, 14.0, 0x9a9a9a).cursor(CursorStyle::PointingHand))
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
                .child(
                    lucide_icon(Icon::GitBranch, 14.0, 0x9a9a9a).cursor(CursorStyle::PointingHand),
                )
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
        TextEditState::split_at_cursor(&self.input, self.cursor)
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
        TextEditState::has_selection(self.selection)
    }

    fn normalized_selection(&self) -> Option<(usize, usize)> {
        TextEditState::normalized_selection(self.selection)
    }

    fn clear_selection(&mut self) {
        TextEditState::clear_selection(&mut self.selection, &mut self.selection_anchor);
    }

    fn set_selection_from_anchor(&mut self, anchor: usize, cursor: usize) {
        TextEditState::set_selection_from_anchor(
            &mut self.selection,
            &mut self.selection_anchor,
            anchor,
            cursor,
        );
    }

    fn delete_selection_if_any(&mut self) -> bool {
        TextEditState::delete_selection_if_any(
            &mut self.input,
            &mut self.cursor,
            &mut self.selection,
            &mut self.selection_anchor,
        )
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
        TextEditState::insert_text(
            &mut self.input,
            &mut self.cursor,
            &mut self.selection,
            &mut self.selection_anchor,
            text,
        );
    }

    fn pop_char_before_cursor(&mut self) {
        TextEditState::pop_char_before_cursor(
            &mut self.input,
            &mut self.cursor,
            &mut self.selection,
            &mut self.selection_anchor,
        );
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
        TextEditState::select_all(
            &self.input,
            &mut self.cursor,
            &mut self.selection,
            &mut self.selection_anchor,
        );
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

    fn load_zsh_history(path: &Path, history: &mut VecDeque<String>, seen: &mut HashSet<String>) {
        let Ok(contents) = std::fs::read_to_string(path) else {
            return;
        };
        load_zsh_history_contents(&contents, history, seen);
    }

    fn load_fish_history(path: &Path, history: &mut VecDeque<String>, seen: &mut HashSet<String>) {
        let Ok(contents) = std::fs::read_to_string(path) else {
            return;
        };
        load_fish_history_contents(&contents, history, seen);
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
        path: &Path,
        history: &mut VecDeque<String>,
        seen: &mut HashSet<String>,
    ) {
        Self::load_history_from_file_with_reader(path, history, seen, |p| {
            std::fs::read_to_string(p)
        });
    }

    fn load_history_from_file_with_reader<F>(
        path: &Path,
        history: &mut VecDeque<String>,
        seen: &mut HashSet<String>,
        read_to_string: F,
    ) where
        F: for<'a> Fn(&'a Path) -> std::io::Result<String>,
    {
        let Ok(contents) = read_to_string(path) else {
            return;
        };
        load_history_from_contents(&contents, history, seen);
    }

    fn append_history_line(path: &Path, command: &str) -> std::io::Result<()> {
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

        let (base, partial, sep) = split_path_token(&token);
        let base_dir = if base.is_empty() {
            PathBuf::from(".")
        } else {
            expand_tilde(&base)
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
            });
        }
    }

    fn ensure_output_block(&mut self) -> &mut Block {
        if self.blocks.is_empty() {
            self.blocks.push(Block {
                command: String::new(),
                output_lines: Vec::new(),
                has_error: false,
                context: None,
            });
            self.selected_block = Some(0);
        }
        if self.selected_block.is_none() {
            self.selected_block = self.blocks.len().checked_sub(1);
        }
        self.blocks.last_mut().expect("blocks is not empty")
    }

    fn append_output(&mut self, chunk: &str, cx: &mut Context<Self>) {
        let normalized = strip_ansi(chunk).replace("\r\n", "\n").replace('\r', "\n");
        if normalized.is_empty() {
            return;
        }

        let mut lines: Vec<&str> = normalized.split('\n').collect();
        let ends_with_newline = normalized.ends_with('\n');
        if ends_with_newline {
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

        let mut appended_any = false;
        let mut last_line_appended = false;
        for (index, line) in lines.iter().enumerate() {
            let append_to_last = index == 0 && self.last_line_incomplete;
            let appended = self.append_output_line(line, append_to_last);
            if appended {
                appended_any = true;
                last_line_appended = true;
            }
        }

        self.last_line_incomplete = !ends_with_newline && last_line_appended;

        if appended_any {
            self.trim_output_lines();
            self.update_follow_output_from_scroll();
            if self.follow_output {
                self.scroll_handle.scroll_to_bottom();
            }
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
        let cwd = expand_tilde(&self.current_path);
        self.git_status = get_git_status(&cwd);
    }

    fn append_output_line(&mut self, line: &str, append_to_last: bool) -> bool {
        if self.should_skip_output_line(line) {
            if append_to_last {
                if let Some(block) = self.blocks.last_mut() {
                    if !block.output_lines.is_empty() {
                        block.output_lines.pop();
                        self.total_output_lines = self.total_output_lines.saturating_sub(1);
                    }
                }
            }
            return false;
        }
        let block = self.ensure_output_block();
        if is_error_line(line) {
            block.has_error = true;
        }
        if append_to_last {
            if let Some(last) = block.output_lines.last_mut() {
                last.push_str(line);
                return true;
            }
        }
        block.output_lines.push(line.to_string());
        self.total_output_lines += 1;
        true
    }

    fn trim_output_lines(&mut self) {
        if self.total_output_lines <= MAX_OUTPUT_LINES {
            return;
        }

        let mut to_remove = self.total_output_lines - MAX_OUTPUT_LINES;
        while to_remove > 0 && !self.blocks.is_empty() {
            if self.blocks[0].output_lines.is_empty() {
                self.blocks.remove(0);
                self.shift_output_indices_after_front_block_removal();
                continue;
            }
            let remove_count = to_remove.min(self.blocks[0].output_lines.len());
            self.blocks[0].output_lines.drain(0..remove_count);
            self.total_output_lines -= remove_count;
            to_remove -= remove_count;
            if self.blocks[0].output_lines.is_empty() {
                self.blocks.remove(0);
                self.shift_output_indices_after_front_block_removal();
            }
        }
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

    fn render_output_line(
        &self,
        line: &str,
        has_error: bool,
        block_index: usize,
        line_index: usize,
        cx: &Context<Self>,
    ) -> Div {
        let color = if has_error && is_error_line(line) {
            rgb(0xff7b72)
        } else if Self::is_dir_header_line(line) {
            rgb(0x8bd06f)
        } else {
            rgb(0xdddddd)
        };

        let is_selected = self.is_output_line_selected(block_index, line_index);
        let mut row = div()
            .min_w(px(0.0))
            .text_color(color)
            .truncate()
            .cursor(CursorStyle::IBeam)
            .px(px(2.0))
            .child(line.to_string())
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |view, event: &MouseDownEvent, _window, cx| {
                    view.on_output_line_mouse_down_at(block_index, line_index, event, cx);
                }),
            )
            .on_mouse_move(
                cx.listener(move |view, event: &MouseMoveEvent, _window, cx| {
                    view.on_output_line_mouse_move_at(block_index, line_index, event, cx);
                }),
            );

        if is_selected {
            row = row.bg(rgb(0x1a2f4a)).rounded(px(4.0));
        }

        row
    }

    fn render_block(
        &self,
        block: &Block,
        index: usize,
        active_index: usize,
        cx: &Context<Self>,
    ) -> Div {
        let has_command = !block.command.is_empty();
        let is_active = index == active_index && has_command;
        let is_selected_block = self.selected_block == Some(index);
        let block_bg = if block.has_error {
            rgb(0x2a1515)
        } else if is_selected_block {
            rgb(0x11283d)
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

        let context_line = if let Some(line) = self.block_context_text(block) {
            div()
                .min_w(px(0.0))
                .text_size(px(11.0))
                .text_color(rgb(0x7a7a7a))
                .truncate()
                .child(line)
        } else {
            div()
        };

        let header = if has_command {
            div()
                .min_w(px(0.0))
                .text_size(px(13.0))
                .text_color(if block.has_error {
                    rgb(0xffa3a3)
                } else {
                    rgb(0xffe29a)
                })
                .font_weight(FontWeight::BOLD)
                .truncate()
                .child(block.command.clone())
        } else {
            div()
        };

        let copy_button = div()
            .flex_none()
            .px(px(6.0))
            .py(px(4.0))
            .rounded(px(5.0))
            .bg(rgb(0x141414))
            .border_1()
            .border_color(rgb(0x2a2a2a))
            .hover(|this| this.bg(rgb(0x242424)).border_color(rgb(0x4a4a4a)))
            .cursor(CursorStyle::PointingHand)
            .child(lucide_icon(Icon::Clipboard, 12.0, 0xb8b8b8))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |view, _event: &MouseDownEvent, _window, cx| {
                    view.copy_block_at(index, cx);
                    cx.stop_propagation();
                }),
            );

        let output = if block.output_lines.is_empty() {
            div()
        } else {
            div().flex_col().gap(px(2.0)).text_size(px(12.0)).children(
                block
                    .output_lines
                    .iter()
                    .enumerate()
                    .map(|(line_index, line)| {
                        self.render_output_line(line, block.has_error, index, line_index, cx)
                    }),
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
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |view, _event: &MouseDownEvent, _window, cx| {
                    view.on_select_block(index, cx);
                }),
            )
            .child(div().w(px(3.0)).bg(accent_color))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .flex_col()
                    .gap(px(6.0))
                    .child(context_line)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap(px(8.0))
                            .child(div().flex_1().min_w(px(0.0)).child(header))
                            .child(copy_button),
                    )
                    .child(output),
            )
    }
}

impl Render for TooltipView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .px(px(8.0))
            .py(px(6.0))
            .rounded(px(6.0))
            .bg(rgb(0x1a1a1a))
            .border_1()
            .border_color(rgb(0x2a2a2a))
            .text_size(px(11.0))
            .text_color(rgb(0xdddddd))
            .child(self.text.clone())
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
                        .relative()
                        .child(
                            div()
                                .p(px(16.0))
                                .id("terminal_output")
                                .track_scroll(&self.scroll_handle)
                                .on_scroll_wheel(cx.listener(Self::on_output_scroll_wheel))
                                .on_mouse_up(
                                    MouseButton::Left,
                                    cx.listener(Self::on_output_mouse_up),
                                )
                                .on_mouse_up_out(
                                    MouseButton::Left,
                                    cx.listener(Self::on_output_mouse_up),
                                )
                                .overflow_scroll()
                                .scrollbar_width(px(16.0))
                                .size_full()
                                .font_family("Cascadia Code")
                                .text_size(px(13.0))
                                .text_color(rgb(0xcccccc))
                                .child({
                                    let active_index = self.blocks.len().saturating_sub(1);
                                    let blocks: Vec<Div> = self
                                        .blocks
                                        .iter()
                                        .enumerate()
                                        .map(|(i, block)| {
                                            self.render_block(block, i, active_index, cx)
                                        })
                                        .collect();
                                    div()
                                        .flex_col()
                                        .gap(px(0.0))
                                        .min_h(px(0.0))
                                        .children(blocks)
                                }),
                        )
                        .child(if self.follow_output {
                            div()
                        } else {
                            self.render_jump_to_bottom(cx)
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
        } else if let TabViewMode::Agent(ref agent) = self.mode {
            root = root.child(div().flex_1().min_h(px(0.0)).child(agent.clone()));
        } else if let TabViewMode::Settings(ref settings) = self.mode {
            root = root.child(div().flex_1().min_h(px(0.0)).child(settings.clone()));
        }

        root
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

fn load_history_from_contents(
    contents: &str,
    history: &mut VecDeque<String>,
    seen: &mut HashSet<String>,
) {
    let lines: Vec<&str> = contents.lines().filter(|l| !l.trim().is_empty()).collect();
    for line in lines.into_iter().rev() {
        if seen.insert(line.to_string()) {
            history.push_front(line.to_string());
        }
    }
}

fn load_zsh_history_contents(
    contents: &str,
    history: &mut VecDeque<String>,
    seen: &mut HashSet<String>,
) {
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

fn load_fish_history_contents(
    contents: &str,
    history: &mut VecDeque<String>,
    seen: &mut HashSet<String>,
) {
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
