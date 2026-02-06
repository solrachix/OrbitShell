use gpui::*;
use lucide_icons::Icon;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::File;
use std::hash::{Hash, Hasher};
use std::io::{BufRead, BufReader, Read, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;

use futures::StreamExt;
use futures::channel::mpsc;
use serde::Deserialize;
use std::io::Seek;

enum SearchMessage {
    Batch(u64, Vec<SearchResult>),
    Done(u64),
}

use crate::git::{GitChange, GitStatus, get_git_changes, get_git_status};
use crate::ui::icons::lucide_icon;
use crate::ui::text_edit::TextEditState;

const ACCENT: u32 = 0x6b9eff;
const ACCENT_BG: u32 = 0x6b9eff22;
const ACCENT_BORDER: u32 = 0x6b9eff66;

#[derive(Clone)]
struct FileEntry {
    name: String,
    path: PathBuf,
    is_dir: bool,
}

#[derive(Clone)]
struct SearchResult {
    path: PathBuf,
    line: usize,
    text: String,
    is_filename: bool,
}

struct TooltipView {
    text: String,
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

#[derive(Clone, Debug, Deserialize)]
struct OrbitshellRules {
    skip_dirs: Vec<String>,
    skip_files: Vec<String>,
    max_file_kb: u64,
    search_limit: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SidebarMode {
    Explorer,
    Search,
    Git,
}

pub struct SidebarView {
    current_path: PathBuf,
    expanded_folders: HashSet<PathBuf>,
    entries: Vec<FileEntry>,
    entries_cache: HashMap<PathBuf, Vec<FileEntry>>,

    mode: SidebarMode,
    focus_handle: FocusHandle,

    search_query: String,
    search_cursor: usize,
    search_selection: Option<(usize, usize)>,
    search_anchor: Option<usize>,
    search_results: Vec<SearchResult>,
    search_generation: u64,
    search_cancel: Arc<AtomicU64>,
    search_pending: bool,
    search_expanded_files: HashSet<PathBuf>,
    search_user_toggled: bool,
    search_scroll: ScrollHandle,

    git_status: Option<GitStatus>,
    git_changes: Vec<GitChange>,
    git_scroll: ScrollHandle,
    explorer_scroll: ScrollHandle,

    rules: OrbitshellRules,
}

impl SidebarView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let current_path = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let rules = Self::load_rules();
        let entries = Self::read_entries(&current_path, &rules);
        let git_status = get_git_status(&current_path);
        let git_changes = get_git_changes(&current_path);
        Self {
            current_path,
            expanded_folders: HashSet::new(),
            entries,
            entries_cache: HashMap::new(),
            mode: SidebarMode::Explorer,
            focus_handle: cx.focus_handle(),
            search_query: String::new(),
            search_cursor: 0,
            search_selection: None,
            search_anchor: None,
            search_results: Vec::new(),
            search_generation: 0,
            search_cancel: Arc::new(AtomicU64::new(0)),
            search_pending: false,
            search_expanded_files: HashSet::new(),
            search_user_toggled: false,
            search_scroll: ScrollHandle::new(),
            git_status,
            git_changes,
            git_scroll: ScrollHandle::new(),
            explorer_scroll: ScrollHandle::new(),
            rules,
        }
    }

    pub fn set_root(&mut self, path: PathBuf) {
        self.current_path = path;
        self.expanded_folders.clear();
        self.entries = Self::read_entries(&self.current_path, &self.rules);
        self.entries_cache.clear();
        self.git_status = get_git_status(&self.current_path);
        self.git_changes = get_git_changes(&self.current_path);
    }

    fn set_mode(&mut self, mode: SidebarMode, cx: &mut Context<Self>) {
        self.mode = mode;
        if mode == SidebarMode::Search {
            TextEditState::clear_selection(&mut self.search_selection, &mut self.search_anchor);
            self.search_cursor = self.search_query.chars().count();
        }
        if mode == SidebarMode::Git {
            self.git_status = get_git_status(&self.current_path);
            self.git_changes = get_git_changes(&self.current_path);
        }
        cx.notify();
    }

    fn read_entries(path: &Path, rules: &OrbitshellRules) -> Vec<FileEntry> {
        let mut entries: Vec<FileEntry> = std::fs::read_dir(path)
            .map(|read_dir| {
                read_dir
                    .filter_map(|entry| entry.ok())
                    .map(|entry| {
                        let file_type = entry.file_type().ok();
                        let name = entry.file_name().to_string_lossy().to_string();
                        FileEntry {
                            name,
                            path: entry.path(),
                            is_dir: file_type.map(|t| t.is_dir()).unwrap_or(false),
                        }
                    })
                    .filter(|entry| {
                        if entry.is_dir {
                            !Self::should_skip_dir(&entry.name, rules)
                        } else {
                            !Self::should_skip_file(&entry.name, rules)
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();

        entries.sort_by(|a, b| match (a.is_dir, b.is_dir) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
        });

        entries
    }

    fn toggle_folder(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        if self.expanded_folders.contains(&path) {
            self.expanded_folders.remove(&path);
        } else {
            let children = Self::read_entries(&path, &self.rules);
            self.expanded_folders.insert(path.clone());
            self.entries_cache.insert(path, children);
        }
        cx.notify();
    }

    fn toggle_search_file(&mut self, path: &PathBuf, cx: &mut Context<Self>) {
        self.search_user_toggled = true;
        if self.search_expanded_files.contains(path) {
            self.search_expanded_files.remove(path);
        } else {
            self.search_expanded_files.insert(path.clone());
        }
        cx.notify();
    }

    fn render_entry(&self, entry: &FileEntry, depth: usize, cx: &Context<Self>) -> Div {
        let is_expanded = entry.is_dir && self.expanded_folders.contains(&entry.path);
        let indent = 8.0 + (depth as f32) * 14.0;

        let row = {
            let mut row = div()
                .flex()
                .items_center()
                .gap(px(6.0))
                .px(px(8.0))
                .py(px(6.0))
                .rounded(px(6.0))
                .ml(px(indent))
                .child(if entry.is_dir {
                    lucide_icon(
                        if is_expanded {
                            Icon::ChevronDown
                        } else {
                            Icon::ChevronRight
                        },
                        12.0,
                        0x777777,
                    )
                } else {
                    div().w(px(12.0)).h(px(12.0))
                })
                .child(lucide_icon(
                    if entry.is_dir {
                        if is_expanded {
                            Icon::FolderOpen
                        } else {
                            Icon::Folder
                        }
                    } else {
                        Icon::File
                    },
                    14.0,
                    0x9a9a9a,
                ))
                .child(
                    div()
                        .text_size(px(13.0))
                        .text_color(rgb(0xcccccc))
                        .child(entry.name.clone()),
                );

            if entry.is_dir {
                let path = entry.path.clone();
                row = row.on_mouse_down(gpui::MouseButton::Left, {
                    let handle = cx.entity().downgrade();
                    move |_event, _window, cx| {
                        let _ = handle.update(cx, |view, cx| {
                            view.toggle_folder(path.clone(), cx);
                        });
                    }
                });
            }

            row
        };

        let mut container = div().flex().flex_col().child(row);
        if is_expanded {
            if let Some(children) = self.entries_cache.get(&entry.path) {
                for child in children {
                    container = container.child(self.render_entry(child, depth + 1, cx));
                }
            }
        }

        container
    }

    fn header_button(&self, icon: Icon, active: bool) -> Div {
        div()
            .flex()
            .items_center()
            .justify_center()
            .w(px(26.0))
            .h(px(26.0))
            .rounded(px(6.0))
            .bg(if active {
                rgba(ACCENT_BG)
            } else {
                rgb(0x141414)
            })
            .border_1()
            .border_color(if active {
                rgba(ACCENT_BORDER)
            } else {
                rgb(0x2a2a2a)
            })
            .child(lucide_icon(
                icon,
                13.0,
                if active { ACCENT } else { 0x9a9a9a },
            ))
    }

    fn on_search_focus(
        &mut self,
        _event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.focus_handle);
        cx.stop_propagation();
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        if self.mode != SidebarMode::Search {
            return;
        }

        let ctrl = event.keystroke.modifiers.control;
        let shift = event.keystroke.modifiers.shift;

        if ctrl && event.keystroke.key.eq_ignore_ascii_case("a") {
            TextEditState::select_all(
                &self.search_query,
                &mut self.search_cursor,
                &mut self.search_selection,
                &mut self.search_anchor,
            );
            cx.notify();
            cx.stop_propagation();
            return;
        }

        match event.keystroke.key.as_str() {
            "enter" | "return" | "numpadenter" => {
                self.run_search(cx);
                cx.notify();
                cx.stop_propagation();
            }
            "backspace" => {
                if TextEditState::delete_selection_if_any(
                    &mut self.search_query,
                    &mut self.search_cursor,
                    &mut self.search_selection,
                    &mut self.search_anchor,
                ) {
                    cx.notify();
                    cx.stop_propagation();
                    return;
                }
                if self.search_cursor > 0 {
                    TextEditState::pop_char_before_cursor(
                        &mut self.search_query,
                        &mut self.search_cursor,
                        &mut self.search_selection,
                        &mut self.search_anchor,
                    );
                    cx.notify();
                }
                cx.stop_propagation();
            }
            "left" | "arrowleft" => {
                if shift {
                    let anchor = self.search_anchor.unwrap_or(self.search_cursor);
                    self.search_cursor = self.search_cursor.saturating_sub(1);
                    TextEditState::set_selection_from_anchor(
                        &mut self.search_selection,
                        &mut self.search_anchor,
                        anchor,
                        self.search_cursor,
                    );
                } else {
                    if TextEditState::has_selection(self.search_selection) {
                        if let Some((a, b)) =
                            TextEditState::normalized_selection(self.search_selection)
                        {
                            self.search_cursor = a.min(b);
                        }
                    } else {
                        self.search_cursor = self.search_cursor.saturating_sub(1);
                    }
                    TextEditState::clear_selection(
                        &mut self.search_selection,
                        &mut self.search_anchor,
                    );
                }
                cx.notify();
                cx.stop_propagation();
            }
            "right" | "arrowright" => {
                let max = self.search_query.chars().count();
                if shift {
                    let anchor = self.search_anchor.unwrap_or(self.search_cursor);
                    self.search_cursor = (self.search_cursor + 1).min(max);
                    TextEditState::set_selection_from_anchor(
                        &mut self.search_selection,
                        &mut self.search_anchor,
                        anchor,
                        self.search_cursor,
                    );
                } else if TextEditState::has_selection(self.search_selection) {
                    if let Some((a, b)) = TextEditState::normalized_selection(self.search_selection)
                    {
                        self.search_cursor = a.max(b);
                    }
                    TextEditState::clear_selection(
                        &mut self.search_selection,
                        &mut self.search_anchor,
                    );
                } else if self.search_cursor < max {
                    self.search_cursor += 1;
                }
                cx.notify();
                cx.stop_propagation();
            }
            "home" => {
                self.search_cursor = 0;
                TextEditState::clear_selection(&mut self.search_selection, &mut self.search_anchor);
                cx.notify();
                cx.stop_propagation();
            }
            "end" => {
                self.search_cursor = self.search_query.chars().count();
                TextEditState::clear_selection(&mut self.search_selection, &mut self.search_anchor);
                cx.notify();
                cx.stop_propagation();
            }
            "escape" => {
                TextEditState::clear_selection(&mut self.search_selection, &mut self.search_anchor);
                cx.notify();
                cx.stop_propagation();
            }
            _ => {
                if let Some(text) = event.keystroke.key_char.as_deref() {
                    if !text.is_empty() && !ctrl {
                        TextEditState::insert_text(
                            &mut self.search_query,
                            &mut self.search_cursor,
                            &mut self.search_selection,
                            &mut self.search_anchor,
                            text,
                        );
                        cx.notify();
                        cx.stop_propagation();
                    }
                } else if event.keystroke.key.len() == 1 && !ctrl {
                    let key = event.keystroke.key.clone();
                    TextEditState::insert_text(
                        &mut self.search_query,
                        &mut self.search_cursor,
                        &mut self.search_selection,
                        &mut self.search_anchor,
                        &key,
                    );
                    cx.notify();
                    cx.stop_propagation();
                }
            }
        }
    }
    // Text input handled via KeyDownEvent for gpui 0.2.2

    fn run_search(&mut self, cx: &mut Context<Self>) {
        let query = self.search_query.trim().to_string();
        self.search_generation = self.search_generation.wrapping_add(1);
        let generation = self.search_generation;
        self.search_cancel.store(generation, Ordering::Relaxed);
        self.search_pending = true;
        self.search_results.clear();
        self.search_expanded_files.clear();
        self.search_user_toggled = false;

        if query.is_empty() {
            self.search_pending = false;
            return;
        }

        let root = self.current_path.clone();
        let rules = self.rules.clone();
        let (tx, mut rx) = mpsc::unbounded::<SearchMessage>();
        let cancel = self.search_cancel.clone();

        thread::spawn(move || {
            let mut total = 0usize;
            let mut batch: Vec<SearchResult> = Vec::with_capacity(32);

            Self::search_in_dir_stream(
                &root,
                &query,
                &rules,
                || cancel.load(Ordering::Relaxed) == generation,
                |result| {
                    if cancel.load(Ordering::Relaxed) != generation {
                        return false;
                    }
                    batch.push(result);
                    total += 1;
                    if batch.len() >= 25 {
                        let to_send = std::mem::take(&mut batch);
                        let _ = tx.unbounded_send(SearchMessage::Batch(generation, to_send));
                    }
                    total < rules.search_limit
                },
            );

            if !batch.is_empty() {
                let to_send = std::mem::take(&mut batch);
                let _ = tx.unbounded_send(SearchMessage::Batch(generation, to_send));
            }
            let _ = tx.unbounded_send(SearchMessage::Done(generation));
        });

        cx.spawn(|view: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut app = cx.clone();
            async move {
                while let Some(message) = rx.next().await {
                    let done = matches!(message, SearchMessage::Done(_));
                    let _ = view.update(&mut app, |view, cx| match message {
                        SearchMessage::Batch(generation_id, mut results) => {
                            if view.search_generation != generation_id {
                                return;
                            }
                            if view.search_results.len() >= view.rules.search_limit {
                                return;
                            }
                            let space = view
                                .rules
                                .search_limit
                                .saturating_sub(view.search_results.len());
                            if results.len() > space {
                                results.truncate(space);
                            }
                            if !view.search_user_toggled {
                                for r in &results {
                                    view.search_expanded_files.insert(r.path.clone());
                                }
                            }
                            view.search_results.extend(results);
                            view.search_pending = true;
                            cx.notify();
                        }
                        SearchMessage::Done(generation_id) => {
                            if view.search_generation == generation_id {
                                view.search_pending = false;
                                cx.notify();
                            }
                        }
                    });
                    if done {
                        break;
                    }
                }
            }
        })
        .detach();
    }

    fn search_in_dir_stream(
        root: &Path,
        query: &str,
        rules: &OrbitshellRules,
        should_continue: impl FnMut() -> bool,
        push: impl FnMut(SearchResult) -> bool,
    ) {
        Self::search_in_dir_stream_with_fs(
            root,
            query,
            rules,
            should_continue,
            push,
            |path| std::fs::read_dir(path),
            |path| File::open(path),
        );
    }

    fn search_in_dir_stream_with_fs<ReadDirFn, OpenFileFn>(
        root: &Path,
        query: &str,
        rules: &OrbitshellRules,
        mut should_continue: impl FnMut() -> bool,
        mut push: impl FnMut(SearchResult) -> bool,
        read_dir: ReadDirFn,
        open_file: OpenFileFn,
    ) where
        ReadDirFn: for<'a> Fn(&'a Path) -> std::io::Result<std::fs::ReadDir>,
        OpenFileFn: for<'a> Fn(&'a Path) -> std::io::Result<File>,
    {
        let query_lower = query.to_ascii_lowercase();
        let mut stack = vec![root.to_path_buf()];

        while let Some(dir) = stack.pop() {
            if !should_continue() {
                return;
            }
            let read = read_dir(&dir);
            let Ok(read) = read else {
                continue;
            };
            for entry in read.flatten() {
                if !should_continue() {
                    return;
                }
                let path = entry.path();
                let name = entry.file_name().to_string_lossy().to_string();
                if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                    if Self::should_skip_dir(&name, rules) {
                        continue;
                    }
                    stack.push(path);
                } else {
                    if Self::should_skip_file(&name, rules) {
                        continue;
                    }
                    if name.to_lowercase().contains(&query_lower) {
                        let keep = push(SearchResult {
                            path: path.clone(),
                            line: 0,
                            text: name.clone(),
                            is_filename: true,
                        });
                        if !keep {
                            return;
                        }
                    }

                    if let Ok(mut file) = open_file(&path) {
                        if !should_continue() {
                            return;
                        }
                        if let Ok(meta) = file.metadata() {
                            if meta.len() > rules.max_file_kb * 1024 {
                                continue;
                            }
                        }
                        let mut peek = [0u8; 512];
                        if let Ok(n) = file.read(&mut peek) {
                            if peek[..n].iter().any(|b| *b == 0) {
                                continue;
                            }
                        }
                        let _ = file.seek(SeekFrom::Start(0));
                        let reader = BufReader::new(file);
                        for (idx, line) in reader.lines().enumerate() {
                            if !should_continue() {
                                return;
                            }
                            let Ok(line) = line else {
                                continue;
                            };
                            if line.to_ascii_lowercase().contains(&query_lower) {
                                let snippet = make_snippet(&line, query, 2);
                                let keep = push(SearchResult {
                                    path: path.clone(),
                                    line: idx + 1,
                                    text: snippet,
                                    is_filename: false,
                                });
                                if !keep {
                                    return;
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    fn should_skip_dir(name: &str, rules: &OrbitshellRules) -> bool {
        let name = name.to_lowercase();
        rules.skip_dirs.iter().any(|entry| entry == &name)
    }

    fn should_skip_file(name: &str, rules: &OrbitshellRules) -> bool {
        let name = name.to_lowercase();
        rules.skip_files.iter().any(|entry| entry == &name)
    }

    fn render_search_input(&self, window: &mut Window) -> Div {
        let is_focused = self.focus_handle.is_focused(window);
        let (left, right) = TextEditState::split_at_cursor(&self.search_query, self.search_cursor);
        let mut pre = left;
        let mut post = right;

        let mut selection_mid = String::new();
        if let Some((a, b)) = TextEditState::normalized_selection(self.search_selection) {
            let mut before = String::new();
            let mut mid = String::new();
            let mut after = String::new();
            for (i, ch) in self.search_query.chars().enumerate() {
                if i < a {
                    before.push(ch);
                } else if i < b {
                    mid.push(ch);
                } else {
                    after.push(ch);
                }
            }
            pre = before;
            selection_mid = mid;
            post = after;
        }

        let caret = if is_focused {
            div()
                .w(px(2.0))
                .h(px(16.0))
                .rounded(px(1.0))
                .bg(rgb(ACCENT))
        } else {
            div().w(px(2.0)).h(px(16.0))
        };

        let placeholder = self.search_query.is_empty();

        let input = if placeholder && !is_focused {
            div()
                .text_size(px(13.0))
                .text_color(rgb(0x666666))
                .child("Search in files...")
        } else {
            div()
                .flex()
                .items_center()
                .gap(px(0.0))
                .text_size(px(13.0))
                .text_color(rgb(0xcccccc))
                .child(div().child(pre))
                .child(if !selection_mid.is_empty() {
                    div()
                        .px(px(1.0))
                        .bg(rgb(0x264d7a))
                        .text_color(rgb(0xffffff))
                        .child(selection_mid)
                } else {
                    div()
                })
                .child(caret)
                .child(div().child(post))
        };

        div().flex().items_center().gap(px(2.0)).child(input)
    }

    fn render_search_results(&self, cx: &Context<Self>) -> AnyElement {
        if self.search_pending && self.search_results.is_empty() {
            return div()
                .px(px(12.0))
                .py(px(8.0))
                .text_size(px(12.0))
                .text_color(rgb(0x888888))
                .child("Searching…")
                .into_any_element();
        }
        let mut grouped: BTreeMap<PathBuf, Vec<&SearchResult>> = BTreeMap::new();
        for r in &self.search_results {
            if r.is_filename {
                continue;
            }
            grouped.entry(r.path.clone()).or_default().push(r);
        }

        if grouped.is_empty() {
            return div()
                .px(px(12.0))
                .py(px(8.0))
                .text_size(px(12.0))
                .text_color(rgb(0x666666))
                .child("No results")
                .into_any_element();
        }

        let results_count: usize = grouped.values().map(|items| items.len()).sum();
        let files_count = grouped.len();
        let summary = format!("{results_count} results in {files_count} files");
        let handle = cx.entity().downgrade();

        div()
            .id("sidebar_search_results")
            .flex()
            .flex_1()
            .min_h(px(0.0))
            .flex_col()
            .gap(px(8.0))
            .track_scroll(&self.search_scroll)
            .overflow_scroll()
            .scrollbar_width(px(12.0))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .px(px(4.0))
                    .child(
                        div()
                            .text_size(px(12.0))
                            .text_color(rgb(0x9a9a9a))
                            .child(summary),
                    )
                    .child(
                        div()
                            .text_size(px(12.0))
                            .text_color(rgb(ACCENT))
                            .child("Open in editor"),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.0))
                    .children(grouped.iter().map(|(path, items)| {
                        let relative = self.relative_path(path);
                        let file_name = relative
                            .file_name()
                            .map(|s| s.to_string_lossy().to_string())
                            .unwrap_or_else(|| relative.to_string_lossy().to_string());
                        let parent_rel = relative
                            .parent()
                            .map(|p| p.to_string_lossy().to_string())
                            .unwrap_or_default();
                        let full_path = path.to_string_lossy().to_string();
                        let expanded = self.search_expanded_files.contains(path);
                        let count = items.len();
                        let id_key = Self::file_id_key(path);

                        let handle = handle.clone();
                        let file_path = path.clone();

                        let mut file_header = div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .px(px(6.0))
                            .py(px(6.0))
                            .rounded(px(6.0))
                            .bg(rgb(0x101010))
                            .border_1()
                            .border_color(rgb(0x1f1f1f))
                            .on_mouse_down(MouseButton::Left, move |_e, _w, cx| {
                                let _ = handle.update(cx, |v, cx| {
                                    v.toggle_search_file(&file_path, cx);
                                });
                            })
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(6.0))
                                    .flex_1()
                                    .min_w(px(0.0))
                                    .child(lucide_icon(
                                        if expanded {
                                            Icon::ChevronDown
                                        } else {
                                            Icon::ChevronRight
                                        },
                                        12.0,
                                        0x8a8a8a,
                                    ))
                                    .child(lucide_icon(Icon::File, 13.0, 0xe09b4f))
                                    .child(
                                        div()
                                            .flex()
                                            .items_baseline()
                                            .gap(px(8.0))
                                            .flex_1()
                                            .min_w(px(0.0))
                                            .child(
                                                div()
                                                    .flex_1()
                                                    .min_w(px(0.0))
                                                    .text_size(px(13.0))
                                                    .text_color(rgb(0xd0d0d0))
                                                    .truncate()
                                                    .child(file_name.clone()),
                                            )
                                            .child(
                                                div()
                                                    .flex_1()
                                                    .min_w(px(0.0))
                                                    .text_size(px(12.0))
                                                    .text_color(rgb(0x6f6f6f))
                                                    .truncate()
                                                    .child(parent_rel.clone()),
                                            ),
                                    ),
                            )
                            .child(
                                div()
                                    .min_w(px(18.0))
                                    .px(px(8.0))
                                    .py(px(2.0))
                                    .rounded(px(999.0))
                                    .bg(rgb(0x1a1a1a))
                                    .border_1()
                                    .border_color(rgb(0x2a2a2a))
                                    .text_size(px(12.0))
                                    .text_color(rgb(0xbfbfbf))
                                    .child(count.to_string()),
                            )
                            .id(("search_file", id_key));

                        file_header.interactivity().tooltip(move |_window, cx| {
                            let text = full_path.clone();
                            cx.new(|_| TooltipView { text }).into()
                        });

                        let mut section = div().flex().flex_col().gap(px(4.0)).child(file_header);

                        if expanded {
                            section = section.child(
                                div().flex().flex_col().gap(px(2.0)).pl(px(22.0)).children(
                                    items.iter().map(|r| {
                                        let text = r.text.clone();
                                        let (pre, mid, post) =
                                            split_match(&text, &self.search_query);
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap(px(8.0))
                                            .px(px(6.0))
                                            .py(px(4.0))
                                            .rounded(px(6.0))
                                            .bg(rgb(0x0c0c0c))
                                            .border_1()
                                            .border_color(rgb(0x141414))
                                            .child(
                                                div()
                                                    .w(px(54.0))
                                                    .text_size(px(11.0))
                                                    .text_color(rgb(0x7a7a7a))
                                                    .font_family("Cascadia Code")
                                                    .child(format!("{}:", r.line)),
                                            )
                                            .child(
                                                div()
                                                    .flex()
                                                    .items_center()
                                                    .gap(px(0.0))
                                                    .text_size(px(12.0))
                                                    .text_color(rgb(0xcccccc))
                                                    .font_family("Cascadia Code")
                                                    .child(pre)
                                                    .child(if mid.is_empty() {
                                                        div()
                                                    } else {
                                                        div()
                                                            .border_b_1()
                                                            .border_color(rgb(ACCENT))
                                                            .text_color(rgb(0xffffff))
                                                            .child(mid)
                                                    })
                                                    .child(post),
                                            )
                                    }),
                                ),
                            );
                        }

                        section
                    })),
            )
            .into_any_element()
    }
}

impl SidebarView {
    fn relative_path(&self, path: &Path) -> PathBuf {
        path.strip_prefix(&self.current_path)
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|_| path.to_path_buf())
    }

    fn file_id_key(path: &Path) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        path.hash(&mut hasher);
        hasher.finish()
    }

    fn load_rules() -> OrbitshellRules {
        let path = PathBuf::from("orbitshell_rules.json");
        if let Ok(contents) = std::fs::read_to_string(&path) {
            if let Ok(parsed) = serde_json::from_str::<OrbitshellRules>(&contents) {
                return Self::normalize_rules(parsed);
            }
        }
        Self::normalize_rules(OrbitshellRules {
            skip_dirs: vec![
                ".git".to_string(),
                "node_modules".to_string(),
                "target".to_string(),
                "dist".to_string(),
                ".next".to_string(),
            ],
            skip_files: vec![],
            max_file_kb: 512,
            search_limit: 200,
        })
    }

    fn normalize_rules(mut rules: OrbitshellRules) -> OrbitshellRules {
        rules.skip_dirs = rules
            .skip_dirs
            .into_iter()
            .map(|entry| entry.to_lowercase())
            .collect();
        rules.skip_files = rules
            .skip_files
            .into_iter()
            .map(|entry| entry.to_lowercase())
            .collect();
        rules
    }
}

impl Render for SidebarView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mode = self.mode;
        let handle = cx.entity().downgrade();
        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(rgb(0x0a0a0a))
            .border_r_1()
            .border_color(rgb(0x2a2a2a))
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(Self::on_key_down))
            .child(
                // Header
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .px(px(12.0))
                    .py(px(8.0))
                    .border_b_1()
                    .border_color(rgb(0x2a2a2a))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(10.0))
                            .child(
                                self.header_button(Icon::File, mode == SidebarMode::Explorer)
                                    .on_mouse_down(MouseButton::Left, {
                                        let handle = handle.clone();
                                        move |_event, _window, cx| {
                                            cx.stop_propagation();
                                            let _ = handle.update(cx, |view, cx| {
                                                view.set_mode(SidebarMode::Explorer, cx);
                                            });
                                        }
                                    }),
                            )
                            .child(
                                self.header_button(Icon::Search, mode == SidebarMode::Search)
                                    .on_mouse_down(MouseButton::Left, {
                                        let handle = handle.clone();
                                        move |_event, _window, cx| {
                                            cx.stop_propagation();
                                            let _ = handle.update(cx, |view, cx| {
                                                view.set_mode(SidebarMode::Search, cx);
                                            });
                                        }
                                    }),
                            )
                            .child(
                                self.header_button(Icon::GitBranch, mode == SidebarMode::Git)
                                    .on_mouse_down(MouseButton::Left, {
                                        let handle = handle.clone();
                                        move |_event, _window, cx| {
                                            cx.stop_propagation();
                                            let _ = handle.update(cx, |view, cx| {
                                                view.set_mode(SidebarMode::Git, cx);
                                            });
                                        }
                                    }),
                            ),
                    ),
            )
            .child(match mode {
                SidebarMode::Explorer => div()
                    .id("sidebar_explorer")
                    .flex()
                    .flex_1()
                    .min_h(px(0.0))
                    .flex_col()
                    .gap(px(2.0))
                    .p(px(8.0))
                    .track_scroll(&self.explorer_scroll)
                    .overflow_scroll()
                    .scrollbar_width(px(12.0))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.0))
                            .px(px(8.0))
                            .py(px(6.0))
                            .rounded(px(6.0))
                            .bg(rgb(0x262626))
                            .child(lucide_icon(Icon::ChevronDown, 12.0, 0xcccccc))
                            .child(lucide_icon(Icon::FolderOpen, 14.0, 0xcccccc))
                            .child(
                                div().text_size(px(14.0)).text_color(rgb(0xeeeeee)).child(
                                    self.current_path
                                        .file_name()
                                        .map(|name| name.to_string_lossy().to_string())
                                        .unwrap_or_else(|| {
                                            self.current_path.to_string_lossy().to_string()
                                        }),
                                ),
                            ),
                    )
                    .children(
                        self.entries
                            .iter()
                            .map(|entry| self.render_entry(entry, 1, cx)),
                    )
                    .into_any_element(),
                SidebarMode::Search => div()
                    .id("sidebar_search")
                    .flex()
                    .flex_1()
                    .min_h(px(0.0))
                    .flex_col()
                    .gap(px(10.0))
                    .p(px(12.0))
                    .child(
                        div()
                            .rounded(px(6.0))
                            .bg(rgb(0x131313))
                            .border_1()
                            .border_color(rgb(0x2a2a2a))
                            .px(px(10.0))
                            .py(px(8.0))
                            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_search_focus))
                            .child(self.render_search_input(window)),
                    )
                    .child(self.render_search_results(cx))
                    .into_any_element(),
                SidebarMode::Git => {
                    let (staged, unstaged): (Vec<_>, Vec<_>) =
                        self.git_changes.iter().cloned().partition(|c| c.staged);
                    div()
                        .id("sidebar_git")
                        .flex()
                        .flex_1()
                        .min_h(px(0.0))
                        .flex_col()
                        .gap(px(10.0))
                        .p(px(12.0))
                        .track_scroll(&self.git_scroll)
                        .overflow_scroll()
                        .scrollbar_width(px(12.0))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .justify_between()
                                .gap(px(6.0))
                                .px(px(6.0))
                                .py(px(4.0))
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap(px(6.0))
                                        .child(lucide_icon(Icon::GitBranch, 12.0, 0x6b9eff))
                                        .child(
                                            div()
                                                .text_size(px(12.0))
                                                .text_color(rgb(0xd7f7c7))
                                                .child(
                                                    self.git_status
                                                        .as_ref()
                                                        .map(|s| s.branch.clone())
                                                        .unwrap_or_else(|| "No repo".to_string()),
                                                ),
                                        ),
                                ),
                        )
                        .child(self.render_git_section("Staged Changes", &staged))
                        .child(self.render_git_section("Changes", &unstaged))
                        .into_any_element()
                }
            })
    }
}

