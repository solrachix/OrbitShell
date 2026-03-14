use anyhow::{Result, anyhow};
use futures::StreamExt;
use futures::channel::mpsc;
use gpui::*;
use lucide_icons::Icon;
use std::fs;
use std::path::PathBuf;
use std::thread;

use crate::acp::client::AcpClient;
use crate::acp::install::binary::{
    BinaryInstallSpec, download_binary_artifact, install_binary_from_file,
};
use crate::acp::install::runner::{
    LaunchCommand, build_npx_package_launch, build_uvx_package_launch, write_launch_wrapper,
};
use crate::acp::install::state::{
    ManagedAgentState, ManagedAgentsStateFile, ManagedInstalledVersion,
};
use crate::acp::manager::AgentSpec;
use crate::acp::registry::cache;
use crate::acp::registry::fetch::{
    CachedRegistryData, UreqRegistryFetchClient, load_cached_registry, load_then_refresh,
};
use crate::acp::registry::model::{
    RegistryInstallStrategy, RegistryManifest, infer_archive_kind_from_url,
};
use crate::acp::resolve::{
    CatalogAgentRow, CatalogFilter, CatalogInstallStatus, ConflictPolicy, build_catalog_rows,
    filter_catalog_rows, load_effective_agent_rows,
};
use crate::acp::storage;
use crate::mcp::config::{GlobalMcpConfig, McpServerConfig};
use crate::mcp::probe::{McpProbeResult, probe_server_config};
use crate::ui::icons::lucide_icon;
use crate::ui::text_edit::TextEditState;

const ACCENT: u32 = 0x6b9eff;
const ACCENT_BORDER: u32 = 0x6b9eff66;
const ACP_REGISTRY_URL: &str =
    "https://cdn.agentclientprotocol.com/registry/v1/latest/registry.json";

pub struct SettingsView {
    sections: Vec<&'static str>,
    active_section: usize,
    focus_handle: FocusHandle,
    search_query: String,
    search_cursor: usize,
    search_selection: Option<(usize, usize)>,
    search_anchor: Option<usize>,
    registry_data: CachedRegistryData,
    registry_refresh_error: Option<String>,
    registry_used_cache: bool,
    registry_refresh_busy: bool,
    effective_agents: Vec<crate::acp::resolve::EffectiveAgentRow>,
    catalog_filter: CatalogFilter,
    agent_action_lines: Vec<String>,
    agent_action_busy: bool,
    mcp_config: GlobalMcpConfig,
    mcp_error: Option<String>,
    mcp_action_lines: Vec<String>,
    mcp_action_busy: bool,
}

