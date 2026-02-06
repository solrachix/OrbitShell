use gpui::AnimationExt as _;
use gpui::*;
use lucide_icons::Icon;
use std::time::Duration;

use crate::ui::icons::lucide_icon;
use crate::ui::move_index;
use crate::ui::text_edit::TextEditState;

const ACCENT: u32 = 0x6b9eff;
const ACCENT_BG: u32 = 0x6b9eff22;
const ACCENT_BORDER: u32 = 0x6b9eff66;

const TAB_H: f32 = 30.0;
const BAR_H: f32 = 44.0;
const PAD_X: f32 = 10.0;
const GAP: f32 = 10.0;

pub enum TabBarEvent {
    NewTab,
    Activate(usize),
    Close(usize),
    ToggleSidebar,
    Reorder(usize, usize),
    ToggleUserMenu,
}

#[derive(Clone)]
struct Tab {
    id: u64,
    name: String,
    path: String,

    // reorder animation
    anim_offset: f32,
    anim_token: u64,
}

pub struct TabBar {
    tabs: Vec<Tab>,
    active_tab: usize,

    // inline rename
    editing_index: Option<usize>,
    edit_value: String,
    edit_cursor: usize,
    edit_selection: Option<(usize, usize)>,
    edit_anchor: Option<usize>,
    edit_original: String,

    sidebar_visible: bool,

    // reorder
    dragging_index: Option<usize>,
    drag_over_index: Option<usize>,
    drag_pending: Option<(usize, f32)>, // (index, start_x) to start drag after threshold
    drag_start_x: Option<f32>,
    drag_delta_x: f32,

    focus_handle: FocusHandle,
    next_tab_id: u64,
}

impl TabBar {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            tabs: Vec::new(),
            active_tab: 0,

            editing_index: None,
            edit_value: String::new(),
            edit_cursor: 0,
            edit_selection: None,
            edit_anchor: None,
            edit_original: String::new(),

            sidebar_visible: true,

            dragging_index: None,
            drag_over_index: None,
            drag_pending: None,
            drag_start_x: None,
            drag_delta_x: 0.0,

