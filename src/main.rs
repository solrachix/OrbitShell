use gpui::*;
use orbitshell::ui;
use std::borrow::Cow;

fn main() {
    let launch_options = ui::launch::LaunchOptions::from_args(std::env::args_os());

    Application::new().run(move |cx: &mut App| {
        cx.text_system()
            .add_fonts(vec![Cow::Borrowed(lucide_icons::LUCIDE_FONT_BYTES)])
            .ok();
        let mut options = WindowOptions::default();
        options.titlebar = Some(TitlebarOptions {
            title: Some("OrbitShell".into()),
            appears_transparent: true,
            ..Default::default()
        });
        options.window_decorations = Some(WindowDecorations::Client);

        let launch_options = launch_options.clone();
        cx.open_window(options, move |_, cx| {
            cx.new(|cx| ui::Workspace::new_with_options(cx, launch_options.clone()))
        })
        .expect("failed to open window");
    });
}
