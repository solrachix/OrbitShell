use gpui::*;
use lucide_icons::Icon;
use std::path::PathBuf;

use crate::ui::{icons::lucide_icon, recent::RecentEntry};

pub struct OpenRepositoryEvent {
    pub path: PathBuf,
}

pub struct WelcomeView {
    focus_handle: FocusHandle,
    recent: Vec<RecentEntry>,
    overlay: Option<WelcomeOverlay>,
    input: String,
    suggest_index: usize,
}

#[derive(Clone, Debug, PartialEq)]
enum WelcomeOverlay {
    CreateProject,
    CloneRepository,
}

impl WelcomeView {
    pub fn with_recent(cx: &mut Context<Self>, recent: Vec<RecentEntry>) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
            recent,
            overlay: None,
            input: String::new(),
            suggest_index: 0,
        }
    }

    pub fn set_recent(&mut self, recent: Vec<RecentEntry>, cx: &mut Context<Self>) {
        self.recent = recent;
        cx.notify();
    }

    fn on_create_project(
        &mut self,
        _event: &MouseDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.overlay = Some(WelcomeOverlay::CreateProject);
        self.input.clear();
        self.suggest_index = 0;
        cx.notify();
    }

    fn on_open_repository(
        &mut self,
        _event: &MouseDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let prompt = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Open repository".into()),
        });

        cx.spawn(|view: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let selected_path = match prompt.await {
                    Ok(Ok(Some(mut paths))) => paths.pop(),
                    _ => None,
                };

                if let Some(path) = selected_path {
                    let _ = view.update(&mut cx, |_view, cx| {
                        cx.emit(OpenRepositoryEvent { path });
                    });
                }
            }
        })
        .detach();
    }

    fn on_clone_repository(
        &mut self,
        _event: &MouseDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.overlay = Some(WelcomeOverlay::CloneRepository);
        self.input.clear();
        cx.notify();
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

    fn handle_key_down(
        &mut self,
        event: &KeyDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.overlay.is_none() {
            return;
        }

        match event.keystroke.key.as_str() {
            "escape" => {
                self.overlay = None;
                cx.notify();
            }
            "enter" | "return" => {
                self.commit_overlay(cx);
            }
            "backspace" => {
                self.input.pop();
                cx.notify();
            }
            "up" | "arrowup" => {
                if self.overlay == Some(WelcomeOverlay::CreateProject) {
                    let suggestions = self.create_project_suggestions();
                    if self.suggest_index > 0 {
                        self.suggest_index -= 1;
                    } else {
                        self.suggest_index = suggestions.len().saturating_sub(1);
                    }
                    cx.notify();
                }
            }
            "down" | "arrowdown" => {
                if self.overlay == Some(WelcomeOverlay::CreateProject) {
                    let suggestions = self.create_project_suggestions();
                    if !suggestions.is_empty() {
                        self.suggest_index = (self.suggest_index + 1) % suggestions.len();
                    }
                    cx.notify();
                }
            }
            _ => {
                if let Some(text) = event.keystroke.key_char.as_deref() {
                    self.input.push_str(text);
                    cx.notify();
                }
            }
        }
    }

    fn commit_overlay(&mut self, cx: &mut Context<Self>) {
        match self.overlay.take() {
            Some(WelcomeOverlay::CloneRepository) => {
                let url = self.input.trim().to_string();
                if !url.is_empty() {
                    println!("Cloning repository: {}", url);
                    // In a real app, this would spawn a git process
                    // For now, we'll just dismiss and notify
                }
            }
            Some(WelcomeOverlay::CreateProject) => {
                let suggestions = self.create_project_suggestions();
                let selected = if self.input.trim().is_empty() {
                    suggestions.get(self.suggest_index).cloned()
                } else {
                    Some(self.input.trim().to_string())
                };

                if let Some(prompt) = selected {
                    println!("Creating project with prompt: {}", prompt);
                }
            }
            None => {}
        }
        cx.notify();
    }

    fn create_project_suggestions(&self) -> Vec<String> {
        vec![
            "Build a Minesweeper clone in React".into(),
            "Code a Node.js server that returns random quotes from a JSON file".into(),
            "Write a CSV to JSON converter CLI".into(),
            "Create a starter template for a résumé web page".into(),
            "Make a Conway's Game of Life simulation".into(),
        ]
    }

    fn render_recent_item(
        &self,
        icon: Icon,
        title: String,
        path: Option<PathBuf>,
        last_opened: Option<i64>,
        cx: &Context<Self>,
    ) -> Div {
        let now = chrono::Utc::now().timestamp();
        let time_label = last_opened.map(|last| format_recent_time(last, now));
        let mut row = div()
            .flex()
            .items_center()
            .gap(px(12.0))
            .p(px(12.0))
            .rounded(px(6.0))
            .child(lucide_icon(icon, 12.0, 0x888888))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.0))
                    .flex_1()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .text_color(rgb(0xeeeeee))
                                    .text_size(px(13.0))
                                    .child(title),
                            )
                            .child(if let Some(label) = time_label.clone() {
                                div()
                                    .text_color(rgb(0x777777))
                                    .text_size(px(11.0))
                                    .child(label)
                            } else {
                                div()
                            }),
                    )
                    .child(
                        div().text_color(rgb(0x666666)).text_size(px(11.0)).child(
                            path.as_ref()
                                .map(|p| p.to_string_lossy().to_string())
                                .unwrap_or_default(),
                        ),
                    ),
            );

        if let Some(path) = path {
            let handle = cx.entity().downgrade();
            row = row.on_mouse_down(gpui::MouseButton::Left, move |_event, _window, cx| {
                let _ = handle.update(cx, |_view, cx| {
                    cx.emit(OpenRepositoryEvent { path: path.clone() });
                });
            });
        }

        row
    }
}

