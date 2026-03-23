use crate::acp::manager::AgentSpec;
use anyhow::{Context, Result, anyhow};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

#[derive(Clone, Debug, PartialEq)]
pub struct IncomingRequest {
    pub id: u64,
    pub method: String,
    pub params: Value,
}

#[derive(Debug)]
pub enum IncomingEvent {
    Json(Value),
    Request(IncomingRequest),
    Stderr(String),
    Closed,
}

pub struct AcpTransport {
    stdin_tx: Sender<String>,
    event_rx: Receiver<IncomingEvent>,
    next_id: AtomicU64,
    _child: Arc<Mutex<Child>>,
}

impl AcpTransport {
    pub fn spawn(spec: &AgentSpec) -> Result<Self> {
        let mut child = {
            let mut candidates = vec![spec.command.clone()];
            if cfg!(windows) && Path::new(&spec.command).extension().is_none() {
                candidates.push(format!("{}.cmd", spec.command));
                candidates.push(format!("{}.exe", spec.command));
            }

            let mut spawned = None;
            let mut first_err = None;
            for candidate in candidates {
                match Self::spawn_child(spec, &candidate) {
                    Ok(child) => {
                        spawned = Some(child);
                        break;
                    }
                    Err(err) => {
                        if first_err.is_none() {
                            first_err = Some(err);
                        }
                    }
                }
            }
            match spawned {
                Some(child) => child,
                None => {
                    return Err(first_err.unwrap_or_else(|| anyhow!("spawn failed"))).with_context(
                        || format!("failed to spawn agent command '{}'", spec.command),
                    );
                }
            }
        };

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow!("agent stdin is not piped"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow!("agent stdout is not piped"))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| anyhow!("agent stderr is not piped"))?;

        let child = Arc::new(Mutex::new(child));
        let (stdin_tx, stdin_rx) = mpsc::channel::<String>();
        let (event_tx, event_rx) = mpsc::channel::<IncomingEvent>();

        Self::spawn_stdin_writer(stdin, stdin_rx);
        Self::spawn_stdout_reader(stdout, event_tx.clone());
        Self::spawn_stderr_reader(stderr, event_tx);