impl SidebarView {
    fn render_git_section(&self, title: &str, items: &[GitChange]) -> Div {
        let count = items.len();
        let list = if items.is_empty() {
            div()
                .text_size(px(12.0))
                .text_color(rgb(0x666666))
                .child("No changes")
        } else {
            div()
                .flex_col()
                .gap(px(8.0))
                .children(items.iter().map(|item| {
                    let path = PathBuf::from(&item.path);
                    let relative = self.relative_path(&path);
                    let file_name = relative
                        .file_name()
                        .map(|s| s.to_string_lossy().to_string())
                        .unwrap_or_else(|| item.path.clone());
                    let parent = relative
                        .parent()
                        .map(|p| p.to_string_lossy().to_string())
                        .unwrap_or_default();
                    let full_path = path.to_string_lossy().to_string();

                    let id_key = Self::git_id_key(item, &path);
                    let mut row = div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap(px(8.0))
                        .px(px(6.0))
                        .py(px(6.0))
                        .rounded(px(6.0))
                        .bg(rgb(0x101010))
                        .border_1()
                        .border_color(rgb(0x1f1f1f))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(8.0))
                                .flex_1()
                                .min_w(px(0.0))
                                .child(lucide_icon(
                                    Icon::File,
                                    12.0,
                                    match item.kind.as_str() {
                                        "A" => 0x8bd06f,
                                        "D" => 0xff7b72,
                                        "M" => 0xe3b341,
                                        _ => 0x9a9a9a,
                                    },
                                ))
                                .child(
                                    div()
                                        .flex()
                                        .items_baseline()
                                        .gap(px(8.0))
                                        .flex_1()
                                        .min_w(px(0.0))
                                        .child(
                                            div()
                                                .flex_1()
                                                .min_w(px(0.0))
                                                .text_size(px(12.0))
                                                .text_color(rgb(0xcccccc))
                                                .truncate()
                                                .child(file_name),
                                        )
                                        .child(if parent.is_empty() {
                                            div()
                                        } else {
                                            div()
                                                .flex_1()
                                                .min_w(px(0.0))
                                                .text_size(px(11.0))
                                                .text_color(rgb(0x6f6f6f))
                                                .truncate()
                                                .child(parent)
                                        }),
                                ),
                        )
                        .child(
                            div()
                                .min_w(px(18.0))
                                .px(px(6.0))
                                .py(px(2.0))
                                .rounded(px(999.0))
                                .bg(rgb(0x1a1a1a))
                                .border_1()
                                .border_color(rgb(0x2a2a2a))
                                .text_size(px(11.0))
                                .text_color(match item.kind.as_str() {
                                    "A" => rgb(0x8bd06f),
                                    "D" => rgb(0xff7b72),
                                    "M" => rgb(0xe3b341),
                                    _ => rgb(0xcccccc),
                                })
                                .child(item.kind.clone()),
                        )
                        .id(("git_item", id_key));

                    row.interactivity().tooltip(move |_window, cx| {
                        let text = full_path.clone();
                        cx.new(|_| TooltipView { text }).into()
                    });

                    row
                }))
        };

        div()
            .flex_col()
            .gap(px(6.0))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_size(px(12.0))
                            .text_color(rgb(0x9a9a9a))
                            .child(title.to_string()),
                    )
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(rgb(0xcccccc))
                            .px(px(6.0))
                            .py(px(2.0))
                            .rounded(px(10.0))
                            .bg(rgb(0x202020))
                            .child(count.to_string()),
                    ),
            )
            .child(list)
    }
}