            focus_handle: cx.focus_handle(),
            next_tab_id: 1,
        }
    }

    pub fn set_sidebar_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        self.sidebar_visible = visible;
        cx.notify();
    }

    pub fn add_tab(&mut self, name: String, path: String, cx: &mut Context<Self>) {
        let id = self.next_tab_id;
        self.next_tab_id = self.next_tab_id.wrapping_add(1);

        self.tabs.push(Tab {
            id,
            name,
            path,
            anim_offset: 0.0,
            anim_token: 0,
        });

        self.active_tab = self.tabs.len().saturating_sub(1);
        cx.notify();
    }

    pub fn rename_tab(&mut self, index: usize, name: String, path: String, cx: &mut Context<Self>) {
        if let Some(tab) = self.tabs.get_mut(index) {
            tab.name = name;
            tab.path = path;
            cx.notify();
        }
    }

    pub fn close_tab(&mut self, index: usize, cx: &mut Context<Self>) {
        if self.tabs.len() <= 1 || index >= self.tabs.len() {
            return;
        }
        self.tabs.remove(index);

        if self.active_tab >= self.tabs.len() {
            self.active_tab = self.tabs.len().saturating_sub(1);
        }

        // if you closed a tab before the active one, active shifts left
        if index < self.active_tab {
            self.active_tab = self.active_tab.saturating_sub(1);
        }

        cx.notify();
    }

    pub fn set_active(&mut self, index: usize, cx: &mut Context<Self>) {
        if index < self.tabs.len() {
            self.active_tab = index;
            cx.notify();
        }
    }

    // --------------------------
    // Edit (rename)
    // --------------------------

    fn start_edit_tab(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(index) else {
            return;
        };
        self.editing_index = Some(index);
        self.edit_value = tab.name.clone();
        self.edit_original = tab.name.clone();
        self.edit_cursor = self.edit_value.chars().count();
        self.edit_selection = None;
        self.edit_anchor = None;
        cx.notify();
    }

    fn clear_edit(&mut self) {
        self.editing_index = None;
        self.edit_value.clear();
        self.edit_original.clear();
        self.edit_cursor = 0;
        self.edit_selection = None;
        self.edit_anchor = None;
    }

    fn commit_tab_edit(&mut self, cx: &mut Context<Self>) {
        if let Some(index) = self.editing_index {
            let trimmed = self.edit_value.trim();
            let final_name = if trimmed.is_empty() {
                self.edit_original.trim()
            } else {
                trimmed
            };

            if !final_name.is_empty() {
                if let Some(tab) = self.tabs.get_mut(index) {
                    tab.name = final_name.to_string();
                }
            }
        }

        self.clear_edit();
        cx.notify();
    }

    fn cancel_tab_edit(&mut self, cx: &mut Context<Self>) {
        self.clear_edit();
        cx.notify();
    }

    fn split_edit_at_cursor(&self) -> (String, String) {
        TextEditState::split_at_cursor(&self.edit_value, self.edit_cursor)
    }

    fn insert_edit_text(&mut self, text: &str) {
        TextEditState::insert_text(
            &mut self.edit_value,
            &mut self.edit_cursor,
            &mut self.edit_selection,
            &mut self.edit_anchor,
            text,
        );
    }

    fn pop_edit_char_before_cursor(&mut self) {
        TextEditState::pop_char_before_cursor(
            &mut self.edit_value,
            &mut self.edit_cursor,
            &mut self.edit_selection,
            &mut self.edit_anchor,
        );
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        if self.editing_index.is_none() {
            return;
        }

        match event.keystroke.key.as_str() {
            "enter" | "return" | "numpadenter" => {
                self.commit_tab_edit(cx);
                cx.stop_propagation();
            }
            "escape" => {
                self.cancel_tab_edit(cx);
                cx.stop_propagation();
            }
            "backspace" => {
                self.pop_edit_char_before_cursor();
                cx.notify();
                cx.stop_propagation();
            }
            "left" | "arrowleft" => {
                self.edit_cursor = self.edit_cursor.saturating_sub(1);
                cx.notify();
                cx.stop_propagation();
            }
            "right" | "arrowright" => {
                let max = self.edit_value.chars().count();
                if self.edit_cursor < max {
                    self.edit_cursor += 1;
                }
                cx.notify();
                cx.stop_propagation();
            }
            _ => {
                if let Some(text) = event.keystroke.key_char.as_deref() {
                    if !text.is_empty() {
                        self.insert_edit_text(text);
                        cx.notify();
                        cx.stop_propagation();
                    }
                } else if event.keystroke.key.len() == 1 {
                    self.insert_edit_text(&event.keystroke.key);
                    cx.notify();
                    cx.stop_propagation();
                }
            }
        }
    }

    fn on_edit_mouse_down_out(
        &mut self,
        _event: &MouseDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.editing_index.is_some() {
            self.commit_tab_edit(cx);
        }
    }

    // --------------------------
    // Drag reorder
    // --------------------------

    fn start_drag(&mut self, index: usize, start_x: f32) {
        if self.editing_index.is_some() {
            return;
        }
        self.dragging_index = Some(index);
        self.drag_over_index = Some(index);
        self.drag_pending = None;
        self.drag_start_x = Some(start_x);
        self.drag_delta_x = 0.0;
    }

    fn end_drag(&mut self) {
        self.dragging_index = None;
        self.drag_over_index = None;
        self.drag_pending = None;
        self.drag_start_x = None;
        self.drag_delta_x = 0.0;
    }

    fn tab_width(&self, index: usize) -> f32 {
        if let Some(tab) = self.tabs.get(index) {
            let name_w = tab.name.chars().count() as f32 * 7.5;
            (name_w + 44.0).max(80.0)
        } else {
            120.0
        }
    }

    fn cumulative_tab_x(&self, index: usize) -> f32 {
        let mut x = 0.0;
        let tab_gap = 6.0;
        for i in 0..index {
            x += self.tab_width(i) + tab_gap;
        }
        x
    }

    fn on_drag_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !event.dragging() {
            return;
        }

        let x: f32 = event.position.x.into();

        if self.dragging_index.is_none() {
            let Some((index, start_x)) = self.drag_pending else {
                return;
            };
            if (x - start_x).abs() >= 3.0 {
                self.start_drag(index, x);
            } else {
                return;
            }
        }

        let Some(from) = self.dragging_index else {
            return;
        };

        let start_x = self.drag_start_x.unwrap_or(x);
        self.drag_delta_x = x - start_x;

        // Hit test against the "new" potential layout slots
        let drag_w = self.tab_width(from);
        let drag_center = self.cumulative_tab_x(from) + self.drag_delta_x + drag_w / 2.0;

        // Best: hit test against original centers to find target index
        let mut best_index = 0;
        let mut min_dist = f32::MAX;
        for i in 0..self.tabs.len() {
            let center = self.cumulative_tab_x(i) + self.tab_width(i) / 2.0;
            let dist = (drag_center - center).abs();
            if dist < min_dist {
                min_dist = dist;
                best_index = i;
            }
        }
        let to = best_index;

        if self.drag_over_index != Some(to) {
            self.drag_over_index = Some(to);
            cx.notify();
        } else {
            cx.notify();
        }
    }

    fn on_drag_end(&mut self, _event: &MouseUpEvent, _window: &mut Window, cx: &mut Context<Self>) {
        if self.dragging_index.is_none() {
            self.drag_pending = None;
            self.drag_start_x = None;
            self.drag_delta_x = 0.0;
            cx.stop_propagation();
            return;
        }

        let from = self.dragging_index.unwrap();
        let to = self.drag_over_index.unwrap_or(from);
        let drag_delta = self.drag_delta_x;

        // 1. Calculate current visual positions of ALL tabs
        let mut visual_positions = Vec::new();
        let tab_widths: Vec<f32> = (0..self.tabs.len()).map(|i| self.tab_width(i)).collect();
        let cumulative_xs: Vec<f32> = (0..self.tabs.len())
            .map(|i| self.cumulative_tab_x(i))
            .collect();
        let from_width = tab_widths[from];

        for i in 0..self.tabs.len() {
            let actual_x = cumulative_xs[i];
            let visual_x = if i == from {
                actual_x + drag_delta
            } else {
                let shift = if to > from && i > from && i <= to {
                    -(from_width + 6.0)
                } else if to < from && i >= to && i < from {
                    from_width + 6.0
                } else {
                    0.0
                };
                actual_x + shift
            };
            visual_positions.push((self.tabs[i].id, visual_x));
        }

        // 2. Perform the actual move
        if from != to {
            let tab = self.tabs.remove(from);
            self.tabs.insert(to, tab);

            self.active_tab = move_index(self.active_tab, from, to);
            if let Some(edit) = self.editing_index {
                self.editing_index = Some(move_index(edit, from, to));
            }
            cx.emit(TabBarEvent::Reorder(from, to));
        }

        // 3. Set settling animations: (old_visual_x - new_actual_x)
        let new_cumulative_xs: Vec<f32> = (0..self.tabs.len())
            .map(|i| self.cumulative_tab_x(i))
            .collect();
        for (i, tab) in self.tabs.iter_mut().enumerate() {
            let new_actual_x = new_cumulative_xs[i];
            if let Some((_, old_visual_x)) = visual_positions.iter().find(|(id, _)| *id == tab.id) {
                let delta = old_visual_x - new_actual_x;
                if delta.abs() > 0.1 {
                    tab.anim_offset = delta;
                    tab.anim_token = tab.anim_token.wrapping_add(1);
                } else {
                    tab.anim_offset = 0.0;
                }
            }
        }

        self.end_drag();
        cx.notify();
        cx.stop_propagation();
    }

    // --------------------------
    // UI events
    // --------------------------

    fn on_new_tab(
        &mut self,
        _event: &MouseDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.stop_propagation();
        cx.emit(TabBarEvent::NewTab);
    }

    fn on_close_tab(&mut self, index: usize, cx: &mut Context<Self>) {
        self.close_tab(index, cx);
        cx.emit(TabBarEvent::Close(index));
    }

    fn on_toggle_sidebar(
        &mut self,
        _event: &MouseDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.stop_propagation();
        cx.emit(TabBarEvent::ToggleSidebar);
    }

    fn on_activate_tab(&mut self, index: usize, cx: &mut Context<Self>) {
        self.set_active(index, cx);
        self.clear_edit();
        cx.emit(TabBarEvent::Activate(index));
    }

    // --------------------------
    // Styling helpers
    // --------------------------

    fn chrome_button(&self, icon: Icon, fg: u32) -> Div {
        div()
            .flex()
            .items_center()
            .justify_center()
            .w(px(26.0))
            .h(px(26.0))
            .rounded(px(6.0))
            .bg(rgb(0x151515))
            .border_1()
            .border_color(rgb(0x2a2a2a))
            .occlude()
            .child(lucide_icon(icon, 12.0, fg))
    }
}

