use gpui::*;
use std::path::{Path, PathBuf};

const DEFAULT_SIDEBAR_WIDTH: f32 = 240.0;
const MIN_SIDEBAR_WIDTH: f32 = 180.0;
const MAX_SIDEBAR_WIDTH: f32 = 420.0;
const SIDEBAR_RESIZE_HANDLE_WIDTH: f32 = 6.0;

pub struct Workspace {
    sidebar_visible: bool,
    sidebar_width: f32,
    sidebar_resize_dragging: bool,
    tabs: Vec<Entity<views::tab_view::TabView>>,
    tab_ids: Vec<EntityId>,
    tab_paths: Vec<PathBuf>,
    tab_kinds: Vec<TabKind>,
    active_tab: usize,
    user_menu_open: bool,
    sidebar: Entity<views::sidebar_view::SidebarView>,
    tab_bar: Entity<views::tab_bar::TabBar>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TabKind {
    Welcome,
    BaseTerminal,
    Project,
    Utility,
}

pub mod views {
    pub mod agent_view;
    pub mod settings_view;
    pub mod sidebar_view;
    pub mod tab_bar;
    pub mod tab_view;
    pub mod welcome_view;
}

pub mod appearance;
pub mod icons;
pub mod launch;
pub mod recent;
pub mod text_edit;

pub(crate) fn move_index(index: usize, from: usize, to: usize) -> usize {
    if index == from {
        return to;
    }
    if from < to {
        if index > from && index <= to {
            return index - 1;
        }
    } else if from > to {
        if index >= to && index < from {
            return index + 1;
        }
    }
    index
}

impl Workspace {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let tab_bar = cx.new(|cx| views::tab_bar::TabBar::new(cx));
        cx.subscribe(
            &tab_bar,
            |workspace, _bar, event: &views::tab_bar::TabBarEvent, cx| {
                workspace.on_tab_event(event, cx);
            },
        )
        .detach();

        let mut workspace = Self {
            sidebar_visible: true,
            sidebar_width: DEFAULT_SIDEBAR_WIDTH,
            sidebar_resize_dragging: false,
            tabs: Vec::new(),
            tab_ids: Vec::new(),
            tab_paths: Vec::new(),
            tab_kinds: Vec::new(),
            active_tab: 0,
            user_menu_open: false,
            sidebar: cx.new(|cx| views::sidebar_view::SidebarView::new(cx)),
            tab_bar,
        };

        cx.subscribe(
            &workspace.sidebar,
            |workspace, _sidebar, event: &views::sidebar_view::OpenFileEvent, cx| {
                workspace.open_file_in_active_tab(
                    event.path.clone(),
                    event.line,
                    event.query.clone(),
                    cx,
                );
            },
        )
        .detach();

        workspace.add_welcome_tab(cx);
        workspace
    }

    pub fn new_with_options(cx: &mut Context<Self>, options: launch::LaunchOptions) -> Self {
        let mut workspace = Self::new(cx);
        if let Some(cwd) = options.base_terminal_cwd {
            let index = workspace.active_tab;
            workspace.open_base_terminal_in_tab(index, Some(cwd), None, cx);
        }
        workspace
    }

    fn clamp_sidebar_width(width: f32) -> f32 {
        width.clamp(MIN_SIDEBAR_WIDTH, MAX_SIDEBAR_WIDTH)
    }

    fn should_show_sidebar_for_tab(tab_kind: TabKind, sidebar_visible: bool) -> bool {
        sidebar_visible && matches!(tab_kind, TabKind::BaseTerminal | TabKind::Project)
    }

    fn on_sidebar_resize_mouse_down(
        &mut self,
        _event: &MouseDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.sidebar_resize_dragging = true;
        cx.notify();
        cx.stop_propagation();
    }

    fn on_sidebar_resize_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.sidebar_resize_dragging || !event.dragging() {
            return;
        }

