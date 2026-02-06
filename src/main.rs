use gpui::*;
use std::borrow::Cow;

mod git;
mod terminal;
mod ui;

fn main() {
    Application::new().run(|cx: &mut App| {
        cx.text_system()
            .add_fonts(vec![Cow::Borrowed(lucide_icons::LUCIDE_FONT_BYTES)])
            .ok();
        let mut options = WindowOptions::default();
        // Use a custom client-decorated window so our TabBar acts as the title bar.
        options.titlebar = Some(TitlebarOptions {
            title: Some("OrbitShell".into()),
            appears_transparent: true,
            ..Default::default()
        });
        options.window_decorations = Some(WindowDecorations::Client);

        cx.open_window(options, |_, cx| cx.new(|cx| ui::Workspace::new(cx)))
            .expect("failed to open window");
    });
}
