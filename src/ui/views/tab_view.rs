use crate::git::get_git_branches;
use crate::git::get_git_status;
use crate::terminal::TerminalPty;
use crate::{
    acp::client::{
        AcpClient, AcpResponseText, PermissionDecision, PermissionOption, PermissionRequest,
    },
    acp::manager::AgentCommandSpec,
    acp::model_discovery::{self, AcpModelOption},
    acp::registry::fetch::load_cached_registry,
    acp::registry::model::RegistryManifest,
    acp::resolve::{AgentKey, ConflictPolicy, EffectiveAgentRow, load_effective_agent_rows},
    acp::runtime_prefs::RuntimePreferences,
    acp::storage,
};
use anyhow::anyhow;
use futures::StreamExt;
use futures::channel::mpsc;
use gpui::*;
use lucide_icons::Icon;
use std::collections::{BTreeMap, HashSet, VecDeque};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crate::ui::icons::{lucide_icon, registry_avatar};
use crate::ui::recent::RecentEntry;
use crate::ui::text_edit::TextEditState;
use crate::ui::views::agent_view::AgentView;
use crate::ui::views::settings_view::SettingsView;
use crate::ui::views::welcome_view::{
    CloneRepositoryEvent, CreateProjectEvent, OpenRepositoryEvent, StartBaseTerminalEvent,
    WelcomeView,
};

const DEFAULT_PREVIEW_TERMINAL_HEIGHT: f32 = 260.0;
const MIN_PREVIEW_TERMINAL_HEIGHT: f32 = 180.0;
const MAX_PREVIEW_TERMINAL_HEIGHT: f32 = 520.0;
const PREVIEW_TERMINAL_RESIZE_HANDLE_HEIGHT: f32 = 6.0;

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
    preview_scroll_handle: ScrollHandle,
    input_visible: bool,
    overlay: Option<Overlay>,
    needs_git_refresh: bool,
    mode: TabViewMode,
    last_line_incomplete: bool,
    total_output_lines: usize,
    follow_output: bool,
    last_scroll_to_bottom_at: Instant,
    input_mode: InputMode,
    agent_rows: Vec<EffectiveAgentRow>,
    agent_selected_key: Option<AgentKey>,
    agent_client: Option<Arc<Mutex<AcpClient>>>,
    agent_client_key: Option<AgentKey>,
    agent_busy: bool,
    agent_needs_auth: bool,
    runtime_preferences: Arc<Mutex<RuntimePreferences>>,
    registry_manifests: BTreeMap<String, RegistryManifest>,
    discovered_models: Vec<AcpModelOption>,
    model_options_loading: bool,
    selected_model_override: Option<String>,
    selected_block: Option<usize>,
    output_selection_anchor: Option<(usize, usize)>,
    output_selection_head: Option<(usize, usize)>,
    output_selecting: bool,
    file_preview: Option<FilePreviewState>,
    preview_search_match: Option<PreviewSearchMatch>,
    preview_code_scroll_handle: UniformListScrollHandle,
    preview_focus: bool,
    preview_selection_anchor: Option<usize>,
    preview_selection_head: Option<usize>,
    preview_selecting: bool,
    preview_active_line: Option<usize>,
    file_preview_mode: FilePreviewMode,
    preview_terminal_height: f32,
    preview_terminal_resize_dragging: bool,
    preview_terminal_resize_start_y: Option<f32>,
    preview_terminal_resize_start_height: f32,
}

#[derive(Clone)]
struct Block {
    command: String,
    output_lines: Vec<String>,
    has_error: bool,
    context: Option<BlockContext>,
    agent_placeholder_active: bool,
    pending_permission: Option<AgentPermissionPrompt>,
    agent_stream_text: String,
    agent_stream_line_index: Option<usize>,
    agent_response: Option<AcpResponseText>,
    agent_response_line_count: usize,
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
    query: PickerQueryState,
    entries: Vec<PathEntry>,
    selected: usize,
}

struct PathEntry {
    name: String,
    path: PathBuf,
    is_dir: bool,
}

struct BranchPickerState {
    query: PickerQueryState,
    all_branches: Vec<String>,
    branches: Vec<String>,
    selected: usize,
}

struct AgentPickerState {
    all_options: Vec<EffectiveAgentRow>,
    query: PickerQueryState,
    options: Vec<EffectiveAgentRow>,
    selected: usize,
}

struct ModelPickerState {
    all_options: Vec<AcpModelOption>,
    query: PickerQueryState,
    options: Vec<AcpModelOption>,
    selected: usize,
}

#[derive(Clone, Default)]
struct PickerQueryState {
    text: String,
    cursor: usize,
    selection: Option<(usize, usize)>,
    anchor: Option<usize>,
}

struct TooltipView {
    text: String,
}

fn build_agent_picker_state(
    agent_rows: &[EffectiveAgentRow],
    selected_key: Option<&AgentKey>,
) -> Option<AgentPickerState> {
    if agent_rows.is_empty() {
        return None;
    }

    let selected = selected_key
        .and_then(|key| agent_rows.iter().position(|row| row.agent_key == *key))
        .unwrap_or(0);

    Some(AgentPickerState {
        all_options: agent_rows.to_vec(),
        query: PickerQueryState::default(),
        options: agent_rows.to_vec(),
        selected,
    })
}

fn build_model_picker_state(
    catalog: &[AcpModelOption],
    selected_model_id: Option<&str>,
) -> Option<ModelPickerState> {
    if catalog.is_empty() {
        return None;
    }

    let selected = selected_model_id
        .and_then(|id| catalog.iter().position(|model| model.id == id))
        .or_else(|| catalog.iter().position(|model| model.is_default))
        .unwrap_or(0);

    Some(ModelPickerState {
        all_options: catalog.to_vec(),
        query: PickerQueryState::default(),
        options: catalog.to_vec(),
        selected,
    })
}

enum Overlay {
    Path(PathPickerState),
    Branch(BranchPickerState),
    Agent(AgentPickerState),
    #[allow(dead_code)]
    Model(ModelPickerState),
}

#[derive(Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
enum PickerKind {
    Path,
    Branch,
    Agent,
    Model,
}

#[derive(Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
enum PickerMode {
    ImmediateSearch,
    ConditionalSearch,
}

#[allow(dead_code)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum InitialFocusTarget {
    SearchInput,
    List,
}

#[allow(dead_code)]
fn picker_has_search_input(kind: PickerKind, option_count: usize) -> bool {
    matches!(kind, PickerKind::Path | PickerKind::Branch) || option_count >= 6
}

#[allow(dead_code)]
fn picker_typeahead_enabled(kind: PickerKind, option_count: usize) -> bool {
    matches!(kind, PickerKind::Agent | PickerKind::Model) && option_count <= 5
}

#[allow(dead_code)]
fn picker_header_is_static(kind: PickerKind, option_count: usize) -> bool {
    matches!(kind, PickerKind::Agent | PickerKind::Model) && option_count <= 5
}

#[allow(dead_code)]
fn picker_initial_focus_target(kind: PickerKind) -> InitialFocusTarget {
    match kind {
        PickerKind::Path | PickerKind::Branch => InitialFocusTarget::SearchInput,
        PickerKind::Agent | PickerKind::Model => InitialFocusTarget::List,
    }
}

fn clickable_cursor() -> CursorStyle {
    CursorStyle::PointingHand
}

fn text_input_cursor() -> CursorStyle {
    CursorStyle::IBeam
}

#[allow(dead_code)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct RowState {
    is_selected: bool,
    is_highlighted: bool,
    is_disabled: bool,
}

#[allow(dead_code)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct TriggerState {
    expanded: bool,
    disabled: bool,
    loading: bool,
    has_search_input: bool,
}

#[allow(dead_code)]
fn compute_row_state(selected: bool, highlighted: bool, disabled: bool) -> RowState {
    RowState {
        is_selected: selected,
        is_highlighted: highlighted,
        is_disabled: disabled,
    }
}

#[allow(dead_code)]
fn compute_trigger_state(
    expanded: bool,
    disabled: bool,
    loading: bool,
    has_search_input: bool,
) -> TriggerState {
    TriggerState {
        expanded,
        disabled,
        loading,
        has_search_input,
    }
}

impl PickerQueryState {
    fn normalized_selection(&self) -> Option<(usize, usize)> {
        TextEditState::normalized_selection(self.selection)
    }

    fn has_selection(&self) -> bool {
        TextEditState::has_selection(self.selection)
    }

    fn clear_selection(&mut self) {
        TextEditState::clear_selection(&mut self.selection, &mut self.anchor);
    }

    fn set_selection_from_anchor(&mut self, anchor: usize, cursor: usize) {
        TextEditState::set_selection_from_anchor(
            &mut self.selection,
            &mut self.anchor,
            anchor,
            cursor,
        );
    }

    fn select_all(&mut self) {
        if self.text.is_empty() {
            return;
        }
        TextEditState::select_all(
            &self.text,
            &mut self.cursor,
            &mut self.selection,
            &mut self.anchor,
        );
    }

    fn insert_text(&mut self, text: &str) {
        TextEditState::insert_text(
            &mut self.text,
            &mut self.cursor,
            &mut self.selection,
            &mut self.anchor,
            text,
        );
    }

    fn pop_char_before_cursor(&mut self) {
        if self.has_selection() {
            let _ = TextEditState::delete_selection_if_any(
                &mut self.text,
                &mut self.cursor,
                &mut self.selection,
                &mut self.anchor,
            );
            return;
        }
        TextEditState::pop_char_before_cursor(
            &mut self.text,
            &mut self.cursor,
            &mut self.selection,
            &mut self.anchor,
        );
    }

    fn delete_char_after_cursor(&mut self) {
        if self.has_selection() {
            let _ = TextEditState::delete_selection_if_any(
                &mut self.text,
                &mut self.cursor,
                &mut self.selection,
                &mut self.anchor,
            );
            return;
        }

        let max = self.text.chars().count();
        if self.cursor >= max {
            return;
        }

        let mut out = String::new();
        for (i, ch) in self.text.chars().enumerate() {
            if i != self.cursor {
                out.push(ch);
            }
        }
        self.text = out;
        self.clear_selection();
    }

    fn move_left(&mut self, shift: bool) {
        if shift {
            let new_cursor = self.cursor.saturating_sub(1);
            let anchor = self.anchor.unwrap_or(self.cursor);
            self.cursor = new_cursor;
            self.set_selection_from_anchor(anchor, self.cursor);
            return;
        }

        if let Some((a, _)) = self.normalized_selection() {
            self.cursor = a;
            self.clear_selection();
            return;
        }

        self.cursor = self.cursor.saturating_sub(1);
    }

    fn move_right(&mut self, shift: bool) {
        let max = self.text.chars().count();
        if shift {
            let new_cursor = (self.cursor + 1).min(max);
            let anchor = self.anchor.unwrap_or(self.cursor);
            self.cursor = new_cursor;
            self.set_selection_from_anchor(anchor, self.cursor);
            return;
        }

        if let Some((_, b)) = self.normalized_selection() {
            self.cursor = b;
            self.clear_selection();
            return;
        }

        self.cursor = (self.cursor + 1).min(max);
    }

    fn move_home(&mut self, shift: bool) {
        if shift {
            let anchor = self.anchor.unwrap_or(self.cursor);
            self.cursor = 0;
            self.set_selection_from_anchor(anchor, self.cursor);
        } else {
            self.cursor = 0;
            self.clear_selection();
        }
    }

    fn move_end(&mut self, shift: bool) {
        let end = self.text.chars().count();
        if shift {
            let anchor = self.anchor.unwrap_or(self.cursor);
            self.cursor = end;
            self.set_selection_from_anchor(anchor, self.cursor);
        } else {
            self.cursor = end;
            self.clear_selection();
        }
    }
}

#[derive(Clone, Debug)]
struct SuggestionItem {
    display: String,
    insert: String,
}

pub enum TabViewEvent {
    CwdChanged(PathBuf),
    OpenRepository(PathBuf),
    StartBaseTerminal { command: String },
    CreateProject { prompt: String, parent: PathBuf },
    CloneRepository { url: String, parent: PathBuf },
}

const MAX_OUTPUT_LINES: usize = 5000;
const MAX_RENDERED_OUTPUT_LINES_PER_BLOCK: usize = 400;

enum TabViewMode {
    Terminal,
    Agent(Entity<AgentView>),
    Welcome(Entity<WelcomeView>),
    Settings(Entity<SettingsView>),
}

#[derive(Clone)]
struct FilePreviewState {
    path: PathBuf,
    relative_label: String,
    language: PreviewLanguage,
    kind: FilePreviewKind,
}