        let x: f32 = event.position.x.into();
        let next_width = Self::clamp_sidebar_width(x);
        if (self.sidebar_width - next_width).abs() > f32::EPSILON {
            self.sidebar_width = next_width;
            cx.notify();
        }
    }

    fn on_sidebar_resize_mouse_up(
        &mut self,
        _event: &MouseUpEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.sidebar_resize_dragging {
            self.sidebar_resize_dragging = false;
            cx.notify();
        }
    }

    fn add_welcome_tab(&mut self, cx: &mut Context<Self>) {
        let recent = recent::load_recent();
        let tab = cx.new(|cx| views::tab_view::TabView::new_welcome(cx, recent));
        let tab_id = tab.entity_id();
        cx.subscribe(
            &tab,
            move |workspace, _tab, event: &views::tab_view::TabViewEvent, cx| {
                workspace.on_tab_view_event(tab_id, event, cx);
            },
        )
        .detach();

        let path = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        self.tabs.push(tab);
        self.tab_ids.push(tab_id);
        self.tab_paths.push(path.clone());
        self.tab_kinds.push(TabKind::Welcome);
        self.active_tab = self.tabs.len().saturating_sub(1);
        let _ = self.tab_bar.update(cx, |tab_bar, cx| {
            tab_bar.add_tab("Welcome".to_string(), "~".to_string(), cx);
            tab_bar.set_sidebar_visible(self.sidebar_visible, cx);
        });
        let _ = self.sidebar.update(cx, |sidebar, _cx| {
            sidebar.set_root(path);
        });
        cx.notify();
    }

    fn add_settings_tab(&mut self, cx: &mut Context<Self>) {
        let tab = cx.new(|cx| views::tab_view::TabView::new_settings(cx));
        let tab_id = tab.entity_id();
        cx.subscribe(
            &tab,
            move |workspace, _tab, event: &views::tab_view::TabViewEvent, cx| {
                workspace.on_tab_view_event(tab_id, event, cx);
            },
        )
        .detach();

        let path = self
            .tab_paths
            .get(self.active_tab)
            .cloned()
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_else(|| PathBuf::from("."));

        self.tabs.push(tab);
        self.tab_ids.push(tab_id);
        self.tab_paths.push(path.clone());
        self.tab_kinds.push(TabKind::Utility);
        self.active_tab = self.tabs.len().saturating_sub(1);

        let _ = self.tab_bar.update(cx, |tab_bar, cx| {
            tab_bar.add_tab("Settings".to_string(), "Settings".to_string(), cx);
            tab_bar.set_sidebar_visible(self.sidebar_visible, cx);
            tab_bar.set_active(self.active_tab, cx);
        });
        cx.notify();
    }

    fn add_agent_tab(&mut self, cx: &mut Context<Self>) {
        let tab = cx.new(|cx| views::tab_view::TabView::new_agent(cx));
        let tab_id = tab.entity_id();
        cx.subscribe(
            &tab,
            move |workspace, _tab, event: &views::tab_view::TabViewEvent, cx| {
                workspace.on_tab_view_event(tab_id, event, cx);
            },
        )
        .detach();

        let path = self
            .tab_paths
            .get(self.active_tab)
            .cloned()
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_else(|| PathBuf::from("."));

        self.tabs.push(tab);
        self.tab_ids.push(tab_id);
        self.tab_paths.push(path);
        self.tab_kinds.push(TabKind::Utility);
        self.active_tab = self.tabs.len().saturating_sub(1);

        let _ = self.tab_bar.update(cx, |tab_bar, cx| {
            tab_bar.add_tab("Agent".to_string(), "ACP".to_string(), cx);
            tab_bar.set_sidebar_visible(self.sidebar_visible, cx);
            tab_bar.set_active(self.active_tab, cx);
        });
        cx.notify();
    }

    fn open_file_in_active_tab(
        &mut self,
        file_path: PathBuf,
        line: Option<usize>,
        query: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.tabs.get(self.active_tab) else {
            return;
        };

        let _ = tab.update(cx, |view, cx| {
            if let (Some(line), Some(query)) = (line, query.clone()) {
                view.open_file_preview_at_search_result(file_path.clone(), line, query, cx);
            } else {
                view.open_file_preview(file_path.clone(), cx);
            }
        });
    }

    fn sync_sidebar_root(&mut self, cx: &mut Context<Self>) {
        if let Some(path) = self.tab_paths.get(self.active_tab).cloned() {
            let _ = self.sidebar.update(cx, |sidebar, _cx| {
                sidebar.set_root(path);
            });
        }
    }

    fn on_tab_event(&mut self, event: &views::tab_bar::TabBarEvent, cx: &mut Context<Self>) {
        match event {
            views::tab_bar::TabBarEvent::NewTab => {
                self.add_welcome_tab(cx);
            }
            views::tab_bar::TabBarEvent::ToggleUserMenu => {
                self.user_menu_open = !self.user_menu_open;
                cx.notify();
            }
            views::tab_bar::TabBarEvent::Activate(index) => {
                if *index < self.tabs.len() {
                    self.active_tab = *index;
                    self.sync_sidebar_root(cx);
                    cx.notify();
                }
            }
            views::tab_bar::TabBarEvent::Close(index) => {
                if self.tabs.len() > 1 && *index < self.tabs.len() {
                    self.tabs.remove(*index);
                    self.tab_ids.remove(*index);
                    self.tab_paths.remove(*index);
                    self.tab_kinds.remove(*index);
                    if self.active_tab >= self.tabs.len() {
                        self.active_tab = self.tabs.len() - 1;
                    }
                    self.sync_sidebar_root(cx);
                    cx.notify();
                }
            }
            views::tab_bar::TabBarEvent::ToggleSidebar => {
                self.toggle_sidebar(cx);
            }
            views::tab_bar::TabBarEvent::Reorder(from, to) => {
                let from = *from;
                let to = *to;
                if from >= self.tabs.len() || to >= self.tabs.len() {
                    return;
                }
                if from == to {
                    return;
                }
                let tab = self.tabs.remove(from);
                self.tabs.insert(to, tab);
                let tab_id = self.tab_ids.remove(from);
                self.tab_ids.insert(to, tab_id);
                let tab_path = self.tab_paths.remove(from);
                self.tab_paths.insert(to, tab_path);
                let tab_kind = self.tab_kinds.remove(from);
                self.tab_kinds.insert(to, tab_kind);

                self.active_tab = move_index(self.active_tab, from, to);
                self.sync_sidebar_root(cx);
                cx.notify();
            }
        }
    }

    fn toggle_sidebar(&mut self, cx: &mut Context<Self>) {
        self.sidebar_visible = !self.sidebar_visible;
        let _ = self.tab_bar.update(cx, |tab_bar, cx| {
            tab_bar.set_sidebar_visible(self.sidebar_visible, cx);
        });
        cx.notify();
    }

    fn render_user_menu(&self, cx: &mut Context<Self>) -> Div {
        let handle = cx.entity().downgrade();

        let overlay =
            div()
                .absolute()
                .size_full()
                .on_mouse_down(MouseButton::Left, move |_e, _w, cx| {
                    let _ = handle.update(cx, |view, cx| {
                        view.user_menu_open = false;
                        cx.notify();
                    });
                });

        let handle_settings = cx.entity().downgrade();
        let handle_settings_settings = handle_settings.clone();
        let handle_settings_appearance = handle_settings.clone();
        let handle_settings_shortcuts = handle_settings.clone();
        let handle_settings_registry = handle_settings.clone();
        let handle_settings_about = handle_settings.clone();
        let menu = div()
            .absolute()
            .right(px(16.0))
            .top(px(52.0))
            .w(px(220.0))
            .flex()
            .flex_col()
            .gap(px(6.0))
            .p(px(8.0))
            .rounded(px(8.0))
            .bg(rgb(0x121212))
            .border_1()
            .border_color(rgb(0x2a2a2a))
            .on_mouse_down(MouseButton::Left, |_event, _window, cx| {
                cx.stop_propagation();
            })
            .child(
                div()
                    .px(px(10.0))
                    .py(px(6.0))
                    .text_size(px(12.0))
                    .text_color(rgb(0x9a9a9a))
                    .child("Solra"),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .px(px(12.0))
                    .py(px(8.0))
                    .rounded(px(6.0))
                    .text_size(px(13.0))
                    .text_color(rgb(0xe6e6e6))
                    .child("Settings")
                    .hover(|this| this.bg(rgb(0x242424)).border_color(rgb(0x4a4a4a)))
                    .cursor(CursorStyle::PointingHand)
                    .on_mouse_down(MouseButton::Left, move |_e, _w, cx| {
                        cx.stop_propagation();
                        let _ = handle_settings_settings.update(cx, |view, cx| {
                            view.user_menu_open = false;
                            view.add_settings_tab(cx);
                        });
                    }),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .px(px(12.0))
                    .py(px(8.0))
                    .rounded(px(6.0))
                    .text_size(px(13.0))
                    .text_color(rgb(0xe6e6e6))
                    .child("Appearance")
                    .hover(|this| this.bg(rgb(0x242424)).border_color(rgb(0x4a4a4a)))
                    .cursor(CursorStyle::PointingHand)
                    .on_mouse_down(MouseButton::Left, move |_e, _w, cx| {
                        cx.stop_propagation();
                        let _ = handle_settings_appearance.update(cx, |view, cx| {
                            view.user_menu_open = false;
                            view.add_settings_tab(cx);
                            if let Some(tab) = view.tabs.get(view.active_tab) {
                                let _ = tab.update(cx, |tab_view, cx| {
                                    tab_view.set_settings_section("Appearance", cx);
                                });
                            }
                        });
                    }),
            )
            .child({
                let handle = cx.entity().downgrade();
                div()
                    .flex()
                    .items_center()
                    .px(px(12.0))
                    .py(px(8.0))
                    .rounded(px(6.0))
                    .text_size(px(13.0))
                    .text_color(rgb(0xe6e6e6))
                    .child("Agent")
                    .hover(|this| this.bg(rgb(0x242424)).border_color(rgb(0x4a4a4a)))
                    .cursor(CursorStyle::PointingHand)
                    .on_mouse_down(MouseButton::Left, move |_e, _w, cx| {
                        cx.stop_propagation();
                        let _ = handle.update(cx, |view, cx| {
                            view.user_menu_open = false;
                            view.add_agent_tab(cx);
                        });
                    })
            })
            .child({
                div()
                    .flex()
                    .items_center()
                    .px(px(12.0))
                    .py(px(8.0))
                    .rounded(px(6.0))
                    .text_size(px(13.0))
                    .text_color(rgb(0xe6e6e6))
                    .child("Keyboard Shortcuts")
                    .hover(|this| this.bg(rgb(0x242424)).border_color(rgb(0x4a4a4a)))
                    .cursor(CursorStyle::PointingHand)
                    .on_mouse_down(MouseButton::Left, move |_e, _w, cx| {
                        cx.stop_propagation();
                        let _ = handle_settings_shortcuts.update(cx, |view, cx| {
                            view.user_menu_open = false;
                            view.add_settings_tab(cx);
                            if let Some(tab) = view.tabs.get(view.active_tab) {
                                let _ = tab.update(cx, |tab_view, cx| {
                                    tab_view.set_settings_section("Keyboard shortcuts", cx);
                                });
                            }
                        });
                    })
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .px(px(12.0))
                    .py(px(8.0))
                    .rounded(px(6.0))
                    .text_size(px(13.0))
                    .text_color(rgb(0xe6e6e6))
                    .child("ACP Registry")
                    .hover(|this| this.bg(rgb(0x242424)).border_color(rgb(0x4a4a4a)))
                    .cursor(CursorStyle::PointingHand)
                    .on_mouse_down(MouseButton::Left, move |_e, _w, cx| {
                        cx.stop_propagation();
                        let _ = handle_settings_registry.update(cx, |view, cx| {
                            view.user_menu_open = false;
                            view.add_settings_tab(cx);
                            if let Some(tab) = view.tabs.get(view.active_tab) {
                                let _ = tab.update(cx, |tab_view, cx| {
                                    tab_view.set_settings_section("ACP Registry", cx);
                                });
                            }
                        });
                    }),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .px(px(12.0))
                    .py(px(8.0))
                    .rounded(px(6.0))
                    .text_size(px(13.0))
                    .text_color(rgb(0xe6e6e6))
                    .child("About")
                    .hover(|this| this.bg(rgb(0x242424)).border_color(rgb(0x4a4a4a)))
                    .cursor(CursorStyle::PointingHand)
                    .on_mouse_down(MouseButton::Left, move |_e, _w, cx| {
                        cx.stop_propagation();
                        let _ = handle_settings_about.update(cx, |view, cx| {
                            view.user_menu_open = false;
                            view.add_settings_tab(cx);
                            if let Some(tab) = view.tabs.get(view.active_tab) {
                                let _ = tab.update(cx, |tab_view, cx| {
                                    tab_view.set_settings_section("About", cx);
                                });
                            }
                        });
                    }),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .px(px(12.0))
                    .py(px(8.0))
                    .rounded(px(6.0))
                    .text_size(px(13.0))
                    .text_color(rgb(0xe6e6e6))
                    .hover(|this| this.bg(rgb(0x242424)).border_color(rgb(0x4a4a4a)))
                    .cursor(CursorStyle::PointingHand)
                    .child("Feedback"),
            );

        div().absolute().size_full().child(overlay).child(menu)
    }

    fn on_tab_view_event(
        &mut self,
        tab_id: EntityId,
        event: &views::tab_view::TabViewEvent,
        cx: &mut Context<Self>,
    ) {
        match event {
            views::tab_view::TabViewEvent::CwdChanged(path) => {
                if let Some(index) = self.tab_ids.iter().position(|id| *id == tab_id) {
                    if let Some(slot) = self.tab_paths.get_mut(index) {
                        *slot = path.clone();
                    }
                    if index == self.active_tab {
                        self.sync_sidebar_root(cx);
                    }
                    cx.notify();
                }
            }
            views::tab_view::TabViewEvent::OpenRepository(path) => {
                if let Some(index) = self.tab_ids.iter().position(|id| *id == tab_id) {
                    self.open_repository_in_tab(index, path.clone(), cx);
                }
            }
            views::tab_view::TabViewEvent::StartBaseTerminal { command } => {
                if let Some(index) = self.tab_ids.iter().position(|id| *id == tab_id) {
                    self.open_base_terminal_in_tab(index, None, Some(command.clone()), cx);
                }
            }
            views::tab_view::TabViewEvent::CreateProject { prompt, parent } => {
                if let Some(index) = self.tab_ids.iter().position(|id| *id == tab_id) {
                    self.create_project_in_tab(index, parent.clone(), prompt.clone(), cx);
                }
            }
            views::tab_view::TabViewEvent::CloneRepository { url, parent } => {
                if let Some(index) = self.tab_ids.iter().position(|id| *id == tab_id) {
                    self.clone_repository_in_tab(index, parent.clone(), url.clone(), cx);
                }
            }
        }
    }

    fn open_base_terminal_in_tab(
        &mut self,
        index: usize,
        path: Option<PathBuf>,
        initial_command: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let cwd = path.unwrap_or_else(launch::default_base_terminal_cwd);
        let tab_name = cwd
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| "Terminal".to_string());
        let tab_path = cwd.to_string_lossy().to_string();

        if let Some(tab) = self.tabs.get(index) {
            let _ = tab.update(cx, |view, cx| {
                view.start_terminal_with_path_and_command(
                    cx,
                    Some(cwd.clone()),
                    initial_command.clone(),
                );
            });
        }

        if let Some(slot) = self.tab_paths.get_mut(index) {
            *slot = cwd;
        }
        if let Some(kind) = self.tab_kinds.get_mut(index) {
            *kind = TabKind::BaseTerminal;
        }
        self.active_tab = index;
        self.sidebar_visible = false;

        let _ = self.tab_bar.update(cx, |tab_bar, cx| {
            tab_bar.rename_tab(index, tab_name, tab_path, cx);
            tab_bar.set_sidebar_visible(self.sidebar_visible, cx);
            tab_bar.set_active(index, cx);
        });
        self.sync_sidebar_root(cx);
        cx.notify();
    }

    fn create_project_in_tab(
        &mut self,
        index: usize,
        parent: PathBuf,
        prompt: String,
        cx: &mut Context<Self>,
    ) {
        let Some(dir_name) = launch::project_dir_name_from_prompt(&prompt) else {
            return;
        };
        let path = Self::unique_child_dir(&parent, &dir_name);
        if let Err(err) = std::fs::create_dir_all(&path) {
            eprintln!(
                "failed to create project directory '{}': {err}",
                path.display()
            );
            return;
        }

        self.open_project_in_tab(index, path, None, Some(prompt), cx);
    }

    fn clone_repository_in_tab(
        &mut self,
        index: usize,
        parent: PathBuf,
        url: String,
        cx: &mut Context<Self>,
    ) {
        let Some(dir_name) = launch::clone_dir_name_from_url(&url) else {
            return;
        };
        let path = Self::unique_child_dir(&parent, &dir_name);
        if let Err(err) = std::fs::create_dir_all(&path) {
            eprintln!(
                "failed to create clone directory '{}': {err}",
                path.display()
            );
            return;
        }

        let command = format!("git clone {} .", Self::quote_terminal_arg(&url));
        self.open_project_in_tab(index, path, Some(command), None, cx);
    }

    fn open_repository_in_tab(&mut self, index: usize, path: PathBuf, cx: &mut Context<Self>) {
        self.open_project_in_tab(index, path, None, None, cx);
    }

    fn open_project_in_tab(
        &mut self,
        index: usize,
        path: PathBuf,
        initial_command: Option<String>,
        initial_agent_prompt: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let tab_name = path
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| path.to_string_lossy().to_string());
        let tab_path = path.to_string_lossy().to_string();

        if let Some(tab) = self.tabs.get(index) {
            let _ = tab.update(cx, |view, cx| {
                if let Some(prompt) = initial_agent_prompt.clone() {
                    view.start_agent_prompt_with_path(cx, Some(path.clone()), prompt);
                } else {
                    view.start_terminal_with_path_and_command(
                        cx,
                        Some(path.clone()),
                        initial_command.clone(),
                    );
                }
            });
        }

        if let Some(slot) = self.tab_paths.get_mut(index) {
            *slot = path.clone();
        }
        if let Some(kind) = self.tab_kinds.get_mut(index) {
            *kind = TabKind::Project;
        }
        self.active_tab = index;

        let recent = recent::add_recent(path.clone());
        for tab in &self.tabs {
            let _ = tab.update(cx, |view, cx| {
                view.set_recent(recent.clone(), cx);
            });
        }

        let _ = self.tab_bar.update(cx, |tab_bar, cx| {
            tab_bar.rename_tab(index, tab_name, tab_path, cx);
            tab_bar.set_active(index, cx);
        });
        self.sync_sidebar_root(cx);
        cx.notify();
    }

    fn unique_child_dir(parent: &Path, dir_name: &str) -> PathBuf {
        let base = parent.join(dir_name);
        if !base.exists() {
            return base;
        }

        for suffix in 2.. {
            let candidate = parent.join(format!("{dir_name}-{suffix}"));
            if !candidate.exists() {
                return candidate;
            }
        }

        unreachable!("unbounded suffix loop should always return")
    }

    fn quote_terminal_arg(value: &str) -> String {
        format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
    }
}