fn format_recent_time(last_opened: i64, now: i64) -> String {
    let diff = (now - last_opened).max(0);
    if diff < 60 {
        return "agora".to_string();
    }
    if diff < 3600 {
        return format!("{}m", diff / 60);
    }
    if diff < 86_400 {
        return format!("{}h", diff / 3600);
    }
    if diff < 2_592_000 {
        return format!("{}d", diff / 86_400);
    }
    let date = chrono::DateTime::<chrono::Utc>::from_timestamp(last_opened, 0)
        .or_else(|| chrono::DateTime::<chrono::Utc>::from_timestamp(now, 0))
        .unwrap();
    date.format("%d/%m/%Y").to_string()
}

impl Render for WelcomeView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let recent_items = if self.recent.is_empty() {
            vec![self.render_recent_item(
                Icon::Clock,
                "Open a repository to get started".to_string(),
                None,
                None,
                cx,
            )]
        } else {
            self.recent
                .iter()
                .map(|entry| {
                    let title = entry
                        .path
                        .file_name()
                        .map(|name| name.to_string_lossy().to_string())
                        .unwrap_or_else(|| entry.path.to_string_lossy().to_string());
                    self.render_recent_item(
                        Icon::Folder,
                        title,
                        Some(entry.path.clone()),
                        Some(entry.last_opened),
                        cx,
                    )
                })
                .collect()
        };

        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(rgb(0x0a0a0a))
            .items_center()
            .justify_center()
            .gap(px(32.0))
            .on_key_down(cx.listener(Self::handle_key_down))
            .child(
                // Action buttons
                div()
                    .flex()
                    .gap(px(16.0))
                    .child(action_button(
                        Icon::Plus,
                        "Create new project",
                        cx.listener(Self::on_create_project),
                    ))
                    .child(action_button(
                        Icon::FolderOpen,
                        "Open repository",
                        cx.listener(Self::on_open_repository),
                    ))
                    .child(action_button(
                        Icon::GitBranch,
                        "Clone repository",
                        cx.listener(Self::on_clone_repository),
                    )),
            )
            .child(
                // Recent section
                div()
                    .flex()
                    .flex_col()
                    .gap(px(12.0))
                    .w(px(600.0))
                    .child(
                        div()
                            .flex()
                            .justify_between()
                            .child(div().text_color(rgb(0x888888)).child("Recent"))
                            .child(div().text_color(rgb(0x6b9eff)).child("View all")),
                    )
                    .children(recent_items),
            )
            .child(self.render_overlay(cx))
    }
}

