use gpui::*;
use lucide_icons::Icon;

pub fn lucide_icon(icon: Icon, size: f32, color: u32) -> Div {
    div()
        .font_family("lucide")
        .text_size(px(size))
        .text_color(rgb(color))
        .child(char::from(icon).to_string())
}

pub fn registry_avatar(label: &str, has_icon: bool) -> Div {
    let letter = label
        .chars()
        .find(|ch| ch.is_alphanumeric())
        .map(|ch| ch.to_uppercase().collect::<String>())
        .unwrap_or_else(|| "?".to_string());

    div()
        .w(px(20.0))
        .h(px(20.0))
        .rounded(px(5.0))
        .border_1()
        .border_color(if has_icon {
            rgb(0x335f2d)
        } else {
            rgb(0x2a2a2a)
        })
        .bg(if has_icon {
            rgb(0x1c2b1a)
        } else {
            rgb(0x151515)
        })
        .flex()
        .items_center()
        .justify_center()
        .text_size(px(10.0))
        .text_color(if has_icon {
            rgb(0xb7e3a1)
        } else {
            rgb(0x8f8f8f)
        })
        .child(letter)
}

#[cfg(test)]
mod tests {
    use super::registry_avatar;

    #[test]
    fn registry_avatar_uses_first_alphanumeric_letter() {
        let _ = registry_avatar("Codex CLI", true);
        let _ = registry_avatar("  ", false);
    }
}