fn split_match(text: &str, query: &str) -> (String, String, String) {
    let lower_text = text.to_lowercase();
    let lower_query = query.to_lowercase();
    if lower_query.is_empty() {
        return (text.to_string(), String::new(), String::new());
    }
    if let Some(pos) = lower_text.find(&lower_query) {
        let mut pre = String::new();
        let mut mid = String::new();
        let mut post = String::new();
        let mut idx = 0usize;
        let match_len = lower_query.chars().count();
        for ch in text.chars() {
            if idx < pos {
                pre.push(ch);
            } else if idx < pos + match_len {
                mid.push(ch);
            } else {
                post.push(ch);
            }
            idx += 1;
        }
        (pre, mid, post)
    } else {
        (text.to_string(), String::new(), String::new())
    }
}

fn make_snippet(line: &str, query: &str, padding: usize) -> String {
    let lower = line.to_lowercase();
    let q = query.to_lowercase();
    if q.is_empty() {
        return line.chars().take(80).collect();
    }
    if let Some(byte_pos) = lower.find(&q) {
        let char_pos = line[..byte_pos].chars().count();
        let q_len = q.chars().count();
        let chars: Vec<char> = line.chars().collect();
        let mut start = char_pos;
        let mut words = 0usize;
        while start > 0 && words < padding {
            start -= 1;
            if chars[start].is_whitespace() {
                while start > 0 && chars[start].is_whitespace() {
                    start -= 1;
                }
                words += 1;
            }
        }
        while start > 0 && !chars[start - 1].is_whitespace() {
            start -= 1;
        }

        let mut end = (char_pos + q_len).min(chars.len());
        let mut words_right = 0usize;
        while end < chars.len() && words_right < padding {
            if chars[end].is_whitespace() {
                while end < chars.len() && chars[end].is_whitespace() {
                    end += 1;
                }
                words_right += 1;
            } else {
                end += 1;
            }
        }
        while end < chars.len() && !chars[end].is_whitespace() {
            end += 1;
        }

        let mut snippet: String = chars[start..end].iter().collect();
        if start > 0 {
            snippet = format!("…{snippet}");
        }
        if end < chars.len() {
            snippet = format!("{snippet}…");
        }
        return snippet;
    }
    line.chars().take(80).collect()
}

impl SidebarView {
    fn git_id_key(item: &GitChange, path: &Path) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        path.hash(&mut hasher);
        item.kind.hash(&mut hasher);
        item.staged.hash(&mut hasher);
        hasher.finish()
    }
}
