use crate::acp::client::AcpClient;
use crate::acp::manager::{AgentCommandSpec, AgentSpec};
use crate::acp::resolve::{AgentKey, ConflictPolicy, EffectiveAgentRow, load_effective_agent_rows};
use futures::StreamExt;
use futures::channel::mpsc;
use gpui::*;
use std::sync::{Arc, Mutex};
use std::thread;

pub struct AgentView {
    focus_handle: FocusHandle,
    agent_rows: Vec<EffectiveAgentRow>,
    selected_agent_key: Option<AgentKey>,
    client: Option<Arc<Mutex<AcpClient>>>,
    client_agent_key: Option<AgentKey>,
    input: String,
    cursor: usize,
    lines: Vec<String>,
    busy: bool,
    action_busy: bool,
}

enum AgentStreamEvent {
    Update { text: String, append: bool },
    Done(Result<Option<String>, String>),
}

impl AgentView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let mut lines = Vec::new();
        let agent_rows = match load_effective_agent_rows(ConflictPolicy::LocalWins) {
            Ok(value) => value,
            Err(err) => {
                lines.push(format!(
                    "[agent] failed to load effective ACP agents: {err}"
                ));
                Vec::new()
            }
        };
        if agent_rows.is_empty() {
            lines
                .push("[agent] no configured agents. create `agents.json` to add one.".to_string());
        }
        let selected_agent_key = agent_rows.first().map(|row| row.agent_key.clone());

        Self {
            focus_handle: cx.focus_handle(),
            agent_rows,
            selected_agent_key,
            client: None,
            client_agent_key: None,
            input: String::new(),
            cursor: 0,
            lines,
            busy: false,
            action_busy: false,
        }
    }

    fn active_agent_row(&self) -> Option<&EffectiveAgentRow> {
        let selected_key = self.selected_agent_key.as_ref()?;
        self.agent_rows
            .iter()
            .find(|row| row.agent_key == *selected_key)
            .or_else(|| self.agent_rows.first())
    }

    fn active_agent(&self) -> Option<&AgentSpec> {
        self.active_agent_row().map(|row| &row.spec)
    }

    fn active_agent_index(&self) -> Option<usize> {
        let selected_key = self.selected_agent_key.as_ref()?;
        self.agent_rows
            .iter()
            .position(|row| row.agent_key == *selected_key)
            .or(if self.agent_rows.is_empty() {
                None
            } else {
                Some(0)
            })
    }

    fn select_agent_index(&mut self, index: usize) {
        self.selected_agent_key = self.agent_rows.get(index).map(|row| row.agent_key.clone());
        self.client = None;
        self.client_agent_key = None;
    }

    fn ensure_client(&mut self) -> Result<Arc<Mutex<AcpClient>>, String> {
        let active_key = self
            .active_agent_row()
            .map(|row| row.agent_key.clone())
            .ok_or_else(|| "no ACP agent configured".to_string())?;
        let agent = self
            .active_agent()
            .cloned()
            .ok_or_else(|| "no ACP agent configured".to_string())?;
        if !agent.is_available() {
            return Err(format!(
                "agent '{}' command '{}' not found in PATH. Open Settings > ACP Registry and click Install.",
                agent.name, agent.command
            ));
        }

        let should_recreate = self.client.is_none()
            || self
                .client_agent_key
                .as_ref()
                .map(|id| id != &active_key)
                .unwrap_or(true);
        if should_recreate {
            let client = AcpClient::connect(&agent).map_err(|err| {
                format!(
                    "failed to spawn agent command '{}'. Check agents.json (on Windows use the `.cmd` shim, e.g. `codex.cmd`). Details: {err}",
                    agent.command
                )
            })?;
            self.client = Some(Arc::new(Mutex::new(client)));
            self.client_agent_key = Some(active_key);
        }

        self.client
            .as_ref()
            .cloned()
            .ok_or_else(|| "failed to initialize ACP client".to_string())
    }

    fn quote_shell_token(token: &str) -> String {
        if token.is_empty() {
            return "\"\"".to_string();
        }
        let needs_quotes = token
            .chars()
            .any(|c| c.is_whitespace() || c == '"' || c == '\'');
        if !needs_quotes {
            return token.to_string();
        }
        format!("\"{}\"", token.replace('"', "`\""))
    }

    fn command_text(cmd: &AgentCommandSpec) -> String {
        let mut parts = Vec::with_capacity(cmd.args.len() + 1);
        parts.push(Self::quote_shell_token(&cmd.command));
        for arg in &cmd.args {
            parts.push(Self::quote_shell_token(arg));
        }
        parts.join(" ")
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

    fn run_agent_command(
        &mut self,
        cmd: AgentCommandSpec,
        label: &'static str,
        cx: &mut Context<Self>,
    ) {
        if self.action_busy || self.busy {
            self.append_line("[agent] another action is already running.");
            cx.notify();
            return;
        }
        self.action_busy = true;
        self.append_line(format!("[{label}] $ {}", Self::command_text(&cmd)));

        let (tx, mut rx) = mpsc::unbounded::<String>();
        thread::spawn(move || {
            use std::io::{BufRead, BufReader};
            use std::process::{Command, Stdio};

            let candidates = Self::resolve_command_candidates(&cmd.command);
            let mut child_opt = None;
            let mut last_error: Option<std::io::Error> = None;
            for candidate in candidates {
                match Command::new(&candidate)
                    .args(&cmd.args)
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
                    let hint = Self::command_not_found_hint(&cmd.command);
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
                            view.append_line(line);
                            cx.notify();
                        })
                        .is_err()
                    {
                        break;
                    }
                }
                let _ = view.update(&mut cx, |view, cx| {
                    view.action_busy = false;
                    view.agent_rows =
                        load_effective_agent_rows(ConflictPolicy::LocalWins).unwrap_or_default();
                    if view.selected_agent_key.is_none() {
                        view.selected_agent_key =
                            view.agent_rows.first().map(|row| row.agent_key.clone());
                    }
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn on_install(
        &mut self,
        _event: &MouseDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(agent) = self.active_agent().cloned() else {
            return;
        };
        let Some(install) = agent.install else {
            self.append_line("[agent] this agent has no install command.");
            cx.notify();
            return;
        };
        self.run_agent_command(install, "install", cx);
    }

    fn on_authenticate(
        &mut self,
        _event: &MouseDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(agent) = self.active_agent().cloned() else {
            return;
        };
        let Some(auth) = agent.auth else {
            self.append_line("[agent] this agent has no auth command.");
            cx.notify();
            return;
        };
        self.run_agent_command(auth, "auth", cx);
    }

    fn append_line(&mut self, line: impl Into<String>) {
        self.lines.push(line.into());
    }

    fn append_stream_update(&mut self, text: String, append: bool) {
        if append {
            if let Some(last) = self.lines.last_mut() {
                last.push_str(&text);
                return;
            }
        }
        self.lines.push(text);
    }

    fn submit_prompt(&mut self, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let prompt = self.input.trim().to_string();
        if prompt.is_empty() {
            return;
        }

        let Some(agent_name) = self.active_agent().map(|a| a.name.clone()) else {
            self.append_line("[agent] no ACP agent configured.");
            cx.notify();
            return;
        };

        let client = match self.ensure_client() {
            Ok(client) => client,
            Err(err) => {
                self.append_line(format!("[agent] {err}"));
                cx.notify();
                return;
            }
        };

        self.append_line(format!("you> {prompt}"));
        self.append_line(format!("{agent_name}>"));
        self.input.clear();
        self.cursor = 0;
        self.busy = true;

        let (tx, mut rx) = mpsc::unbounded::<AgentStreamEvent>();
        thread::spawn(move || {
            let result = (|| -> Result<Option<String>, String> {
                let mut guard = client
                    .lock()
                    .map_err(|_| "ACP client lock poisoned".to_string())?;
                if guard.protocol_version.is_none() {
                    guard.initialize().map_err(|err| err.to_string())?;
                }
                let cwd = std::env::current_dir()
                    .map_err(|err| err.to_string())?
                    .to_string_lossy()
                    .to_string();
                let runtime_mcp = crate::mcp::probe::load_enabled_runtime_mcp_servers();
                let session_id = guard
                    .ensure_session(&cwd, &runtime_mcp)
                    .map_err(|err| err.to_string())?;
                let mut on_update = |text: String, append: bool| {
                    let _ = tx.unbounded_send(AgentStreamEvent::Update { text, append });
                };
                guard
                    .prompt(&session_id, &prompt, &mut on_update)
                    .map_err(|err| err.to_string())
            })();
            let _ = tx.unbounded_send(AgentStreamEvent::Done(result));
        });

        cx.spawn(|view: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                while let Some(event) = rx.next().await {
                    if view
                        .update(&mut cx, |view, cx| {
                            match event {
                                AgentStreamEvent::Update { text, append } => {
                                    view.append_stream_update(text, append);
                                }
                                AgentStreamEvent::Done(result) => {
                                    view.busy = false;
                                    match result {
                                        Ok(Some(final_text)) => view.append_line(final_text),
                                        Ok(None) => {}
                                        Err(err) => view.append_line(format!("[agent] {err}")),
                                    }
                                }
                            }
                            cx.notify();
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            }
        })
        .detach();
    }

    fn cancel_prompt(&mut self, cx: &mut Context<Self>) {
        if !self.busy {
            return;
        }
        let Some(client) = self.client.as_ref().cloned() else {
            self.busy = false;
            cx.notify();
            return;
        };
        self.append_line("[agent] cancel requested.");
        self.busy = false;
        thread::spawn(move || {
            let Ok(client) = client.lock() else {
                return;
            };
            if let Some(session_id) = client.session_id.as_ref() {
                let _ = client.cancel(session_id);
            }
        });
        cx.notify();
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let ctrl = event.keystroke.modifiers.control;
        match event.keystroke.key.as_str() {
            "enter" | "return" | "numpadenter" => {
                self.submit_prompt(cx);
                cx.stop_propagation();
            }
            "backspace" => {
                if self.cursor > 0 {
                    let start = self.cursor - 1;
                    self.input.replace_range(start..self.cursor, "");
                    self.cursor = start;
                    cx.notify();
                }
                cx.stop_propagation();
            }
            "left" | "arrowleft" => {
                self.cursor = self.cursor.saturating_sub(1);
                cx.notify();
                cx.stop_propagation();
            }
            "right" | "arrowright" => {
                let max = self.input.chars().count();
                if self.cursor < max {
                    self.cursor += 1;
                }
                cx.notify();
                cx.stop_propagation();
            }
            "home" => {
                self.cursor = 0;
                cx.notify();
                cx.stop_propagation();
            }
            "end" => {
                self.cursor = self.input.chars().count();
                cx.notify();
                cx.stop_propagation();
            }
            "c" if ctrl => {
                self.cancel_prompt(cx);
                cx.stop_propagation();
            }
            _ => {
                if ctrl {
                    return;
                }
                if let Some(text) = event.keystroke.key_char.as_deref() {
                    if !text.is_empty() {
                        let byte_index = char_index_to_byte_index(&self.input, self.cursor);
                        self.input.insert_str(byte_index, text);
                        self.cursor += text.chars().count();
                        cx.notify();
                        cx.stop_propagation();
                    }
                } else if event.keystroke.key.len() == 1 {
                    let key = event.keystroke.key.clone();
                    let byte_index = char_index_to_byte_index(&self.input, self.cursor);
                    self.input.insert_str(byte_index, &key);
                    self.cursor += key.chars().count();
                    cx.notify();
                    cx.stop_propagation();
                }
            }
        }
    }

    fn on_prev_agent(
        &mut self,
        _event: &MouseDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.agent_rows.is_empty() {
            return;
        }
        let next_index = match self.active_agent_index() {
            Some(0) | None => self.agent_rows.len().saturating_sub(1),
            Some(index) => index.saturating_sub(1),
        };
        self.select_agent_index(next_index);
        cx.notify();
    }

    fn on_next_agent(
        &mut self,
        _event: &MouseDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.agent_rows.is_empty() {
            return;
        }
        let next_index = match self.active_agent_index() {
            Some(index) => (index + 1) % self.agent_rows.len(),
            None => 0,
        };
        self.select_agent_index(next_index);
        cx.notify();
    }
}

impl Render for AgentView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        window.focus(&self.focus_handle);
        let (left, right) = split_at_cursor(&self.input, self.cursor);
        let active = self.active_agent().cloned();
        let agent_label = active
            .as_ref()
            .map(|agent| format!("Agent: {}", agent.name))
            .unwrap_or_else(|| "Agent: none".to_string());
        let available = active
            .as_ref()
            .map(|agent| agent.is_available())
            .unwrap_or(false);
        let has_install = active
            .as_ref()
            .and_then(|agent| agent.install.as_ref())
            .is_some();
        let has_auth = active
            .as_ref()
            .and_then(|agent| agent.auth.as_ref())
            .is_some();

        div()
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(Self::on_key_down))
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(0x0a0a0a))
            .child(
                div()
                    .h(px(42.0))
                    .flex_none()
                    .px(px(12.0))
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .border_b_1()
                    .border_color(rgb(0x232323))
                    .child(
                        div()
                            .px(px(8.0))
                            .py(px(4.0))
                            .rounded(px(6.0))
                            .bg(rgb(0x141414))
                            .border_1()
                            .border_color(rgb(0x2a2a2a))
                            .text_color(rgb(0x9f9f9f))
                            .text_size(px(12.0))
                            .child("ACP"),
                    )
                    .child(
                        div()
                            .w(px(22.0))
                            .h(px(22.0))
                            .rounded(px(5.0))
                            .bg(rgb(0x161616))
                            .border_1()
                            .border_color(rgb(0x2a2a2a))
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_color(rgb(0xcccccc))
                            .text_size(px(12.0))
                            .child("<")
                            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_prev_agent)),
                    )
                    .child(
                        div()
                            .text_size(px(12.0))
                            .text_color(rgb(0xd0d0d0))
                            .child(agent_label),
                    )
                    .child(
                        div()
                            .px(px(8.0))
                            .py(px(3.0))
                            .rounded(px(999.0))
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
                            .text_size(px(11.0))
                            .text_color(if available {
                                rgb(0x8bd06f)
                            } else {
                                rgb(0xffb366)
                            })
                            .child(if available {
                                "Installed"
                            } else {
                                "Not installed"
                            }),
                    )
                    .child(if !available && has_install {
                        div()
                            .px(px(8.0))
                            .py(px(4.0))
                            .rounded(px(6.0))
                            .bg(rgb(0x101010))
                            .border_1()
                            .border_color(rgb(0x2a2a2a))
                            .text_size(px(11.0))
                            .text_color(rgb(0xd0d0d0))
                            .child("Install")
                            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_install))
                    } else {
                        div()
                    })
                    .child(if has_auth {
                        div()
                            .px(px(8.0))
                            .py(px(4.0))
                            .rounded(px(6.0))
                            .bg(rgb(0x101010))
                            .border_1()
                            .border_color(rgb(0x2a2a2a))
                            .text_size(px(11.0))
                            .text_color(rgb(0xd0d0d0))
                            .child("Authenticate")
                            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_authenticate))
                    } else {
                        div()
                    })
                    .child(
                        div()
                            .w(px(22.0))
                            .h(px(22.0))
                            .rounded(px(5.0))
                            .bg(rgb(0x161616))
                            .border_1()
                            .border_color(rgb(0x2a2a2a))
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_color(rgb(0xcccccc))
                            .text_size(px(12.0))
                            .child(">")
                            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_next_agent)),
                    )
                    .child(if self.busy {
                        div()
                            .text_size(px(12.0))
                            .text_color(rgb(0x8dafff))
                            .child("running...")
                    } else {
                        div()
                    }),
            )
            .child(
                div().flex_1().min_h(px(0.0)).p(px(12.0)).child(
                    div()
                        .flex_col()
                        .gap(px(6.0))
                        .children(self.lines.iter().map(|line| {
                            div()
                                .text_size(px(12.5))
                                .font_family("Cascadia Code")
                                .text_color(rgb(0xc9c9c9))
                                .child(line.clone())
                        })),
                ),
            )
            .child(
                div().flex_none().px(px(12.0)).pb(px(12.0)).child(
                    div()
                        .h(px(38.0))
                        .rounded(px(8.0))
                        .bg(rgb(0x111111))
                        .border_1()
                        .border_color(rgb(0x2a2a2a))
                        .px(px(10.0))
                        .flex()
                        .items_center()
                        .font_family("Cascadia Code")
                        .text_size(px(13.0))
                        .child(div().text_color(rgb(0x5d8cff)).mr(px(8.0)).child(">"))
                        .child(if self.input.is_empty() {
                            div()
                                .text_color(rgb(0x5f5f5f))
                                .child("Digite um prompt e Enter para enviar")
                        } else {
                            div()
                                .flex()
                                .items_center()
                                .child(div().text_color(rgb(0xe9e9e9)).child(left))
                                .child(div().w(px(2.0)).h(px(16.0)).bg(rgb(0x6b9eff)))
                                .child(div().text_color(rgb(0xe9e9e9)).child(right))
                        }),
                ),
            )
    }
}

impl Focusable for AgentView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

fn char_index_to_byte_index(s: &str, index: usize) -> usize {
    s.char_indices()
        .map(|(byte_idx, _)| byte_idx)
        .nth(index)
        .unwrap_or_else(|| s.len())
}

fn split_at_cursor(input: &str, cursor: usize) -> (String, String) {
    let mut left = String::new();
    let mut right = String::new();
    for (i, ch) in input.chars().enumerate() {
        if i < cursor {
            left.push(ch);
        } else {
            right.push(ch);
        }
    }
    (left, right)
}