impl SettingsView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let registry_data = Self::load_registry_data_from_disk().unwrap_or_default();
        let effective_agents =
            load_effective_agent_rows(ConflictPolicy::LocalWins).unwrap_or_default();
        let (mcp_config, mcp_error) = match GlobalMcpConfig::load_default() {
            Ok(config) => (config, None),
            Err(err) => (GlobalMcpConfig::default(), Some(err.to_string())),
        };

        let mut view = Self {
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
            registry_data,
            registry_refresh_error: None,
            registry_used_cache: true,
            registry_refresh_busy: false,
            effective_agents,
            catalog_filter: CatalogFilter::All,
            agent_action_lines: Vec::new(),
            agent_action_busy: false,
            mcp_config,
            mcp_error,
            mcp_action_lines: Vec::new(),
            mcp_action_busy: false,
        };
        view.refresh_registry_in_background(cx);
        view
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

    fn load_registry_data_from_disk() -> Result<CachedRegistryData> {
        let app_root = storage::app_root()?;
        Ok(load_cached_registry(&app_root)?.unwrap_or_default())
    }

    fn reload_catalog_state(&mut self) {
        self.registry_data = Self::load_registry_data_from_disk().unwrap_or_default();
        self.effective_agents =
            load_effective_agent_rows(ConflictPolicy::LocalWins).unwrap_or_default();
    }

    fn visible_catalog_rows(&self) -> Vec<CatalogAgentRow> {
        let rows = build_catalog_rows(Some(&self.registry_data), &self.effective_agents);
        filter_catalog_rows(&rows, self.catalog_filter, &self.search_query)
    }

    fn refresh_registry_in_background(&mut self, cx: &mut Context<Self>) {
        if self.registry_refresh_busy {
            return;
        }

        self.registry_refresh_busy = true;
        let (tx, mut rx) =
            mpsc::unbounded::<Result<(CachedRegistryData, bool, Option<String>), String>>();
        thread::spawn(move || {
            let result = (|| -> Result<(CachedRegistryData, bool, Option<String>)> {
                let app_root = storage::app_root()?;
                let mut managed_state = ManagedAgentsStateFile::load_default()?;
                let refresh = load_then_refresh(
                    &app_root,
                    &UreqRegistryFetchClient {
                        index_url: ACP_REGISTRY_URL.into(),
                    },
                    Some(&mut managed_state),
                )?;
                managed_state.save_default()?;
                Ok((refresh.data, refresh.used_cache, refresh.refresh_error))
            })()
            .map_err(|err| err.to_string());
            let _ = tx.unbounded_send(result);
        });

        cx.spawn(|view: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                while let Some(result) = rx.next().await {
                    let _ = view.update(&mut cx, |view, cx| {
                        view.registry_refresh_busy = false;
                        match result {
                            Ok((data, used_cache, refresh_error)) => {
                                view.registry_data = data;
                                view.registry_used_cache = used_cache;
                                view.registry_refresh_error = refresh_error;
                            }
                            Err(err) => {
                                view.registry_refresh_error = Some(err);
                                view.registry_used_cache = true;
                                view.registry_data =
                                    Self::load_registry_data_from_disk().unwrap_or_default();
                            }
                        }
                        view.effective_agents =
                            load_effective_agent_rows(ConflictPolicy::LocalWins)
                                .unwrap_or_default();
                        cx.notify();
                    });
                }
            }
        })
        .detach();

        cx.notify();
    }

    fn on_refresh_registry(
        &mut self,
        _event: &MouseDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.refresh_registry_in_background(cx);
    }

    fn start_logged_action<F>(&mut self, label: &'static str, cx: &mut Context<Self>, job: F)
    where
        F: FnOnce(mpsc::UnboundedSender<String>) -> Result<()> + Send + 'static,
    {
        if self.agent_action_busy {
            self.agent_action_lines
                .push("[info] another agent action is already running".to_string());
            cx.notify();
            return;
        }

        self.agent_action_busy = true;
        let (tx, mut rx) = mpsc::unbounded::<String>();
        thread::spawn(move || {
            let result = job(tx.clone());
            if let Err(err) = result {
                let _ = tx.unbounded_send(format!("[error] {err}"));
            }
            let _ = tx.unbounded_send(format!("[done] {label}"));
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
                        return;
                    }
                }

                let _ = view.update(&mut cx, |view, cx| {
                    view.agent_action_busy = false;
                    view.reload_catalog_state();
                    cx.notify();
                });
            }
        })
        .detach();

        cx.notify();
    }

    fn run_spawned_command(
        &mut self,
        launch: LaunchCommand,
        label: &'static str,
        cx: &mut Context<Self>,
    ) {
        let rendered = if launch.args.is_empty() {
            launch.command.clone()
        } else {
            format!("{} {}", launch.command, launch.args.join(" "))
        };
        self.agent_action_lines
            .push(format!("[{label}] $ {rendered}"));

        self.start_logged_action(label, cx, move |tx| {
            use std::io::{BufRead, BufReader};
            use std::process::{Command, Stdio};

            let mut child_opt = None;
            let mut last_error: Option<std::io::Error> = None;
            for candidate in Self::resolve_command_candidates(&launch.command) {
                match Command::new(&candidate)
                    .args(&launch.args)
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
                    Err(err) => last_error = Some(err),
                }
            }

            let mut child = match child_opt {
                Some(child) => child,
                None => {
                    let detail = last_error
                        .map(|err| err.to_string())
                        .unwrap_or_else(|| "spawn failed".to_string());
                    let _ = tx.unbounded_send(format!("[error] failed to spawn: {detail}"));
                    let _ = tx.unbounded_send(format!(
                        "[hint] {}",
                        Self::command_not_found_hint(&launch.command)
                    ));
                    return Ok(());
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

            let status = child.wait()?;
            let _ = tx.unbounded_send(format!("[status] {status}"));
            Ok(())
        });
    }

    fn on_install_agent(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(row) = self.visible_catalog_rows().get(index).cloned() else {
            return;
        };

        if let Some(selected) = row.selected_source.clone() {
            if let Some(install) = selected.spec.install.clone() {
                self.run_spawned_command(
                    LaunchCommand {
                        command: install.command,
                        args: install.args,
                    },
                    "install",
                    cx,
                );
                return;
            }
        }

        let Some(manifest) = row.registry_manifest.clone() else {
            self.agent_action_lines
                .push(format!("[install] '{}' has no install action", row.name));
            cx.notify();
            return;
        };

        self.agent_action_lines
            .push(format!("[install] {}", manifest.name));
        self.start_logged_action("install", cx, move |tx| {
            Self::install_registry_manifest_job(manifest, tx)
        });
    }

    fn on_update_agent(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(row) = self.visible_catalog_rows().get(index).cloned() else {
            return;
        };
        let Some(manifest) = row.registry_manifest.clone() else {
            self.agent_action_lines
                .push(format!("[update] '{}' has no registry manifest", row.name));
            cx.notify();
            return;
        };

        self.agent_action_lines
            .push(format!("[update] {}", manifest.name));
        self.start_logged_action("update", cx, move |tx| {
            Self::install_registry_manifest_job(manifest, tx)
        });
    }

    fn on_remove_agent(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(row) = self.visible_catalog_rows().get(index).cloned() else {
            return;
        };

        self.agent_action_lines
            .push(format!("[remove] {}", row.name));
        self.start_logged_action("remove", cx, move |tx| {
            Self::remove_registry_agent_job(row.acp_id, tx)
        });
    }

    fn on_auth_agent(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(row) = self.visible_catalog_rows().get(index).cloned() else {
            return;
        };
        let Some(agent) = row.selected_source.map(|source| source.spec) else {
            return;
        };
        let Some(auth) = agent.auth else {
            self.agent_action_lines
                .push(format!("[auth] agent '{}' has no auth command", agent.name));
            cx.notify();
            return;
        };

        self.run_spawned_command(
            LaunchCommand {
                command: auth.command,
                args: auth.args,
            },
            "auth",
            cx,
        );
    }

    fn on_test_agent(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(row) = self.visible_catalog_rows().get(index).cloned() else {
            return;
        };
        let Some(agent) = row.selected_source.map(|source| source.spec) else {
            self.agent_action_lines.push(format!(
                "[test] '{}' is not installed yet, so there is nothing to test",
                row.name
            ));
            cx.notify();
            return;
        };

        self.agent_action_lines.push(format!(
            "[test] ACP handshake for '{}' ({})",
            agent.name,
            agent.display_command()
        ));
        self.start_logged_action("test", cx, move |tx| Self::test_agent_job(agent, tx));
    }

    fn test_agent_job(agent: AgentSpec, tx: mpsc::UnboundedSender<String>) -> Result<()> {
        if !agent.is_available() {
            let _ = tx.unbounded_send(format!(
                "[test] FAIL: '{}' not found in PATH",
                agent.command
            ));
            return Ok(());
        }

        let mut client = AcpClient::connect(&agent)?;
        client.initialize()?;
        let protocol = client
            .protocol_version
            .clone()
            .unwrap_or_else(|| "unknown".to_string());
        let _ = tx.unbounded_send(format!("[test] initialize OK (protocol={protocol})"));

        let cwd = std::env::current_dir()?.to_string_lossy().to_string();
        let runtime_mcp = crate::mcp::probe::load_enabled_runtime_mcp_servers();
        let session_id = client.ensure_session(&cwd, &runtime_mcp)?;
        let _ = tx.unbounded_send(format!("[test] session/new OK (sessionId={session_id})"));
        Ok(())
    }

    fn install_registry_manifest_job(
        manifest: RegistryManifest,
        tx: mpsc::UnboundedSender<String>,
    ) -> Result<()> {
        let app_root = storage::app_root()?;
        let installs_root = storage::registry_installs_root(&app_root).join(&manifest.id);
        cache::save_registry_manifest(&app_root, &manifest)?;

        let mut managed_state = ManagedAgentsStateFile::load_default()?;
        let agent_index = match managed_state
            .agents
            .iter()
            .position(|agent| agent.id == manifest.id)
        {
            Some(index) => index,
            None => {
                managed_state.agents.push(ManagedAgentState {
                    id: manifest.id.clone(),
                    ..Default::default()
                });
                managed_state.agents.len().saturating_sub(1)
            }
        };

        let strategy = manifest
            .preferred_install_strategy()
            .ok_or_else(|| anyhow!("no supported install strategy for '{}'", manifest.id))?;
        {
            let state = &mut managed_state.agents[agent_index];
            match strategy {
                RegistryInstallStrategy::Npx(dist) => {
                    let launch = build_npx_package_launch(&dist.package, &dist.args);
                    let version_root = installs_root.join(&manifest.version);
                    let wrapper = write_launch_wrapper(&version_root, &manifest.id, &launch)?;
                    let resolved_command = wrapper.to_string_lossy().to_string();
                    state.record_installed_version(ManagedInstalledVersion {
                        version: manifest.version.clone(),
                        install_root: version_root.clone(),
                        resolved_command: resolved_command.clone(),
                        resolved_args: Vec::new(),
                    });
                    state.set_active_version(&manifest.version);
                    state.distribution_kind = Some("npx".into());
                    state.install_root = Some(version_root);
                    state.resolved_command = Some(resolved_command);
                    state.resolved_args.clear();
                    let _ = tx
                        .unbounded_send(format!("[install] prepared npx wrapper {}", dist.package));
                }
                RegistryInstallStrategy::Uvx(dist) => {
                    let launch = build_uvx_package_launch(&dist.package, &dist.args);
                    let version_root = installs_root.join(&manifest.version);
                    let wrapper = write_launch_wrapper(&version_root, &manifest.id, &launch)?;
                    let resolved_command = wrapper.to_string_lossy().to_string();
                    state.record_installed_version(ManagedInstalledVersion {
                        version: manifest.version.clone(),
                        install_root: version_root.clone(),
                        resolved_command: resolved_command.clone(),
                        resolved_args: Vec::new(),
                    });
                    state.set_active_version(&manifest.version);
                    state.distribution_kind = Some("uvx".into());
                    state.install_root = Some(version_root);
                    state.resolved_command = Some(resolved_command);
                    state.resolved_args.clear();
                    let _ = tx
                        .unbounded_send(format!("[install] prepared uvx wrapper {}", dist.package));
                }
                RegistryInstallStrategy::Binary { target, .. } => {
                    let sha256 = target.sha256.clone().ok_or_else(|| {
                        anyhow!(
                            "binary distribution for '{}' is missing sha256 metadata",
                            manifest.id
                        )
                    })?;
                    let temp_path = Self::download_artifact_to_temp(
                        &manifest.id,
                        &manifest.version,
                        &target.archive,
                    )?;
                    let _ = tx.unbounded_send(format!(
                        "[install] downloaded binary artifact {}",
                        target.archive
                    ));
                    let state = &mut managed_state.agents[agent_index];
                    install_binary_from_file(
                        &temp_path,
                        &installs_root,
                        &BinaryInstallSpec {
                            version: manifest.version.clone(),
                            sha256,
                            executable_path: Self::normalize_binary_cmd_path(&target.cmd),
                            archive_kind: infer_archive_kind_from_url(&target.archive),
                            args: target.args.clone(),
                        },
                        state,
                    )?;
                    state.distribution_kind = Some("binary".into());
                    let _ = fs::remove_file(&temp_path);
                }
            }

            let state = &mut managed_state.agents[agent_index];
            state.latest_registry_version = Some(manifest.version.clone());
            state.update_available = false;
            state.status = Some("installed".into());
            state.install_error = None;
            state.last_install_at = Some(Self::current_timestamp());
        }

        managed_state.save_default()?;
        Ok(())
    }

    fn remove_registry_agent_job(acp_id: String, tx: mpsc::UnboundedSender<String>) -> Result<()> {
        let app_root = storage::app_root()?;
        let mut managed_state = ManagedAgentsStateFile::load_default()?;
        if let Some(index) = managed_state
            .agents
            .iter()
            .position(|agent| agent.id == acp_id)
        {
            let install_root = managed_state.agents[index].install_root.clone();
            managed_state.agents.remove(index);
            managed_state.save_default()?;
            if let Some(path) = install_root {
                if path.exists() {
                    fs::remove_dir_all(&path)?;
                    let _ = tx.unbounded_send(format!("[remove] deleted {}", path.display()));
                }
            }
            let agent_root = storage::registry_installs_root(&app_root).join(&acp_id);
            if agent_root.exists() {
                let _ = fs::remove_dir_all(&agent_root);
            }
        }
        Ok(())
    }

    fn download_artifact_to_temp(id: &str, version: &str, url: &str) -> Result<PathBuf> {
        let bytes = download_binary_artifact(url)?;
        let temp_path = std::env::temp_dir().join(format!(
            "orbitshell-acp-{}-{}-{}",
            id,
            version,
            url.rsplit('/').next().unwrap_or("artifact.bin")
        ));
        fs::write(&temp_path, bytes)?;
        Ok(temp_path)
    }

    fn normalize_binary_cmd_path(cmd: &str) -> PathBuf {
        let trimmed = cmd
            .trim_start_matches("./")
            .trim_start_matches(".\\")
            .replace('\\', "/");
        PathBuf::from(trimmed)
    }

    fn current_timestamp() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_secs() as i64)
            .unwrap_or_default()
    }

    fn save_mcp_config(&mut self) {
        self.mcp_error = self
            .mcp_config
            .save_default()
            .err()
            .map(|err| err.to_string());
    }

    fn next_mcp_id(&self, base: &str) -> String {
        if !self
            .mcp_config
            .servers
            .iter()
            .any(|server| server.id == base)
        {
            return base.to_string();
        }

        let mut index = 2usize;
        loop {
            let candidate = format!("{base}-{index}");
            if !self
                .mcp_config
                .servers
                .iter()
                .any(|server| server.id == candidate)
            {
                return candidate;
            }
            index += 1;
        }
    }

    fn on_add_stdio_mcp(
        &mut self,
        _event: &MouseDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let id = self.next_mcp_id("stdio");
        self.mcp_config.upsert_server(McpServerConfig {
            id: id.clone(),
            name: format!("STDIO MCP {}", self.mcp_config.servers.len() + 1),
            transport: "stdio".into(),
            command: Some("mcp-server-fs".into()),
            url: None,
            args: vec![".".into()],
            env: Default::default(),
            enabled: true,
            last_tested_at: None,
            last_status: None,
            last_error: None,
        });
        self.save_mcp_config();
        self.mcp_action_lines
            .push(format!("[add] created stdio MCP '{id}'"));
        cx.notify();
    }

    fn on_add_http_mcp(
        &mut self,
        _event: &MouseDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let id = self.next_mcp_id("http");
        self.mcp_config.upsert_server(McpServerConfig {
            id: id.clone(),
            name: format!("HTTP MCP {}", self.mcp_config.servers.len() + 1),
            transport: "http".into(),
            command: None,
            url: Some("http://127.0.0.1:8000/mcp".into()),
            args: Vec::new(),
            env: Default::default(),
            enabled: true,
            last_tested_at: None,
            last_status: None,
            last_error: None,
        });
        self.save_mcp_config();
        self.mcp_action_lines
            .push(format!("[add] created http MCP '{id}'"));
        cx.notify();
    }

    fn on_toggle_mcp_enabled(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(server) = self.mcp_config.servers.get_mut(index) else {
            return;
        };
        server.enabled = !server.enabled;
        let id = server.id.clone();
        let enabled = server.enabled;
        self.save_mcp_config();
        self.mcp_action_lines.push(format!(
            "[toggle] '{}' is now {}",
            id,
            if enabled { "enabled" } else { "disabled" }
        ));
        cx.notify();
    }

    fn on_duplicate_mcp(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(server) = self.mcp_config.servers.get(index).cloned() else {
            return;
        };
        let mut duplicate = server.clone();
        duplicate.id = self.next_mcp_id(&server.id);
        duplicate.name = format!("{} Copy", server.name);
        duplicate.last_tested_at = None;
        duplicate.last_status = None;
        duplicate.last_error = None;
        self.mcp_config.upsert_server(duplicate.clone());
        self.save_mcp_config();
        self.mcp_action_lines.push(format!(
            "[duplicate] copied '{}' to '{}'",
            server.id, duplicate.id
        ));
        cx.notify();
    }

    fn on_delete_mcp(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(server) = self.mcp_config.servers.get(index).cloned() else {
            return;
        };
        if self.mcp_config.remove_server(&server.id) {
            self.save_mcp_config();
            self.mcp_action_lines
                .push(format!("[delete] removed '{}'", server.id));
            cx.notify();
        }
    }

    fn on_test_mcp(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(server) = self.mcp_config.servers.get(index).cloned() else {
            return;
        };
        if self.mcp_action_busy {
            self.mcp_action_lines
                .push("[info] another MCP action is already running".to_string());
            cx.notify();
            return;
        }

        self.mcp_action_busy = true;
        self.mcp_action_lines
            .push(format!("[test] probing MCP '{}'", server.id));
        let (tx, mut rx) = mpsc::unbounded::<Result<(String, McpProbeResult), String>>();
        thread::spawn(move || {
            let _ = tx.unbounded_send(Ok((server.id.clone(), probe_server_config(&server))));
        });

        cx.spawn(|view: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                while let Some(result) = rx.next().await {
                    let _ = view.update(&mut cx, |view, cx| {
                        view.mcp_action_busy = false;
                        match result {
                            Ok((id, probe)) => {
                                if view.mcp_config.apply_probe_result(&id, &probe) {
                                    view.save_mcp_config();
                                }
                                let detail =
                                    probe.error.clone().unwrap_or_else(|| "ok".to_string());
                                view.mcp_action_lines
                                    .push(format!("[test] {} => {} ({detail})", id, probe.status));
                            }
                            Err(err) => {
                                view.mcp_action_lines.push(format!("[error] {err}"));
                            }
                        }
                        cx.notify();
                    });
                }
            }
        })
        .detach();

        cx.notify();
    }

    fn render_status_badge(&self, status: CatalogInstallStatus) -> Div {
        let (label, fg, bg, border) = match status {
            CatalogInstallStatus::Installed => ("Installed", 0x8bd06f, 0x122016, 0x244a2b),
            CatalogInstallStatus::NotInstalled => ("Not installed", 0xffb366, 0x201612, 0x4a2b24),
            CatalogInstallStatus::UpdateAvailable => {
                ("Update available", 0x7db6ff, 0x122033, 0x21466d)
            }
        };

        div()
            .px(px(8.0))
            .py(px(3.0))
            .rounded(px(999.0))
            .text_size(px(11.0))
            .border_1()
            .border_color(rgb(border))
            .bg(rgb(bg))
            .text_color(rgb(fg))
            .child(label)
    }

    fn render_source_badge(&self, row: &CatalogAgentRow) -> Div {
        let (label, fg, bg, border) = match row
            .selected_source
            .as_ref()
            .map(|source| source.source_type)
        {
            Some(crate::acp::resolve::AgentSourceKind::Registry) | None => {
                ("Registry", 0x5fb0ff, 0x112033, 0x244b7d)
            }
            Some(crate::acp::resolve::AgentSourceKind::GlobalCustom) => {
                ("Custom", 0xffd479, 0x2a2010, 0x6d5321)
            }
            Some(crate::acp::resolve::AgentSourceKind::WorkspaceCustom) => {
                ("Workspace", 0xf29cff, 0x26142b, 0x5c2d66)
            }
        };

        div()
            .px(px(8.0))
            .py(px(3.0))
            .rounded(px(999.0))
            .text_size(px(11.0))
            .border_1()
            .border_color(rgb(border))
            .bg(rgb(bg))
            .text_color(rgb(fg))
            .child(label)
    }

    fn render_filter_button(&self, label: &'static str, active: bool) -> Div {
        div()
            .px(px(10.0))
            .py(px(6.0))
            .rounded(px(6.0))
            .bg(if active { rgb(0x163a59) } else { rgb(0x101010) })
            .border_1()
            .border_color(if active { rgb(0x2e75ad) } else { rgb(0x2a2a2a) })
            .text_size(px(12.0))
            .text_color(if active { rgb(0xffffff) } else { rgb(0xd0d0d0) })
            .child(label)
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
        if lower == "uvx" {
            return "uvx not found in PATH. Install uv or restart the app so PATH is reloaded."
                .to_string();
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
                let add_stdio_handle = cx.entity().downgrade();
                let add_http_handle = cx.entity().downgrade();
                let mcp_path = GlobalMcpConfig::default_path().ok();
                let recent_logs: Vec<String> = self
                    .mcp_action_lines
                    .iter()
                    .rev()
                    .take(10)
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
                            .child("Global MCP servers shared by every ACP session."),
                    )
                    .child(div().text_size(px(12.0)).text_color(rgb(0x9a9a9a)).child(
                        if let Some(path) = mcp_path {
                            format!("Config file: {}", path.to_string_lossy())
                        } else {
                            "Config file unavailable".to_string()
                        },
                    ))
                    .child(if let Some(err) = &self.mcp_error {
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
                            .child(self.render_action_button("Add STDIO").on_mouse_down(
                                MouseButton::Left,
                                move |event, window, cx| {
                                    cx.stop_propagation();
                                    let _ = add_stdio_handle.update(cx, |view, cx| {
                                        view.on_add_stdio_mcp(event, window, cx);
                                    });
                                },
                            ))
                            .child(self.render_action_button("Add HTTP").on_mouse_down(
                                MouseButton::Left,
                                move |event, window, cx| {
                                    cx.stop_propagation();
                                    let _ = add_http_handle.update(cx, |view, cx| {
                                        view.on_add_http_mcp(event, window, cx);
                                    });
                                },
                            )),
                    )
                    .child(div().h(px(1.0)).bg(rgb(0x1f1f1f)))
                    .child(
                        div().flex().flex_col().gap(px(8.0)).children(
                            self.mcp_config
                                .servers
                                .iter()
                                .enumerate()
                                .map(|(index, server)| {
                                    let toggle_handle = cx.entity().downgrade();
                                    let duplicate_handle = cx.entity().downgrade();
                                    let delete_handle = cx.entity().downgrade();
                                    let test_handle = cx.entity().downgrade();
                                    let status_label = server
                                        .last_status
                                        .clone()
                                        .unwrap_or_else(|| "untested".to_string());
                                    let status_color = match status_label.as_str() {
                                        "online" => rgb(0x8bd06f),
                                        "offline" => rgb(0xffb366),
                                        "misconfigured" => rgb(0xff7b72),
                                        _ => rgb(0x8a8a8a),
                                    };

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
                                                                .text_size(px(12.0))
                                                                .text_color(rgb(0xd0d0d0))
                                                                .child(format!(
                                                                    "{} ({})",
                                                                    server.name, server.id
                                                                )),
                                                        )
                                                        .child(
                                                            div()
                                                                .text_size(px(11.0))
                                                                .text_color(status_color)
                                                                .child(status_label),
                                                        ),
                                                )
                                                .child(
                                                    div()
                                                        .text_size(px(11.0))
                                                        .text_color(rgb(0x8a8a8a))
                                                        .font_family("Cascadia Code")
                                                        .truncate()
                                                        .child(format!(
                                                            "{} {}",
                                                            server.transport,
                                                            server
                                                                .command
                                                                .clone()
                                                                .or_else(|| server.url.clone())
                                                                .unwrap_or_else(|| "<unset>".into())
                                                        )),
                                                )
                                                .child(if let Some(error) = &server.last_error {
                                                    div()
                                                        .text_size(px(11.0))
                                                        .text_color(rgb(0xff7b72))
                                                        .truncate()
                                                        .child(error.clone())
                                                } else {
                                                    div()
                                                }),
                                        )
                                        .child(
                                            div()
                                                .flex()
                                                .items_center()
                                                .gap(px(8.0))
                                                .child(
                                                    self.render_action_button(if server.enabled {
                                                        "Disable"
                                                    } else {
                                                        "Enable"
                                                    })
                                                    .on_mouse_down(
                                                        MouseButton::Left,
                                                        move |_e, _w, cx| {
                                                            cx.stop_propagation();
                                                            let _ = toggle_handle.update(
                                                                cx,
                                                                |view, cx| {
                                                                    view.on_toggle_mcp_enabled(
                                                                        index, cx,
                                                                    );
                                                                },
                                                            );
                                                        },
                                                    ),
                                                )
                                                .child(
                                                    self.render_action_button("Duplicate")
                                                        .on_mouse_down(
                                                            MouseButton::Left,
                                                            move |_e, _w, cx| {
                                                                cx.stop_propagation();
                                                                let _ = duplicate_handle.update(
                                                                    cx,
                                                                    |view, cx| {
                                                                        view.on_duplicate_mcp(
                                                                            index, cx,
                                                                        );
                                                                    },
                                                                );
                                                            },
                                                        ),
                                                )
                                                .child(
                                                    self.render_action_button("Test")
                                                        .on_mouse_down(
                                                            MouseButton::Left,
                                                            move |_e, _w, cx| {
                                                                cx.stop_propagation();
                                                                let _ = test_handle.update(
                                                                    cx,
                                                                    |view, cx| {
                                                                        view.on_test_mcp(index, cx);
                                                                    },
                                                                );
                                                            },
                                                        ),
                                                )
                                                .child(
                                                    self.render_action_button("Delete")
                                                        .on_mouse_down(
                                                            MouseButton::Left,
                                                            move |_e, _w, cx| {
                                                                cx.stop_propagation();
                                                                let _ = delete_handle.update(
                                                                    cx,
                                                                    |view, cx| {
                                                                        view.on_delete_mcp(
                                                                            index, cx,
                                                                        );
                                                                    },
                                                                );
                                                            },
                                                        ),
                                                ),
                                        )
                                }),
                        ),
                    )
                    .child(if recent_logs.is_empty() {
                        div()
                    } else {
                        div()
                            .rounded(px(8.0))
                            .bg(rgb(0x0f0f0f))
                            .border_1()
                            .border_color(rgb(0x1f1f1f))
                            .p(px(10.0))
                            .child(div().text_size(px(11.0)).text_color(rgb(0x8a8a8a)).child(
                                if self.mcp_action_busy {
                                    "MCP log (running...)"
                                } else {
                                    "MCP log"
                                },
                            ))
                            .child(div().mt(px(8.0)).flex().flex_col().gap(px(4.0)).children(
                                recent_logs.into_iter().map(|line| {
                                    div()
                                        .text_size(px(11.0))
                                        .text_color(rgb(0xbdbdbd))
                                        .font_family("Cascadia Code")
                                        .truncate()
                                        .child(line)
                                }),
                            ))
                    });
            }
            "ACP Registry" => {
                let all_rows =
                    build_catalog_rows(Some(&self.registry_data), &self.effective_agents);
                let visible_rows = self.visible_catalog_rows();
                let installed_count = all_rows
                    .iter()
                    .filter(|row| {
                        matches!(
                            row.install_status,
                            CatalogInstallStatus::Installed | CatalogInstallStatus::UpdateAvailable
                        )
                    })
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
                let registry_status = if self.registry_refresh_busy {
                    "Refreshing registry...".to_string()
                } else if let Some(err) = &self.registry_refresh_error {
                    format!("Using cached registry after refresh error: {err}")
                } else if self.registry_used_cache {
                    "Using cached registry".to_string()
                } else {
                    "Registry is up to date".to_string()
                };
                let refresh_handle = cx.entity().downgrade();
                let all_handle = cx.entity().downgrade();
                let installed_handle = cx.entity().downgrade();
                let not_installed_handle = cx.entity().downgrade();
                let update_handle = cx.entity().downgrade();

                content = content
                    .child(
                        div()
                            .text_size(px(12.0))
                            .text_color(rgb(0x8a8a8a))
                            .child("Unified ACP catalog from the official registry plus your global and workspace agents."),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.0))
                            .text_size(px(12.0))
                            .text_color(rgb(0x9a9a9a))
                            .child(registry_status),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap(px(8.0))
                            .child(
                                div()
                                    .text_size(px(12.0))
                                    .text_color(rgb(0x9a9a9a))
                                    .child(format!(
                                        "Agents: {} total, {} installed",
                                        all_rows.len(),
                                        installed_count
                                    )),
                            )
                            .child(
                                self.render_action_button("Refresh")
                                    .on_mouse_down(MouseButton::Left, move |event, window, cx| {
                                        cx.stop_propagation();
                                        let _ = refresh_handle.update(cx, |view, cx| {
                                            view.on_refresh_registry(event, window, cx);
                                        });
                                    }),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.0))
                            .child(
                                self.render_filter_button(
                                    "All",
                                    self.catalog_filter == CatalogFilter::All,
                                )
                                .on_mouse_down(MouseButton::Left, move |_e, _w, cx| {
                                    cx.stop_propagation();
                                    let _ = all_handle.update(cx, |view, cx| {
                                        view.catalog_filter = CatalogFilter::All;
                                        cx.notify();
                                    });
                                }),
                            )
                            .child(
                                self.render_filter_button(
                                    "Installed",
                                    self.catalog_filter == CatalogFilter::Installed,
                                )
                                .on_mouse_down(MouseButton::Left, move |_e, _w, cx| {
                                    cx.stop_propagation();
                                    let _ = installed_handle.update(cx, |view, cx| {
                                        view.catalog_filter = CatalogFilter::Installed;
                                        cx.notify();
                                    });
                                }),
                            )
                            .child(
                                self.render_filter_button(
                                    "Not Installed",
                                    self.catalog_filter == CatalogFilter::NotInstalled,
                                )
                                .on_mouse_down(MouseButton::Left, move |_e, _w, cx| {
                                    cx.stop_propagation();
                                    let _ = not_installed_handle.update(cx, |view, cx| {
                                        view.catalog_filter = CatalogFilter::NotInstalled;
                                        cx.notify();
                                    });
                                }),
                            )
                            .child(
                                self.render_filter_button(
                                    "Update Available",
                                    self.catalog_filter == CatalogFilter::UpdateAvailable,
                                )
                                .on_mouse_down(MouseButton::Left, move |_e, _w, cx| {
                                    cx.stop_propagation();
                                    let _ = update_handle.update(cx, |view, cx| {
                                        view.catalog_filter = CatalogFilter::UpdateAvailable;
                                        cx.notify();
                                    });
                                }),
                            ),
                    )
                    .child(div().h(px(1.0)).bg(rgb(0x1f1f1f)))
                    .child(
                        div().flex().flex_col().gap(px(8.0)).children(
                            visible_rows
                                .iter()
                                .enumerate()
                                .map(|(index, row)| {
                                    let can_install = matches!(
                                        row.install_status,
                                        CatalogInstallStatus::NotInstalled
                                    ) && (row
                                        .selected_source
                                        .as_ref()
                                        .and_then(|source| source.spec.install.as_ref())
                                        .is_some()
                                        || row
                                            .registry_manifest
                                            .as_ref()
                                            .and_then(|manifest| manifest.preferred_install_strategy())
                                            .is_some());
                                    let can_update = matches!(
                                        row.install_status,
                                        CatalogInstallStatus::UpdateAvailable
                                    ) && row.registry_manifest.is_some();
                                    let can_remove = row
                                        .selected_source
                                        .as_ref()
                                        .map(|source| {
                                            matches!(
                                                source.source_type,
                                                crate::acp::resolve::AgentSourceKind::Registry
                                            )
                                        })
                                        .unwrap_or(false);
                                    let can_auth = row
                                        .selected_source
                                        .as_ref()
                                        .and_then(|source| source.spec.auth.as_ref())
                                        .is_some();
                                    let can_test = row.selected_source.is_some();
                                    let other_sources = if row.other_sources.is_empty() {
                                        None
                                    } else {
                                        Some(
                                            row.other_sources
                                                .iter()
                                                .map(|source| match source.source_type {
                                                    crate::acp::resolve::AgentSourceKind::Registry => {
                                                        "Registry".to_string()
                                                    }
                                                    crate::acp::resolve::AgentSourceKind::GlobalCustom => {
                                                        "Custom".to_string()
                                                    }
                                                    crate::acp::resolve::AgentSourceKind::WorkspaceCustom => {
                                                        "Workspace".to_string()
                                                    }
                                                })
                                                .collect::<Vec<_>>()
                                                .join(", "),
                                        )
                                    };
                                    let install_handle = cx.entity().downgrade();
                                    let update_action_handle = cx.entity().downgrade();
                                    let remove_handle = cx.entity().downgrade();
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
                                                        .gap(px(6.0))
                                                        .child(
                                                            div()
                                                                .flex_1()
                                                                .min_w(px(0.0))
                                                                .text_size(px(12.0))
                                                                .text_color(rgb(0xd0d0d0))
                                                                .truncate()
                                                                .child(format!("{} ({})", row.name, row.acp_id)),
                                                        )
                                                        .child(self.render_source_badge(row))
                                                        .child(self.render_status_badge(row.install_status)),
                                                )
                                                .child(
                                                    div()
                                                        .flex_1()
                                                        .min_w(px(0.0))
                                                        .text_size(px(11.0))
                                                        .text_color(rgb(0x8a8a8a))
                                                        .truncate()
                                                        .child(if row.description.is_empty() {
                                                            "No description available".to_string()
                                                        } else {
                                                            row.description.clone()
                                                        }),
                                                )
                                                .child(
                                                    div()
                                                        .flex_1()
                                                        .min_w(px(0.0))
                                                        .text_size(px(11.0))
                                                        .text_color(rgb(0x6f6f6f))
                                                        .font_family("Cascadia Code")
                                                        .truncate()
                                                        .child(if let Some(source) = &row.selected_source {
                                                            source.spec.display_command()
                                                        } else {
                                                            row.version
                                                                .as_ref()
                                                                .map(|version| format!("registry v{version}"))
                                                                .unwrap_or_else(|| "registry".to_string())
                                                        }),
                                                )
                                                .child(if let Some(other_sources) = other_sources {
                                                    div()
                                                        .flex_1()
                                                        .min_w(px(0.0))
                                                        .text_size(px(11.0))
                                                        .text_color(rgb(0x9b89ff))
                                                        .truncate()
                                                        .child(format!("Other sources: {other_sources}"))
                                                } else {
                                                    div()
                                                }),
                                        )
                                        .child(
                                            div()
                                                .flex()
                                                .items_center()
                                                .gap(px(8.0))
                                                .child(if can_install {
                                                    self.render_action_button("Install")
                                                        .on_mouse_down(
                                                            MouseButton::Left,
                                                            move |_e, _w, cx| {
                                                                cx.stop_propagation();
                                                                let _ = install_handle.update(
                                                                    cx,
                                                                    |view, cx| {
                                                                        view.on_install_agent(
                                                                            index, cx
                                                                        );
                                                                    },
                                                                );
                                                            },
                                                        )
                                                } else {
                                                    div()
                                                })
                                                .child(if can_update {
                                                    self.render_action_button("Update")
                                                        .on_mouse_down(
                                                            MouseButton::Left,
                                                            move |_e, _w, cx| {
                                                                cx.stop_propagation();
                                                                let _ = update_action_handle.update(
                                                                    cx,
                                                                    |view, cx| {
                                                                        view.on_update_agent(
                                                                            index, cx
                                                                        );
                                                                    },
                                                                );
                                                            },
                                                        )
                                                } else {
                                                    div()
                                                })
                                                .child(if can_auth {
                                                    self.render_action_button("Authenticate")
                                                        .on_mouse_down(
                                                            MouseButton::Left,
                                                            move |_e, _w, cx| {
                                                                cx.stop_propagation();
                                                                let _ = auth_handle.update(
                                                                    cx,
                                                                    |view, cx| {
                                                                        view.on_auth_agent(
                                                                            index, cx
                                                                        );
                                                                    },
                                                                );
                                                            },
                                                        )
                                                } else {
                                                    div()
                                                })
                                                .child(if can_remove {
                                                    self.render_action_button("Remove")
                                                        .on_mouse_down(
                                                            MouseButton::Left,
                                                            move |_e, _w, cx| {
                                                                cx.stop_propagation();
                                                                let _ = remove_handle.update(
                                                                    cx,
                                                                    |view, cx| {
                                                                        view.on_remove_agent(
                                                                            index, cx
                                                                        );
                                                                    },
                                                                );
                                                            },
                                                        )
                                                } else {
                                                    div()
                                                })
                                                .child(
                                                    if can_test {
                                                        self.render_action_button("Test")
                                                            .on_mouse_down(
                                                                MouseButton::Left,
                                                                move |_e, _w, cx| {
                                                                    cx.stop_propagation();
                                                                    let _ = test_handle.update(
                                                                        cx,
                                                                        |view, cx| {
                                                                            view.on_test_agent(
                                                                                index, cx
                                                                            );
                                                                        },
                                                                    );
                                                                },
                                                            )
                                                    } else {
                                                        div()
                                                    },
                                                ),
                                        )
                                }),
                        ),
                    )
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
                            .child(div().text_size(px(11.0)).text_color(rgb(0x8a8a8a)).child(
                                if self.agent_action_busy {
                                    "Action log (running...)"
                                } else {
                                    "Action log"
                                },
                            ))
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
