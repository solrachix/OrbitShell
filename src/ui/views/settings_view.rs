use futures::StreamExt;
use futures::channel::mpsc;
use gpui::*;
use lucide_icons::Icon;
use std::path::PathBuf;
use std::thread;

use crate::acp::client::AcpClient;
use crate::acp::manager::{AgentRegistry, AgentSpec};
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
    agent_registry: AgentRegistry,
    agent_registry_path: PathBuf,
    agent_registry_error: Option<String>,
    agent_action_lines: Vec<String>,
    agent_action_busy: bool,
}

impl SettingsView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let agent_registry_path = AgentRegistry::default_path();
        let (agent_registry, agent_registry_error) = match AgentRegistry::load_default() {
            Ok(registry) => (registry, None),
            Err(err) => (AgentRegistry::default(), Some(err.to_string())),
        };

        Self {
            sections: vec![
                "Account",
                "Code",
                "Appearance",
                "Keyboard shortcuts",
                "Referrals",
                "ACP Registry",
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
            agent_registry,
            agent_registry_path,
            agent_registry_error,
            agent_action_lines: Vec::new(),
            agent_action_busy: false,
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

    fn on_reload_agent_registry(
        &mut self,
        _event: &MouseDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match AgentRegistry::load_default() {
            Ok(registry) => {
                self.agent_registry = registry;
                self.agent_registry_error = None;
            }
            Err(err) => {
                self.agent_registry = AgentRegistry::default();
                self.agent_registry_error = Some(err.to_string());
            }
        }
        cx.notify();
    }

    fn on_create_agent_registry(
        &mut self,
        _event: &MouseDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let to_write = if self.agent_registry.agents.is_empty() {
            AgentRegistry::load_default().unwrap_or_default()
        } else {
            self.agent_registry.clone()
        };
        match to_write.save_default() {
            Ok(_) => {
                self.agent_registry = to_write;
                self.agent_registry_error = None;
            }
            Err(err) => {
                self.agent_registry_error = Some(err.to_string());
            }
        }
        cx.notify();
    }

    fn run_agent_command(
        &mut self,
        command: String,
        args: Vec<String>,
        label: &'static str,
        cx: &mut Context<Self>,
    ) {
        if self.agent_action_busy {
            self.agent_action_lines
                .push("[info] another agent action is already running".to_string());
            cx.notify();
            return;
        }

        self.agent_action_busy = true;
        let rendered = if args.is_empty() {
            command.clone()
        } else {
            format!("{command} {}", args.join(" "))
        };
        self.agent_action_lines.push(format!("[{label}] $ {rendered}"));

        let (tx, mut rx) = mpsc::unbounded::<String>();
        thread::spawn(move || {
            use std::io::{BufRead, BufReader};
            use std::process::{Command, Stdio};

            let candidates = Self::resolve_command_candidates(&command);
            let mut child_opt = None;
            let mut last_error: Option<std::io::Error> = None;
            for candidate in candidates {
                match Command::new(&candidate)
                    .args(&args)
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .spawn()
                {
                    Ok(child) => {
                        let _ =
                            tx.unbounded_send(format!("[{label}] resolved command: {candidate}"));
                        child_opt = Some(child);
                        break;
                    }
                    Err(err) => {
                        last_error = Some(err);
                    }
                }
            }

            let mut child = match child_opt {
                Some(child) => child,
                None => {
                    let detail = last_error
                        .map(|err| err.to_string())
                        .unwrap_or_else(|| "spawn failed".to_string());
                    let hint = Self::command_not_found_hint(&command);
                    let _ = tx.unbounded_send(format!("[error] failed to spawn: {detail}"));
                    let _ = tx.unbounded_send(format!("[hint] {hint}"));
                    return;
                }
            };

            if let Some(stdout) = child.stdout.take() {
                let tx_out = tx.clone();
                thread::spawn(move || {
                    let reader = BufReader::new(stdout);
                    for line in reader.lines().map_while(Result::ok) {
                        let _ = tx_out.unbounded_send(line);
                    }
                });
            }
            if let Some(stderr) = child.stderr.take() {
                let tx_err = tx.clone();
                thread::spawn(move || {
                    let reader = BufReader::new(stderr);
                    for line in reader.lines().map_while(Result::ok) {
                        let _ = tx_err.unbounded_send(format!("[stderr] {line}"));
                    }
                });
            }

            let status = child.wait();
            let _ = tx.unbounded_send(format!("[done] {status:?}"));
        });

        cx.spawn(|view: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                while let Some(line) = rx.next().await {
                    if view
                        .update(&mut cx, |view, cx| {
                            view.agent_action_lines.push(line);
                            cx.notify();
                        })
                        .is_err()
                    {
                        break;
                    }
                }

                let _ = view.update(&mut cx, |view, cx| {
                    view.agent_action_busy = false;
                    match AgentRegistry::load_default() {
                        Ok(registry) => {
                            view.agent_registry = registry;
                            view.agent_registry_error = None;
                        }
                        Err(err) => {
                            view.agent_registry_error = Some(err.to_string());
                        }
                    }
                    cx.notify();
                });
            }
        })
        .detach();

        cx.notify();
    }

    fn on_install_agent(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(agent) = self.agent_registry.agents.get(index).cloned() else {
            return;
        };
        let Some(install) = agent.install else {
            self.agent_action_lines
                .push(format!("[install] agent '{}' has no install command", agent.name));
            cx.notify();
            return;
        };
        self.run_agent_command(install.command, install.args, "install", cx);
    }

    fn on_auth_agent(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(agent) = self.agent_registry.agents.get(index).cloned() else {
            return;
        };
        let Some(auth) = agent.auth else {
            self.agent_action_lines
                .push(format!("[auth] agent '{}' has no auth command", agent.name));
            cx.notify();
            return;
        };
        self.run_agent_command(auth.command, auth.args, "auth", cx);
    }

    fn on_test_agent(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(agent) = self.agent_registry.agents.get(index).cloned() else {
            return;
        };

        if self.agent_action_busy {
            self.agent_action_lines
                .push("[info] another agent action is already running".to_string());
            cx.notify();
            return;
        }

        self.agent_action_busy = true;
        self.agent_action_lines
            .push(format!("[test] ACP handshake for '{}' ({})", agent.name, agent.display_command()));

        let (tx, mut rx) = mpsc::unbounded::<String>();
        thread::spawn(move || {
            if !agent.is_available() {
                let _ = tx.unbounded_send(format!(
                    "[test] FAIL: '{}' not found in PATH",
                    agent.command
                ));
                let _ = tx.unbounded_send("[done] test finished with errors".to_string());
                return;
            }

            let mut client = match AcpClient::connect(&agent) {
                Ok(client) => client,
                Err(err) => {
                    let _ = tx.unbounded_send(format!("[test] FAIL: connect error: {err}"));
                    let _ = tx.unbounded_send("[done] test finished with errors".to_string());
                    return;
                }
            };

            if let Err(err) = client.initialize() {
                let _ = tx.unbounded_send(format!("[test] FAIL: initialize error: {err}"));
                let _ = tx.unbounded_send("[done] test finished with errors".to_string());
                return;
            }

            let protocol = client
                .protocol_version
                .clone()
                .unwrap_or_else(|| "unknown".to_string());
            let _ = tx.unbounded_send(format!("[test] initialize OK (protocol={protocol})"));

            let cwd = match std::env::current_dir() {
                Ok(path) => path.to_string_lossy().to_string(),
                Err(err) => {
                    let _ = tx.unbounded_send(format!("[test] FAIL: unable to resolve cwd: {err}"));
                    let _ = tx.unbounded_send("[done] test finished with errors".to_string());
                    return;
                }
            };

            match client.ensure_session(&cwd) {
                Ok(session_id) => {
                    let _ = tx.unbounded_send(format!("[test] session/new OK (sessionId={session_id})"));
                    let _ = tx.unbounded_send("[done] test finished successfully".to_string());
                }
                Err(err) => {
                    let _ = tx.unbounded_send(format!("[test] FAIL: session/new error: {err}"));
                    let _ = tx.unbounded_send("[done] test finished with errors".to_string());
                }
            }
        });

        cx.spawn(|view: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                while let Some(line) = rx.next().await {
                    if view
                        .update(&mut cx, |view, cx| {
                            view.agent_action_lines.push(line);
                            cx.notify();
                        })
                        .is_err()
                    {
                        break;
                    }
                }

                let _ = view.update(&mut cx, |view, cx| {
                    view.agent_action_busy = false;
                    cx.notify();
                });
            }
        })
        .detach();

        cx.notify();
    }

    fn render_agent_badge(&self, agent: &AgentSpec) -> Div {
        let available = agent.is_available();
        div()
            .px(px(8.0))
            .py(px(3.0))
            .rounded(px(999.0))
            .text_size(px(11.0))
            .border_1()
            .border_color(if available {
                rgb(0x244a2b)
            } else {
                rgb(0x4a2b24)
            })
            .bg(if available {
                rgb(0x122016)
            } else {
                rgb(0x201612)
            })
            .text_color(if available {
                rgb(0x8bd06f)
            } else {
                rgb(0xffb366)
            })
            .child(if available { "Installed" } else { "Not installed" })
    }

    fn render_action_button(&self, label: &'static str) -> Div {
        div()
            .px(px(10.0))
            .py(px(6.0))
            .rounded(px(6.0))
            .bg(rgb(0x101010))
            .border_1()
            .border_color(rgb(0x2a2a2a))
            .text_size(px(12.0))
            .text_color(rgb(0xd0d0d0))
            .child(label)
    }

    fn resolve_command_candidates(command: &str) -> Vec<String> {
        if !cfg!(windows) {
            return vec![command.to_string()];
        }
        let path = std::path::Path::new(command);
        if path.extension().is_some() {
            return vec![command.to_string()];
        }
        vec![
            command.to_string(),
            format!("{command}.cmd"),
            format!("{command}.exe"),
        ]
    }

    fn command_not_found_hint(command: &str) -> String {
        let lower = command.to_ascii_lowercase();
        if lower == "npm" || lower == "npx" {
            return "Node.js/npm not found in PATH. Install Node.js or restart the app so PATH is reloaded.".to_string();
        }
        format!("program '{command}' not found in PATH")
    }

    fn render_section_content(&self, cx: &Context<Self>) -> Div {
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
            "ACP Registry" => {
                let exists = self.agent_registry_path.exists();
                let installed_count = self
                    .agent_registry
                    .agents
                    .iter()
                    .filter(|agent| agent.is_available())
                    .count();
                let recent_logs: Vec<String> = self
                    .agent_action_lines
                    .iter()
                    .rev()
                    .take(14)
                    .cloned()
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect();

                content = content
                    .child(
                        div()
                            .text_size(px(12.0))
                            .text_color(rgb(0x8a8a8a))
                            .child("Configure ACP agents loaded from local agents.json"),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.0))
                            .child(
                                div()
                                    .text_size(px(12.0))
                                    .text_color(rgb(0x9a9a9a))
                                    .child("File:"),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(0.0))
                                    .text_size(px(12.0))
                                    .text_color(rgb(0xd0d0d0))
                                    .font_family("Cascadia Code")
                                    .truncate()
                                    .child(self.agent_registry_path.to_string_lossy().to_string()),
                            ),
                    )
                    .child(
                        div()
                            .text_size(px(12.0))
                            .text_color(if exists { rgb(0x8bd06f) } else { rgb(0xffb366) })
                            .child(if exists {
                                "Status: file found"
                            } else {
                                "Status: file missing (using in-memory defaults)"
                            }),
                    )
                    .child(if let Some(err) = &self.agent_registry_error {
                        div()
                            .text_size(px(12.0))
                            .text_color(rgb(0xff7b72))
                            .child(format!("Last error: {err}"))
                    } else {
                        div()
                    })
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.0))
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
                                    .child("Reload")
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(Self::on_reload_agent_registry),
                                    ),
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
                                    .child("Create agents.json")
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(Self::on_create_agent_registry),
                                    ),
                            ),
                    )
                    .child(div().h(px(1.0)).bg(rgb(0x1f1f1f)))
                    .child(
                        div()
                            .text_size(px(12.0))
                            .text_color(rgb(0x9a9a9a))
                            .child(format!(
                                "Agents: {} total, {} installed",
                                self.agent_registry.agents.len(),
                                installed_count
                            )),
                    )
                    .child(div().flex().flex_col().gap(px(8.0)).children(
                        self.agent_registry.agents.iter().enumerate().map(|(index, agent)| {
                            let available = agent.is_available();
                            let can_install = agent.install.is_some() && !available;
                            let can_auth = agent.auth.is_some();
                            let install_handle = cx.entity().downgrade();
                            let auth_handle = cx.entity().downgrade();
                            let test_handle = cx.entity().downgrade();

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
                                        .flex()
                                        .flex_col()
                                        .gap(px(2.0))
                                        .child(
                                            div()
                                                .flex()
                                                .items_center()
                                                .gap(px(8.0))
                                        .child(
                                            div()
                                                .flex_1()
                                                .min_w(px(0.0))
                                                .text_size(px(12.0))
                                                .text_color(rgb(0xd0d0d0))
                                                .truncate()
                                                .child(format!(
                                                    "{} ({})",
                                                    agent.name, agent.id
                                                )),
                                        )
                                                .child(self.render_agent_badge(agent)),
                                        )
                                        .child(
                                            div()
                                                .flex_1()
                                                .min_w(px(0.0))
                                                .text_size(px(11.0))
                                                .text_color(rgb(0x8a8a8a))
                                                .font_family("Cascadia Code")
                                                .truncate()
                                                .child(agent.display_command()),
                                        ),
                                )
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap(px(8.0))
                                        .child(if can_install {
                                            self.render_action_button("Install").on_mouse_down(
                                                MouseButton::Left,
                                                move |_e, _w, cx| {
                                                    cx.stop_propagation();
                                                    let _ = install_handle.update(cx, |view, cx| {
                                                        view.on_install_agent(index, cx);
                                                    });
                                                },
                                            )
                                        } else {
                                            div()
                                        })
                                        .child(if can_auth {
                                            self.render_action_button("Authenticate").on_mouse_down(
                                                MouseButton::Left,
                                                move |_e, _w, cx| {
                                                    cx.stop_propagation();
                                                    let _ = auth_handle.update(cx, |view, cx| {
                                                        view.on_auth_agent(index, cx);
                                                    });
                                                },
                                            )
                                        } else {
                                            div()
                                        })
                                        .child(
                                            self.render_action_button("Test").on_mouse_down(
                                                MouseButton::Left,
                                                move |_e, _w, cx| {
                                                    cx.stop_propagation();
                                                    let _ = test_handle.update(cx, |view, cx| {
                                                        view.on_test_agent(index, cx);
                                                    });
                                                },
                                            ),
                                        ),
                                )
                        }),
                    ))
                    .child(if recent_logs.is_empty() {
                        div()
                    } else {
                        div()
                            .mt(px(8.0))
                            .min_w(px(0.0))
                            .overflow_hidden()
                            .p(px(10.0))
                            .rounded(px(8.0))
                            .bg(rgb(0x0f0f0f))
                            .border_1()
                            .border_color(rgb(0x1f1f1f))
                            .child(
                                div()
                                    .text_size(px(11.0))
                                    .text_color(rgb(0x8a8a8a))
                                    .child(if self.agent_action_busy {
                                        "Action log (running...)"
                                    } else {
                                        "Action log"
                                    }),
                            )
                            .child(
                                div()
                                    .mt(px(8.0))
                                    .flex()
                                    .flex_col()
                                    .gap(px(4.0))
                                    .min_w(px(0.0))
                                    .overflow_hidden()
                                    .children(recent_logs.into_iter().map(|line| {
                                        div()
                                            .min_w(px(0.0))
                                            .text_size(px(11.0))
                                            .text_color(rgb(0xbdbdbd))
                                            .font_family("Cascadia Code")
                                            .truncate()
                                            .child(line)
                                    })),
                            )
                    });
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
                            .min_w(px(0.0))
                            .min_h(px(0.0))
                            .p(px(28.0))
                            .gap(px(18.0))
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .flex_1()
                                    .min_w(px(0.0))
                                    .min_h(px(0.0))
                                    .gap(px(16.0))
                                    .child(self.render_section_content(cx)),
                            ),
                    ),
            )
    }
}
