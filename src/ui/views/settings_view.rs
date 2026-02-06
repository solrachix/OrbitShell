use gpui::*;
use lucide_icons::Icon;

use crate::ui::icons::lucide_icon;
use crate::ui::text_edit::TextEditState;

const ACCENT: u32 = 0x6b9eff;
const ACCENT_BORDER: u32 = 0x6b9eff66;

pub struct SettingsView {
    sections: Vec<&'static str>,
    active_section: usize,
    focus_handle: FocusHandle,
    search_query: String,
    search_cursor: usize,
    search_selection: Option<(usize, usize)>,
    search_anchor: Option<usize>,
}

impl SettingsView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            sections: vec![
                "Account",
                "Code",
                "Appearance",
                "Keyboard shortcuts",
                "Referrals",
                "MCP servers",
                "Privacy",
                "About",
            ],
            active_section: 0,
            focus_handle: cx.focus_handle(),
            search_query: String::new(),
            search_cursor: 0,
            search_selection: None,
            search_anchor: None,
        }
    }

    pub fn set_active_section(&mut self, section: &str, cx: &mut Context<Self>) {
        if let Some(index) = self.sections.iter().position(|s| *s == section) {
            self.active_section = index;
            cx.notify();
        }
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
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
                    if let Some((a, b)) = TextEditState::normalized_selection(self.search_selection)
                    {
                        self.search_cursor = a.min(b);
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
                } else if let Some((a, b)) =
                    TextEditState::normalized_selection(self.search_selection)
                {
                    self.search_cursor = a.max(b);
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

    fn render_search_input(&self, is_focused: bool) -> Div {
        let placeholder = self.search_query.is_empty();
        let caret = div()
            .w(px(2.0))
            .h(px(16.0))
            .rounded(px(1.0))
            .bg(if is_focused {
                rgb(ACCENT)
            } else {
                rgb(0x2a2a2a)
            });

        let text_normal = |text: String| {
            div()
                .text_size(px(13.0))
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
                        .text_size(px(13.0))
                        .text_color(rgb(0xf0f0f0))
                        .font_family("Cascadia Code")
                        .child(text),
                )
        };

        if placeholder {
            return div()
                .flex()
                .items_center()
                .gap(px(6.0))
                .child(lucide_icon(Icon::Search, 14.0, 0x7a7a7a))
                .child(caret)
                .child(
                    div()
                        .text_size(px(13.0))
                        .text_color(rgb(0x666666))
                        .child("Search"),
                );
        }

        if let Some((a, b)) =
            TextEditState::normalized_selection(self.search_selection).filter(|(a, b)| a != b)
        {
            let (pre, rest) = split_string(&self.search_query, a);
            let (sel, post) = split_string(&rest, b.saturating_sub(a));
            return div()
                .flex()
                .items_center()
                .gap(px(0.0))
                .child(text_normal(pre))
                .child(text_selected(sel))
                .child(caret)
                .child(text_normal(post));
        }

        let (left, right) = TextEditState::split_at_cursor(&self.search_query, self.search_cursor);
        div()
            .flex()
            .items_center()
            .gap(px(0.0))
            .child(text_normal(left))
            .child(caret)
            .child(text_normal(right))
    }

    fn render_toggle(&self, on: bool) -> Div {
        div()
            .w(px(44.0))
            .h(px(24.0))
            .rounded(px(999.0))
            .bg(if on { rgb(ACCENT) } else { rgb(0x2a2a2a) })
            .child(
                div()
                    .w(px(20.0))
                    .h(px(20.0))
                    .rounded(px(999.0))
                    .bg(rgb(0xffffff))
                    .ml(px(if on { 22.0 } else { 2.0 })),
            )
    }

    fn render_kbd_chip(&self, label: &str, active: bool) -> Div {
        div()
            .px(px(8.0))
            .py(px(4.0))
            .rounded(px(6.0))
            .bg(if active { rgb(0x1b1b1b) } else { rgb(0x111111) })
            .border_1()
            .border_color(if active { rgb(0xf0b44c) } else { rgb(0x2a2a2a) })
            .text_size(px(11.0))
            .text_color(if active { rgb(0xf0b44c) } else { rgb(0x9a9a9a) })
            .child(label.to_string())
    }

    fn render_section_content(&self) -> Div {
        let title = self.sections[self.active_section];

        let mut content = div().flex().flex_col().gap(px(16.0)).child(
            div()
                .text_size(px(20.0))
                .text_color(rgb(0xffffff))
                .child(title),
        );

        match title {
            "Account" => {
                content = content
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(12.0))
                            .child(
                                div()
                                    .w(px(36.0))
                                    .h(px(36.0))
                                    .rounded(px(999.0))
                                    .bg(rgb(0x1f1f1f))
                                    .border_1()
                                    .border_color(rgb(0x2a2a2a))
                                    .child(
                                        div()
                                            .text_size(px(12.0))
                                            .text_color(rgb(0xdddddd))
                                            .child("S"),
                                    ),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .child(
                                        div()
                                            .text_size(px(13.0))
                                            .text_color(rgb(0xffffff))
                                            .child("Solra"),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(12.0))
                                            .text_color(rgb(0x8a8a8a))
                                            .child("solra@email.com"),
                                    ),
                            ),
                    )
                    .child(div().h(px(1.0)).bg(rgb(0x1f1f1f)))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .text_size(px(12.0))
                                    .text_color(rgb(0x9a9a9a))
                                    .child("Settings sync"),
                            )
                            .child(self.render_toggle(true)),
                    )
                    .child(
                        div()
                            .text_size(px(12.0))
                            .text_color(rgb(0x8a8a8a))
                            .child("Earn rewards by sharing OrbitShell with friends & colleagues"),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(10.0))
                            .child(
                                div()
                                    .px(px(12.0))
                                    .py(px(6.0))
                                    .rounded(px(6.0))
                                    .bg(rgb(0x101010))
                                    .border_1()
                                    .border_color(rgb(0x2a2a2a))
                                    .text_size(px(12.0))
                                    .text_color(rgb(0xd0d0d0))
                                    .child("Refer a friend"),
                            )
                            .child(
                                div()
                                    .px(px(12.0))
                                    .py(px(6.0))
                                    .rounded(px(6.0))
                                    .bg(rgb(0x101010))
                                    .border_1()
                                    .border_color(rgb(0x2a2a2a))
                                    .text_size(px(12.0))
                                    .text_color(rgb(0xd0d0d0))
                                    .child("Relaunch OrbitShell"),
                            ),
                    )
                    .child(
                        div()
                            .text_size(px(12.0))
                            .text_color(rgb(0x8a8a8a))
                            .child("Version v0.2026.02.05-stable"),
                    )
                    .child(
                        div()
                            .px(px(12.0))
                            .py(px(6.0))
                            .rounded(px(6.0))
                            .bg(rgb(0x0f0f0f))
                            .border_1()
                            .border_color(rgb(0x2a2a2a))
                            .text_size(px(12.0))
                            .text_color(rgb(0xd0d0d0))
                            .child("Log out"),
                    );
            }
            "Code" => {
                content = content
                    .child(
                        div()
                            .text_size(px(13.0))
                            .text_color(rgb(0x9a9a9a))
                            .child("Codebase index"),
                    )
                    .child(
                        div()
                            .text_size(px(12.0))
                            .text_color(rgb(0x8a8a8a))
                            .child("OrbitShell can automatically index code repositories as you navigate them, helping agents quickly understand context."),
                    )
                    .child(div().h(px(1.0)).bg(rgb(0x1f1f1f)))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .text_size(px(12.0))
                                    .text_color(rgb(0x9a9a9a))
                                    .child("Index new folders by default"),
                            )
                            .child(self.render_toggle(false)),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .text_size(px(12.0))
                                    .text_color(rgb(0x8a8a8a))
                                    .child("Indexed folders"),
                            )
                            .child(
                                div()
                                    .px(px(10.0))
                                    .py(px(6.0))
                                    .rounded(px(6.0))
                                    .bg(rgb(0x101010))
                                    .border_1()
                                    .border_color(rgb(0x2a2a2a))
                                    .text_size(px(12.0))
                                    .text_color(rgb(0xd0d0d0))
                                    .child("Index new folder"),
                            ),
                    );
            }
            "Appearance" => {
                content = content
                    .child(
                        div()
                            .text_size(px(13.0))
                            .text_color(rgb(0x9a9a9a))
                            .child("Themes"),
                    )
                    .child(
                        div()
                            .text_size(px(12.0))
                            .text_color(rgb(0x8a8a8a))
                            .child("Create your own custom theme"),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .text_size(px(12.0))
                                    .text_color(rgb(0x9a9a9a))
                                    .child("Sync with OS"),
                            )
                            .child(self.render_toggle(false)),
                    )
                    .child(
                        div()
                            .rounded(px(10.0))
                            .bg(rgb(0x101010))
                            .border_1()
                            .border_color(rgb(0x1f1f1f))
                            .p(px(12.0))
                            .child(
                                div()
                                    .text_size(px(12.0))
                                    .text_color(rgb(0x9a9a9a))
                                    .child("Current theme"),
                            )
                            .child(
                                div()
                                    .mt(px(8.0))
                                    .text_size(px(12.0))
                                    .text_color(rgb(0xd0d0d0))
                                    .child("Dark"),
                            ),
                    )
                    .child(div().h(px(1.0)).bg(rgb(0x1f1f1f)))
                    .child(
                        div()
                            .text_size(px(13.0))
                            .text_color(rgb(0x9a9a9a))
                            .child("Window"),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .text_size(px(12.0))
                                    .text_color(rgb(0x9a9a9a))
                                    .child("Open new windows with custom size"),
                            )
                            .child(self.render_toggle(false)),
                    );
            }
            "Keyboard shortcuts" => {
                let rows = vec![
                    ("Accept Autosuggestion", vec!["Ctrl", "Shift", "→"], true),
                    ("Activate Next Tab", vec!["Ctrl", "PageDown"], false),
                    ("Activate Previous Tab", vec!["Ctrl", "PageUp"], false),
                    ("Add Cursor Above", vec!["Ctrl", "Shift", "↑"], true),
                    ("Add Cursor Below", vec!["Ctrl", "Shift", "↓"], true),
                    ("Alternate Terminal Paste", vec!["Ctrl", "V"], false),
                ];
                content =
                    content
                        .child(
                            div()
                                .text_size(px(12.0))
                                .text_color(rgb(0x8a8a8a))
                                .child("Configure keyboard shortcuts"),
                        )
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .justify_between()
                                .child(
                                    div()
                                        .text_size(px(12.0))
                                        .text_color(rgb(0x9a9a9a))
                                        .child("Command"),
                                )
                                .child(
                                    div()
                                        .text_size(px(12.0))
                                        .text_color(rgb(0x9a9a9a))
                                        .child("Shortcut"),
                                ),
                        )
                        .child(div().flex().flex_col().gap(px(8.0)).children(
                            rows.into_iter().map(|(label, keys, active)| {
                                div()
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .px(px(10.0))
                                    .py(px(8.0))
                                    .rounded(px(8.0))
                                    .bg(rgb(0x101010))
                                    .border_1()
                                    .border_color(rgb(0x1f1f1f))
                                    .child(
                                        div()
                                            .text_size(px(12.0))
                                            .text_color(rgb(0xd0d0d0))
                                            .child(label),
                                    )
                                    .child(
                                        div().flex().items_center().gap(px(6.0)).children(
                                            keys.into_iter()
                                                .map(|key| self.render_kbd_chip(key, active)),
                                        ),
                                    )
                            }),
                        ));
            }
            "Referrals" => {
                content = content
                    .child(
                        div()
                            .text_size(px(12.0))
                            .text_color(rgb(0x8a8a8a))
                            .child("Invite your team and earn rewards."),
                    )
                    .child(
                        div()
                            .px(px(12.0))
                            .py(px(8.0))
                            .rounded(px(6.0))
                            .bg(rgb(0x101010))
                            .border_1()
                            .border_color(rgb(0x2a2a2a))
                            .text_size(px(12.0))
                            .text_color(rgb(0xd0d0d0))
                            .child("Invite a friend"),
                    );
            }
            "MCP servers" => {
                content = content
                    .child(
                        div()
                            .text_size(px(12.0))
                            .text_color(rgb(0x8a8a8a))
                            .child("Manage MCP server connections."),
                    )
                    .child(
                        div()
                            .px(px(12.0))
                            .py(px(8.0))
                            .rounded(px(6.0))
                            .bg(rgb(0x101010))
                            .border_1()
                            .border_color(rgb(0x2a2a2a))
                            .text_size(px(12.0))
                            .text_color(rgb(0xd0d0d0))
                            .child("Add MCP Server"),
                    );
            }
            "Privacy" => {
                content = content
                    .child(
                        div()
                            .text_size(px(13.0))
                            .text_color(rgb(0x9a9a9a))
                            .child("Secret redaction"),
                    )
                    .child(
                        div()
                            .text_size(px(12.0))
                            .text_color(rgb(0x8a8a8a))
                            .child("When enabled, OrbitShell scans for sensitive info and prevents sending to servers."),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .text_size(px(12.0))
                                    .text_color(rgb(0x9a9a9a))
                                    .child("Help improve OrbitShell"),
                            )
                            .child(self.render_toggle(true)),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .text_size(px(12.0))
                                    .text_color(rgb(0x9a9a9a))
                                    .child("Send crash reports"),
                            )
                            .child(self.render_toggle(true)),
                    );
            }
            "About" => {
                content = content
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_center()
                            .h(px(260.0))
                            .child(
                                div()
                                    .text_size(px(54.0))
                                    .text_color(rgb(0xffffff))
                                    .child("OrbitShell"),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_size(px(12.0))
                            .text_color(rgb(0x8a8a8a))
                            .child("v0.2026.02.05-stable"),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_size(px(11.0))
                            .text_color(rgb(0x6f6f6f))
                            .child("Copyright 2026 OrbitShell"),
                    );
            }
            _ => {}
        }

        content
    }
}