impl Render for Workspace {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let tab_kind = self
            .tab_kinds
            .get(self.active_tab)
            .copied()
            .unwrap_or(TabKind::Welcome);
        let show_sidebar = Self::should_show_sidebar_for_tab(tab_kind, self.sidebar_visible);
        let sidebar_width = Self::clamp_sidebar_width(self.sidebar_width);

        let mut root = div()
            .flex()
            .flex_col()
            .size_full()
            .bg(rgb(0x0a0a0a))
            .relative()
            .child(
                // Tab bar
                self.tab_bar.clone(),
            )
            .child(
                // Main content area
                div()
                    .flex()
                    .flex_1()
                    .min_h(px(0.0))
                    .on_mouse_move(_cx.listener(Self::on_sidebar_resize_mouse_move))
                    .on_mouse_up(
                        MouseButton::Left,
                        _cx.listener(Self::on_sidebar_resize_mouse_up),
                    )
                    .on_mouse_up_out(
                        MouseButton::Left,
                        _cx.listener(Self::on_sidebar_resize_mouse_up),
                    )
                    .child(
                        // Sidebar
                        if show_sidebar {
                            div()
                                .flex_none()
                                .w(px(sidebar_width))
                                .child(self.sidebar.clone())
                        } else {
                            div()
                        },
                    )
                    .child(if show_sidebar {
                        div()
                            .flex_none()
                            .w(px(SIDEBAR_RESIZE_HANDLE_WIDTH))
                            .h_full()
                            .cursor(CursorStyle::ResizeLeftRight)
                            .bg(if self.sidebar_resize_dragging {
                                rgb(0x2a4a73)
                            } else {
                                rgb(0x141414)
                            })
                            .border_l_1()
                            .border_r_1()
                            .border_color(if self.sidebar_resize_dragging {
                                rgb(0x3f669c)
                            } else {
                                rgb(0x1f1f1f)
                            })
                            .hover(|style| style.bg(rgb(0x1a1f28)).border_color(rgb(0x2f3b4f)))
                            .on_mouse_down(
                                MouseButton::Left,
                                _cx.listener(Self::on_sidebar_resize_mouse_down),
                            )
                    } else {
                        div()
                    })
                    .child(
                        // Terminal view
                        div().flex_1().min_h(px(0.0)).child(
                            if let Some(tab) = self.tabs.get(self.active_tab) {
                                div()
                                    .size_full()
                                    .min_h(px(0.0))
                                    .min_w(px(0.0))
                                    .child(tab.clone())
                            } else {
                                div().size_full().min_h(px(0.0)).min_w(px(0.0))
                            },
                        ),
                    ),
            );