impl Render for TabBar {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.active_tab >= self.tabs.len() && !self.tabs.is_empty() {
            self.active_tab = self.tabs.len() - 1;
        }

        let active_tab = self.active_tab;
        let sidebar_active = self.sidebar_visible;

        let root = div()
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(Self::on_key_down))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_drag_end))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_drag_end))
            .relative()
            .flex()
            .items_center()
            .size_full()
            .h(px(BAR_H))
            .bg(rgb(0x1a1a1a))
            .border_b_1()
            .border_color(rgb(0x2a2a2a))
            .px(px(PAD_X))
            .gap(px(GAP))
            // left controls
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(8.0))
                    .child(
                        // sidebar toggle
                        div()
                            .flex()
                            .items_center()
                            .justify_center()
                            .w(px(28.0))
                            .h(px(28.0))
                            .rounded(px(6.0))
                            .bg(if sidebar_active {
                                rgba(ACCENT_BG)
                            } else {
                                rgb(0x151515)
                            })
                            .border_1()
                            .border_color(if sidebar_active {
                                rgba(ACCENT_BORDER)
                            } else {
                                rgb(0x2a2a2a)
                            })
                            .occlude()
                            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_toggle_sidebar))
                            .child(lucide_icon(
                                Icon::PanelLeft,
                                14.0,
                                if sidebar_active { ACCENT } else { 0x9a9a9a },
                            )),
                    )
                    .child(div()),
            )
            // tabs
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .min_w(px(200.0))
                    .flex_none()
                    .on_mouse_move(cx.listener(Self::on_drag_mouse_move))
                    .relative()
                    .children({
                        let mut elements: Vec<AnyElement> = Vec::with_capacity(self.tabs.len() + 1);
                        let mut dragged: Option<AnyElement> = None;

                        let drag_from = self.dragging_index;
                        let drag_over = self.drag_over_index;

                        for (i, tab) in self.tabs.iter().enumerate() {
                            let is_active = i == active_tab;
                            let is_dragging = drag_from == Some(i);
                            let is_editing = self.editing_index == Some(i);

                            let (edit_left, edit_right) = if is_editing {
                                self.split_edit_at_cursor()
                            } else {
                                (String::new(), String::new())
                            };

                            let focus_handle = self.focus_handle.clone();
                            let handle = cx.entity().downgrade();
                            let handle_down = handle.clone();

                            let mut tab_container = div()
                                .flex()
                                .items_center()
                                .gap(px(8.0))
                                .px(px(10.0))
                                .w(px(self.tab_width(i)))
                                .h(px(TAB_H))
                                .rounded(px(6.0))
                                .bg(if is_active || is_dragging {
                                    rgba(ACCENT_BG)
                                } else {
                                    rgb(0x151515)
                                })
                                .border_1()
                                .border_color(if is_editing {
                                    rgb(ACCENT)
                                } else if is_active || is_dragging {
                                    rgba(ACCENT_BORDER)
                                } else {
                                    rgb(0x2a2a2a)
                                })
                                .occlude()
                                .cursor(if is_dragging {
                                    CursorStyle::ClosedHand
                                } else {
                                    CursorStyle::OpenHand
                                })
                                .on_mouse_down(MouseButton::Left, {
                                    let index = i;
                                    move |event, window, cx| {
                                        cx.stop_propagation();

                                        if event.click_count >= 2 {
                                            window.focus(&focus_handle);
                                            let _ = handle_down.update(cx, |view, cx| {
                                                view.start_edit_tab(index, cx);
                                                view.end_drag();
                                            });
                                            return;
                                        }

                                        let _ = handle_down.update(cx, |view, cx| {
                                            let start_x: f32 = event.position.x.into();
                                            view.drag_pending = Some((index, start_x));
                                            view.on_activate_tab(index, cx);
                                        });
                                    }
                                })
                                .child(if is_editing {
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap(px(2.0))
                                        .text_size(px(12.0))
                                        .text_color(rgb(0xffffff))
                                        .font_family("Cascadia Code")
                                        .child(edit_left)
                                        .child(div().w(px(2.0)).h(px(14.0)).bg(rgb(ACCENT)))
                                        .child(edit_right)
                                } else {
                                    div()
                                        .text_size(px(12.0))
                                        .text_color(if is_active {
                                            rgb(0xffffff)
                                        } else {
                                            rgb(0x7a7a7a)
                                        })
                                        .font_family("Cascadia Code")
                                        .child(tab.name.clone())
                                })
                                .child(
                                    div()
                                        .on_mouse_down(MouseButton::Left, {
                                            let index = i;
                                            let handle = cx.entity().downgrade();
                                            move |_event, _window, cx| {
                                                cx.stop_propagation();
                                                let _ = handle.update(cx, |view, cx| {
                                                    view.on_close_tab(index, cx);
                                                });
                                            }
                                        })
                                        .child(lucide_icon(Icon::X, 12.0, 0x666666)),
                                );

                            if is_editing {
                                tab_container = tab_container
                                    .on_mouse_down_out(cx.listener(Self::on_edit_mouse_down_out));
                            }

                            if is_dragging {
                                let x_offset = self.cumulative_tab_x(i) + self.drag_delta_x;
                                dragged = Some(
                                    tab_container
                                        .absolute()
                                        .left(px(x_offset))
                                        .top(px(0.0))
                                        .into_any_element(),
                                );
                                // Render a placeholder in the flex flow
                                elements.push(div().w(px(self.tab_width(i))).into_any_element());
                            } else {
                                let shift = if let (Some(from), Some(over)) = (drag_from, drag_over)
                                {
                                    let from_w = self.tab_width(from) + 6.0;
                                    if over > from && i > from && i <= over {
                                        -from_w
                                    } else if over < from && i >= over && i < from {
                                        from_w
                                    } else {
                                        0.0
                                    }
                                } else {
                                    0.0
                                };

                                let tab_element: AnyElement = if shift.abs() > 0.1 {
                                    tab_container
                                        .with_animation(
                                            "tab_drag_shift",
                                            Animation::new(Duration::from_millis(150))
                                                .with_easing(ease_in_out),
                                            move |el, delta| el.relative().left(px(shift * delta)),
                                        )
                                        .into_any_element()
                                } else if tab.anim_offset.abs() > 0.1 {
                                    let offset = tab.anim_offset;
                                    let anim_key = (tab.id << 32) ^ (tab.anim_token & 0xffff_ffff);

                                    tab_container
                                        .with_animation(
                                            ("tab_shift", anim_key),
                                            Animation::new(Duration::from_millis(160))
                                                .with_easing(ease_in_out),
                                            move |el, delta| {
                                                let x = offset * (1.0 - delta);
                                                el.relative().left(px(x))
                                            },
                                        )
                                        .into_any_element()
                                } else {
                                    tab_container.into_any_element()
                                };
                                elements.push(tab_element);
                            }
                        }

                        if let Some(d) = dragged {
                            elements.push(d);
                        }

                        elements
                    })
                    // + button
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_center()
                            .w(px(30.0))
                            .h(px(30.0))
                            .rounded(px(6.0))
                            .bg(rgb(0x151515))
                            .border_1()
                            .border_color(rgb(0x2a2a2a))
                            .occlude()
                            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_new_tab))
                            .child(lucide_icon(Icon::Plus, 14.0, 0x9a9a9a)),
                    )
                    .child(div()),
            )
            // drag area spacer
            .child(
                div()
                    .flex_1()
                    .h(px(BAR_H))
                    .min_w(px(120.0))
                    .bg(rgb(0x1a1a1a))
                    .occlude()
                    .cursor(CursorStyle::OpenHand)
                    .window_control_area(WindowControlArea::Drag),
            )
            // window controls + user menu
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(10.0))
                    .child(
                        // user avatar
                        div()
                            .flex()
                            .items_center()
                            .justify_center()
                            .w(px(28.0))
                            .h(px(28.0))
                            .rounded(px(999.0))
                            .bg(rgb(0x1f1f1f))
                            .border_1()
                            .border_color(rgb(0x2a2a2a))
                            .cursor(CursorStyle::PointingHand)
                            .occlude()
                            .on_mouse_down(MouseButton::Left, {
                                let handle = cx.entity().downgrade();
                                move |_event, _window, cx| {
                                    cx.stop_propagation();
                                    let _ = handle.update(cx, |_view, cx| {
                                        cx.emit(TabBarEvent::ToggleUserMenu);
                                    });
                                }
                            })
                            .child(
                                div()
                                    .text_size(px(11.0))
                                    .text_color(rgb(0xdddddd))
                                    .child("S"),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.0))
                            .child(self.chrome_button(Icon::Minimize, 0x9a9a9a).on_mouse_down(
                                MouseButton::Left,
                                |_event, window, cx| {
                                    cx.stop_propagation();
                                    window.minimize_window();
                                },
                            ))
                            .child(self.chrome_button(Icon::Maximize2, 0x9a9a9a).on_mouse_down(
                                MouseButton::Left,
                                |_event, window, cx| {
                                    cx.stop_propagation();
                                    window.zoom_window();
                                },
                            ))
                            .child(self.chrome_button(Icon::X, 0xc86b6b).on_mouse_down(
                                MouseButton::Left,
                                |_event, _window, cx| {
                                    cx.stop_propagation();
                                    cx.quit();
                                },
                            )),
                    ),
            );

        root
    }
}

impl EventEmitter<TabBarEvent> for TabBar {}