fn split_string(input: &str, idx: usize) -> (String, String) {
    let mut left = String::new();
    let mut right = String::new();
    for (i, ch) in input.chars().enumerate() {
        if i < idx {
            left.push(ch);
        } else {
            right.push(ch);
        }
    }
    (left, right)
}

impl Render for SettingsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let is_focused = self.focus_handle.is_focused(window);

        let focus_handle = self.focus_handle.clone();

        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(rgb(0x0a0a0a))
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(Self::on_key_down))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .py(px(10.0))
                    .border_b_1()
                    .border_color(rgb(0x1f1f1f))
                    .child(
                        div()
                            .text_size(px(12.0))
                            .text_color(rgb(0x9a9a9a))
                            .child("Settings"),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h(px(0.0))
                    .child(
                        // Left menu
                        div()
                            .w(px(240.0))
                            .min_h(px(0.0))
                            .flex()
                            .flex_col()
                            .gap(px(10.0))
                            .p(px(16.0))
                            .border_r_1()
                            .border_color(rgb(0x1f1f1f))
                            .child(
                                div()
                                    .rounded(px(8.0))
                                    .bg(rgb(0x131313))
                                    .border_1()
                                    .border_color(rgb(0x2a2a2a))
                                    .px(px(10.0))
                                    .py(px(8.0))
                                    .on_mouse_down(MouseButton::Left, move |_event, window, cx| {
                                        cx.stop_propagation();
                                        window.focus(&focus_handle);
                                    })
                                    .child(self.render_search_input(is_focused)),
                            )
                            .child(div().flex().flex_col().gap(px(4.0)).children(
                                self.sections.iter().enumerate().map(|(i, label)| {
                                    let is_active = i == self.active_section;
                                    let handle = cx.entity().downgrade();
                                    div()
                                        .flex()
                                        .items_center()
                                        .px(px(10.0))
                                        .py(px(8.0))
                                        .rounded(px(6.0))
                                        .bg(if is_active {
                                            rgb(0x13354f)
                                        } else {
                                            rgb(0x0a0a0a)
                                        })
                                        .border_1()
                                        .border_color(if is_active {
                                            rgba(ACCENT_BORDER)
                                        } else {
                                            rgb(0x0a0a0a)
                                        })
                                        .cursor(CursorStyle::PointingHand)
                                        .on_mouse_down(MouseButton::Left, move |_e, _w, cx| {
                                            cx.stop_propagation();
                                            let _ = handle.update(cx, |view, cx| {
                                                view.active_section = i;
                                                cx.notify();
                                            });
                                        })
                                        .child(
                                            div()
                                                .text_size(px(13.0))
                                                .text_color(if is_active {
                                                    rgb(0xffffff)
                                                } else {
                                                    rgb(0xaaaaaa)
                                                })
                                                .child((*label).to_string()),
                                        )
                                }),
                            )),
                    )
                    .child(
                        // Right content
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_h(px(0.0))
                            .p(px(28.0))
                            .gap(px(18.0))
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .flex_1()
                                    .min_h(px(0.0))
                                    .gap(px(16.0))
                                    .child(self.render_section_content()),
                            ),
                    ),
            )
    }
}
