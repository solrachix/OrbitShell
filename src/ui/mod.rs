use gpui::*;
use std::path::PathBuf;

pub struct Workspace {
    sidebar_visible: bool,
    tabs: Vec<Entity<views::tab_view::TabView>>,
    tab_ids: Vec<EntityId>,
    tab_paths: Vec<PathBuf>,
    tab_is_welcome: Vec<bool>,
    active_tab: usize,
    user_menu_open: bool,
    sidebar: Entity<views::sidebar_view::SidebarView>,
    tab_bar: Entity<views::tab_bar::TabBar>,
}

pub mod views {
    pub mod settings_view;
    pub mod sidebar_view;
    pub mod tab_bar;
    pub mod tab_view;
    pub mod top_bar;
    pub mod welcome_view;
}

pub mod icons;
pub mod recent;

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
            tabs: Vec::new(),
            tab_ids: Vec::new(),
            tab_paths: Vec::new(),
            tab_is_welcome: Vec::new(),
            active_tab: 0,
            user_menu_open: false,
            sidebar: cx.new(|cx| views::sidebar_view::SidebarView::new(cx)),
            tab_bar,
        };

        workspace.add_welcome_tab(cx);
        workspace
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
        self.tab_is_welcome.push(true);
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
        self.tab_is_welcome.push(true);
        self.active_tab = self.tabs.len().saturating_sub(1);

        let _ = self.tab_bar.update(cx, |tab_bar, cx| {
            tab_bar.add_tab("Settings".to_string(), "Settings".to_string(), cx);
            tab_bar.set_sidebar_visible(self.sidebar_visible, cx);
            tab_bar.set_active(self.active_tab, cx);
        });
        cx.notify();
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
                    if let Some(path) = self.tab_paths.get(*index).cloned() {
                        let _ = self.sidebar.update(cx, |sidebar, _cx| {
                            sidebar.set_root(path);
                        });
                    }
                    cx.notify();
                }
            }
            views::tab_bar::TabBarEvent::Close(index) => {
                if self.tabs.len() > 1 && *index < self.tabs.len() {
                    self.tabs.remove(*index);
                    self.tab_ids.remove(*index);
                    self.tab_paths.remove(*index);
                    self.tab_is_welcome.remove(*index);
                    if self.active_tab >= self.tabs.len() {
                        self.active_tab = self.tabs.len() - 1;
                    }
                    if let Some(path) = self.tab_paths.get(self.active_tab).cloned() {
                        let _ = self.sidebar.update(cx, |sidebar, _cx| {
                            sidebar.set_root(path);
                        });
                    }
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
                let tab_welcome = self.tab_is_welcome.remove(from);
                self.tab_is_welcome.insert(to, tab_welcome);

                self.active_tab = Self::move_index(self.active_tab, from, to);
                if let Some(path) = self.tab_paths.get(self.active_tab).cloned() {
                    let _ = self.sidebar.update(cx, |sidebar, _cx| {
                        sidebar.set_root(path);
                    });
                }
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

        let overlay = div()
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
        let handle_settings_invite = handle_settings.clone();
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
                    .on_mouse_down(MouseButton::Left, move |_e, _w, cx| {
                        cx.stop_propagation();
                        let _ = handle_settings_settings.update(cx, |view, cx| {
                            view.user_menu_open = false;
                            view.add_settings_tab(cx);
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
                    .child("Keyboard Shortcuts")
                    .on_mouse_down(MouseButton::Left, move |_e, _w, cx| {
                        cx.stop_propagation();
                        let _ = handle.update(cx, |view, cx| {
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
                    .child("Invite a friend")
                    .on_mouse_down(MouseButton::Left, move |_e, _w, cx| {
                        cx.stop_propagation();
                        let _ = handle_settings_invite.update(cx, |view, cx| {
                            view.user_menu_open = false;
                            view.add_settings_tab(cx);
                            if let Some(tab) = view.tabs.get(view.active_tab) {
                                let _ = tab.update(cx, |tab_view, cx| {
                                    tab_view.set_settings_section("Referrals", cx);
                                });
                            }
                        });
                    })
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
                    .child("Feedback"),
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
                    .child("Log out"),
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
                        let _ = self.sidebar.update(cx, |sidebar, _cx| {
                            sidebar.set_root(path.clone());
                        });
                    }
                    cx.notify();
                }
            }
            views::tab_view::TabViewEvent::OpenRepository(path) => {
                if let Some(index) = self.tab_ids.iter().position(|id| *id == tab_id) {
                    self.open_repository_in_tab(index, path.clone(), cx);
                }
            }
        }
    }

    fn open_repository_in_tab(&mut self, index: usize, path: PathBuf, cx: &mut Context<Self>) {
        let tab_name = path
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| path.to_string_lossy().to_string());
        let tab_path = path.to_string_lossy().to_string();

        if let Some(tab) = self.tabs.get(index) {
            let _ = tab.update(cx, |view, cx| {
                view.start_terminal_with_path(cx, Some(path.clone()));
            });
        }

        if let Some(slot) = self.tab_paths.get_mut(index) {
            *slot = path.clone();
        }
        if let Some(flag) = self.tab_is_welcome.get_mut(index) {
            *flag = false;
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
        let _ = self.sidebar.update(cx, |sidebar, _cx| {
            sidebar.set_root(path);
        });
        cx.notify();
    }

    fn move_index(index: usize, from: usize, to: usize) -> usize {
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
}

impl Render for Workspace {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let is_welcome = self
            .tab_is_welcome
            .get(self.active_tab)
            .copied()
            .unwrap_or(false);
        let show_sidebar = self.sidebar_visible && !is_welcome;

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
                    .child(
                        // Sidebar
                        if show_sidebar {
                            div().w(px(240.0)).child(self.sidebar.clone())
                        } else {
                            div()
                        },
                    )
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
            )
            ;

        if self.user_menu_open {
            root = root.child(self.render_user_menu(_cx));
        }

        root
    }
}