impl WelcomeView {
    fn render_overlay(&self, cx: &Context<Self>) -> Div {
        let Some(ref overlay) = self.overlay else {
            return div().h(px(0.0));
        };

        let placeholder = if self.input.is_empty() {
            match overlay {
                WelcomeOverlay::CloneRepository => {
                    "Provide a repository URL e.g. \"git@github.com:username/project.git\""
                }
                WelcomeOverlay::CreateProject => "What do you want to build?",
            }
        } else {
            ""
        };

        let suggestions = if overlay == &WelcomeOverlay::CreateProject {
            self.create_project_suggestions()
        } else {
            vec![]
        };

        div()
            .size_full()
            .absolute()
            .top_0()
            .left_0()
            .flex()
            .items_center()
            .justify_center()
            .bg(rgba(0x00000088))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_overlay_dismiss))
            .child(
                div()
                    .w(px(600.0))
                    .flex()
                    .flex_col()
                    .gap(px(16.0))
                    .on_mouse_down(MouseButton::Left, |_, _, _| {}) // Prevent dismiss when clicking inside
                    .child(
                        // Main prompt input
                        div()
                            .flex()
                            .items_center()
                            .gap(px(12.0))
                            .px(px(16.0))
                            .py(px(20.0))
                            .rounded(px(8.0))
                            .bg(rgb(0x0a0a0a))
                            .border_1()
                            .border_color(rgb(0x1a1a1a))
                            .shadow_lg()
                            .child(lucide_icon(
                                if overlay == &WelcomeOverlay::CloneRepository {
                                    Icon::GitBranch
                                } else {
                                    Icon::Sparkles
                                },
                                18.0,
                                0x888888,
                            ))
                            .child(
                                div()
                                    .flex_1()
                                    .relative()
                                    .child(
                                        div()
                                            .text_size(px(14.0))
                                            .text_color(rgb(0x888888))
                                            .child(placeholder),
                                    )
                                    .child(
                                        div()
                                            .absolute()
                                            .top_0()
                                            .left_0()
                                            .flex()
                                            .items_center()
                                            .child(
                                                div()
                                                    .text_size(px(14.0))
                                                    .text_color(rgb(0xeeeeee))
                                                    .child(self.input.clone()),
                                            )
                                            .child(
                                                // Cursor
                                                div().w(px(2.0)).h(px(16.0)).bg(rgb(0x6b9eff)),
                                            ),
                                    ),
                            ),
                    )
                    .child(if !suggestions.is_empty() && self.input.is_empty() {
                        div().flex().flex_col().gap(px(4.0)).children(
                            suggestions.into_iter().enumerate().map(|(i, s)| {
                                let is_selected = i == self.suggest_index;
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(12.0))
                                    .px(px(12.0))
                                    .py(px(8.0))
                                    .rounded(px(6.0))
                                    .bg(if is_selected {
                                        rgb(0x1a1a1a)
                                    } else {
                                        rgb(0x000000)
                                    })
                                    .child(lucide_icon(Icon::MessageSquarePlus, 14.0, 0x666666))
                                    .child(
                                        div()
                                            .text_size(px(13.0))
                                            .text_color(if is_selected {
                                                rgb(0xeeeeee)
                                            } else {
                                                rgb(0x888888)
                                            })
                                            .child(s),
                                    )
                            }),
                        )
                    } else {
                        div()
                    }),
            )
    }
}

fn action_button(
    icon: Icon,
    label: &'static str,
    on_click: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
) -> Div {
    div()
        .flex()
        .flex_col()
        .items_center()
        .gap(px(8.0))
        .p(px(16.0))
        .bg(rgb(0x1a1a1a))
        .rounded(px(8.0))
        .border_1()
        .border_color(rgb(0x2a2a2a))
        .on_mouse_down(gpui::MouseButton::Left, move |event, window, cx| {
            on_click(event, window, cx)
        })
        .child(lucide_icon(icon, 20.0, 0xcccccc))
        .child(
            div()
                .text_size(px(12.0))
                .text_color(rgb(0xcccccc))
                .child(label),
        )
}

// recent items are rendered in WelcomeView to attach click handlers

impl Focusable for WelcomeView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl EventEmitter<OpenRepositoryEvent> for WelcomeView {}
