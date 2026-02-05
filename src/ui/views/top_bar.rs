use gpui::*;

pub struct TopBar {
    sidebar_visible: bool,
}

impl TopBar {
    pub fn new() -> Self {
        Self {
            sidebar_visible: true,
        }
    }

    fn on_toggle_sidebar(
        &mut self,
        _event: &MouseDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.sidebar_visible = !self.sidebar_visible;
        cx.notify();
        // TODO: Emit event to Workspace to toggle sidebar
    }
}

impl Render for TopBar {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .items_center()
            .justify_between()
            .h(px(36.0))
            .bg(rgb(0x0a0a0a))
            .border_b_1()
            .border_color(rgb(0x2a2a2a))
            .px(px(12.0))
            .child(
                // Left side - sidebar toggle
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.0))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_center()
                            .w(px(28.0))
                            .h(px(28.0))
                            .rounded(px(4.0))
                            .on_mouse_down(
                                gpui::MouseButton::Left,
                                cx.listener(Self::on_toggle_sidebar),
                            )
                            .child(
                                div()
                                    .text_size(px(16.0))
                                    .text_color(rgb(0x888888))
                                    .child("☰"),
                            ),
                    )
                    .child(
                        div()
                            .text_size(px(13.0))
                            .text_color(rgb(0xcccccc))
                            .child("OrbitShell"),
                    ),
            )
            .child(
                // Right side - update button
                div().flex().items_center().gap(px(8.0)).child(
                    // Single custom window control
                    div()
                        .w(px(28.0))
                        .h(px(28.0))
                        .rounded(px(14.0))
                        .bg(rgb(0x1a1a1a))
                        .border_1()
                        .border_color(rgb(0x2a2a2a))
                        .child(
                            div()
                                .text_size(px(12.0))
                                .text_color(rgb(0x888888))
                                .child("×"),
                        ),
                ),
            )
    }
}
