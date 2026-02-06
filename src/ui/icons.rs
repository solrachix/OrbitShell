use gpui::*;
use lucide_icons::Icon;

pub fn lucide_icon(icon: Icon, size: f32, color: u32) -> Div {
    div()
        .font_family("lucide")
        .text_size(px(size))
        .text_color(rgb(color))
        .child(char::from(icon).to_string())
}
