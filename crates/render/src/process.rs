//! Bridge transport over a child process: one JSON message per line each way.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tessel_editor_core::backend::BackendError;
use tessel_editor_core::bridge::{BRIDGE_VERSION, Transport};

use crate::runtime::RuntimeConfig;

const STDERR_LINES: usize = 20;

struct Running {
    child: Child,
    stdin: Option<ChildStdin>,
    messages: Receiver<Value>,
    stderr: Arc<Mutex<VecDeque<String>>>,
}

pub struct ProcessTransport {
    config: RuntimeConfig,
    running: Option<Running>,
    /// Set when the process ended; later sends fail with this instead of respawning.
    ended: Option<String>,
}

impl ProcessTransport {
    pub fn new(config: RuntimeConfig) -> Self {
        Self {
            config,
            running: None,
            ended: None,
        }
    }

    fn spawn(&self) -> Result<Running, BackendError> {
        self.config.check()?;
        let mut command = Command::new(&self.config.node);
        command
            .arg(self.config.worker_script())
            .current_dir(&self.config.runtime_dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(gl) = &self.config.gl {
            command.env("TESSEL_GL", gl);
        }
        let mut child = command.spawn().map_err(|error| {
            BackendError::new(
                "backend_unavailable",
                format!("Cannot start `{}`: {error}", self.config.node.display()),
            )
        })?;
        let stdin = child.stdin.take();
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| BackendError::new("backend_unavailable", "No worker stdout"))?;
        let stderr_pipe = child
            .stderr
            .take()
            .ok_or_else(|| BackendError::new("backend_unavailable", "No worker stderr"))?;

        let (sender, messages) = mpsc::channel();
        let stderr = Arc::new(Mutex::new(VecDeque::new()));
        let tail = Arc::clone(&stderr);
        thread::spawn(move || {
            for line in BufReader::new(stderr_pipe).lines().map_while(Result::ok) {
                if let Ok(mut lines) = tail.lock() {
                    if lines.len() == STDERR_LINES {
                        lines.pop_front();
                    }
                    lines.push_back(line);
                }
            }
        });
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if let Ok(message) = serde_json::from_str::<Value>(&line)
                    && sender.send(message).is_err()
                {
                    return;
                }
            }
            // The channel disconnecting is how `recv` learns the process ended.
        });
        Ok(Running {
            child,
            stdin,
            messages,
            stderr,
        })
    }

    fn stderr_tail(running: &Running) -> String {
        running
            .stderr
            .lock()
            .map(|lines| lines.iter().cloned().collect::<Vec<_>>().join("\n"))
            .unwrap_or_default()
    }

    fn closed(reason: String) -> Value {
        json!({ "v": BRIDGE_VERSION, "type": "closed", "reason": reason })
    }

    fn end(&mut self, why: &str) -> String {
        let detail = self
            .running
            .take()
            .map(|mut running| {
                let _ = running.child.kill();
                let status = running.child.wait().ok().map(|status| status.to_string());
                let stderr = Self::stderr_tail(&running);
                format!(
                    "{why} ({}){}",
                    status.unwrap_or_default(),
                    if stderr.is_empty() {
                        String::new()
                    } else {
                        format!(": {stderr}")
                    }
                )
            })
            .unwrap_or_else(|| why.to_string());
        self.ended = Some(detail.clone());
        detail
    }
}

impl Transport for ProcessTransport {
    fn send(&mut self, message: &Value) -> Result<(), BackendError> {
        if let Some(reason) = &self.ended {
            return Err(BackendError::new("backend_unavailable", reason.clone()));
        }
        if self.running.is_none() {
            self.running = Some(self.spawn()?);
        }
        let Some(running) = self.running.as_mut() else {
            return Err(BackendError::new(
                "backend_unavailable",
                "The worker is not running",
            ));
        };
        let Some(stdin) = running.stdin.as_mut() else {
            return Err(BackendError::new(
                "backend_unavailable",
                "The worker input is closed",
            ));
        };
        let written = writeln!(stdin, "{message}").and_then(|()| stdin.flush());
        if let Err(error) = written {
            let reason = self.end(&format!("The worker stopped accepting messages ({error})"));
            return Err(BackendError::new("backend_unavailable", reason));
        }
        Ok(())
    }

    fn recv(&mut self, timeout: Duration) -> Option<Value> {
        let running = self.running.as_ref()?;
        match running.messages.recv_timeout(timeout) {
            Ok(message) => Some(message),
            Err(RecvTimeoutError::Timeout) => None,
            Err(RecvTimeoutError::Disconnected) => {
                // Give the stderr thread a moment to drain what the dying process printed.
                let deadline = Instant::now() + Duration::from_millis(100);
                while Instant::now() < deadline {
                    thread::sleep(Duration::from_millis(10));
                }
                let reason = self.end("The worker process exited");
                Some(Self::closed(reason))
            }
        }
    }

    fn close(&mut self) {
        let Some(mut running) = self.running.take() else {
            return;
        };
        // Closing stdin is the polite shutdown: the worker closes Chrome and exits.
        drop(running.stdin.take());
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if matches!(running.child.try_wait(), Ok(Some(_))) {
                return;
            }
            thread::sleep(Duration::from_millis(25));
        }
        let _ = running.child.kill();
        let _ = running.child.wait();
    }
}

impl Drop for ProcessTransport {
    fn drop(&mut self) {
        self.close();
    }
}
