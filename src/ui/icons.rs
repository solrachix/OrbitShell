use gpui::*;
use lucide_icons::Icon;

pub fn lucide_icon(icon: Icon, size: f32, color: u32) -> Div {
    div()
        .font_family("lucide")
        .text_size(px(size))
        .text_color(rgb(color))
        .child(char::from(icon).to_string())
}

pub fn lucide_icon_button(icon: Icon, size: f32, color: u32) -> Div {
    div()
        .flex()
        .items_center()
        .justify_center()
        .w(px(size + 10.0))
        .h(px(size + 10.0))
        .rounded(px(6.0))
        .bg(rgb(0x1a1a1a))
        .border_1()
        .border_color(rgb(0x2a2a2a))
        .child(lucide_icon(icon, size, color))
}