        if self.user_menu_open {
            root = root.child(self.render_user_menu(_cx));
        }

        root
    }
}

#[cfg(test)]
mod tests {
    use super::{TabKind, Workspace};
    use std::ffi::OsString;

    #[test]
    fn sidebar_width_is_clamped_to_session_limits() {
        assert_eq!(Workspace::clamp_sidebar_width(120.0), 180.0);
        assert_eq!(Workspace::clamp_sidebar_width(240.0), 240.0);
        assert_eq!(Workspace::clamp_sidebar_width(520.0), 420.0);
    }

    #[test]
    fn launch_options_select_existing_directory_argument() {
        let temp = tempfile::tempdir().expect("temp dir");
        let path = temp.path().to_path_buf();

        let options = super::launch::LaunchOptions::from_args([
            OsString::from("orbitshell"),
            path.clone().into_os_string(),
        ]);

        assert_eq!(options.base_terminal_cwd, Some(path));
    }

    #[test]
    fn launch_options_ignore_invalid_directory_argument() {
        let options = super::launch::LaunchOptions::from_args([
            OsString::from("orbitshell"),
            OsString::from("/definitely/not/a/real/orbitshell/path"),
        ]);

        assert_eq!(options.base_terminal_cwd, None);
    }

    #[test]
    fn base_terminal_sidebar_renders_only_when_sidebar_is_visible() {
        assert!(!Workspace::should_show_sidebar_for_tab(
            TabKind::BaseTerminal,
            false
        ));
        assert!(Workspace::should_show_sidebar_for_tab(
            TabKind::BaseTerminal,
            true
        ));
    }

    #[test]
    fn welcome_and_utility_tabs_never_show_sidebar() {
        assert!(!Workspace::should_show_sidebar_for_tab(
            TabKind::Welcome,
            true
        ));
        assert!(!Workspace::should_show_sidebar_for_tab(
            TabKind::Utility,
            true
        ));
    }

    #[test]
    fn project_tabs_follow_sidebar_visibility() {
        assert!(!Workspace::should_show_sidebar_for_tab(
            TabKind::Project,
            false
        ));
        assert!(Workspace::should_show_sidebar_for_tab(
            TabKind::Project,
            true
        ));
    }

    #[test]
    fn unique_child_dir_adds_suffix_when_directory_exists() {
        let temp = tempfile::tempdir().expect("temp dir");
        std::fs::create_dir(temp.path().join("orbitshell")).expect("create existing dir");

        assert_eq!(
            Workspace::unique_child_dir(temp.path(), "orbitshell"),
            temp.path().join("orbitshell-2")
        );
    }

    #[test]
    fn quote_terminal_arg_wraps_values_for_clone_command() {
        assert_eq!(
            Workspace::quote_terminal_arg("https://github.com/owner/my app.git"),
            "\"https://github.com/owner/my app.git\""
        );
    }
}