#[derive(Clone)]
enum FilePreviewKind {
    Text {
        contents: String,
        lines: Arc<Vec<String>>,
    },
    Image,
    Unsupported {
        message: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PreviewLanguage {
    Rust,
    Markdown,
    Json,
    Toml,
    Yaml,
    Shell,
    JavaScript,
    JavaScriptReact,
    TypeScript,
    TypeScriptReact,
    Html,
    Css,
    Sql,
    Python,
    Go,
    Dockerfile,
    PlainText,
}

#[derive(Clone)]
struct HighlightSegment {
    text: String,
    color: u32,
    bold: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PreviewSearchMatch {
    line_index: usize,
    query: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PreviewSearchMatchSegment {
    text: String,
    color: u32,
    bold: bool,
    matched: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FilePreviewMode {
    Code,
    Preview,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum InputMode {
    Terminal,
    Agent,
}

enum AgentPromptEvent {
    Status(String),
    ClientReady {
        client: Arc<Mutex<AcpClient>>,
        agent_key: AgentKey,
    },
    PermissionRequest(AgentPermissionEvent),
    Update {
        text: String,
        append: bool,
    },
    Models(Vec<AcpModelOption>),
    Done(Result<Option<AcpResponseText>, String>),
}

enum ModelButtonState {
    Loading,
    Ready(String),
    Unavailable,
}

#[derive(Clone)]
struct AgentPermissionPrompt {
    request: PermissionRequest,
    response_tx: std::sync::mpsc::Sender<PermissionDecision>,
}

#[derive(Clone)]
struct AgentPermissionEvent {
    request: PermissionRequest,
    response_tx: std::sync::mpsc::Sender<PermissionDecision>,
}

const AGENT_CONNECTING_PLACEHOLDER: &str = "[agent] connecting...";
const AGENT_STARTING_SESSION_PLACEHOLDER: &str = "[agent] starting session...";
const AGENT_SENDING_PROMPT_PLACEHOLDER: &str = "[agent] sending prompt...";
const MAX_TEXT_PREVIEW_BYTES: usize = 512 * 1024;

fn model_trigger_label(state: &ModelButtonState) -> (String, bool) {
    match state {
        ModelButtonState::Loading => ("Loading models...".into(), true),
        ModelButtonState::Ready(label) => (label.clone(), false),
        ModelButtonState::Unavailable => ("No models available".into(), true),
    }
}

fn update_agent_placeholder_block(block: &mut Block, text: &str) -> bool {
    if !block.agent_placeholder_active {
        return false;
    }

    if let Some(last) = block.output_lines.last_mut() {
        *last = text.to_string();
        if is_error_line(text.trim()) {
            block.has_error = true;
        }
        true
    } else {
        false
    }
}

#[cfg(test)]
fn streaming_snapshot_delta(previous: &str, next: &str) -> Option<String> {
    if next == previous {
        return None;
    }
    if let Some(rest) = next.strip_prefix(previous) {
        return Some(rest.to_string());
    }
    Some(next.to_string())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AgentStreamOp {
    Append,
    Replace,
    NewLines,
    Ignore,
}

fn is_likely_stream_snapshot(text: &str) -> bool {
    let trimmed = text.trim_start();
    let lower = trimmed.to_ascii_lowercase();

    trimmed.starts_with("```")
        || lower.starts_with("directory:")
        || lower.starts_with("mode")
        || lower.contains("\ndirectory:")
        || lower.contains("\nmode")
        || (text.contains('\n') && text.ends_with('\n'))
}

fn classify_agent_stream_op(previous: &str, next: &str, append_to_last: bool) -> AgentStreamOp {
    if next == previous {
        return AgentStreamOp::Ignore;
    }

    if append_to_last {
        if !previous.is_empty() && (next.starts_with(previous) || is_likely_stream_snapshot(next)) {
            return AgentStreamOp::Replace;
        }

        return AgentStreamOp::Append;
    }

    if !previous.is_empty() && (next.starts_with(previous) || is_likely_stream_snapshot(next)) {
        return AgentStreamOp::Replace;
    }

    AgentStreamOp::NewLines
}

fn agent_stream_rendered_lines(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim_end)
        .filter(|line| !line.is_empty())
        .map(ToString::to_string)
        .collect()
}

fn append_agent_stream_delta(block: &mut Block, delta: &str, total_output_lines: &mut usize) {
    if delta.is_empty() {
        return;
    }

    let mut segments: Vec<&str> = delta.split('\n').collect();
    let ends_with_newline = delta.ends_with('\n');
    if ends_with_newline && matches!(segments.last(), Some(&"")) {
        segments.pop();
    }

    let mut insert_index = block.agent_stream_line_index;
    for (index, segment) in segments.iter().enumerate() {
        if index == 0 {
            let line_index = insert_index.unwrap_or_else(|| {
                block.output_lines.push(String::new());
                let line_index = block.output_lines.len() - 1;
                *total_output_lines += 1;
                line_index
            });
            if let Some(last) = block.output_lines.get_mut(line_index) {
                last.push_str(segment);
                if is_error_line(last.trim()) {
                    block.has_error = true;
                }
                block.agent_stream_line_index = Some(line_index);
                insert_index = Some(line_index);
                continue;
            }
        }

        let trimmed = segment.trim_end();
        if trimmed.is_empty() {
            continue;
        }
        if is_error_line(trimmed) {
            block.has_error = true;
        }
        let insert_at = insert_index
            .map(|index| index + 1)
            .unwrap_or(block.output_lines.len());
        block.output_lines.insert(insert_at, trimmed.to_string());
        *total_output_lines += 1;
        insert_index = Some(insert_at);
        block.agent_stream_line_index = insert_index;
    }
}

fn replace_agent_stream_snapshot(
    block: &mut Block,
    snapshot: &str,
    total_output_lines: &mut usize,
) {
    let previous_count = agent_stream_rendered_lines(&block.agent_stream_text).len();
    let mut insert_at = block.output_lines.len();

    if let Some(end_index) = block.agent_stream_line_index
        && previous_count > 0
    {
        let start_index = end_index
            .saturating_add(1)
            .saturating_sub(previous_count)
            .min(block.output_lines.len());
        let drain_end = end_index.min(block.output_lines.len().saturating_sub(1));
        if start_index <= drain_end && start_index < block.output_lines.len() {
            let removed = drain_end - start_index + 1;
            block.output_lines.drain(start_index..=drain_end);
            *total_output_lines = total_output_lines.saturating_sub(removed);
            insert_at = start_index;
        }
    }

    let rendered_lines = agent_stream_rendered_lines(snapshot);
    let mut last_line_index = None;

    for (offset, line) in rendered_lines.iter().enumerate() {
        if is_error_line(line) {
            block.has_error = true;
        }
        let target_index = insert_at + offset;
        block.output_lines.insert(target_index, line.clone());
        *total_output_lines += 1;
        last_line_index = Some(target_index);
    }

    block.agent_stream_text = snapshot.to_string();
    block.agent_stream_line_index = last_line_index;
}

fn append_output_batch_to_block(
    block: &mut Block,
    lines: &[String],
    append_first_to_last: bool,
) -> usize {
    if lines.is_empty() {
        return 0;
    }

    let mut added_lines = 0;

    for (index, line) in lines.iter().enumerate() {
        if is_error_line(line) {
            block.has_error = true;
        }

        if index == 0 && append_first_to_last {
            if let Some(last) = block.output_lines.last_mut() {
                last.push_str(line);
                continue;
            }
        }

        block.output_lines.push(line.clone());
        added_lines += 1;
    }

    added_lines
}

impl TabView {
    fn request_scroll_to_bottom(&mut self, _cx: &mut Context<Self>) {
        if !self.follow_output {
            return;
        }

        let now = Instant::now();
        if now.duration_since(self.last_scroll_to_bottom_at) < Duration::from_millis(16) {
            return;
        }

        self.last_scroll_to_bottom_at = now;
        self.scroll_handle.scroll_to_bottom();
    }

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

    fn clear_preview_selection(&mut self) {
        self.preview_selection_anchor = None;
        self.preview_selection_head = None;
        self.preview_selecting = false;
    }

    fn preview_supports_rendered_markdown(preview: &FilePreviewState) -> bool {
        matches!(preview.language, PreviewLanguage::Markdown)
            && matches!(preview.kind, FilePreviewKind::Text { .. })
    }

    fn clamp_preview_terminal_height(height: f32) -> f32 {
        height.clamp(MIN_PREVIEW_TERMINAL_HEIGHT, MAX_PREVIEW_TERMINAL_HEIGHT)
    }

    #[cfg(test)]
    fn preview_scroll_uses_custom_step(delta: ScrollDelta) -> bool {
        let _ = delta;
        false
    }

    fn preview_text_lines(&self) -> Option<&[String]> {
        let preview = self.file_preview.as_ref()?;
        match &preview.kind {
            FilePreviewKind::Text { lines, .. } => Some(lines.as_slice()),
            _ => None,
        }
    }

    fn preview_line_count(&self) -> usize {
        self.preview_text_lines()
            .map(|lines| lines.len())
            .unwrap_or(0)
    }

    fn normalize_preview_selection(&self) -> Option<(usize, usize)> {
        Self::normalize_linear_selection(self.preview_selection_anchor, self.preview_selection_head)
    }

    fn selected_preview_text(&self) -> Option<String> {
        let lines = self.preview_text_lines()?;
        Self::selected_text_from_lines(
            lines,
            self.preview_selection_anchor,
            self.preview_selection_head,
        )
    }

    fn focus_preview(&mut self) {
        self.preview_focus = true;
    }

    fn blur_preview(&mut self) {
        self.preview_focus = false;
        self.preview_selecting = false;
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

    fn on_preview_line_mouse_down_at(
        &mut self,
        line_index: usize,
        event: &MouseDownEvent,
        cx: &mut Context<Self>,
    ) {
        self.focus_preview();
        if self
            .preview_search_match
            .as_ref()
            .is_some_and(|search_match| search_match.line_index != line_index)
        {
            self.preview_search_match = None;
        }
        if event.modifiers.shift {
            if self.preview_selection_anchor.is_none() {
                self.preview_selection_anchor =
                    Some(self.preview_active_line.unwrap_or(line_index));
            }
            self.preview_selection_head = Some(line_index);
        } else {
            self.preview_selection_anchor = Some(line_index);
            self.preview_selection_head = Some(line_index);
        }
        self.preview_active_line = Some(line_index);
        self.preview_selecting = true;
        cx.notify();
        cx.stop_propagation();
    }

    fn on_preview_line_mouse_move_at(
        &mut self,
        line_index: usize,
        event: &MouseMoveEvent,
        cx: &mut Context<Self>,
    ) {
        if !self.preview_selecting || !event.dragging() {
            return;
        }
        self.preview_selection_head = Some(line_index);
        self.preview_active_line = Some(line_index);
        cx.notify();
    }

    fn on_preview_mouse_up(
        &mut self,
        _event: &MouseUpEvent,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
        self.preview_selecting = false;
    }

    fn on_toggle_file_preview_mode(
        &mut self,
        _event: &MouseDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_file_preview_mode(cx);
        cx.stop_propagation();
    }

    fn on_preview_terminal_resize_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let y: f32 = event.position.y.into();
        self.preview_terminal_resize_dragging = true;
        self.preview_terminal_resize_start_y = Some(y);
        self.preview_terminal_resize_start_height = self.preview_terminal_height;
        cx.notify();
        cx.stop_propagation();
    }

    fn on_preview_terminal_resize_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.preview_terminal_resize_dragging || !event.dragging() {
            return;
        }

        let Some(start_y) = self.preview_terminal_resize_start_y else {
            return;
        };

        let current_y: f32 = event.position.y.into();
        let delta = start_y - current_y;
        let next_height =
            Self::clamp_preview_terminal_height(self.preview_terminal_resize_start_height + delta);
        if (self.preview_terminal_height - next_height).abs() > f32::EPSILON {
            self.preview_terminal_height = next_height;
            cx.notify();
        }
    }

    fn on_preview_terminal_resize_mouse_up(
        &mut self,
        _event: &MouseUpEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.preview_terminal_resize_dragging {
            self.preview_terminal_resize_dragging = false;
            self.preview_terminal_resize_start_y = None;
            cx.notify();
        }
    }

    fn copy_selected_preview(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(text) = self.selected_preview_text() else {
            return false;
        };
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        true
    }

    fn select_all_preview(&mut self) -> bool {
        let count = self.preview_line_count();
        if count == 0 {
            return false;
        }
        self.preview_selection_anchor = Some(0);
        self.preview_selection_head = Some(count.saturating_sub(1));
        self.preview_active_line = Some(count.saturating_sub(1));
        true
    }

    fn build_preview_search_match(
        line_number: usize,
        query: &str,
        line_count: usize,
    ) -> Option<(usize, String)> {
        let trimmed = query.trim();
        if line_number == 0 || trimmed.is_empty() || line_count == 0 {
            return None;
        }

        Some((
            line_number.saturating_sub(1).min(line_count - 1),
            trimmed.to_string(),
        ))
    }

    fn apply_preview_search_match_segments(
        segments: &[HighlightSegment],
        query: &str,
    ) -> Vec<PreviewSearchMatchSegment> {
        let trimmed = query.trim();
        if trimmed.is_empty() {
            return segments
                .iter()
                .map(|segment| PreviewSearchMatchSegment {
                    text: segment.text.clone(),
                    color: segment.color,
                    bold: segment.bold,
                    matched: false,
                })
                .collect();
        }

        let mut rendered = Vec::new();
        let query_lower = trimmed.to_ascii_lowercase();

        for segment in segments {
            if segment.text.is_empty() {
                continue;
            }

            let text = segment.text.as_str();
            let text_lower = text.to_ascii_lowercase();
            let mut search_from = 0usize;

            while let Some(found_at) = text_lower[search_from..].find(&query_lower) {
                let match_start = search_from + found_at;
                let match_end = match_start + trimmed.len();

                if match_start > search_from {
                    rendered.push(PreviewSearchMatchSegment {
                        text: text[search_from..match_start].to_string(),
                        color: segment.color,
                        bold: segment.bold,
                        matched: false,
                    });
                }

                rendered.push(PreviewSearchMatchSegment {
                    text: text[match_start..match_end].to_string(),
                    color: segment.color,
                    bold: segment.bold,
                    matched: true,
                });
                search_from = match_end;
            }

            if search_from < text.len() {
                rendered.push(PreviewSearchMatchSegment {
                    text: text[search_from..].to_string(),
                    color: segment.color,
                    bold: segment.bold,
                    matched: false,
                });
            }
        }

        if rendered.is_empty() {
            segments
                .iter()
                .map(|segment| PreviewSearchMatchSegment {
                    text: segment.text.clone(),
                    color: segment.color,
                    bold: segment.bold,
                    matched: false,
                })
                .collect()
        } else {
            rendered
        }
    }

    fn move_preview_selection_by(&mut self, delta: isize, extend: bool) -> bool {
        let count = self.preview_line_count();
        if count == 0 {
            return false;
        }

        let (active_line, selection_anchor, selection_head) = Self::move_linear_selection(
            count,
            self.preview_active_line,
            self.preview_selection_anchor,
            self.preview_selection_head,
            delta,
            extend,
        );
        self.focus_preview();
        self.preview_active_line = active_line;
        self.preview_selection_anchor = selection_anchor;
        self.preview_selection_head = selection_head;
        true
    }

    fn move_preview_selection_to_edge(&mut self, to_end: bool, extend: bool) -> bool {
        let count = self.preview_line_count();
        if count == 0 {
            return false;
        }

        let target = if to_end { count.saturating_sub(1) } else { 0 };
        let current = self
            .preview_active_line
            .unwrap_or(target)
            .min(count.saturating_sub(1));
        self.focus_preview();
        self.preview_active_line = Some(target);
        if extend {
            if self.preview_selection_anchor.is_none() {
                self.preview_selection_anchor = Some(current);
            }
            self.preview_selection_head = Some(target);
        } else {
            self.preview_selection_anchor = Some(target);
            self.preview_selection_head = Some(target);
        }
        true
    }

    fn normalize_linear_selection(
        anchor: Option<usize>,
        head: Option<usize>,
    ) -> Option<(usize, usize)> {
        let anchor = anchor?;
        let head = head?;
        if anchor == head {
            return None;
        }
        Some((anchor.min(head), anchor.max(head)))
    }

    fn selected_text_from_lines(
        lines: &[String],
        anchor: Option<usize>,
        head: Option<usize>,
    ) -> Option<String> {
        let (start, end) = Self::normalize_linear_selection(anchor, head)?;
        if start >= lines.len() {
            return None;
        }
        let end = end.min(lines.len().saturating_sub(1));
        if start > end {
            return None;
        }
        Some(lines[start..=end].join("\n"))
    }

    fn move_linear_selection(
        count: usize,
        active_line: Option<usize>,
        selection_anchor: Option<usize>,
        selection_head: Option<usize>,
        delta: isize,
        extend: bool,
    ) -> (Option<usize>, Option<usize>, Option<usize>) {
        if count == 0 {
            return (active_line, selection_anchor, selection_head);
        }

        let current = active_line.unwrap_or(0).min(count.saturating_sub(1));
        let next = if delta < 0 {
            current.saturating_sub(delta.unsigned_abs())
        } else {
            (current + delta as usize).min(count.saturating_sub(1))
        };

        if extend {
            let anchor = selection_anchor.or(Some(current));
            (Some(next), anchor, Some(next))
        } else {
            (Some(next), Some(next), Some(next))
        }
    }

    fn handle_preview_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) -> bool {
        if !self.preview_focus || self.file_preview.is_none() {
            return false;
        }

        let ctrl = event.keystroke.modifiers.control;
        let shift = event.keystroke.modifiers.shift;
        match event.keystroke.key.as_str() {
            "c" if ctrl => {
                self.copy_selected_preview(cx);
                true
            }
            "a" if ctrl => {
                if self.select_all_preview() {
                    cx.notify();
                }
                true
            }
            "up" | "arrowup" => {
                if self.move_preview_selection_by(-1, shift) {
                    cx.notify();
                }
                true
            }
            "down" | "arrowdown" => {
                if self.move_preview_selection_by(1, shift) {
                    cx.notify();
                }
                true
            }
            "left" | "arrowleft" => {
                if self.move_preview_selection_by(-1, shift) {
                    cx.notify();
                }
                true
            }
            "right" | "arrowright" => {
                if self.move_preview_selection_by(1, shift) {
                    cx.notify();
                }
                true
            }
            "home" => {
                if self.move_preview_selection_to_edge(false, shift) {
                    cx.notify();
                }
                true
            }
            "end" => {
                if self.move_preview_selection_to_edge(true, shift) {
                    cx.notify();
                }
                true
            }
            _ => false,
        }
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
        cx.subscribe(
            &welcome,
            |_, _welcome, event: &StartBaseTerminalEvent, cx| {
                cx.emit(TabViewEvent::StartBaseTerminal {
                    command: event.command.clone(),
                });
            },
        )
        .detach();
        cx.subscribe(&welcome, |_, _welcome, event: &CreateProjectEvent, cx| {
            cx.emit(TabViewEvent::CreateProject {
                prompt: event.prompt.clone(),
                parent: event.parent.clone(),
            });
        })
        .detach();
        cx.subscribe(&welcome, |_, _welcome, event: &CloneRepositoryEvent, cx| {
            cx.emit(TabViewEvent::CloneRepository {
                url: event.url.clone(),
                parent: event.parent.clone(),
            });
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
        let model_options_loading = agent_selected_key.is_some();
        let runtime_preferences =
            Arc::new(Mutex::new(RuntimePreferences::load().unwrap_or_default()));
        let registry_manifests = storage::app_root()
            .ok()
            .and_then(|app_root| load_cached_registry(&app_root).ok().flatten())
            .map(|cached| cached.manifests)
            .unwrap_or_default();
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
            preview_scroll_handle: ScrollHandle::new(),
            preview_code_scroll_handle: UniformListScrollHandle::new(),
            input_visible: true,
            overlay: None,
            needs_git_refresh: false,
            mode: TabViewMode::Terminal,
            last_line_incomplete: false,
            total_output_lines: 0,
            follow_output: true,
            last_scroll_to_bottom_at: Instant::now(),
            input_mode: InputMode::Terminal,
            agent_rows,
            agent_selected_key,
            agent_client: None,
            agent_client_key: None,
            agent_busy: false,
            agent_needs_auth: false,
            runtime_preferences,
            registry_manifests,
            discovered_models: Vec::new(),
            model_options_loading,
            selected_model_override: None,
            selected_block: None,
            output_selection_anchor: None,
            output_selection_head: None,
            output_selecting: false,
            file_preview: None,
            preview_search_match: None,
            preview_focus: false,
            preview_selection_anchor: None,
            preview_selection_head: None,
            preview_selecting: false,
            preview_active_line: None,
            file_preview_mode: FilePreviewMode::Code,
            preview_terminal_height: DEFAULT_PREVIEW_TERMINAL_HEIGHT,
            preview_terminal_resize_dragging: false,
            preview_terminal_resize_start_y: None,
            preview_terminal_resize_start_height: DEFAULT_PREVIEW_TERMINAL_HEIGHT,
        }
    }

    fn build_file_preview_state(path: &Path, root: Option<&Path>) -> FilePreviewState {
        let relative_label = root
            .and_then(|root| path.strip_prefix(root).ok())
            .map(Self::format_path)
            .unwrap_or_else(|| Self::format_path(path));
        let language = Self::detect_preview_language(path);
        let extension = path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.to_ascii_lowercase());

        let image_extensions = [
            "png", "jpg", "jpeg", "gif", "webp", "bmp", "ico", "svg", "avif", "tif", "tiff",
        ];

        let kind = if extension
            .as_deref()
            .map(|ext| image_extensions.contains(&ext))
            .unwrap_or(false)
        {
            FilePreviewKind::Image
        } else {
            match std::fs::read(path) {
                Ok(bytes) if bytes.len() > MAX_TEXT_PREVIEW_BYTES => FilePreviewKind::Unsupported {
                    message: format!(
                        "File too large to preview inline ({} KB limit).",
                        MAX_TEXT_PREVIEW_BYTES / 1024
                    ),
                },
                Ok(bytes) if bytes.contains(&0) => FilePreviewKind::Unsupported {
                    message: "Binary file preview is not supported yet.".into(),
                },
                Ok(bytes) => match String::from_utf8(bytes) {
                    Ok(contents) => {
                        let lines =
                            Arc::new(contents.lines().map(|line| line.to_string()).collect());
                        FilePreviewKind::Text { contents, lines }
                    }
                    Err(_) => FilePreviewKind::Unsupported {
                        message: "This file is not valid UTF-8 text.".into(),
                    },
                },
                Err(err) => FilePreviewKind::Unsupported {
                    message: format!("Failed to read file: {err}"),
                },
            }
        };

        FilePreviewState {
            path: path.to_path_buf(),
            relative_label,
            language,
            kind,
        }
    }

    fn detect_preview_language(path: &Path) -> PreviewLanguage {
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .map(|name| name.to_ascii_lowercase())
            .unwrap_or_default();

        match file_name.as_str() {
            "cargo.toml" | "agents.json" | "repo.config.json" | "registry-sample.json" => {
                return if file_name.ends_with(".toml") {
                    PreviewLanguage::Toml
                } else {
                    PreviewLanguage::Json
                };
            }
            "dockerfile" => {
                return PreviewLanguage::Dockerfile;
            }
            ".gitignore" | ".dockerignore" | ".eslintignore" | ".prettierignore" => {
                return PreviewLanguage::PlainText;
            }
            _ => {}
        }

        match path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.to_ascii_lowercase())
            .as_deref()
        {
            Some("rs") => PreviewLanguage::Rust,
            Some("md") => PreviewLanguage::Markdown,
            Some("json") => PreviewLanguage::Json,
            Some("toml") => PreviewLanguage::Toml,
            Some("yaml") | Some("yml") => PreviewLanguage::Yaml,
            Some("sh") | Some("bash") | Some("zsh") => PreviewLanguage::Shell,
            Some("js") => PreviewLanguage::JavaScript,
            Some("jsx") => PreviewLanguage::JavaScriptReact,
            Some("ts") => PreviewLanguage::TypeScript,
            Some("tsx") => PreviewLanguage::TypeScriptReact,
            Some("html") | Some("htm") => PreviewLanguage::Html,
            Some("css") | Some("scss") | Some("less") => PreviewLanguage::Css,
            Some("sql") => PreviewLanguage::Sql,
            Some("py") => PreviewLanguage::Python,
            Some("go") => PreviewLanguage::Go,
            _ => PreviewLanguage::PlainText,
        }
    }

    fn preview_language_label(language: PreviewLanguage) -> &'static str {
        match language {
            PreviewLanguage::Rust => "Rust",
            PreviewLanguage::Markdown => "Markdown",
            PreviewLanguage::Json => "JSON",
            PreviewLanguage::Toml => "TOML",
            PreviewLanguage::Yaml => "YAML",
            PreviewLanguage::Shell => "Shell",
            PreviewLanguage::JavaScript => "JavaScript",
            PreviewLanguage::JavaScriptReact => "JSX",
            PreviewLanguage::TypeScript => "TypeScript",
            PreviewLanguage::TypeScriptReact => "TSX",
            PreviewLanguage::Html => "HTML",
            PreviewLanguage::Css => "CSS",
            PreviewLanguage::Sql => "SQL",
            PreviewLanguage::Python => "Python",
            PreviewLanguage::Go => "Go",
            PreviewLanguage::Dockerfile => "Dockerfile",
            PreviewLanguage::PlainText => "Plain text",
        }
    }

    fn highlight_line(line: &str, language: PreviewLanguage) -> Vec<HighlightSegment> {
        match language {
            PreviewLanguage::Markdown => Self::highlight_markdown_line(line),
            PreviewLanguage::Html => Self::highlight_html_line(line),
            PreviewLanguage::PlainText => vec![HighlightSegment {
                text: line.to_string(),
                color: 0xd8dee9,
                bold: false,
            }],
            _ => Self::highlight_code_like_line(line, language),
        }
    }

    fn highlight_markdown_line(line: &str) -> Vec<HighlightSegment> {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") {
            return vec![HighlightSegment {
                text: line.to_string(),
                color: 0xe3b341,
                bold: true,
            }];
        }
        if trimmed.starts_with('#') {
            return vec![HighlightSegment {
                text: line.to_string(),
                color: 0x79c0ff,
                bold: true,
            }];
        }
        if trimmed.starts_with("- ") || trimmed.starts_with("* ") {
            return vec![
                HighlightSegment {
                    text: line.chars().take_while(|ch| ch.is_whitespace()).collect(),
                    color: 0xd8dee9,
                    bold: false,
                },
                HighlightSegment {
                    text: trimmed.chars().take(1).collect(),
                    color: 0xe3b341,
                    bold: true,
                },
                HighlightSegment {
                    text: trimmed.chars().skip(1).collect(),
                    color: 0xd8dee9,
                    bold: false,
                },
            ];
        }

        vec![HighlightSegment {
            text: line.to_string(),
            color: 0xd8dee9,
            bold: false,
        }]
    }

    fn highlight_html_line(line: &str) -> Vec<HighlightSegment> {
        if let Some(comment_start) = line.find("<!--")
            && let Some(comment_end) = line[comment_start..].find("-->")
        {
            let comment_end = comment_start + comment_end + 3;
            let mut head = Self::highlight_html_line(&line[..comment_start]);
            head.push(HighlightSegment {
                text: line[comment_start..comment_end].to_string(),
                color: 0x6a9955,
                bold: false,
            });
            if comment_end < line.len() {
                head.extend(Self::highlight_html_line(&line[comment_end..]));
            }
            return head;
        }

        let mut segments = Vec::new();
        let chars: Vec<char> = line.chars().collect();
        let mut index = 0;
        while index < chars.len() {
            let ch = chars[index];
            if ch == '<' {
                let start = index;
                index += 1;
                while index < chars.len() && !chars[index].is_whitespace() && chars[index] != '>' {
                    index += 1;
                }
                segments.push(HighlightSegment {
                    text: chars[start..index].iter().collect(),
                    color: 0x79c0ff,
                    bold: true,
                });
                continue;
            }
            if ch == '"' || ch == '\'' {
                let quote = ch;
                let start = index;
                index += 1;
                while index < chars.len() {
                    let current = chars[index];
                    index += 1;
                    if current == quote {
                        break;
                    }
                }
                segments.push(HighlightSegment {
                    text: chars[start..index].iter().collect(),
                    color: 0xce9178,
                    bold: false,
                });
                continue;
            }
            if ch.is_ascii_alphabetic() || ch == '-' {
                let start = index;
                index += 1;
                while index < chars.len()
                    && (chars[index].is_ascii_alphanumeric() || matches!(chars[index], '-' | '_'))
                {
                    index += 1;
                }
                let token: String = chars[start..index].iter().collect();
                let is_attr = index < chars.len() && chars[index] == '=';
                segments.push(HighlightSegment {
                    text: token,
                    color: if is_attr { 0x9cdcfe } else { 0xd8dee9 },
                    bold: is_attr,
                });
                continue;
            }
            segments.push(HighlightSegment {
                text: ch.to_string(),
                color: 0x8b949e,
                bold: false,
            });
            index += 1;
        }

        segments
    }

    fn highlight_code_like_line(line: &str, language: PreviewLanguage) -> Vec<HighlightSegment> {
        let comment_prefix = match language {
            PreviewLanguage::Rust | PreviewLanguage::JavaScript | PreviewLanguage::TypeScript => {
                Some("//")
            }
            PreviewLanguage::Toml
            | PreviewLanguage::Yaml
            | PreviewLanguage::Shell
            | PreviewLanguage::Python
            | PreviewLanguage::Dockerfile => Some("#"),
            PreviewLanguage::Sql => Some("--"),
            _ => None,
        };

        if let Some(prefix) = comment_prefix
            && let Some(index) = line.find(prefix)
        {
            let mut head = Self::highlight_code_like_line(&line[..index], language);
            head.push(HighlightSegment {
                text: line[index..].to_string(),
                color: 0x6a9955,
                bold: false,
            });
            return head;
        }

        let keywords: &[&str] = match language {
            PreviewLanguage::Rust => &[
                "fn", "let", "mut", "pub", "struct", "enum", "impl", "match", "if", "else", "use",
                "mod", "self", "Self", "return",
            ],
            PreviewLanguage::Json => &["true", "false", "null"],
            PreviewLanguage::Toml | PreviewLanguage::Yaml => &["true", "false"],
            PreviewLanguage::Shell | PreviewLanguage::Dockerfile => &[
                "if",
                "then",
                "else",
                "fi",
                "for",
                "do",
                "done",
                "case",
                "from",
                "run",
                "copy",
                "workdir",
                "env",
                "cmd",
                "entrypoint",
                "expose",
                "arg",
            ],
            PreviewLanguage::JavaScript
            | PreviewLanguage::TypeScript
            | PreviewLanguage::JavaScriptReact
            | PreviewLanguage::TypeScriptReact => &[
                "const",
                "let",
                "function",
                "return",
                "if",
                "else",
                "import",
                "from",
                "export",
                "class",
                "extends",
                "interface",
                "type",
                "async",
                "await",
                "props",
            ],
            PreviewLanguage::Css => &[
                "display",
                "position",
                "color",
                "background",
                "padding",
                "margin",
                "gap",
                "flex",
                "grid",
                "width",
                "height",
            ],
            PreviewLanguage::Sql => &[
                "select", "from", "where", "insert", "into", "update", "delete", "join", "left",
                "right", "inner", "outer", "group", "by", "order", "limit", "and", "or", "as",
                "on",
            ],
            PreviewLanguage::Python => &[
                "def", "class", "return", "if", "elif", "else", "for", "while", "in", "import",
                "from", "try", "except", "with", "as", "pass", "None", "True", "False",
            ],
            PreviewLanguage::Go => &[
                "func",
                "package",
                "import",
                "return",
                "if",
                "else",
                "for",
                "range",
                "go",
                "defer",
                "struct",
                "type",
                "interface",
                "map",
                "var",
                "const",
            ],
            PreviewLanguage::Markdown | PreviewLanguage::PlainText => &[],
            PreviewLanguage::Html => &[],
        };

        let mut segments = Vec::new();
        let chars: Vec<char> = line.chars().collect();
        let mut index = 0;
        while index < chars.len() {
            let ch = chars[index];
            if ch.is_whitespace() {
                let start = index;
                while index < chars.len() && chars[index].is_whitespace() {
                    index += 1;
                }
                segments.push(HighlightSegment {
                    text: chars[start..index].iter().collect(),
                    color: 0xd8dee9,
                    bold: false,
                });
                continue;
            }

            if ch == '"' || ch == '\'' {
                let quote = ch;
                let start = index;
                index += 1;
                while index < chars.len() {
                    let current = chars[index];
                    index += 1;
                    if current == '\\' && index < chars.len() {
                        index += 1;
                        continue;
                    }
                    if current == quote {
                        break;
                    }
                }
                segments.push(HighlightSegment {
                    text: chars[start..index].iter().collect(),
                    color: 0xce9178,
                    bold: false,
                });
                continue;
            }

            if ch.is_ascii_digit() {
                let start = index;
                index += 1;
                while index < chars.len()
                    && (chars[index].is_ascii_digit() || matches!(chars[index], '.' | '_'))
                {
                    index += 1;
                }
                segments.push(HighlightSegment {
                    text: chars[start..index].iter().collect(),
                    color: 0xb5cea8,
                    bold: false,
                });
                continue;
            }

            if ch.is_ascii_alphabetic() || ch == '_' || ch == '$' {
                let start = index;
                index += 1;
                while index < chars.len()
                    && (chars[index].is_ascii_alphanumeric()
                        || matches!(chars[index], '_' | '-' | '$'))
                {
                    index += 1;
                }
                let word: String = chars[start..index].iter().collect();
                let lower_word = word.to_ascii_lowercase();
                let is_keyword = keywords.iter().any(|keyword| *keyword == lower_word);
                segments.push(HighlightSegment {
                    text: word,
                    color: if is_keyword { 0x79c0ff } else { 0xd8dee9 },
                    bold: is_keyword,
                });
                continue;
            }

            segments.push(HighlightSegment {
                text: ch.to_string(),
                color: 0x8b949e,
                bold: false,
            });
            index += 1;
        }

        segments
    }

    pub fn open_file_preview(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        if !matches!(self.mode, TabViewMode::Terminal) {
            return;
        }

        let root = expand_tilde(&self.current_path);
        self.file_preview = Some(Self::build_file_preview_state(&path, Some(root.as_path())));
        self.preview_scroll_handle
            .set_offset(point(px(0.0), px(0.0)));
        self.preview_code_scroll_handle
            .0
            .borrow()
            .base_handle
            .set_offset(point(px(0.0), px(0.0)));
        self.preview_terminal_resize_dragging = false;
        self.preview_terminal_resize_start_y = None;
        self.preview_terminal_resize_start_height = self.preview_terminal_height;
        self.preview_search_match = None;
        self.preview_active_line = Some(0);
        self.clear_preview_selection();
        self.focus_preview();
        self.file_preview_mode = FilePreviewMode::Code;
        cx.notify();
    }

    pub fn open_file_preview_at_search_result(
        &mut self,
        path: PathBuf,
        line_number: usize,
        query: String,
        cx: &mut Context<Self>,
    ) {
        if !matches!(self.mode, TabViewMode::Terminal) {
            return;
        }

        let root = expand_tilde(&self.current_path);
        let preview = Self::build_file_preview_state(&path, Some(root.as_path()));
        let line_count = match &preview.kind {
            FilePreviewKind::Text { lines, .. } => lines.len(),
            _ => 0,
        };
        let preview_search_match =
            Self::build_preview_search_match(line_number, &query, line_count)
                .map(|(line_index, query)| PreviewSearchMatch { line_index, query });
        let active_line = preview_search_match
            .as_ref()
            .map(|search_match| search_match.line_index)
            .unwrap_or(0);

        self.file_preview = Some(preview);
        self.preview_scroll_handle
            .set_offset(point(px(0.0), px(0.0)));
        self.preview_code_scroll_handle
            .scroll_to_item_strict(active_line, ScrollStrategy::Center);
        self.preview_terminal_resize_dragging = false;
        self.preview_terminal_resize_start_y = None;
        self.preview_terminal_resize_start_height = self.preview_terminal_height;
        self.preview_search_match = preview_search_match;
        self.preview_active_line = Some(active_line);
        self.clear_preview_selection();
        self.focus_preview();
        self.file_preview_mode = FilePreviewMode::Code;
        cx.notify();
    }

    fn close_file_preview(&mut self, cx: &mut Context<Self>) {
        self.file_preview = None;
        self.preview_scroll_handle
            .set_offset(point(px(0.0), px(0.0)));
        self.preview_code_scroll_handle
            .0
            .borrow()
            .base_handle
            .set_offset(point(px(0.0), px(0.0)));
        self.preview_terminal_resize_dragging = false;
        self.preview_terminal_resize_start_y = None;
        self.preview_terminal_resize_start_height = self.preview_terminal_height;
        self.preview_search_match = None;
        self.preview_active_line = None;
        self.clear_preview_selection();
        self.blur_preview();
        self.file_preview_mode = FilePreviewMode::Code;
        cx.notify();
    }

    fn toggle_file_preview_mode(&mut self, cx: &mut Context<Self>) {
        let Some(preview) = self.file_preview.as_ref() else {
            return;
        };
        if !Self::preview_supports_rendered_markdown(preview) {
            return;
        }

        self.file_preview_mode = match self.file_preview_mode {
            FilePreviewMode::Code => FilePreviewMode::Preview,
            FilePreviewMode::Preview => FilePreviewMode::Code,
        };
        self.preview_selecting = false;
        cx.notify();
    }

    fn render_preview_code_row(
        language: PreviewLanguage,
        selection: Option<(usize, usize)>,
        active_line: Option<usize>,
        persistent_match: Option<&PreviewSearchMatch>,
        line: &str,
        index: usize,
        view_handle: &WeakEntity<Self>,
    ) -> Div {
        let segments = Self::highlight_line(line, language);
        let rendered_segments = persistent_match
            .filter(|search_match| search_match.line_index == index)
            .map(|search_match| {
                Self::apply_preview_search_match_segments(&segments, &search_match.query)
            })
            .unwrap_or_else(|| {
                segments
                    .iter()
                    .map(|segment| PreviewSearchMatchSegment {
                        text: segment.text.clone(),
                        color: segment.color,
                        bold: segment.bold,
                        matched: false,
                    })
                    .collect::<Vec<_>>()
            });
        let is_selected = selection
            .map(|(start, end)| index >= start && index <= end)
            .unwrap_or(false);
        let is_active_line = active_line == Some(index);

        let mut row = div()
            .h(px(22.0))
            .min_h(px(22.0))
            .flex()
            .items_center()
            .gap(px(12.0))
            .cursor(CursorStyle::IBeam)
            .child(
                div()
                    .w(px(44.0))
                    .text_size(px(11.0))
                    .text_color(rgb(if is_selected { 0xbdd9ff } else { 0x5f6b7a }))
                    .font_family("Cascadia Code")
                    .child((index + 1).to_string()),
            )
            .child(
                div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .gap(px(0.0))
                    .overflow_hidden()
                    .text_size(px(12.0))
                    .font_family("Cascadia Code")
                    .whitespace_nowrap()
                    .children(if rendered_segments.is_empty() {
                        vec![
                            div()
                                .text_color(rgb(0xd8dee9))
                                .child(" ")
                                .into_any_element(),
                        ]
                    } else {
                        rendered_segments
                            .into_iter()
                            .map(|segment| {
                                let mut span =
                                    div().text_color(rgb(segment.color)).child(segment.text);
                                if segment.matched {
                                    span = span.px(px(1.0)).rounded(px(3.0)).bg(rgb(0x5c4a16));
                                }
                                if segment.bold {
                                    span = span.font_weight(FontWeight::BOLD);
                                }
                                span.into_any_element()
                            })
                            .collect::<Vec<_>>()
                    }),
            );

        let mouse_down_handle = view_handle.clone();
        row = row.on_mouse_down(MouseButton::Left, move |event, _window, cx| {
            let _ = mouse_down_handle.update(cx, |view, cx| {
                view.on_preview_line_mouse_down_at(index, event, cx);
            });
        });

        let mouse_move_handle = view_handle.clone();
        row = row.on_mouse_move(move |event, _window, cx| {
            let _ = mouse_move_handle.update(cx, |view, cx| {
                view.on_preview_line_mouse_move_at(index, event, cx);
            });
        });

        if is_selected {
            row = row.bg(rgb(0x1a2f4a)).rounded(px(4.0));
        } else if is_active_line {
            row = row.bg(rgb(0x122033)).rounded(px(4.0));
        }

        row
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

    fn render_file_preview(&self, preview: &FilePreviewState, cx: &Context<Self>) -> Div {
        let file_name = preview
            .path
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| preview.path.to_string_lossy().to_string());
        let close_handle = cx.entity().downgrade();
        let language_label = Self::preview_language_label(preview.language);
        let supports_markdown_preview = Self::preview_supports_rendered_markdown(preview);
        let show_rendered_preview =
            supports_markdown_preview && self.file_preview_mode == FilePreviewMode::Preview;
        let preview_view_handle = cx.entity().downgrade();
        let markdown_toggle: AnyElement = if supports_markdown_preview {
            div()
                .size(px(28.0))
                .rounded(px(6.0))
                .flex()
                .items_center()
                .justify_center()
                .bg(if show_rendered_preview {
                    rgb(0x162235)
                } else {
                    rgb(0x121212)
                })
                .border_1()
                .border_color(if show_rendered_preview {
                    rgb(0x2f4f7a)
                } else {
                    rgb(0x2a2a2a)
                })
                .hover(|style| style.bg(rgb(0x1a1f2a)).border_color(rgb(0x3a3a3a)))
                .child(
                    lucide_icon(
                        if show_rendered_preview {
                            Icon::EyeOff
                        } else {
                            Icon::Eye
                        },
                        14.0,
                        if show_rendered_preview {
                            0x8fb7ff
                        } else {
                            0x9a9a9a
                        },
                    )
                    .cursor(CursorStyle::PointingHand),
                )
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(Self::on_toggle_file_preview_mode),
                )
                .into_any_element()
        } else {
            div().into_any_element()
        };

        let body: AnyElement = if show_rendered_preview {
            match &preview.kind {
                FilePreviewKind::Text { contents, .. } => div()
                    .id("file_preview_markdown")
                    .track_scroll(&self.preview_scroll_handle)
                    .flex()
                    .flex_col()
                    .on_mouse_down(MouseButton::Left, cx.listener(Self::on_preview_focus))
                    .on_mouse_up(MouseButton::Left, cx.listener(Self::on_preview_mouse_up))
                    .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_preview_mouse_up))
                    .overflow_y_scroll()
                    .scrollbar_width(px(12.0))
                    .flex_1()
                    .min_h(px(0.0))
                    .min_w(px(0.0))
                    .p(px(20.0))
                    .child(
                        div()
                            .max_w(px(960.0))
                            .min_h(px(0.0))
                            .min_w(px(0.0))
                            .pr(px(24.0))
                            .child(render_markdown_response_content(contents, false)),
                    )
                    .into_any_element(),
                _ => div().into_any_element(),
            }
        } else {
            match &preview.kind {
                FilePreviewKind::Text { lines, .. } => {
                    let lines = Arc::clone(lines);
                    let selection = self.normalize_preview_selection();
                    let language = preview.language;
                    let active_line = self.preview_active_line;
                    let persistent_match = self.preview_search_match.clone();
                    div()
                        .id("file_preview_text")
                        .flex()
                        .flex_col()
                        .on_mouse_down(MouseButton::Left, cx.listener(Self::on_preview_focus))
                        .on_mouse_up(MouseButton::Left, cx.listener(Self::on_preview_mouse_up))
                        .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_preview_mouse_up))
                        .flex_1()
                        .min_h(px(0.0))
                        .min_w(px(0.0))
                        .p(px(16.0))
                        .child(
                            uniform_list("file_preview_text_list", lines.len(), {
                                let lines = Arc::clone(&lines);
                                let view_handle = preview_view_handle.clone();
                                move |range, _window, _cx| {
                                    range
                                        .map(|index| {
                                            Self::render_preview_code_row(
                                                language,
                                                selection,
                                                active_line,
                                                persistent_match.as_ref(),
                                                &lines[index],
                                                index,
                                                &view_handle,
                                            )
                                        })
                                        .collect::<Vec<_>>()
                                }
                            })
                            .track_scroll(self.preview_code_scroll_handle.clone())
                            .w_full()
                            .h_full(),
                        )
                        .into_any_element()
                }
                FilePreviewKind::Image => div()
                    .id("file_preview_image")
                    .track_scroll(&self.preview_scroll_handle)
                    .flex()
                    .flex_col()
                    .overflow_y_scroll()
                    .scrollbar_width(px(12.0))
                    .flex_1()
                    .min_h(px(0.0))
                    .min_w(px(0.0))
                    .p(px(20.0))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_center()
                            .min_h(px(0.0))
                            .child(
                                img(preview.path.clone())
                                    .object_fit(ObjectFit::Contain)
                                    .with_fallback(|| {
                                        div()
                                            .text_size(px(12.0))
                                            .text_color(rgb(0xff7b72))
                                            .child("Failed to render image.")
                                            .into_any_element()
                                    }),
                            ),
                    )
                    .into_any_element(),
                FilePreviewKind::Unsupported { message } => div()
                    .flex()
                    .flex_1()
                    .min_h(px(0.0))
                    .items_center()
                    .justify_center()
                    .child(
                        div()
                            .text_size(px(12.0))
                            .text_color(rgb(0x8a97a8))
                            .child(message.clone()),
                    )
                    .into_any_element(),
            }
        };

        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h(px(0.0))
            .min_w(px(0.0))
            .overflow_hidden()
            .bg(rgb(0x0a0a0a))
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_between()
                    .px(px(16.0))
                    .py(px(12.0))
                    .border_b_1()
                    .border_color(rgb(0x1f1f1f))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(4.0))
                            .child(
                                div()
                                    .text_size(px(13.0))
                                    .text_color(rgb(0xf0f0f0))
                                    .child(file_name),
                            )
                            .child(
                                div()
                                    .text_size(px(11.0))
                                    .text_color(rgb(0x7f8b99))
                                    .child(preview.relative_label.clone()),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.0))
                            .child(markdown_toggle)
                            .child(
                                div()
                                    .px(px(8.0))
                                    .py(px(4.0))
                                    .rounded(px(999.0))
                                    .bg(rgb(0x121826))
                                    .border_1()
                                    .border_color(rgb(0x26324d))
                                    .text_size(px(11.0))
                                    .text_color(rgb(0x8fb7ff))
                                    .child(language_label),
                            ),
                    )
                    .child(
                        div()
                            .size(px(28.0))
                            .rounded(px(6.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .bg(rgb(0x121212))
                            .border_1()
                            .border_color(rgb(0x2a2a2a))
                            .hover(|style| style.bg(rgb(0x1a1a1a)).border_color(rgb(0x3a3a3a)))
                            .child(
                                lucide_icon(Icon::X, 14.0, 0x9a9a9a)
                                    .cursor(CursorStyle::PointingHand),
                            )
                            .on_mouse_down(MouseButton::Left, move |_event, _window, cx| {
                                cx.stop_propagation();
                                let _ = close_handle.update(cx, |view, cx| {
                                    view.close_file_preview(cx);
                                });
                            }),
                    ),
            )
            .child(body)
    }

    fn render_terminal_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let _ = window;
        let mut panel = div()
            .flex()
            .flex_col()
            .size_full()
            .min_h(px(0.0))
            .min_w(px(0.0))
            .relative()
            .child(
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
                            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_output_mouse_up))
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
                                    .map(|(i, block)| self.render_block(block, i, active_index, cx))
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
            .child(self.render_overlay(cx));

        if self.input_visible {
            panel = panel.child(
                div()
                    .flex_none()
                    .child(self.render_history_menu_container())
                    .child(
                        div()
                            .px(px(16.0))
                            .pb(px(12.0))
                            .child(self.render_input_bar(window, cx)),
                    ),
            );
        }

        panel
    }

    fn render_terminal_workspace(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        if let Some(preview) = self.file_preview.clone() {
            let terminal_height = Self::clamp_preview_terminal_height(self.preview_terminal_height);
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_h(px(0.0))
                .min_w(px(0.0))
                .overflow_hidden()
                .on_mouse_move(cx.listener(Self::on_preview_terminal_resize_mouse_move))
                .on_mouse_up(
                    MouseButton::Left,
                    cx.listener(Self::on_preview_terminal_resize_mouse_up),
                )
                .on_mouse_up_out(
                    MouseButton::Left,
                    cx.listener(Self::on_preview_terminal_resize_mouse_up),
                )
                .child(
                    div()
                        .flex()
                        .flex_1()
                        .min_h(px(0.0))
                        .min_w(px(0.0))
                        .overflow_hidden()
                        .child(self.render_file_preview(&preview, cx)),
                )
                .child(
                    div()
                        .flex_none()
                        .h(px(PREVIEW_TERMINAL_RESIZE_HANDLE_HEIGHT))
                        .cursor(CursorStyle::ResizeUpDown)
                        .bg(if self.preview_terminal_resize_dragging {
                            rgb(0x2a4a73)
                        } else {
                            rgb(0x141414)
                        })
                        .border_t_1()
                        .border_b_1()
                        .border_color(if self.preview_terminal_resize_dragging {
                            rgb(0x3f669c)
                        } else {
                            rgb(0x1f1f1f)
                        })
                        .hover(|style| style.bg(rgb(0x1a1f28)).border_color(rgb(0x2f3b4f)))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(Self::on_preview_terminal_resize_mouse_down),
                        ),
                )
                .child(
                    div()
                        .flex_none()
                        .h(px(terminal_height))
                        .min_h(px(180.0))
                        .min_w(px(0.0))
                        .bg(rgb(0x0a0a0a))
                        .child(self.render_terminal_panel(window, cx).size_full()),
                )
        } else {
            self.render_terminal_panel(window, cx)
        }
    }

    pub fn normalize_initial_terminal_command(command: Option<&str>) -> Option<String> {
        let command = command?.trim();
        (!command.is_empty()).then(|| command.to_string())
    }

    pub fn start_terminal_with_path(&mut self, cx: &mut Context<Self>, path: Option<PathBuf>) {
        self.start_terminal_with_path_and_command(cx, path, None);
    }

    pub fn start_terminal_with_path_and_command(
        &mut self,
        cx: &mut Context<Self>,
        path: Option<PathBuf>,
        initial_command: Option<String>,
    ) {
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

        if let Some(command) = Self::normalize_initial_terminal_command(initial_command.as_deref())
        {
            self.run_command(command, cx);
        }
    }

    pub fn start_agent_prompt_with_path(
        &mut self,
        cx: &mut Context<Self>,
        path: Option<PathBuf>,
        prompt: String,
    ) {
        self.start_terminal_with_path_and_command(cx, path, None);
        self.input_mode = InputMode::Agent;
        if let Some(prompt) = Self::normalize_initial_terminal_command(Some(&prompt)) {
            self.run_agent_prompt(prompt, cx);
        }
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        if self.handle_overlay_key(event, cx) {
            cx.stop_propagation();
            return;
        }
        if self.handle_preview_key(event, cx) {
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
            self.cancel_agent_prompt(cx);
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
        self.blur_preview();
        window.focus(&self.focus_handle);
    }

    fn on_preview_focus(
        &mut self,
        _event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.focus_preview();
        window.focus(&self.focus_handle);
        cx.notify();
        cx.stop_propagation();
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
            query: PickerQueryState::default(),
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
            query: PickerQueryState::default(),
            all_branches: all.clone(),
            branches: all,
            selected: 0,
        };
        Self::filter_branch_picker(&mut picker);
        self.overlay = Some(Overlay::Branch(picker));
        cx.notify();
    }

    fn on_open_agent_picker(
        &mut self,
        _event: &MouseDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.input_visible || self.agent_rows.is_empty() {
            return;
        }
        if let Some(picker) =
            build_agent_picker_state(&self.agent_rows, self.agent_selected_key.as_ref())
        {
            self.overlay = Some(Overlay::Agent(picker));
            cx.notify();
        }
    }

    fn on_open_model_picker(
        &mut self,
        _event: &MouseDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.input_visible {
            return;
        }
        if self.model_options_loading || self.discovered_models.is_empty() {
            return;
        }
        let catalog = self.current_model_catalog();
        let selected_model_id = self.current_selected_model_id();
        if let Some(picker) = build_model_picker_state(&catalog, selected_model_id.as_deref()) {
            self.overlay = Some(Overlay::Model(picker));
            cx.notify();
        }
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
        self.refresh_model_options(cx);
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
            "a" if event.keystroke.modifiers.control && Self::overlay_has_search_input(overlay) => {
                match overlay {
                    Overlay::Path(picker) => picker.query.select_all(),
                    Overlay::Branch(picker) => picker.query.select_all(),
                    Overlay::Agent(picker) => picker.query.select_all(),
                    Overlay::Model(picker) => picker.query.select_all(),
                }
                cx.notify();
                return true;
            }
            "backspace" => {
                Self::overlay_query_pop(overlay);
                cx.notify();
                return true;
            }
            "delete" if Self::overlay_has_search_input(overlay) => {
                Self::overlay_query_delete_forward(overlay);
                cx.notify();
                return true;
            }
            "enter" | "return" | "numpadenter" => {
                self.accept_overlay_selection(cx);
                return true;
            }
            "up" | "arrowup" => {
                Self::overlay_move_selection_up(overlay);
                cx.notify();
                return true;
            }
            "down" | "arrowdown" => {
                Self::overlay_move_selection_down(overlay);
                cx.notify();
                return true;
            }
            "home" => {
                if Self::overlay_has_search_input(overlay) {
                    match overlay {
                        Overlay::Path(picker) => {
                            picker.query.move_home(event.keystroke.modifiers.shift)
                        }
                        Overlay::Branch(picker) => {
                            picker.query.move_home(event.keystroke.modifiers.shift)
                        }
                        Overlay::Agent(picker) => {
                            picker.query.move_home(event.keystroke.modifiers.shift)
                        }
                        Overlay::Model(picker) => {
                            picker.query.move_home(event.keystroke.modifiers.shift)
                        }
                    }
                } else {
                    Self::overlay_select_first(overlay);
                }
                cx.notify();
                return true;
            }
            "end" => {
                if Self::overlay_has_search_input(overlay) {
                    match overlay {
                        Overlay::Path(picker) => {
                            picker.query.move_end(event.keystroke.modifiers.shift)
                        }
                        Overlay::Branch(picker) => {
                            picker.query.move_end(event.keystroke.modifiers.shift)
                        }
                        Overlay::Agent(picker) => {
                            picker.query.move_end(event.keystroke.modifiers.shift)
                        }
                        Overlay::Model(picker) => {
                            picker.query.move_end(event.keystroke.modifiers.shift)
                        }
                    }
                } else {
                    Self::overlay_select_last(overlay);
                }
                cx.notify();
                return true;
            }
            "left" | "arrowleft" if Self::overlay_has_search_input(overlay) => {
                match overlay {
                    Overlay::Path(picker) => {
                        picker.query.move_left(event.keystroke.modifiers.shift)
                    }
                    Overlay::Branch(picker) => {
                        picker.query.move_left(event.keystroke.modifiers.shift)
                    }
                    Overlay::Agent(picker) => {
                        picker.query.move_left(event.keystroke.modifiers.shift)
                    }
                    Overlay::Model(picker) => {
                        picker.query.move_left(event.keystroke.modifiers.shift)
                    }
                }
                cx.notify();
                return true;
            }
            "right" | "arrowright" if Self::overlay_has_search_input(overlay) => {
                match overlay {
                    Overlay::Path(picker) => {
                        picker.query.move_right(event.keystroke.modifiers.shift)
                    }
                    Overlay::Branch(picker) => {
                        picker.query.move_right(event.keystroke.modifiers.shift)
                    }
                    Overlay::Agent(picker) => {
                        picker.query.move_right(event.keystroke.modifiers.shift)
                    }
                    Overlay::Model(picker) => {
                        picker.query.move_right(event.keystroke.modifiers.shift)
                    }
                }
                cx.notify();
                return true;
            }
            _ => {}
        }

        if Self::overlay_has_search_input(overlay)
            && ((event.keystroke.modifiers.control
                && event.keystroke.key.eq_ignore_ascii_case("v"))
                || (event.keystroke.modifiers.shift
                    && event.keystroke.key.eq_ignore_ascii_case("insert")))
        {
            if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                let paste = text.replace("\r\n", "\n").replace('\r', "\n");
                if !paste.is_empty() {
                    Self::overlay_insert_text(overlay, &paste);
                    cx.notify();
                    return true;
                }
            }
        }

        if !event.keystroke.modifiers.control {
            if let Some(text) = event.keystroke.key_char.as_deref() {
                if !text.is_empty() && Self::overlay_consume_text_input(overlay, text) {
                    cx.notify();
                    return true;
                }
            }

            if event.keystroke.key.len() == 1 {
                if Self::overlay_consume_text_input(overlay, &event.keystroke.key) {
                    cx.notify();
                    return true;
                }
            }
        }

        true
    }

    fn overlay_kind(overlay: &Overlay) -> PickerKind {
        match overlay {
            Overlay::Path(_) => PickerKind::Path,
            Overlay::Branch(_) => PickerKind::Branch,
            Overlay::Agent(_) => PickerKind::Agent,
            Overlay::Model(_) => PickerKind::Model,
        }
    }

    fn overlay_has_search_input(overlay: &Overlay) -> bool {
        match overlay {
            Overlay::Path(_) => true,
            Overlay::Branch(_) => true,
            Overlay::Agent(picker) => {
                picker_has_search_input(PickerKind::Agent, picker.all_options.len())
            }
            Overlay::Model(picker) => {
                picker_has_search_input(PickerKind::Model, picker.all_options.len())
            }
        }
    }

    fn overlay_move_selection_up(overlay: &mut Overlay) {
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
            Overlay::Agent(picker) => {
                if picker.selected > 0 {
                    picker.selected -= 1;
                }
            }
            Overlay::Model(picker) => {
                if picker.selected > 0 {
                    picker.selected -= 1;
                }
            }
        }
    }

    fn overlay_move_selection_down(overlay: &mut Overlay) {
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
            Overlay::Agent(picker) => {
                if picker.selected + 1 < picker.options.len() {
                    picker.selected += 1;
                }
            }
            Overlay::Model(picker) => {
                if picker.selected + 1 < picker.options.len() {
                    picker.selected += 1;
                }
            }
        }
    }

    fn overlay_select_first(overlay: &mut Overlay) {
        match overlay {
            Overlay::Path(picker) => picker.selected = 0,
            Overlay::Branch(picker) => picker.selected = 0,
            Overlay::Agent(picker) => picker.selected = 0,
            Overlay::Model(picker) => picker.selected = 0,
        }
    }

    fn overlay_select_last(overlay: &mut Overlay) {
        match overlay {
            Overlay::Path(picker) => {
                picker.selected = picker.entries.len().saturating_sub(1);
            }
            Overlay::Branch(picker) => {
                picker.selected = picker.branches.len().saturating_sub(1);
            }
            Overlay::Agent(picker) => {
                picker.selected = picker.options.len().saturating_sub(1);
            }
            Overlay::Model(picker) => {
                picker.selected = picker.options.len().saturating_sub(1);
            }
        }
    }

    fn overlay_query_push(overlay: &mut Overlay, ch: char) {
        match overlay {
            Overlay::Path(picker) => {
                picker.query.insert_text(&ch.to_string());
                picker.selected = 0;
                Self::populate_path_picker(picker);
            }
            Overlay::Branch(picker) => {
                picker.query.insert_text(&ch.to_string());
                picker.selected = 0;
                Self::filter_branch_picker(picker);
            }
            Overlay::Agent(picker) => {
                picker.query.insert_text(&ch.to_string());
                picker.selected = 0;
                Self::filter_agent_picker(picker);
            }
            Overlay::Model(picker) => {
                picker.query.insert_text(&ch.to_string());
                picker.selected = 0;
                Self::filter_model_picker(picker);
            }
        }
    }

    fn overlay_query_pop(overlay: &mut Overlay) {
        match overlay {
            Overlay::Path(picker) => {
                picker.query.pop_char_before_cursor();
                picker.selected = 0;
                Self::populate_path_picker(picker);
            }
            Overlay::Branch(picker) => {
                picker.query.pop_char_before_cursor();
                picker.selected = 0;
                Self::filter_branch_picker(picker);
            }
            Overlay::Agent(picker) => {
                picker.query.pop_char_before_cursor();
                picker.selected = 0;
                Self::filter_agent_picker(picker);
            }
            Overlay::Model(picker) => {
                picker.query.pop_char_before_cursor();
                picker.selected = 0;
                Self::filter_model_picker(picker);
            }
        }
    }

    fn overlay_insert_text(overlay: &mut Overlay, text: &str) {
        match overlay {
            Overlay::Path(picker) => {
                picker.query.insert_text(text);
                picker.selected = 0;
                Self::populate_path_picker(picker);
            }
            Overlay::Branch(picker) => {
                picker.query.insert_text(text);
                picker.selected = 0;
                Self::filter_branch_picker(picker);
            }
            Overlay::Agent(picker) => {
                picker.query.insert_text(text);
                picker.selected = 0;
                Self::filter_agent_picker(picker);
            }
            Overlay::Model(picker) => {
                picker.query.insert_text(text);
                picker.selected = 0;
                Self::filter_model_picker(picker);
            }
        }
    }

    fn overlay_query_delete_forward(overlay: &mut Overlay) {
        match overlay {
            Overlay::Path(picker) => {
                picker.query.delete_char_after_cursor();
                picker.selected = 0;
                Self::populate_path_picker(picker);
            }
            Overlay::Branch(picker) => {
                picker.query.delete_char_after_cursor();
                picker.selected = 0;
                Self::filter_branch_picker(picker);
            }
            Overlay::Agent(picker) => {
                picker.query.delete_char_after_cursor();
                picker.selected = 0;
                Self::filter_agent_picker(picker);
            }
            Overlay::Model(picker) => {
                picker.query.delete_char_after_cursor();
                picker.selected = 0;
                Self::filter_model_picker(picker);
            }
        }
    }

    fn overlay_apply_typeahead(overlay: &mut Overlay, ch: char) -> bool {
        let lower = ch.to_ascii_lowercase();
        match overlay {
            Overlay::Agent(picker) => {
                if let Some(idx) = picker.options.iter().position(|row| {
                    row.name
                        .chars()
                        .next()
                        .map(|c| c.to_ascii_lowercase() == lower)
                        .unwrap_or(false)
                }) {
                    picker.selected = idx;
                    return true;
                }
            }
            Overlay::Model(picker) => {
                if let Some(idx) = picker.options.iter().position(|model| {
                    model
                        .label
                        .chars()
                        .next()
                        .map(|c| c.to_ascii_lowercase() == lower)
                        .unwrap_or(false)
                }) {
                    picker.selected = idx;
                    return true;
                }
            }
            _ => {}
        }
        false
    }

    fn overlay_consume_text_input(overlay: &mut Overlay, text: &str) -> bool {
        let mut handled = false;
        for ch in text.chars() {
            match Self::overlay_kind(overlay) {
                PickerKind::Path | PickerKind::Branch => {
                    Self::overlay_query_push(overlay, ch);
                    handled = true;
                }
                PickerKind::Agent | PickerKind::Model
                    if Self::overlay_has_search_input(overlay) =>
                {
                    Self::overlay_query_push(overlay, ch);
                    handled = true;
                }
                PickerKind::Agent | PickerKind::Model => {
                    if Self::overlay_apply_typeahead(overlay, ch) {
                        handled = true;
                    }
                }
            }
        }
        handled
    }

    fn populate_path_picker(picker: &mut PathPickerState) {
        let query = picker.query.text.to_lowercase();
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
        let query = picker.query.text.to_lowercase();
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

    fn filter_agent_picker(picker: &mut AgentPickerState) {
        let query = picker.query.text.to_lowercase();
        if query.is_empty() {
            picker.options = picker.all_options.clone();
        } else {
            picker.options = picker
                .all_options
                .iter()
                .filter(|row| {
                    row.name.to_lowercase().contains(&query)
                        || row.acp_id.to_lowercase().contains(&query)
                })
                .cloned()
                .collect();
        }
        if picker.selected >= picker.options.len() {
            picker.selected = picker.options.len().saturating_sub(1);
        }
    }

    fn filter_model_picker(picker: &mut ModelPickerState) {
        let query = picker.query.text.to_lowercase();
        if query.is_empty() {
            picker.options = picker.all_options.clone();
        } else {
            picker.options = picker
                .all_options
                .iter()
                .filter(|model| {
                    model.label.to_lowercase().contains(&query)
                        || model.id.to_lowercase().contains(&query)
                        || model
                            .description
                            .as_ref()
                            .map(|d| d.to_lowercase().contains(&query))
                            .unwrap_or(false)
                })
                .cloned()
                .collect();
        }
        if picker.selected >= picker.options.len() {
            picker.selected = picker.options.len().saturating_sub(1);
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
            Overlay::Agent(picker) => {
                if let Some(selected) = picker.options.get(picker.selected) {
                    if let Some(index) = self
                        .agent_rows
                        .iter()
                        .position(|row| row.agent_key == selected.agent_key)
                    {
                        self.select_agent_index(index);
                        self.refresh_model_options(cx);
                    }
                }
                self.overlay = None;
                cx.notify();
            }
            Overlay::Model(picker) => {
                if let Some(selected) = picker.options.get(picker.selected) {
                    self.selected_model_override = Some(selected.id.clone());
                    if let Some(agent_key) = self.agent_selected_key.clone()
                        && let Ok(mut preferences) = self.runtime_preferences.lock()
                    {
                        let _ = preferences.set_default_model(agent_key, Some(selected.id.clone()));
                    }
                    if let Some(client) = self.agent_client.as_ref().cloned() {
                        let selected_model = selected.id.clone();
                        thread::spawn(move || {
                            if let Ok(client) = client.lock()
                                && let Some(session_id) = client.session_id.as_deref()
                            {
                                let _ = client.set_session_config_option(
                                    session_id,
                                    "model",
                                    &selected_model,
                                );
                            }
                        });
                    }
                }
                self.overlay = None;
                cx.notify();
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
                .border_color(rgb(0x262626))
                .cursor(clickable_cursor())
                .child(lucide_icon(icon, 12.0, 0x8a8a8a).cursor(CursorStyle::PointingHand))
        };
        let agent_name = self
            .active_agent_name()
            .unwrap_or_else(|| "No agent".to_string());
        let agent_mode = self.input_mode == InputMode::Agent;
        let agent_icon_present = self
            .active_registry_manifest()
            .and_then(|manifest| manifest.icon.as_ref())
            .is_some();
        let model_button_state = self.current_model_button_state();

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
                                    .cursor(clickable_cursor())
                                    .child(
                                        lucide_icon(
                                            Icon::Sparkles,
                                            13.0,
                                            if agent_mode { 0x8eb8ff } else { 0x7f7f7f },
                                        )
                                        .cursor(clickable_cursor()),
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
                            .on_mouse_down(
                                gpui::MouseButton::Left,
                                cx.listener(Self::on_open_agent_picker),
                            )
                            .child(registry_avatar(&agent_name, agent_icon_present))
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
                                div().flex().items_center().justify_center().child(
                                    lucide_icon(Icon::ChevronDown, 12.0, 0x9a9a9a)
                                        .cursor(clickable_cursor()),
                                ),
                            )
                    } else {
                        div()
                    })
                    .child(if agent_mode {
                        {
                            let (label, label_disabled) = model_trigger_label(&model_button_state);
                            let (icon_color, text_color, border_color, bg_color, disabled_default) =
                                match &model_button_state {
                                    ModelButtonState::Loading => {
                                        (0x7a7a7a, 0x8f8f8f, 0x2a2a2a, 0x111111, true)
                                    }
                                    ModelButtonState::Ready(_) => {
                                        (0x8eb8ff, 0xcfcfcf, 0x2a2a2a, 0x141414, false)
                                    }
                                    ModelButtonState::Unavailable => {
                                        (0x666666, 0x7f7f7f, 0x252525, 0x111111, true)
                                    }
                                };
                            let disabled = disabled_default || label_disabled;

                            let chip = div()
                                .flex()
                                .items_center()
                                .gap(px(6.0))
                                .px(px(8.0))
                                .py(px(5.0))
                                .rounded(px(6.0))
                                .bg(rgb(bg_color))
                                .border_1()
                                .border_color(rgb(border_color))
                                .child(
                                    lucide_icon(Icon::Sparkles, 12.0, icon_color)
                                        .cursor(CursorStyle::PointingHand),
                                )
                                .child(
                                    div()
                                        .text_size(px(11.0))
                                        .text_color(rgb(text_color))
                                        .child(label),
                                )
                                .child(
                                    lucide_icon(Icon::ChevronDown, 12.0, 0x9a9a9a)
                                        .cursor(CursorStyle::PointingHand),
                                );

                            if disabled {
                                chip
                            } else {
                                chip.on_mouse_down(
                                    gpui::MouseButton::Left,
                                    cx.listener(Self::on_open_model_picker),
                                )
                            }
                        }
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
                            .cursor(clickable_cursor())
                            .on_mouse_down(
                                gpui::MouseButton::Left,
                                cx.listener(Self::on_open_path_picker),
                            )
                            .child(
                                lucide_icon(Icon::Folder, 12.0, 0x6b9eff)
                                    .cursor(clickable_cursor()),
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
                            .cursor(clickable_cursor())
                            .on_mouse_down(
                                gpui::MouseButton::Left,
                                cx.listener(Self::on_open_branch_picker),
                            )
                            .child(
                                lucide_icon(Icon::GitBranch, 12.0, 0x6b9eff)
                                    .cursor(clickable_cursor()),
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
                                    .cursor(clickable_cursor()),
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
            .text_color(rgb(0x7c7c7c))
            .child(placeholder_text);

        let ghost_div = div()
            .text_size(px(15.0))
            .text_color(rgb(0x626262))
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

    fn active_registry_manifest(&self) -> Option<&RegistryManifest> {
        let row = self.active_agent_row()?;
        self.registry_manifests.get(&row.acp_id)
    }

    fn current_model_catalog(&self) -> Vec<AcpModelOption> {
        self.discovered_models.clone()
    }

    fn current_selected_model_id(&self) -> Option<String> {
        let catalog = self.current_model_catalog();
        if catalog.is_empty() {
            return None;
        }

        let persisted_default = self
            .agent_selected_key
            .as_ref()
            .and_then(|key| self.runtime_preferences.lock().ok()?.default_model_for(key));

        model_discovery::resolve_selected_model(
            self.selected_model_override.as_deref(),
            persisted_default.as_deref(),
            &catalog,
        )
    }

    fn current_selected_model_label(&self) -> Option<String> {
        let catalog = self.current_model_catalog();
        let selected = self.current_selected_model_id()?;
        catalog
            .iter()
            .find(|model| model.id == selected)
            .map(|model| model.label.clone())
    }

    fn current_model_button_state(&self) -> ModelButtonState {
        if self.model_options_loading {
            return ModelButtonState::Loading;
        }

        if let Some(label) = self.current_selected_model_label() {
            return ModelButtonState::Ready(label);
        }

        ModelButtonState::Unavailable
    }

    fn refresh_model_options(&mut self, cx: &mut Context<Self>) {
        let Some(client) = self.ensure_agent_client().ok() else {
            self.model_options_loading = false;
            self.discovered_models.clear();
            cx.notify();
            return;
        };

        self.model_options_loading = true;
        self.discovered_models.clear();
        let selected_key = self.agent_selected_key.clone();
        let runtime_preferences = self.runtime_preferences.clone();
        let session_cwd = expand_tilde(&self.current_path)
            .to_string_lossy()
            .to_string();

        let (tx, mut rx) = mpsc::unbounded::<Result<Vec<AcpModelOption>, String>>();
        thread::spawn(move || {
            let result = (|| -> Result<Vec<AcpModelOption>, String> {
                let mut guard = client
                    .lock()
                    .map_err(|_| "agent lock poisoned".to_string())?;
                if guard.protocol_version.is_none() {
                    guard.initialize().map_err(|err| err.to_string())?;
                }
                let runtime_mcp = crate::mcp::probe::load_enabled_runtime_mcp_servers();
                let persisted_default = selected_key.as_ref().and_then(|key| {
                    runtime_preferences
                        .lock()
                        .ok()
                        .and_then(|prefs| prefs.default_model_for(key))
                });
                let bootstrap = guard
                    .ensure_session(&session_cwd, &runtime_mcp, persisted_default.as_deref())
                    .map_err(|err| err.to_string())?;
                Ok(bootstrap.model_options)
            })();

            let _ = tx.unbounded_send(result);
        });

        cx.spawn(|view: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                while let Some(result) = rx.next().await {
                    let _ = view.update(&mut cx, |view, cx| {
                        view.model_options_loading = false;
                        match result {
                            Ok(models) => {
                                view.discovered_models = models;
                            }
                            Err(err) => {
                                view.discovered_models.clear();
                                view.append_agent_update_line(
                                    &format!("[agent] failed to load model selector: {err}"),
                                    cx,
                                );
                            }
                        }
                        cx.notify();
                    });
                }
            }
        })
        .detach();
    }

    fn select_agent_index(&mut self, index: usize) {
        self.agent_selected_key = self.agent_rows.get(index).map(|row| row.agent_key.clone());
        self.agent_client = None;
        self.agent_client_key = None;
        self.agent_needs_auth = false;
        self.discovered_models.clear();
        self.model_options_loading = self.agent_selected_key.is_some();
        self.selected_model_override = None;
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
            self.append_agent_update_line("[agent] no selected agent.", cx);
            cx.notify();
            return;
        };
        let Some(auth_cmd) = spec.auth else {
            self.append_agent_update_line("[agent] this agent has no auth command configured.", cx);
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

    fn append_agent_update_line(&mut self, text: &str, cx: &mut Context<Self>) {
        self.append_agent_update(text, false, cx);
    }

    fn clear_last_agent_placeholder(&mut self) {
        let had_placeholder = self
            .blocks
            .last()
            .map(|block| block.agent_placeholder_active)
            .unwrap_or(false);
        if !had_placeholder {
            return;
        }

        if let Some(block) = self.blocks.last_mut() {
            block.agent_placeholder_active = false;
            block.agent_response = None;
            block.agent_response_line_count = 0;
            if block.output_lines.pop().is_some() {
                self.total_output_lines = self.total_output_lines.saturating_sub(1);
            }
        }
    }

    fn replace_last_agent_placeholder(
        &mut self,
        response: AcpResponseText,
        cx: &mut Context<Self>,
    ) {
        self.clear_last_agent_placeholder();
        self.append_agent_response(response, cx);
    }

    fn update_last_agent_placeholder(&mut self, text: &str, cx: &mut Context<Self>) {
        if let Some(block) = self.blocks.last_mut() {
            if !update_agent_placeholder_block(block, text) && block.agent_placeholder_active {
                block.output_lines.push(text.to_string());
                self.total_output_lines += 1;
            }
        }
        self.trim_output_lines();
        self.request_scroll_to_bottom(cx);
    }

    fn last_block_has_agent_placeholder(&self) -> bool {
        self.blocks
            .last()
            .map(|block| block.agent_placeholder_active)
            .unwrap_or(false)
    }

    fn append_agent_response(&mut self, response: AcpResponseText, cx: &mut Context<Self>) {
        let response_text = response.text().to_string();
        let normalized = response_text.replace("\r\n", "\n").replace('\r', "\n");
        let response_lines = markdown_response_line_count(&normalized);

        if let Some(block) = self.blocks.last_mut() {
            block.agent_response = Some(response);
            block.agent_response_line_count = response_lines;
            for line in normalized.lines() {
                block.output_lines.push(line.to_string());
            }
            if normalized.is_empty() {
                block.output_lines.push(String::new());
            }
            self.total_output_lines += response_lines.max(1);
        }
        self.trim_output_lines();
        self.request_scroll_to_bottom(cx);
    }

    fn append_agent_update(&mut self, text: &str, append_to_last: bool, cx: &mut Context<Self>) {
        let normalized = strip_ansi(text).replace("\r\n", "\n").replace('\r', "\n");
        if self.blocks.is_empty() {
            self.blocks.push(Block {
                command: String::new(),
                output_lines: Vec::new(),
                has_error: false,
                context: None,
                agent_placeholder_active: false,
                pending_permission: None,
                agent_stream_text: String::new(),
                agent_stream_line_index: None,
                agent_response: None,
                agent_response_line_count: 0,
            });
        }
        if !normalized.trim().is_empty() {
            self.clear_last_agent_placeholder();
        }
        if let Some(block) = self.blocks.last_mut() {
            match classify_agent_stream_op(&block.agent_stream_text, &normalized, append_to_last) {
                AgentStreamOp::Ignore => {
                    block.agent_stream_text = normalized;
                    return;
                }
                AgentStreamOp::Append => {
                    block.agent_stream_text.push_str(&normalized);
                    append_agent_stream_delta(block, &normalized, &mut self.total_output_lines);
                    self.trim_output_lines();
                    self.request_scroll_to_bottom(cx);
                    return;
                }
                AgentStreamOp::Replace => {
                    replace_agent_stream_snapshot(block, &normalized, &mut self.total_output_lines);
                    self.trim_output_lines();
                    self.request_scroll_to_bottom(cx);
                    return;
                }
                AgentStreamOp::NewLines => {}
            }

            block.agent_stream_text.clear();
            block.agent_stream_line_index = None;
            let mut last_agent_line_index = None;

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
                last_agent_line_index = Some(block.output_lines.len() - 1);
            }
            block.agent_stream_line_index = last_agent_line_index;
        }
        self.trim_output_lines();
        self.request_scroll_to_bottom(cx);
    }

    fn run_agent_prompt(&mut self, prompt: String, cx: &mut Context<Self>) {
        if self.agent_busy {
            return;
        }
        let Some(row) = self.active_agent_row().cloned() else {
            self.blocks.push(Block {
                command: format!("agent> {prompt}"),
                output_lines: vec!["[agent] no selected agent.".to_string()],
                has_error: true,
                context: Some(BlockContext {
                    cwd: self.current_path.clone(),
                    git_branch: self.git_status.as_ref().map(|g| g.branch.clone()),
                    git_files: self.git_status.as_ref().map(|g| g.files_changed),
                    git_added: self.git_status.as_ref().map(|g| g.added),
                    git_deleted: self.git_status.as_ref().map(|g| g.deleted),
                    git_modified: self.git_status.as_ref().map(|g| g.modified),
                }),
                agent_placeholder_active: false,
                pending_permission: None,
                agent_stream_text: String::new(),
                agent_stream_line_index: None,
                agent_response: None,
                agent_response_line_count: 0,
            });
            self.selected_block = self.blocks.len().checked_sub(1);
            self.clear_output_selection();
            self.total_output_lines += 1;
            self.trim_output_lines();
            cx.notify();
            return;
        };
        let spec = row.spec.clone();
        let agent_key = row.agent_key.clone();
        let agent_label = self
            .active_agent_name()
            .unwrap_or_else(|| "Agent".to_string());
        self.push_history(&prompt);
        self.blocks.push(Block {
            command: format!("{agent_label}> {prompt}"),
            output_lines: vec![AGENT_CONNECTING_PLACEHOLDER.to_string()],
            has_error: false,
            context: Some(BlockContext {
                cwd: self.current_path.clone(),
                git_branch: self.git_status.as_ref().map(|g| g.branch.clone()),
                git_files: self.git_status.as_ref().map(|g| g.files_changed),
                git_added: self.git_status.as_ref().map(|g| g.added),
                git_deleted: self.git_status.as_ref().map(|g| g.deleted),
                git_modified: self.git_status.as_ref().map(|g| g.modified),
            }),
            agent_placeholder_active: true,
            pending_permission: None,
            agent_stream_text: String::new(),
            agent_stream_line_index: None,
            agent_response: None,
            agent_response_line_count: 0,
        });
        self.selected_block = self.blocks.len().checked_sub(1);
        self.clear_output_selection();
        self.total_output_lines += 1;
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
        cx.notify();

        let cached_client = self.agent_client.as_ref().and_then(|client| {
            if self
                .agent_client_key
                .as_ref()
                .map(|key| key == &agent_key)
                .unwrap_or(false)
            {
                Some(client.clone())
            } else {
                None
            }
        });

        let selected_key = self.agent_selected_key.clone();
        let runtime_preferences = self.runtime_preferences.clone();
        let session_override = self.selected_model_override.clone();
        let session_cwd = expand_tilde(&self.current_path)
            .to_string_lossy()
            .to_string();

        let (tx, mut rx) = mpsc::unbounded::<AgentPromptEvent>();
        thread::spawn(move || {
            let result = (|| -> Result<Option<AcpResponseText>, String> {
                let _ = tx.unbounded_send(AgentPromptEvent::Status(
                    AGENT_CONNECTING_PLACEHOLDER.to_string(),
                ));
                let client = if let Some(client) = cached_client {
                    client
                } else {
                    let client = Arc::new(Mutex::new(AcpClient::connect(&spec).map_err(
                        |err| {
                            format!(
                                "failed to spawn agent command '{}'. Check agents.json (Windows often needs `.cmd` shim like `codex.cmd`). Details: {err}",
                                spec.command
                            )
                        },
                    )?));
                    let _ = tx.unbounded_send(AgentPromptEvent::ClientReady {
                        client: client.clone(),
                        agent_key: agent_key.clone(),
                    });
                    client
                };
                let mut guard = client
                    .lock()
                    .map_err(|_| "agent lock poisoned".to_string())?;
                if guard.protocol_version.is_none() {
                    guard.initialize().map_err(|err| err.to_string())?;
                }
                let mut on_permission_request =
                    |request: PermissionRequest| -> Result<PermissionDecision> {
                        let (response_tx, response_rx) = std::sync::mpsc::channel();
                        let _ = tx.unbounded_send(AgentPromptEvent::PermissionRequest(
                            AgentPermissionEvent {
                                request: request.clone(),
                                response_tx,
                            },
                        ));
                        response_rx
                            .recv()
                            .map_err(|_| anyhow!("permission request was dropped"))
                    };
                let _ = tx.unbounded_send(AgentPromptEvent::Status(
                    AGENT_STARTING_SESSION_PLACEHOLDER.to_string(),
                ));
                let runtime_mcp = crate::mcp::probe::load_enabled_runtime_mcp_servers();
                if let Some(ref key) = selected_key {
                    let mut prefs = runtime_preferences
                        .lock()
                        .map_err(|_| "runtime preferences lock poisoned".to_string())?;
                    let bootstrap = guard
                        .ensure_session(&session_cwd, &runtime_mcp, session_override.as_deref())
                        .map_err(|err| err.to_string())?;
                    let catalog = bootstrap.model_options;
                    let ids = catalog
                        .iter()
                        .map(|model| model.id.clone())
                        .collect::<Vec<_>>();
                    let _ = prefs.ensure_default_model_valid(key, &ids);
                    let persisted_default = prefs.default_model_for(key);
                    let _ = tx.unbounded_send(AgentPromptEvent::Models(catalog.clone()));
                    let effective_model = model_discovery::resolve_selected_model(
                        session_override.as_deref(),
                        persisted_default.as_deref(),
                        &catalog,
                    );
                    if let Some(model_id) = effective_model.as_deref() {
                        let _ = guard.set_session_config_option(
                            &bootstrap.session_id,
                            "model",
                            model_id,
                        );
                    }
                    let _ = tx.unbounded_send(AgentPromptEvent::Status(
                        AGENT_SENDING_PROMPT_PLACEHOLDER.to_string(),
                    ));
                    let mut on_update = |text: String, append: bool| {
                        let _ = tx.unbounded_send(AgentPromptEvent::Update { text, append });
                    };
                    return guard
                        .prompt(
                            &bootstrap.session_id,
                            &prompt,
                            &mut on_update,
                            &mut on_permission_request,
                        )
                        .map_err(|err| err.to_string());
                }
                let bootstrap = guard
                    .ensure_session(&session_cwd, &runtime_mcp, session_override.as_deref())
                    .map_err(|err| err.to_string())?;
                let _ =
                    tx.unbounded_send(AgentPromptEvent::Models(bootstrap.model_options.clone()));
                let _ = tx.unbounded_send(AgentPromptEvent::Status(
                    AGENT_SENDING_PROMPT_PLACEHOLDER.to_string(),
                ));
                let mut on_update = |text: String, append: bool| {
                    let _ = tx.unbounded_send(AgentPromptEvent::Update { text, append });
                };
                guard
                    .prompt(
                        &bootstrap.session_id,
                        &prompt,
                        &mut on_update,
                        &mut on_permission_request,
                    )
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
                                AgentPromptEvent::ClientReady { client, agent_key } => {
                                    view.agent_client = Some(client);
                                    view.agent_client_key = Some(agent_key);
                                }
                                AgentPromptEvent::PermissionRequest(request) => {
                                    view.update_last_agent_placeholder(
                                        "[agent] awaiting permission...",
                                        cx,
                                    );
                                    if let Some(block) = view.blocks.last_mut() {
                                        block.pending_permission = Some(AgentPermissionPrompt {
                                            request: request.request,
                                            response_tx: request.response_tx,
                                        });
                                    }
                                }
                                AgentPromptEvent::Status(status) => {
                                    view.update_last_agent_placeholder(&status, cx);
                                }
                                AgentPromptEvent::Update { text, append } => {
                                    view.append_agent_update(&text, append, cx)
                                }
                                AgentPromptEvent::Models(models) => {
                                    view.model_options_loading = false;
                                    view.discovered_models = models;
                                }
                                AgentPromptEvent::Done(result) => {
                                    view.agent_busy = false;
                                    match result {
                                        Ok(Some(final_text)) => {
                                            view.agent_needs_auth =
                                                Self::is_auth_related_error(final_text.text());
                                            if view.last_block_has_agent_placeholder() {
                                                view.replace_last_agent_placeholder(final_text, cx);
                                            }
                                        }
                                        Ok(None) => {
                                            if view.last_block_has_agent_placeholder() {
                                                view.replace_last_agent_placeholder(
                                                    AcpResponseText::Plain(
                                                        "[agent] no response received.".to_string(),
                                                    ),
                                                    cx,
                                                );
                                            }
                                        }
                                        Err(err) => {
                                            view.agent_needs_auth =
                                                Self::is_auth_related_error(&err);
                                            let message = format!("[agent] {err}");
                                            if view.last_block_has_agent_placeholder() {
                                                view.replace_last_agent_placeholder(
                                                    AcpResponseText::Plain(message),
                                                    cx,
                                                );
                                            } else {
                                                view.append_agent_update_line(&message, cx);
                                            }
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

    fn cancel_agent_prompt(&mut self, cx: &mut Context<Self>) {
        if !self.agent_busy {
            return;
        }
        self.agent_busy = false;
        self.append_agent_update_line("[agent] cancel requested", cx);
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
            agent_placeholder_active: false,
            pending_permission: None,
            agent_stream_text: String::new(),
            agent_stream_line_index: None,
            agent_response: None,
            agent_response_line_count: 0,
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
            Overlay::Agent(picker) => self.render_agent_picker(picker, cx),
            Overlay::Model(picker) => self.render_model_picker(picker, cx),
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
            .child(
                panel.on_mouse_down(gpui::MouseButton::Left, |_event, _window, cx| {
                    cx.stop_propagation();
                }),
            )
    }

    fn render_picker_query_input(&self, query: &PickerQueryState, placeholder: &str) -> Div {
        let caret = div()
            .w(px(2.0))
            .h(px(14.0))
            .rounded(px(1.0))
            .bg(rgb(0x6b9eff));

        let text_normal = |text: String| {
            div()
                .text_size(px(12.0))
                .text_color(rgb(0xcccccc))
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
                        .text_size(px(12.0))
                        .text_color(rgb(0xf0f0f0))
                        .font_family("Cascadia Code")
                        .child(text),
                )
        };

        let content = if query.text.is_empty() {
            div().flex().items_center().gap(px(6.0)).child(caret).child(
                div()
                    .text_size(px(12.0))
                    .text_color(rgb(0x666666))
                    .child(placeholder.to_string()),
            )
        } else if let Some((a, b)) = query.normalized_selection().filter(|(a, b)| a != b) {
            let (pre, rest) = TextEditState::split_at_cursor(&query.text, a);
            let (sel, post) = TextEditState::split_at_cursor(&rest, b.saturating_sub(a));
            if query.cursor <= a {
                let (left, right) = TextEditState::split_at_cursor(&pre, query.cursor.min(a));
                div()
                    .flex()
                    .items_center()
                    .gap(px(0.0))
                    .child(text_normal(left))
                    .child(caret)
                    .child(text_normal(right))
                    .child(text_selected(sel))
                    .child(text_normal(post))
            } else if query.cursor >= b {
                let offset = query.cursor.saturating_sub(b);
                let (left, right) =
                    TextEditState::split_at_cursor(&post, offset.min(post.chars().count()));
                div()
                    .flex()
                    .items_center()
                    .gap(px(0.0))
                    .child(text_normal(pre))
                    .child(text_selected(sel))
                    .child(text_normal(left))
                    .child(caret)
                    .child(text_normal(right))
            } else {
                let offset = query.cursor.saturating_sub(a);
                let (left, right) =
                    TextEditState::split_at_cursor(&sel, offset.min(sel.chars().count()));
                div()
                    .flex()
                    .items_center()
                    .gap(px(0.0))
                    .child(text_normal(pre))
                    .child(text_selected(left))
                    .child(caret)
                    .child(text_selected(right))
                    .child(text_normal(post))
            }
        } else {
            let (left, right) = TextEditState::split_at_cursor(&query.text, query.cursor);
            div()
                .flex()
                .items_center()
                .gap(px(0.0))
                .child(text_normal(left))
                .child(caret)
                .child(text_normal(right))
        };

        div()
            .px(px(10.0))
            .py(px(6.0))
            .rounded(px(6.0))
            .bg(rgb(0x111111))
            .border_1()
            .border_color(rgb(0x252525))
            .cursor(text_input_cursor())
            .child(content)
    }

    fn render_path_picker(&self, picker: &PathPickerState, cx: &Context<Self>) -> Div {
        let handle = cx.entity().downgrade();

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
            .child(self.render_picker_query_input(&picker.query, "Search directories..."))
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
        let current = self.git_status.as_ref().map(|g| g.branch.clone());

        let truncate_branch = |label: &str| {
            if label.len() > 32 {
                format!("...{}", &label[label.len() - 29..])
            } else {
                label.to_string()
            }
        };

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
                        .child(truncate_branch(branch)),
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
            .child(self.render_picker_query_input(&picker.query, "Search branches..."))
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

    fn render_agent_picker(&self, picker: &AgentPickerState, cx: &Context<Self>) -> Div {
        let handle = cx.entity().downgrade();
        let search_visible = picker_has_search_input(PickerKind::Agent, picker.all_options.len());

        let header = if search_visible {
            self.render_picker_query_input(&picker.query, "Search ACPs...")
        } else {
            div()
                .px(px(10.0))
                .py(px(6.0))
                .rounded(px(6.0))
                .bg(rgb(0x111111))
                .border_1()
                .border_color(rgb(0x252525))
                .text_size(px(12.0))
                .text_color(rgb(0xaaaaaa))
                .child("Select ACP")
        };

        let items = picker.options.iter().enumerate().map(|(i, row)| {
            let is_active = i == picker.selected;
            let icon_present = self
                .registry_manifests
                .get(&row.acp_id)
                .and_then(|manifest| manifest.icon.as_ref())
                .is_some();
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
                .cursor(clickable_cursor())
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .child(registry_avatar(&row.name, icon_present))
                        .child(
                            div()
                                .text_size(px(13.0))
                                .text_color(rgb(0xeeeeee))
                                .child(row.name.clone()),
                        ),
                )
                .child(
                    div()
                        .text_size(px(11.0))
                        .text_color(rgb(0x8d8d8d))
                        .child(row.acp_id.clone()),
                )
                .on_mouse_down(gpui::MouseButton::Left, {
                    let handle = handle.clone();
                    move |_event, _window, cx| {
                        let _ = handle.update(cx, |view, cx| {
                            view.on_agent_picker_select(i, cx);
                        });
                    }
                })
        });

        div()
            .absolute()
            .left(px(116.0))
            .bottom(px(120.0))
            .w(px(360.0))
            .rounded(px(10.0))
            .bg(rgb(0x171717))
            .border_1()
            .border_color(rgb(0x2a2a2a))
            .p(px(10.0))
            .flex()
            .flex_col()
            .gap(px(8.0))
            .child(header)
            .child(
                div()
                    .id("agent_picker_list")
                    .flex_col()
                    .gap(px(6.0))
                    .max_h(px(260.0))
                    .overflow_y_scroll()
                    .children(items),
            )
    }

    fn render_model_picker(&self, picker: &ModelPickerState, cx: &Context<Self>) -> Div {
        let handle = cx.entity().downgrade();
        let search_visible = picker_has_search_input(PickerKind::Model, picker.all_options.len());

        let header = if search_visible {
            self.render_picker_query_input(&picker.query, "Search models...")
        } else {
            div()
                .px(px(10.0))
                .py(px(6.0))
                .rounded(px(6.0))
                .bg(rgb(0x111111))
                .border_1()
                .border_color(rgb(0x252525))
                .text_size(px(12.0))
                .text_color(rgb(0xaaaaaa))
                .child("Select model")
        };

        let items = picker.options.iter().enumerate().map(|(i, model)| {
            let is_active = i == picker.selected;
            div()
                .flex()
                .flex_col()
                .gap(px(4.0))
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
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap(px(8.0))
                        .child(
                            div()
                                .text_size(px(13.0))
                                .text_color(rgb(0xeeeeee))
                                .child(model.label.clone()),
                        )
                        .child(if model.is_default {
                            div()
                                .px(px(6.0))
                                .py(px(2.0))
                                .rounded(px(5.0))
                                .bg(rgb(0x1c2b1a))
                                .border_1()
                                .border_color(rgb(0x335f2d))
                                .text_size(px(10.0))
                                .text_color(rgb(0xb7e3a1))
                                .child("Default")
                        } else {
                            div()
                        }),
                )
                .child(if let Some(description) = model.description.clone() {
                    div()
                        .text_size(px(11.0))
                        .text_color(rgb(0x8d8d8d))
                        .child(description)
                } else {
                    div()
                })
                .on_mouse_down(gpui::MouseButton::Left, {
                    let handle = handle.clone();
                    move |_event, _window, cx| {
                        let _ = handle.update(cx, |view, cx| {
                            view.on_model_picker_select(i, cx);
                        });
                    }
                })
        });

        div()
            .absolute()
            .left(px(320.0))
            .bottom(px(120.0))
            .w(px(360.0))
            .rounded(px(10.0))
            .bg(rgb(0x171717))
            .border_1()
            .border_color(rgb(0x2a2a2a))
            .p(px(10.0))
            .flex()
            .flex_col()
            .gap(px(8.0))
            .child(header)
            .child(
                div()
                    .id("model_picker_list")
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

    fn on_agent_picker_select(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(Overlay::Agent(ref mut picker)) = self.overlay {
            picker.selected = index;
        }
        self.accept_overlay_selection(cx);
    }

    fn on_model_picker_select(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(Overlay::Model(ref mut picker)) = self.overlay {
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
            let command = command.to_string();
            thread::spawn(move || {
                let _ = Self::append_history_line(&path, &command);
            });
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
                agent_placeholder_active: false,
                pending_permission: None,
                agent_stream_text: String::new(),
                agent_stream_line_index: None,
                agent_response: None,
                agent_response_line_count: 0,
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

        let mut batch = Vec::new();
        let mut append_first_to_last = false;
        for (index, line) in lines.iter().enumerate() {
            let append_to_last = index == 0 && self.last_line_incomplete;
            if self.should_skip_output_line(line) {
                if append_to_last {
                    self.rollback_last_incomplete_output_line();
                }
                continue;
            }

            if batch.is_empty() {
                append_first_to_last = append_to_last;
            }
            batch.push((*line).to_string());
        }

        let appended_any =
            self.append_output_batch(&batch, append_first_to_last) && !batch.is_empty();
        self.last_line_incomplete = !ends_with_newline && appended_any;

        if appended_any {
            self.trim_output_lines();
            self.update_follow_output_from_scroll();
            self.request_scroll_to_bottom(cx);
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
        if trimmed.starts_with("PS ") && trimmed.ends_with('>') {
            return true;
        }

        let Some(prompt_char) = trimmed.chars().last() else {
            return false;
        };
        if !matches!(prompt_char, '$' | '#' | '%') {
            return false;
        }

        let body = trimmed[..trimmed.len() - prompt_char.len_utf8()].trim_end();
        if body.is_empty() {
            return false;
        }

        if body.contains('@') && body.contains(':') {
            return true;
        }

        let looks_like_path = body.starts_with("~/")
            || body.starts_with('/')
            || body.starts_with("./")
            || body.starts_with("../");
        if looks_like_path && !body.contains(char::is_whitespace) {
            return true;
        }

        false
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

    fn rollback_last_incomplete_output_line(&mut self) {
        if let Some(block) = self.blocks.last_mut()
            && !block.output_lines.is_empty()
        {
            block.output_lines.pop();
            self.total_output_lines = self.total_output_lines.saturating_sub(1);
        }
    }

    fn append_output_batch(&mut self, lines: &[String], append_first_to_last: bool) -> bool {
        if lines.is_empty() {
            return false;
        }

        let block = self.ensure_output_block();
        let added_lines = append_output_batch_to_block(block, lines, append_first_to_last);
        self.total_output_lines += added_lines;
        added_lines > 0 || append_first_to_last
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
            .cursor(CursorStyle::IBeam)
            .px(px(2.0))
            .child(
                div().flex_col().gap(px(0.0)).children(
                    wrap_terminal_text_lines(line, 96)
                        .into_iter()
                        .map(|wrapped| div().min_w(px(0.0)).child(wrapped)),
                ),
            )
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

    fn permission_prompt_options(request: &PermissionRequest) -> Vec<PermissionOption> {
        if !request.options.is_empty() {
            return request.options.clone();
        }

        vec![
            PermissionOption {
                id: "allow_once".into(),
                name: "Permitir uma vez".into(),
                description: Some("Allow this action only for this turn.".into()),
            },
            PermissionOption {
                id: "allow_always".into(),
                name: "Sempre permitir".into(),
                description: Some("Allow this action for future turns.".into()),
            },
            PermissionOption {
                id: "reject".into(),
                name: "Negar".into(),
                description: Some("Reject this request.".into()),
            },
        ]
    }

    fn permission_status_text(decision: &PermissionDecision) -> &'static str {
        match decision {
            PermissionDecision::Selected { .. } => "[agent] permission granted",
            PermissionDecision::Cancelled => "[agent] permission denied",
        }
    }

    fn resolve_permission_prompt(
        &mut self,
        block_index: usize,
        decision: PermissionDecision,
        cx: &mut Context<Self>,
    ) {
        let is_last_block = block_index == self.blocks.len().saturating_sub(1);
        let Some(block) = self.blocks.get_mut(block_index) else {
            return;
        };
        let Some(pending) = block.pending_permission.take() else {
            return;
        };

        let _ = pending.response_tx.send(decision.clone());
        let status = Self::permission_status_text(&decision);

        if is_last_block {
            if !update_agent_placeholder_block(block, status) {
                block.output_lines.push(status.to_string());
                self.total_output_lines += 1;
            }
        } else {
            block.output_lines.push(status.to_string());
            self.total_output_lines += 1;
        }
        self.trim_output_lines();
        self.request_scroll_to_bottom(cx);
        cx.notify();
    }

    fn render_permission_prompt(
        &self,
        block_index: usize,
        block: &Block,
        cx: &Context<Self>,
    ) -> Div {
        let Some(pending) = block.pending_permission.as_ref() else {
            return div();
        };

        let request = &pending.request;
        let title = request
            .title
            .clone()
            .unwrap_or_else(|| "Permission request".to_string());
        let description = request.description.clone();
        let tool_name = request.tool_name.clone();
        let options = Self::permission_prompt_options(request);
        let handle = cx.entity().downgrade();

        let mut card = div()
            .mt(px(4.0))
            .px(px(10.0))
            .py(px(10.0))
            .rounded(px(8.0))
            .border_1()
            .border_color(rgb(0x27485f))
            .bg(rgb(0x0d1620))
            .flex()
            .flex_col()
            .gap(px(8.0));

        card = card.child(
            div()
                .text_size(px(12.0))
                .font_weight(FontWeight::BOLD)
                .text_color(rgb(0xe4f1ff))
                .child(title),
        );

        if let Some(tool_name) = tool_name {
            card = card.child(
                div()
                    .text_size(px(11.0))
                    .text_color(rgb(0x9bb4c9))
                    .child(format!("Tool: {tool_name}")),
            );
        }

        if let Some(description) = description {
            card = card.child(
                div()
                    .text_size(px(11.0))
                    .text_color(rgb(0xc0cfdd))
                    .whitespace_normal()
                    .child(description),
            );
        }

        let buttons = options.into_iter().map(move |option| {
            let option_id = option.id.clone();
            let option_label = option.name.clone();
            let option_description = option.description.clone();
            let handle = handle.clone();
            let mut button = div()
                .flex_1()
                .min_w(px(0.0))
                .px(px(10.0))
                .py(px(8.0))
                .rounded(px(7.0))
                .border_1()
                .border_color(rgb(0x2b3e50))
                .bg(rgb(0x111a24))
                .hover(|this| this.bg(rgb(0x162432)).border_color(rgb(0x3d5a75)))
                .cursor(CursorStyle::PointingHand)
                .flex()
                .flex_col()
                .items_start()
                .gap(px(3.0))
                .child(
                    div()
                        .text_size(px(11.0))
                        .font_weight(FontWeight::BOLD)
                        .text_color(rgb(0xeaf2ff))
                        .child(option_label),
                );

            if let Some(description) = option_description {
                button = button.child(
                    div()
                        .text_size(px(10.0))
                        .text_color(rgb(0xa6bbcf))
                        .whitespace_normal()
                        .child(description),
                );
            }

            button.on_mouse_down(MouseButton::Left, move |_event, _window, cx| {
                let _ = handle.update(cx, |view, cx| {
                    view.resolve_permission_prompt(
                        block_index,
                        PermissionDecision::Selected {
                            option_id: option_id.clone(),
                        },
                        cx,
                    );
                });
                cx.stop_propagation();
            })
        });

        card.child(div().flex().gap(px(6.0)).children(buttons))
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

        let response_line_count = block
            .agent_response_line_count
            .min(block.output_lines.len());
        let output_lines = if response_line_count == 0 {
            block.output_lines.as_slice()
        } else {
            &block.output_lines[..block.output_lines.len() - response_line_count]
        };
        let output = if output_lines.is_empty() {
            div()
        } else {
            let (render_start, visible_lines) =
                renderable_output_window(output_lines, MAX_RENDERED_OUTPUT_LINES_PER_BLOCK);
            div().flex_col().gap(px(2.0)).text_size(px(12.0)).children(
                (render_start > 0)
                    .then(|| {
                        self.render_output_line(
                            &format!(
                                "[... {} earlier lines hidden for performance ...]",
                                render_start
                            ),
                            false,
                            index,
                            render_start.saturating_sub(1),
                            cx,
                        )
                    })
                    .into_iter()
                    .chain(
                        visible_lines
                            .iter()
                            .enumerate()
                            .map(|(visible_index, line)| {
                                let line_index = render_start + visible_index;
                                self.render_output_line(
                                    line,
                                    block.has_error,
                                    index,
                                    line_index,
                                    cx,
                                )
                            }),
                    ),
            )
        };
        let agent_response =
            render_agent_response_content(block.agent_response.as_ref(), block.has_error);
        let permission_prompt = self.render_permission_prompt(index, block, cx);

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
                    .child(output)
                    .child(agent_response)
                    .child(permission_prompt),
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
                .child(self.render_terminal_workspace(window, cx));
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

fn wrap_terminal_line(line: &str, max_chars: usize) -> Vec<String> {
    if max_chars == 0 || line.is_empty() {
        return vec![line.to_string()];
    }

    let mut wrapped = Vec::new();
    let mut chunk = String::new();
    let mut chunk_len = 0usize;

    for ch in line.chars() {
        chunk.push(ch);
        chunk_len += 1;

        if chunk_len >= max_chars {
            wrapped.push(std::mem::take(&mut chunk));
            chunk_len = 0;
        }
    }

    if !chunk.is_empty() {
        wrapped.push(chunk);
    }

    if wrapped.is_empty() {
        wrapped.push(String::new());
    }

    wrapped
}

fn wrap_terminal_text_lines(text: &str, max_chars: usize) -> Vec<String> {
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    let mut wrapped = Vec::new();

    for line in normalized.split('\n') {
        wrapped.extend(wrap_terminal_line(line, max_chars));
    }

    if wrapped.is_empty() {
        wrapped.push(String::new());
    }

    wrapped
}

fn renderable_output_window(lines: &[String], max_lines: usize) -> (usize, &[String]) {
    if lines.len() <= max_lines {
        return (0, lines);
    }

    let start = lines.len() - max_lines;
    (start, &lines[start..])
}

fn markdown_response_line_count(text: &str) -> usize {
    let count = text
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .lines()
        .count();
    count.max(1)
}

fn render_agent_response_content(response: Option<&AcpResponseText>, has_error: bool) -> Div {
    let Some(response) = response else {
        return div();
    };

    let container = div()
        .mt(px(4.0))
        .px(px(10.0))
        .py(px(10.0))
        .rounded(px(8.0))
        .border_1()
        .border_color(rgb(0x355c7d))
        .bg(rgb(0x0d1620))
        .flex()
        .flex_col()
        .gap(px(6.0));

    let body = render_markdown_response_content(response.text(), has_error);

    container.child(body)
}

fn render_markdown_response_content(text: &str, has_error: bool) -> Div {
    let blocks = parse_markdown_blocks(text);
    if markdown_should_fallback_to_raw(text, &blocks) {
        return render_markdown_raw_content(text, has_error);
    }
    let base_color = if has_error {
        rgb(0xffd0d0)
    } else {
        rgb(0xd8e6f2)
    };

    div().flex_col().gap(px(8.0)).children(
        blocks
            .into_iter()
            .map(|block| render_markdown_block(block, base_color.into())),
    )
}

fn render_markdown_raw_content(text: &str, has_error: bool) -> Div {
    let color = if has_error {
        rgb(0xffd0d0)
    } else {
        rgb(0xd8e6f2)
    };
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    if let Some(items) = extract_compact_list_items(&normalized) {
        return render_markdown_compact_list(items, color);
    }

    div()
        .flex_col()
        .gap(px(2.0))
        .px(px(10.0))
        .py(px(8.0))
        .rounded(px(6.0))
        .bg(rgb(0x111820))
        .border_1()
        .border_color(rgb(0x253545))
        .font_family("Cascadia Code")
        .text_color(color)
        .children(normalized.lines().map(|line| {
            if line.is_empty() {
                div().h(px(12.0))
            } else {
                div().min_w(px(0.0)).child(line.to_string())
            }
        }))
}

fn render_markdown_compact_list(items: Vec<String>, color: Rgba) -> Div {
    div()
        .flex_col()
        .gap(px(4.0))
        .px(px(10.0))
        .py(px(8.0))
        .rounded(px(6.0))
        .bg(rgb(0x111820))
        .border_1()
        .border_color(rgb(0x253545))
        .font_family("Cascadia Code")
        .text_color(color)
        .children(items.into_iter().map(|item| {
            div()
                .flex()
                .items_start()
                .gap(px(8.0))
                .child(div().flex_none().font_weight(FontWeight::BOLD).child("•"))
                .child(div().min_w(px(0.0)).child(item))
        }))
}

fn extract_compact_list_items(text: &str) -> Option<Vec<String>> {
    let trimmed = text.trim();
    if trimmed.is_empty() || trimmed.contains('\n') {
        return None;
    }
    if trimmed.contains("```") || trimmed.starts_with('#') {
        return None;
    }

    let mut items = Vec::new();
    let mut rest = trimmed;

    loop {
        let rest_trimmed = rest.trim_start_matches(|ch: char| ch == ',' || ch.is_whitespace());
        if rest_trimmed.is_empty() {
            break;
        }
        rest = rest_trimmed;

        let Some(open) = rest.find('`') else {
            return None;
        };
        if open > 0 {
            let prefix = rest[..open].trim();
            if !prefix.is_empty() && items.is_empty() {
                return None;
            }
        }

        let after_open = &rest[open + 1..];
        let Some(close) = after_open.find('`') else {
            return None;
        };
        let item = after_open[..close].trim().to_string();
        if item.is_empty() {
            return None;
        }
        items.push(item);
        rest = &after_open[close + 1..];

        let rest_check = rest.trim_start();
        if rest_check.is_empty() {
            break;
        }
        if !rest_check.starts_with(',') {
            return None;
        }
    }

    if items.len() >= 2 { Some(items) } else { None }
}

fn markdown_should_fallback_to_raw(text: &str, blocks: &[MarkdownBlock]) -> bool {
    // Structural markdown elements → always keep markdown rendering.
    if blocks.iter().any(|block| {
        matches!(
            block,
            MarkdownBlock::Heading { .. }
                | MarkdownBlock::ListItem { .. }
                | MarkdownBlock::CodeBlock { .. }
        )
    }) {
        return false;
    }

    // Check for paired inline markers (not just single chars).
    let has_bold = text.contains("**");
    let has_inline_code = {
        if let Some(first) = text.find('`') {
            text[first + 1..].contains('`')
        } else {
            false
        }
    };
    let has_link = text.contains('[') && text.contains("](");

    let has_inline_markers = has_bold || has_inline_code || has_link;
    if !has_inline_markers {
        return true;
    }

    // Odd backtick count suggests malformed inline code, but only fall back
    // if there are no other strong inline markers to render.
    let backtick_count = text.chars().filter(|ch| *ch == '`').count();
    if backtick_count % 2 != 0 && !has_bold && !has_link {
        return true;
    }

    false
}

fn render_markdown_block(block: MarkdownBlock, base_color: gpui::Hsla) -> Div {
    match block {
        MarkdownBlock::Heading { level, text } => {
            let size = match level {
                1 => 17.0,
                2 => 15.5,
                _ => 14.0,
            };
            div()
                .flex_col()
                .gap(px(2.0))
                .text_color(base_color)
                .text_size(px(size))
                .font_weight(FontWeight::BOLD)
                .children(vec![render_markdown_inline_line(&text, base_color)])
        }
        MarkdownBlock::Paragraph(text) => render_markdown_inline_line(&text, base_color),
        MarkdownBlock::ListItem {
            ordered,
            index,
            text,
        } => {
            let prefix = if ordered {
                format!("{}.", index.unwrap_or(1))
            } else {
                "•".to_string()
            };
            div()
                .flex()
                .items_start()
                .gap(px(8.0))
                .text_color(base_color)
                .children(vec![
                    div()
                        .flex_none()
                        .font_weight(FontWeight::BOLD)
                        .child(prefix),
                    render_markdown_inline_line(&text, base_color),
                ])
        }
        MarkdownBlock::CodeBlock { language, lines } => {
            let mut code = div()
                .flex_col()
                .gap(px(2.0))
                .px(px(10.0))
                .py(px(8.0))
                .rounded(px(6.0))
                .bg(rgb(0x111820))
                .border_1()
                .border_color(rgb(0x253545))
                .font_family("Cascadia Code")
                .text_color(rgb(0xcfd8e3));

            if let Some(language) = language {
                code = code.child(
                    div()
                        .text_size(px(10.0))
                        .text_color(rgb(0x8aa1b3))
                        .child(language),
                );
            }

            code.children(
                lines
                    .into_iter()
                    .map(|line| div().font_family("Cascadia Code").child(line)),
            )
        }
    }
}

fn render_markdown_inline_line(text: &str, base_color: gpui::Hsla) -> Div {
    let segments = parse_markdown_inline(text);
    div()
        .flex()
        .flex_wrap()
        .gap(px(0.0))
        .text_color(base_color)
        .children(
            segments
                .into_iter()
                .map(|segment| render_markdown_inline_segment(segment)),
        )
}

fn render_markdown_inline_segment(segment: MarkdownInlineSegment) -> Div {
    match segment {
        MarkdownInlineSegment::Text(text) => div().min_w(px(0.0)).child(text),
        MarkdownInlineSegment::Strong(text) => div()
            .min_w(px(0.0))
            .font_weight(FontWeight::BOLD)
            .child(text),
        MarkdownInlineSegment::Emphasis(text) => div().min_w(px(0.0)).child(text),
        MarkdownInlineSegment::Code(text) => div()
            .min_w(px(0.0))
            .font_family("Cascadia Code")
            .px(px(4.0))
            .py(px(1.0))
            .rounded(px(4.0))
            .bg(rgb(0x1a2430))
            .text_color(rgb(0xd6e1ec))
            .child(text),
        MarkdownInlineSegment::Link { label, url } => div()
            .min_w(px(0.0))
            .text_color(rgb(0x7fb3ff))
            .child(format!("{label} ({url})")),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum MarkdownBlock {
    Heading {
        level: usize,
        text: String,
    },
    Paragraph(String),
    ListItem {
        ordered: bool,
        index: Option<usize>,
        text: String,
    },
    CodeBlock {
        language: Option<String>,
        lines: Vec<String>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum MarkdownInlineSegment {
    Text(String),
    Strong(String),
    Emphasis(String),
    Code(String),
    Link { label: String, url: String },
}

fn parse_markdown_blocks(text: &str) -> Vec<MarkdownBlock> {
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    let mut blocks = Vec::new();
    let mut paragraph_lines = Vec::new();
    let lines: Vec<&str> = normalized.lines().collect();
    let mut index = 0;

    let flush_paragraph = |blocks: &mut Vec<MarkdownBlock>, paragraph_lines: &mut Vec<String>| {
        if paragraph_lines.is_empty() {
            return;
        }
        blocks.push(MarkdownBlock::Paragraph(paragraph_lines.join(" ")));
        paragraph_lines.clear();
    };

    while index < lines.len() {
        let line = lines[index];
        let trimmed = line.trim_end();
        let stripped = trimmed.trim_start();

        if stripped.is_empty() {
            flush_paragraph(&mut blocks, &mut paragraph_lines);
            index += 1;
            continue;
        }

        if let Some((language, consumed)) = parse_markdown_fence_start(stripped) {
            flush_paragraph(&mut blocks, &mut paragraph_lines);
            let mut code_lines = Vec::new();
            index += 1;
            while index < lines.len() {
                let current = lines[index].trim_end();
                if current.trim_start().starts_with("```") {
                    break;
                }
                code_lines.push(current.to_string());
                index += 1;
            }
            blocks.push(MarkdownBlock::CodeBlock {
                language,
                lines: code_lines,
            });
            if consumed {
                index += 1;
            }
            continue;
        }

        if let Some((level, heading_text)) = parse_markdown_heading(stripped) {
            flush_paragraph(&mut blocks, &mut paragraph_lines);
            blocks.push(MarkdownBlock::Heading {
                level,
                text: heading_text,
            });
            index += 1;
            continue;
        }

        if let Some((ordered, list_index, list_text)) = parse_markdown_list_item(stripped) {
            flush_paragraph(&mut blocks, &mut paragraph_lines);
            blocks.push(MarkdownBlock::ListItem {
                ordered,
                index: list_index,
                text: list_text,
            });
            index += 1;
            continue;
        }

        paragraph_lines.push(stripped.to_string());
        index += 1;
    }

    flush_paragraph(&mut blocks, &mut paragraph_lines);
    blocks
}

fn parse_markdown_fence_start(line: &str) -> Option<(Option<String>, bool)> {
    let trimmed = line.trim_start();
    if !trimmed.starts_with("```") {
        return None;
    }

    let language = trimmed
        .trim_start_matches("```")
        .trim()
        .chars()
        .next()
        .map(|_| trimmed.trim_start_matches("```").trim().to_string())
        .filter(|lang| !lang.is_empty());

    Some((language, true))
}

fn parse_markdown_heading(line: &str) -> Option<(usize, String)> {
    let count = line.chars().take_while(|ch| *ch == '#').count();
    if !(1..=6).contains(&count) {
        return None;
    }
    let rest = line[count..].trim_start();
    if rest.is_empty() {
        return None;
    }
    Some((count, rest.to_string()))
}

fn parse_markdown_list_item(line: &str) -> Option<(bool, Option<usize>, String)> {
    if let Some(rest) = line.strip_prefix("- ") {
        return Some((false, None, rest.trim().to_string()));
    }
    if let Some(rest) = line.strip_prefix("* ") {
        return Some((false, None, rest.trim().to_string()));
    }
    if let Some(rest) = line.strip_prefix("+ ") {
        return Some((false, None, rest.trim().to_string()));
    }

    let mut digits = String::new();
    let mut chars = line.chars().peekable();
    while let Some(ch) = chars.peek().copied() {
        if ch.is_ascii_digit() {
            digits.push(ch);
            chars.next();
            continue;
        }
        break;
    }

    if digits.is_empty() || !matches!(chars.next(), Some('.')) || !matches!(chars.next(), Some(' '))
    {
        return None;
    }

    let text = chars.collect::<String>().trim().to_string();
    let index = digits.parse::<usize>().ok();
    Some((true, index, text))
}

fn parse_markdown_inline(text: &str) -> Vec<MarkdownInlineSegment> {
    let mut segments = Vec::new();
    let mut remaining = text;

    while !remaining.is_empty() {
        if let Some(rest) = remaining.strip_prefix("**") {
            if let Some(end) = rest.find("**") {
                segments.push(MarkdownInlineSegment::Strong(rest[..end].to_string()));
                remaining = &rest[end + 2..];
                continue;
            }
            segments.push(MarkdownInlineSegment::Text("*".to_string()));
            remaining = rest;
            continue;
        }

        if let Some(rest) = remaining.strip_prefix('*') {
            if !rest.starts_with('*')
                && let Some(end) = rest.find('*')
            {
                segments.push(MarkdownInlineSegment::Emphasis(rest[..end].to_string()));
                remaining = &rest[end + 1..];
                continue;
            }
            segments.push(MarkdownInlineSegment::Text("*".to_string()));
            remaining = rest;
            continue;
        }

        if let Some(rest) = remaining.strip_prefix('`')
            && let Some(end) = rest.find('`')
        {
            segments.push(MarkdownInlineSegment::Code(rest[..end].to_string()));
            remaining = &rest[end + 1..];
            continue;
        }
        if remaining.starts_with('`') {
            segments.push(MarkdownInlineSegment::Text("`".to_string()));
            remaining = &remaining[1..];
            continue;
        }

        if let Some(rest) = remaining.strip_prefix('[')
            && let Some(label_end) = rest.find("](")
        {
            let label = &rest[..label_end];
            let after = &rest[label_end + 2..];
            if let Some(url_end) = after.find(')') {
                segments.push(MarkdownInlineSegment::Link {
                    label: label.to_string(),
                    url: after[..url_end].to_string(),
                });
                remaining = &after[url_end + 1..];
                continue;
            }
        }
        if remaining.starts_with('[') {
            segments.push(MarkdownInlineSegment::Text("[".to_string()));
            remaining = &remaining[1..];
            continue;
        }

        let next_marker = find_next_markdown_marker(remaining).unwrap_or(remaining.len());
        if next_marker == 0 {
            let mut chars = remaining.chars();
            if let Some(first) = chars.next() {
                segments.push(MarkdownInlineSegment::Text(first.to_string()));
                remaining = chars.as_str();
                continue;
            }
            break;
        }
        segments.push(MarkdownInlineSegment::Text(
            remaining[..next_marker].to_string(),
        ));
        remaining = &remaining[next_marker..];
    }

    segments
}

fn find_next_markdown_marker(text: &str) -> Option<usize> {
    let mut next: Option<usize> = None;
    for marker in ["**", "*", "`", "["] {
        if let Some(index) = text.find(marker) {
            next = Some(match next {
                Some(current) => current.min(index),
                None => index,
            });
        }
    }
    next
}

impl Focusable for TabView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl EventEmitter<TabViewEvent> for TabView {}

#[cfg(test)]
mod tests {
    use super::{
        AGENT_CONNECTING_PLACEHOLDER, AGENT_SENDING_PROMPT_PLACEHOLDER, AgentStreamOp, Block,
        FilePreviewKind, FilePreviewState, HighlightSegment, InitialFocusTarget, MarkdownBlock,
        MarkdownInlineSegment, ModelButtonState, PermissionDecision, PermissionRequest, PickerKind,
        PickerQueryState, PreviewLanguage, PreviewSearchMatchSegment, TabView,
        append_agent_stream_delta, append_output_batch_to_block, build_agent_picker_state,
        build_model_picker_state, classify_agent_stream_op, clickable_cursor, compute_row_state,
        compute_trigger_state, extract_compact_list_items, model_trigger_label,
        parse_markdown_blocks, parse_markdown_inline, picker_has_search_input,
        picker_header_is_static, picker_initial_focus_target, picker_typeahead_enabled,
        renderable_output_window, replace_agent_stream_snapshot, streaming_snapshot_delta,
        text_input_cursor, update_agent_placeholder_block, wrap_terminal_line,
        wrap_terminal_text_lines,
    };
    use crate::acp::manager::AgentSpec;
    use crate::acp::model_discovery::AcpModelOption;
    use crate::acp::resolve::{AgentKey, AgentSourceKind, EffectiveAgentRow};
    use gpui::{CursorStyle, ScrollDelta, point, px};
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use tempfile::tempdir;

    fn row(id: &str, name: &str) -> EffectiveAgentRow {
        EffectiveAgentRow {
            agent_key: AgentKey {
                source_type: AgentSourceKind::Registry,
                source_id: "managed".into(),
                acp_id: id.into(),
            },
            acp_id: id.into(),
            name: name.into(),
            source_type: AgentSourceKind::Registry,
            source_id: "managed".into(),
            spec: AgentSpec {
                id: id.into(),
                name: name.into(),
                command: format!("{id}.cmd"),
                args: Vec::new(),
                fixed_env: Default::default(),
                env_keys: Vec::new(),
                install: None,
                auth: None,
            },
            managed_state: None,
            is_selected: true,
            is_alternate: false,
        }
    }

    #[test]
    fn agent_picker_defaults_to_first_option_without_selected_key() {
        let picker =
            build_agent_picker_state(&[row("codex", "Codex"), row("claude", "Claude")], None)
                .expect("picker should exist");

        assert_eq!(picker.selected, 0);
        assert_eq!(picker.options.len(), 2);
        assert_eq!(picker.options[0].acp_id, "codex");
    }

    #[test]
    fn preview_terminal_height_is_clamped_to_session_limits() {
        assert_eq!(TabView::clamp_preview_terminal_height(120.0), 180.0);
        assert_eq!(TabView::clamp_preview_terminal_height(260.0), 260.0);
        assert_eq!(TabView::clamp_preview_terminal_height(720.0), 520.0);
    }

    #[test]
    fn prompt_detection_supports_powershell_and_posix_shells() {
        assert!(TabView::is_prompt_line("PS C:\\repo>"));
        assert!(TabView::is_prompt_line(
            "carlos@carlos-960XFH:~/projects/sympla/white-lion$"
        ));
        assert!(TabView::is_prompt_line("~/projects/OrbitShell$"));
    }

    #[test]
    fn prompt_detection_ignores_regular_output_lines() {
        assert!(!TabView::is_prompt_line("Downloads: $120"));
        assert!(!TabView::is_prompt_line("assets docs packages"));
        assert!(!TabView::is_prompt_line("error code #42"));
    }

    #[test]
    fn normalize_initial_terminal_command_ignores_empty_values() {
        assert_eq!(
            TabView::normalize_initial_terminal_command(Some("  ls  ")),
            Some("ls".to_string())
        );
        assert_eq!(
            TabView::normalize_initial_terminal_command(Some("   ")),
            None
        );
        assert_eq!(TabView::normalize_initial_terminal_command(None), None);
    }

    #[test]
    fn preview_search_match_uses_one_based_line_numbers_and_trimmed_query() {
        assert_eq!(
            TabView::build_preview_search_match(5, "  needle  ", 12),
            Some((4, "needle".to_string()))
        );
        assert_eq!(TabView::build_preview_search_match(0, "needle", 12), None);
        assert_eq!(TabView::build_preview_search_match(3, "   ", 12), None);
        assert_eq!(
            TabView::build_preview_search_match(40, "needle", 12),
            Some((11, "needle".to_string()))
        );
    }

    #[test]
    fn preview_search_match_segments_mark_the_matching_substring() {
        let segments = vec![HighlightSegment {
            text: "const needle = haystack".into(),
            color: 0x79c0ff,
            bold: false,
        }];

        assert_eq!(
            TabView::apply_preview_search_match_segments(&segments, "needle"),
            vec![
                PreviewSearchMatchSegment {
                    text: "const ".into(),
                    color: 0x79c0ff,
                    bold: false,
                    matched: false,
                },
                PreviewSearchMatchSegment {
                    text: "needle".into(),
                    color: 0x79c0ff,
                    bold: false,
                    matched: true,
                },
                PreviewSearchMatchSegment {
                    text: " = haystack".into(),
                    color: 0x79c0ff,
                    bold: false,
                    matched: false,
                },
            ]
        );
    }

    #[test]
    fn file_preview_uses_workspace_relative_label_for_text_files() {
        let temp = tempdir().expect("temp dir");
        let root = temp.path().join("workspace");
        let nested = root.join("docs");
        fs::create_dir_all(&nested).expect("create nested dir");
        let file_path = nested.join("README.md");
        fs::write(&file_path, "# OrbitShell\n").expect("write text file");

        let preview = TabView::build_file_preview_state(&file_path, Some(root.as_path()));

        assert_eq!(preview.relative_label, "docs/README.md");
        match preview.kind {
            FilePreviewKind::Text { contents, lines } => {
                assert_eq!(contents, "# OrbitShell\n");
                assert_eq!(lines.len(), 1);
                assert_eq!(lines[0], "# OrbitShell");
            }
            other => panic!(
                "expected text preview, got {:?}",
                std::mem::discriminant(&other)
            ),
        }
    }

    #[test]
    fn file_preview_marks_binary_files_as_unsupported() {
        let temp = tempdir().expect("temp dir");
        let file_path = temp.path().join("data.bin");
        fs::write(&file_path, [0_u8, 159, 146, 150]).expect("write binary file");

        let preview = TabView::build_file_preview_state(&file_path, Some(temp.path()));

        assert_eq!(preview.relative_label, "data.bin");
        match preview.kind {
            FilePreviewKind::Unsupported { message } => {
                assert!(message.contains("Binary file preview"));
            }
            other => panic!(
                "expected unsupported preview, got {:?}",
                std::mem::discriminant(&other)
            ),
        }
    }

    #[test]
    fn detect_preview_language_uses_extension_and_special_filenames() {
        assert_eq!(
            TabView::detect_preview_language(Path::new("/tmp/src/main.rs")),
            PreviewLanguage::Rust
        );
        assert_eq!(
            TabView::detect_preview_language(Path::new("/tmp/Cargo.toml")),
            PreviewLanguage::Toml
        );
        assert_eq!(
            TabView::detect_preview_language(Path::new("/tmp/.gitignore")),
            PreviewLanguage::PlainText
        );
        assert_eq!(
            TabView::detect_preview_language(Path::new("/tmp/readme.md")),
            PreviewLanguage::Markdown
        );
        assert_eq!(
            TabView::detect_preview_language(Path::new("/tmp/app.tsx")),
            PreviewLanguage::TypeScriptReact
        );
        assert_eq!(
            TabView::detect_preview_language(Path::new("/tmp/component.jsx")),
            PreviewLanguage::JavaScriptReact
        );
        assert_eq!(
            TabView::detect_preview_language(Path::new("/tmp/index.html")),
            PreviewLanguage::Html
        );
        assert_eq!(
            TabView::detect_preview_language(Path::new("/tmp/site.css")),
            PreviewLanguage::Css
        );
        assert_eq!(
            TabView::detect_preview_language(Path::new("/tmp/query.sql")),
            PreviewLanguage::Sql
        );
        assert_eq!(
            TabView::detect_preview_language(Path::new("/tmp/main.py")),
            PreviewLanguage::Python
        );
        assert_eq!(
            TabView::detect_preview_language(Path::new("/tmp/main.go")),
            PreviewLanguage::Go
        );
        assert_eq!(
            TabView::detect_preview_language(Path::new("/tmp/Dockerfile")),
            PreviewLanguage::Dockerfile
        );
    }

    #[test]
    fn rust_highlight_marks_keywords_strings_and_comments() {
        let segments = TabView::highlight_line(
            "fn main() { let name = \"Orbit\"; // comment }",
            PreviewLanguage::Rust,
        );
        let rendered = segments
            .iter()
            .map(|segment| (segment.text.as_str(), segment.color))
            .collect::<Vec<_>>();

        assert!(rendered.iter().any(|(text, _)| *text == "fn"));
        assert!(rendered.iter().any(|(text, _)| *text == "\"Orbit\""));
        assert!(
            rendered
                .iter()
                .any(|(text, _)| text.starts_with("// comment"))
        );
    }

    #[test]
    fn html_highlight_marks_tags_strings_and_comments() {
        let segments = TabView::highlight_line(
            "<div class=\"hero\">Hello</div><!-- note -->",
            PreviewLanguage::Html,
        );
        let rendered = segments
            .iter()
            .map(|segment| segment.text.as_str())
            .collect::<Vec<_>>();

        assert!(rendered.iter().any(|text| *text == "<div"));
        assert!(rendered.iter().any(|text| *text == "\"hero\""));
        assert!(
            rendered
                .iter()
                .any(|text| text.starts_with("<!-- note -->"))
        );
    }

    #[test]
    fn python_highlight_marks_keywords_numbers_and_comments() {
        let segments = TabView::highlight_line(
            "def load(path): return 42  # comment",
            PreviewLanguage::Python,
        );
        let rendered = segments
            .iter()
            .map(|segment| segment.text.as_str())
            .collect::<Vec<_>>();

        assert!(rendered.iter().any(|text| *text == "def"));
        assert!(rendered.iter().any(|text| *text == "return"));
        assert!(rendered.iter().any(|text| *text == "42"));
        assert!(rendered.iter().any(|text| text.starts_with("# comment")));
    }

    #[test]
    fn preview_select_all_collects_all_lines_for_copy() {
        let lines = vec![
            "first".to_string(),
            "second".to_string(),
            "third".to_string(),
        ];
        assert_eq!(
            TabView::selected_text_from_lines(&lines, Some(0), Some(2)).as_deref(),
            Some("first\nsecond\nthird")
        );
    }

    #[test]
    fn preview_keyboard_selection_expands_from_active_line() {
        let (active_line, anchor, head) =
            TabView::move_linear_selection(3, Some(0), Some(0), Some(0), 1, true);

        assert_eq!(active_line, Some(1));
        assert_eq!(
            TabView::normalize_linear_selection(anchor, head),
            Some((0, 1))
        );
    }

    #[test]
    fn markdown_preview_support_depends_on_language_and_text_kind() {
        let markdown = FilePreviewState {
            path: PathBuf::from("/tmp/README.md"),
            relative_label: "README.md".into(),
            language: PreviewLanguage::Markdown,
            kind: FilePreviewKind::Text {
                contents: "# Title".into(),
                lines: Arc::new(vec!["# Title".into()]),
            },
        };
        let rust = FilePreviewState {
            path: PathBuf::from("/tmp/main.rs"),
            relative_label: "main.rs".into(),
            language: PreviewLanguage::Rust,
            kind: FilePreviewKind::Text {
                contents: "fn main() {}".into(),
                lines: Arc::new(vec!["fn main() {}".into()]),
            },
        };

        assert!(TabView::preview_supports_rendered_markdown(&markdown));
        assert!(!TabView::preview_supports_rendered_markdown(&rust));
    }

    #[test]
    fn preview_scroll_uses_native_handling_for_all_input() {
        assert!(!TabView::preview_scroll_uses_custom_step(
            ScrollDelta::Lines(point(0.0, 1.0))
        ));
        assert!(!TabView::preview_scroll_uses_custom_step(
            ScrollDelta::Pixels(point(px(0.0), px(16.0),))
        ));
    }

    #[test]
    fn agent_picker_uses_selected_key_when_present() {
        let rows = vec![row("codex", "Codex"), row("claude", "Claude")];
        let selected_key = rows[1].agent_key.clone();

        let picker =
            build_agent_picker_state(&rows, Some(&selected_key)).expect("picker should exist");

        assert_eq!(picker.selected, 1);
        assert_eq!(picker.options[picker.selected].acp_id, "claude");
    }

    #[test]
    fn model_picker_uses_selected_model_when_present() {
        let catalog = vec![
            AcpModelOption {
                id: "gpt-5.3".into(),
                label: "GPT-5.3".into(),
                description: None,
                is_default: true,
            },
            AcpModelOption {
                id: "gpt-5.4".into(),
                label: "GPT-5.4".into(),
                description: None,
                is_default: false,
            },
        ];

        let picker =
            build_model_picker_state(&catalog, Some("gpt-5.4")).expect("picker should exist");

        assert_eq!(picker.selected, 1);
        assert_eq!(picker.options[picker.selected].id, "gpt-5.4");
    }

    #[test]
    fn conditional_search_picker_hides_search_for_five_or_fewer_items() {
        assert!(!picker_has_search_input(PickerKind::Agent, 5));
    }

    #[test]
    fn conditional_search_picker_shows_search_for_six_or_more_items() {
        assert!(picker_has_search_input(PickerKind::Agent, 6));
    }

    #[test]
    fn short_conditional_picker_uses_typeahead_without_search_input() {
        assert!(picker_typeahead_enabled(PickerKind::Agent, 5));
        assert!(picker_header_is_static(PickerKind::Agent, 5));
    }

    #[test]
    fn clickable_rows_and_triggers_report_pointer_semantics() {
        assert_eq!(clickable_cursor(), CursorStyle::PointingHand);
    }

    #[test]
    fn editable_search_inputs_report_text_semantics() {
        assert_eq!(text_input_cursor(), CursorStyle::IBeam);
    }

    #[test]
    fn conditional_picker_initial_focus_is_list() {
        assert_eq!(
            picker_initial_focus_target(PickerKind::Agent),
            InitialFocusTarget::List
        );
        assert_eq!(
            picker_initial_focus_target(PickerKind::Path),
            InitialFocusTarget::SearchInput
        );
    }

    #[test]
    fn selected_row_keeps_selected_state_when_also_highlighted() {
        let row = compute_row_state(true, true, false);
        assert!(row.is_selected);
        assert!(row.is_highlighted);
    }

    #[test]
    fn trigger_state_reports_loading_and_has_search_input() {
        let state = compute_trigger_state(true, false, true, true);
        assert!(state.loading);
        assert!(state.has_search_input);
    }

    #[test]
    fn model_trigger_label_returns_expected_texts() {
        assert_eq!(
            model_trigger_label(&ModelButtonState::Loading),
            ("Loading models...".to_string(), true)
        );
        assert_eq!(
            model_trigger_label(&ModelButtonState::Unavailable),
            ("No models available".to_string(), true)
        );
        assert_eq!(
            model_trigger_label(&ModelButtonState::Ready("GPT".to_string())),
            ("GPT".to_string(), false)
        );
    }

    #[test]
    fn model_picker_search_threshold() {
        assert!(!picker_has_search_input(PickerKind::Model, 5));
        assert!(picker_has_search_input(PickerKind::Model, 6));
    }

    #[test]
    fn permission_prompt_options_fallback_to_default_choices() {
        let request = PermissionRequest {
            request_id: 1,
            session_id: Some("sess_123".into()),
            title: Some("Approve".into()),
            description: None,
            tool_name: None,
            options: Vec::new(),
            raw_params: serde_json::json!({}),
        };

        let options = TabView::permission_prompt_options(&request);
        assert_eq!(options.len(), 3);
        assert_eq!(options[0].id, "allow_once");
        assert_eq!(options[2].name, "Negar");
    }

    #[test]
    fn permission_status_text_matches_decision() {
        assert_eq!(
            TabView::permission_status_text(&PermissionDecision::Selected {
                option_id: "allow_once".into()
            }),
            "[agent] permission granted"
        );
        assert_eq!(
            TabView::permission_status_text(&PermissionDecision::Cancelled),
            "[agent] permission denied"
        );
    }

    #[test]
    fn streaming_snapshot_delta_returns_only_new_suffix() {
        assert_eq!(
            streaming_snapshot_delta("Directory:\nassets", "Directory:\nassets\ndocs"),
            Some("\ndocs".to_string())
        );
        assert_eq!(streaming_snapshot_delta("same", "same"), None);
    }

    #[test]
    fn classify_agent_stream_op_detects_snapshot_growth_even_without_append_flag() {
        assert_eq!(
            classify_agent_stream_op("Directory:\nassets", "Directory:\nassets\ndocs", false),
            AgentStreamOp::Replace
        );
        assert_eq!(
            classify_agent_stream_op("same", "same", false),
            AgentStreamOp::Ignore
        );
        assert_eq!(
            classify_agent_stream_op("assets", "docs", false),
            AgentStreamOp::NewLines
        );
    }

    #[test]
    fn classify_agent_stream_op_treats_small_inline_chunks_as_append() {
        assert_eq!(
            classify_agent_stream_op("Vou listar", " o diretório", true),
            AgentStreamOp::Append
        );
    }

    #[test]
    fn replace_agent_stream_snapshot_replaces_existing_stream_block_in_place() {
        let mut block = Block {
            command: "Codex> ls".into(),
            output_lines: vec![
                "Directory: C:\\repo".into(),
                "assets".into(),
                "[agent stderr] sandbox retry".into(),
            ],
            has_error: true,
            context: None,
            agent_placeholder_active: false,
            pending_permission: None,
            agent_stream_text: "Directory: C:\\repo\nassets".into(),
            agent_stream_line_index: Some(1),
            agent_response: None,
            agent_response_line_count: 0,
        };
        let mut total_output_lines = block.output_lines.len();

        replace_agent_stream_snapshot(
            &mut block,
            "```sh\nDirectory: C:\\repo\nassets\ndocs\n```",
            &mut total_output_lines,
        );

        assert_eq!(
            block.output_lines,
            vec![
                "```sh".to_string(),
                "Directory: C:\\repo".to_string(),
                "assets".to_string(),
                "docs".to_string(),
                "```".to_string(),
                "[agent stderr] sandbox retry".to_string(),
            ]
        );
        assert_eq!(total_output_lines, 6);
        assert_eq!(block.agent_stream_line_index, Some(4));
    }

    #[test]
    fn append_agent_stream_delta_extends_existing_lines_without_duplication() {
        let mut block = Block {
            command: "Codex> ls".into(),
            output_lines: vec!["Directory: C:\\repo".into(), "assets".into()],
            has_error: false,
            context: None,
            agent_placeholder_active: false,
            pending_permission: None,
            agent_stream_text: "Directory: C:\\repo\nassets".into(),
            agent_stream_line_index: Some(1),
            agent_response: None,
            agent_response_line_count: 0,
        };
        let mut total_output_lines = block.output_lines.len();

        let delta = streaming_snapshot_delta(
            &block.agent_stream_text,
            "Directory: C:\\repo\nassets\ndocs",
        )
        .expect("expected suffix delta");
        block.agent_stream_text = "Directory: C:\\repo\nassets\ndocs".into();
        append_agent_stream_delta(&mut block, &delta, &mut total_output_lines);

        assert_eq!(
            block.output_lines,
            vec![
                "Directory: C:\\repo".to_string(),
                "assets".to_string(),
                "docs".to_string()
            ]
        );
        assert_eq!(total_output_lines, 3);
    }

    #[test]
    fn append_agent_stream_delta_inserts_before_later_stderr_lines() {
        let mut block = Block {
            command: "Codex> ls".into(),
            output_lines: vec![
                "Directory: C:\\repo".into(),
                "assets".into(),
                "[agent stderr] sandbox retry".into(),
            ],
            has_error: true,
            context: None,
            agent_placeholder_active: false,
            pending_permission: None,
            agent_stream_text: "Directory: C:\\repo\nassets".into(),
            agent_stream_line_index: Some(1),
            agent_response: None,
            agent_response_line_count: 0,
        };
        let mut total_output_lines = block.output_lines.len();

        let delta = streaming_snapshot_delta(
            &block.agent_stream_text,
            "Directory: C:\\repo\nassets\ndocs",
        )
        .expect("expected suffix delta");
        block.agent_stream_text = "Directory: C:\\repo\nassets\ndocs".into();
        append_agent_stream_delta(&mut block, &delta, &mut total_output_lines);

        assert_eq!(
            block.output_lines,
            vec![
                "Directory: C:\\repo".to_string(),
                "assets".to_string(),
                "docs".to_string(),
                "[agent stderr] sandbox retry".to_string(),
            ]
        );
        assert_eq!(total_output_lines, 4);
        assert_eq!(block.agent_stream_line_index, Some(2));
    }

    #[test]
    fn picker_query_select_all_and_replace_text() {
        let mut query = PickerQueryState::default();
        query.insert_text("codex");
        query.select_all();
        assert_eq!(query.normalized_selection(), Some((0, 5)));

        query.insert_text("gpt");
        assert_eq!(query.text, "gpt");
        assert_eq!(query.cursor, 3);
        assert_eq!(query.normalized_selection(), None);
    }

    #[test]
    fn picker_query_arrow_navigation_updates_cursor_and_selection() {
        let mut query = PickerQueryState::default();
        query.insert_text("model");
        query.move_left(false);
        query.move_left(false);
        assert_eq!(query.cursor, 3);

        query.move_left(true);
        assert_eq!(query.cursor, 2);
        assert_eq!(query.normalized_selection(), Some((2, 3)));

        query.move_right(true);
        assert_eq!(query.cursor, 3);
        assert_eq!(query.normalized_selection(), Some((3, 3)));

        query.move_home(false);
        assert_eq!(query.cursor, 0);
        assert_eq!(query.normalized_selection(), None);

        query.move_end(false);
        assert_eq!(query.cursor, 5);
    }

    #[test]
    fn picker_query_delete_forward_removes_character_at_cursor() {
        let mut query = PickerQueryState::default();
        query.insert_text("branch");
        query.move_left(false);
        query.move_left(false);
        query.delete_char_after_cursor();

        assert_eq!(query.text, "branh");
        assert_eq!(query.cursor, 4);
    }

    #[test]
    fn wrap_terminal_line_preserves_multiple_rows() {
        assert_eq!(
            wrap_terminal_line("abcdefgh", 3),
            vec!["abc".to_string(), "def".to_string(), "gh".to_string()]
        );
    }

    #[test]
    fn wrap_terminal_text_lines_splits_embedded_newlines_before_wrapping() {
        assert_eq!(
            wrap_terminal_text_lines("abc\ndefghi", 3),
            vec!["abc".to_string(), "def".to_string(), "ghi".to_string(),]
        );
    }

    #[test]
    fn renderable_output_window_keeps_recent_tail_for_large_blocks() {
        let lines = (0..6).map(|i| format!("line-{i}")).collect::<Vec<_>>();
        let (start, visible) = renderable_output_window(&lines, 3);

        assert_eq!(start, 3);
        assert_eq!(
            visible,
            &[
                "line-3".to_string(),
                "line-4".to_string(),
                "line-5".to_string(),
            ]
        );
    }

    #[test]
    fn append_output_batch_to_block_appends_first_fragment_and_batches_remaining_lines() {
        let mut block = Block {
            command: "ls".into(),
            output_lines: vec!["Dire".into()],
            has_error: false,
            context: None,
            agent_placeholder_active: false,
            pending_permission: None,
            agent_stream_text: String::new(),
            agent_stream_line_index: None,
            agent_response: None,
            agent_response_line_count: 0,
        };

        let added = append_output_batch_to_block(
            &mut block,
            &[
                "ctory: C:\\repo".to_string(),
                "assets".to_string(),
                "docs".to_string(),
            ],
            true,
        );

        assert_eq!(
            block.output_lines,
            vec![
                "Directory: C:\\repo".to_string(),
                "assets".to_string(),
                "docs".to_string(),
            ]
        );
        assert_eq!(added, 2);
    }

    #[test]
    fn parse_markdown_blocks_separates_headings_lists_and_code_blocks() {
        let blocks = parse_markdown_blocks("# Title\n\n- item\n\n```sh\nls\n```\n");

        assert_eq!(
            blocks,
            vec![
                MarkdownBlock::Heading {
                    level: 1,
                    text: "Title".to_string(),
                },
                MarkdownBlock::ListItem {
                    ordered: false,
                    index: None,
                    text: "item".to_string(),
                },
                MarkdownBlock::CodeBlock {
                    language: Some("sh".to_string()),
                    lines: vec!["ls".to_string()],
                },
            ]
        );
    }

    #[test]
    fn parse_markdown_inline_detects_strong_emphasis_code_and_links() {
        let segments = parse_markdown_inline(
            "Hello **world** and *friends* with `ls` and [docs](https://example.com)",
        );

        assert_eq!(
            segments,
            vec![
                MarkdownInlineSegment::Text("Hello ".to_string()),
                MarkdownInlineSegment::Strong("world".to_string()),
                MarkdownInlineSegment::Text(" and ".to_string()),
                MarkdownInlineSegment::Emphasis("friends".to_string()),
                MarkdownInlineSegment::Text(" with ".to_string()),
                MarkdownInlineSegment::Code("ls".to_string()),
                MarkdownInlineSegment::Text(" and ".to_string()),
                MarkdownInlineSegment::Link {
                    label: "docs".to_string(),
                    url: "https://example.com".to_string(),
                },
            ]
        );
    }

    #[test]
    fn extract_compact_list_items_detects_inline_code_enumerations() {
        let items =
            extract_compact_list_items("`a`, `b`, `c`, `d`").expect("expected compact list items");

        assert_eq!(
            items,
            vec![
                "a".to_string(),
                "b".to_string(),
                "c".to_string(),
                "d".to_string(),
            ]
        );
    }

    #[test]
    fn agent_placeholder_status_can_be_updated_in_place() {
        let mut block = Block {
            command: "Codex> oi".into(),
            output_lines: vec![AGENT_CONNECTING_PLACEHOLDER.into()],
            has_error: false,
            context: None,
            agent_placeholder_active: true,
            pending_permission: None,
            agent_stream_text: String::new(),
            agent_stream_line_index: None,
            agent_response: None,
            agent_response_line_count: 0,
        };

        assert!(update_agent_placeholder_block(
            &mut block,
            AGENT_SENDING_PROMPT_PLACEHOLDER
        ));

        assert_eq!(
            block.output_lines,
            vec![AGENT_SENDING_PROMPT_PLACEHOLDER.to_string()]
        );
    }
}