        Ok(Self {
            stdin_tx,
            event_rx,
            next_id: AtomicU64::new(1),
            _child: child,
        })
    }

    fn spawn_child(spec: &AgentSpec, command_name: &str) -> Result<Child> {
        let mut command = Command::new(command_name);
        for arg in &spec.args {
            command.arg(arg);
        }
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (key, value) in &spec.fixed_env {
            command.env(key, value);
        }
        for key in &spec.env_keys {
            if let Ok(value) = std::env::var(key) {
                command.env(key, value);
            }
        }
        command
            .spawn()
            .with_context(|| format!("failed to spawn process for command '{}'", command_name))
    }

    fn spawn_stdin_writer(mut stdin: ChildStdin, rx: Receiver<String>) {
        thread::spawn(move || {
            while let Ok(line) = rx.recv() {
                if stdin.write_all(line.as_bytes()).is_err() {
                    break;
                }
                if stdin.write_all(b"\n").is_err() {
                    break;
                }
                if stdin.flush().is_err() {
                    break;
                }
            }
        });
    }

    fn spawn_stdout_reader(stdout: impl std::io::Read + Send + 'static, tx: Sender<IncomingEvent>) {
        thread::spawn(move || {
            let reader = BufReader::new(stdout);
            for line in reader.lines() {
                match line {
                    Ok(content) => {
                        let trimmed = content.trim();
                        if trimmed.is_empty() {
                            continue;
                        }
                        match serde_json::from_str::<Value>(trimmed) {
                            Ok(json) => {
                                let _ = tx.send(classify_incoming_json_message(&json));
                            }
                            Err(_) => {
                                let _ = tx.send(IncomingEvent::Stderr(content));
                            }
                        }
                    }
                    Err(_) => {
                        let _ = tx.send(IncomingEvent::Closed);
                        break;
                    }
                }
            }
        });
    }

    fn spawn_stderr_reader(stderr: impl std::io::Read + Send + 'static, tx: Sender<IncomingEvent>) {
        thread::spawn(move || {
            let reader = BufReader::new(stderr);
            for line in reader.lines() {
                match line {
                    Ok(content) => {
                        let _ = tx.send(IncomingEvent::Stderr(content));
                    }
                    Err(_) => {
                        let _ = tx.send(IncomingEvent::Closed);
                        break;
                    }
                }
            }
        });
    }

    pub fn notify(&self, method: &str, params: Value) -> Result<()> {
        let payload = json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params
        });
        self.stdin_tx
            .send(payload.to_string())
            .context("failed to write ACP notification")
    }

    pub fn respond_result(&self, id: u64, result: Value) -> Result<()> {
        let payload = json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": result
        });
        self.stdin_tx
            .send(payload.to_string())
            .context("failed to write ACP response")
    }

    pub fn respond_error(
        &self,
        id: u64,
        code: i64,
        message: impl Into<String>,
        data: Option<Value>,
    ) -> Result<()> {
        let mut error = json!({
            "code": code,
            "message": message.into(),
        });
        if let Some(data) = data
            && let Some(object) = error.as_object_mut()
        {
            object.insert("data".into(), data);
        }
        let payload = json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": error
        });
        self.stdin_tx
            .send(payload.to_string())
            .context("failed to write ACP error response")
    }

    pub fn request(
        &self,
        method: &str,
        params: Value,
        timeout: Duration,
        mut on_notification: Option<&mut dyn FnMut(&str, &Value)>,
        mut on_request: Option<&mut dyn FnMut(&IncomingRequest) -> Result<()>>,
    ) -> Result<Value> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let payload = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params
        });

        self.stdin_tx
            .send(payload.to_string())
            .context("failed to write ACP request")?;

        let started = Instant::now();
        let mut buffered_responses: HashMap<u64, Value> = HashMap::new();
        let mut last_diagnostic: Option<String> = None;
        loop {
            if let Some(response) = buffered_responses.remove(&id) {
                if let Some(err) = response.get("error") {
                    return Err(anyhow!("ACP request failed for method '{method}': {err}"));
                }
                return Ok(response.get("result").cloned().unwrap_or(Value::Null));
            }

            let elapsed = started.elapsed();
            if elapsed >= timeout {
                let hint = last_diagnostic.unwrap_or_else(|| {
                    "no JSON-RPC response received; selected command may not speak ACP".to_string()
                });
                return Err(anyhow!("ACP request '{method}' timed out ({hint})"));
            }
            let wait = timeout.saturating_sub(elapsed);

            let event = self.event_rx.recv_timeout(wait).with_context(|| {
                format!("ACP request '{method}' failed while waiting for response")
            })?;
            match event {
                IncomingEvent::Json(msg) => {
                    if let Some(method_name) = msg.get("method").and_then(Value::as_str) {
                        if let Some(params) = msg.get("params") {
                            if let Some(handler) = on_notification.as_mut() {
                                (*handler)(method_name, params);
                            }
                        }
                        continue;
                    }

                    if let Some(resp_id) = msg.get("id").and_then(Value::as_u64) {
                        buffered_responses.insert(resp_id, msg);
                        continue;
                    }
                }
                IncomingEvent::Request(request) => {
                    if let Some(handler) = on_request.as_mut() {
                        (*handler)(&request)?;
                    } else {
                        return Err(anyhow!(
                            "unexpected inbound ACP request '{}' while waiting for method '{method}'",
                            request.method
                        ));
                    }
                }
                IncomingEvent::Stderr(line) => {
                    if !line.trim().is_empty() {
                        last_diagnostic = Some(line.clone());
                    }
                    if let Some(handler) = on_notification.as_mut() {
                        (*handler)("stderr", &Value::String(line));
                    }
                }
                IncomingEvent::Closed => {
                    return Err(anyhow!("ACP agent process closed unexpectedly"));
                }
            }
        }
    }
}

pub fn classify_incoming_json_message(msg: &Value) -> IncomingEvent {
    if let Some(id) = msg.get("id").and_then(Value::as_u64)
        && let Some(method) = msg.get("method").and_then(Value::as_str)
    {
        return IncomingEvent::Request(IncomingRequest {
            id,
            method: method.to_string(),
            params: msg.get("params").cloned().unwrap_or(Value::Null),
        });
    }

    IncomingEvent::Json(msg.clone())
}

#[cfg(test)]
mod tests {
    use super::classify_incoming_json_message;
    use serde_json::json;

    #[test]
    fn classify_json_request_distinguishes_request_from_notification() {
        let request = json!({
            "jsonrpc": "2.0",
            "id": 99,
            "method": "session/request_permission",
            "params": {
                "title": "Approve access"
            }
        });
        let notification = json!({
            "jsonrpc": "2.0",
            "method": "session/update",
            "params": {
                "message": "hello"
            }
        });

        match classify_incoming_json_message(&request) {
            super::IncomingEvent::Request(request) => {
                assert_eq!(request.id, 99);
                assert_eq!(request.method, "session/request_permission");
                assert_eq!(request.params["title"], "Approve access");
            }
            other => panic!("expected request event, got {other:?}"),
        }

        match classify_incoming_json_message(&notification) {
            super::IncomingEvent::Json(value) => {
                assert_eq!(value["method"], "session/update");
            }
            other => panic!("expected notification json event, got {other:?}"),
        }
    }
}
