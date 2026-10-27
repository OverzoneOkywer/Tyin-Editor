use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant};
use url::Url;

const INITIALIZE_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LspDiagnostic {
    pub uri: String,
    pub line: u32,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LspCompletion {
    pub label: String,
    pub insert_text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LspLocation {
    pub uri: String,
    pub line: u32,
    pub character: u32,
}

struct LspClient {
    child: Child,
    stdin: ChildStdin,
    next_request_id: u64,
    document_versions: HashMap<PathBuf, i32>,
    diagnostics_rx: Receiver<(String, Vec<LspDiagnostic>)>,
    responses_rx: Receiver<(u64, Result<Value, String>)>,
    last_used: Instant,
}

#[derive(Default)]
pub struct LspPool {
    clients: HashMap<String, LspClient>,
    diagnostics: HashMap<String, Vec<LspDiagnostic>>,
}

impl LspPool {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn start(&mut self, key: &str, command: &str, args: &[String], root: &Path) -> Result<()> {
        if self.clients.contains_key(key) {
            return Ok(());
        }

        let mut child = Command::new(command)
            .args(args)
            .current_dir(root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .with_context(|| format!("failed to start language server `{command}`"))?;
        let mut stdin = child
            .stdin
            .take()
            .context("language server stdin unavailable")?;
        let stdout = child
            .stdout
            .take()
            .context("language server stdout unavailable")?;
        let (diagnostics_tx, diagnostics_rx) = mpsc::channel();
        let (responses_tx, responses_rx) = mpsc::channel();
        let (initialized_tx, initialized_rx) = mpsc::channel();
        thread::spawn(move || {
            read_messages(stdout, diagnostics_tx, initialized_tx, responses_tx);
        });

        let root_uri = match file_uri(root) {
            Ok(uri) => uri,
            Err(error) => {
                let _ = child.kill();
                return Err(error);
            }
        };
        let initialize = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "processId": std::process::id(),
                "rootUri": root_uri.clone(),
                "capabilities": {
                    "textDocument": {
                        "publishDiagnostics": { "relatedInformation": true },
                        "completion": {
                            "completionItem": {
                                "snippetSupport": false
                            }
                        },
                        "definition": {}
                    }
                },
                "workspaceFolders": [{
                    "uri": root_uri,
                    "name": root.file_name().and_then(|name| name.to_str()).unwrap_or("workspace")
                }]
            }
        });
        if let Err(error) = write_message(&mut stdin, &initialize) {
            let _ = child.kill();
            return Err(error);
        }
        match initialized_rx.recv_timeout(INITIALIZE_TIMEOUT) {
            Ok(Ok(())) => {}
            Ok(Err(message)) => {
                let _ = child.kill();
                return Err(anyhow::anyhow!(
                    "language server initialize failed: {message}"
                ));
            }
            Err(_) => {
                let _ = child.kill();
                return Err(anyhow::anyhow!(
                    "language server did not respond to initialize within 5 seconds"
                ));
            }
        }
        if let Err(error) = write_message(
            &mut stdin,
            &json!({"jsonrpc":"2.0","method":"initialized","params":{}}),
        ) {
            let _ = child.kill();
            return Err(error);
        }

        self.clients.insert(
            key.to_string(),
            LspClient {
                child,
                stdin,
                next_request_id: 2,
                document_versions: HashMap::new(),
                diagnostics_rx,
                responses_rx,
                last_used: Instant::now(),
            },
        );
        Ok(())
    }

    pub fn open_document(
        &mut self,
        key: &str,
        language_id: &str,
        path: &Path,
        text: &str,
    ) -> Result<()> {
        let client = self
            .clients
            .get_mut(key)
            .context("language server is not running")?;
        let uri = file_uri(path)?;
        client.last_used = Instant::now();
        if let Some(version) = client.document_versions.get_mut(path) {
            *version += 1;
            return write_message(
                &mut client.stdin,
                &json!({
                    "jsonrpc":"2.0",
                    "method":"textDocument/didChange",
                    "params":{
                        "textDocument":{"uri":uri,"version":*version},
                        "contentChanges":[{"text":text}]
                    }
                }),
            );
        }
        client.document_versions.insert(path.to_path_buf(), 1);
        write_message(
            &mut client.stdin,
            &json!({
                "jsonrpc":"2.0",
                "method":"textDocument/didOpen",
                "params":{"textDocument":{
                    "uri":uri,
                    "languageId":language_id,
                    "version":1,
                    "text":text
                }}
            }),
        )
    }

    pub fn change_document(&mut self, key: &str, path: &Path, text: &str) -> Result<()> {
        let client = self
            .clients
            .get_mut(key)
            .context("language server is not running")?;
        let uri = file_uri(path)?;
        client.last_used = Instant::now();
        let version = client
            .document_versions
            .entry(path.to_path_buf())
            .or_insert(1);
        *version += 1;
        write_message(
            &mut client.stdin,
            &json!({
                "jsonrpc":"2.0",
                "method":"textDocument/didChange",
                "params":{
                    "textDocument":{"uri":uri,"version":*version},
                    "contentChanges":[{"text":text}]
                }
            }),
        )
    }

    pub fn close_document(&mut self, key: &str, path: &Path) -> Result<()> {
        let client = self
            .clients
            .get_mut(key)
            .context("language server is not running")?;
        if client.document_versions.remove(path).is_none() {
            return Ok(());
        }
        client.last_used = Instant::now();
        write_message(
            &mut client.stdin,
            &json!({
                "jsonrpc":"2.0",
                "method":"textDocument/didClose",
                "params":{"textDocument":{"uri":file_uri(path)?}}
            }),
        )
    }

    pub fn completions(
        &mut self,
        key: &str,
        path: &Path,
        line: u32,
        character: u32,
    ) -> Result<Vec<LspCompletion>> {
        let uri = file_uri(path)?;
        let result = self.request(
            key,
            "textDocument/completion",
            json!({"textDocument":{"uri":uri},"position":{"line":line,"character":character}}),
        )?;
        let items = result
            .get("items")
            .and_then(Value::as_array)
            .or_else(|| result.as_array())
            .context("language server returned invalid completion results")?;
        Ok(items
            .iter()
            .filter_map(|item| {
                let label = item.get("label")?.as_str()?.to_string();
                let insert_text = item
                    .get("textEdit")
                    .and_then(|edit| edit.get("newText"))
                    .and_then(Value::as_str)
                    .or_else(|| item.get("insertText").and_then(Value::as_str))
                    .unwrap_or(&label)
                    .to_string();
                Some(LspCompletion { label, insert_text })
            })
            .collect())
    }

    pub fn definition(
        &mut self,
        key: &str,
        path: &Path,
        line: u32,
        character: u32,
    ) -> Result<Vec<LspLocation>> {
        let uri = file_uri(path)?;
        let result = self.request(
            key,
            "textDocument/definition",
            json!({"textDocument":{"uri":uri},"position":{"line":line,"character":character}}),
        )?;
        let locations = result.as_array().cloned().unwrap_or_else(|| vec![result]);
        Ok(locations
            .iter()
            .filter_map(|location| {
                let uri = location
                    .get("uri")
                    .or_else(|| location.get("targetUri"))?
                    .as_str()?
                    .to_string();
                let start = location
                    .pointer("/range/start")
                    .or_else(|| location.pointer("/targetSelectionRange/start"))?;
                Some(LspLocation {
                    uri,
                    line: start.get("line")?.as_u64()? as u32,
                    character: start.get("character")?.as_u64()? as u32,
                })
            })
            .collect())
    }

    fn request(&mut self, key: &str, method: &str, params: Value) -> Result<Value> {
        let client = self
            .clients
            .get_mut(key)
            .context("language server is not running")?;
        let id = client.next_request_id;
        client.next_request_id = client.next_request_id.wrapping_add(1);
        client.last_used = Instant::now();
        write_message(
            &mut client.stdin,
            &json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}),
        )?;
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                anyhow::bail!("language server request `{method}` timed out");
            }
            match client.responses_rx.recv_timeout(remaining) {
                Ok((response_id, response)) if response_id == id => {
                    return response.map_err(|message| anyhow::anyhow!(message));
                }
                Ok(_) => continue,
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    anyhow::bail!("language server request `{method}` timed out");
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    anyhow::bail!("language server response channel closed");
                }
            }
        }
    }

    pub fn poll_diagnostics(&mut self) -> Vec<String> {
        for client in self.clients.values() {
            while let Ok((uri, diagnostics)) = client.diagnostics_rx.try_recv() {
                self.diagnostics.insert(uri, diagnostics);
            }
        }
        let stopped: Vec<_> = self
            .clients
            .iter_mut()
            .filter_map(|(key, client)| match client.child.try_wait() {
                Ok(Some(status)) => Some((
                    key.clone(),
                    format!("Language server `{key}` exited with status {status}"),
                )),
                Err(error) => Some((
                    key.clone(),
                    format!("Could not monitor language server `{key}`: {error}"),
                )),
                Ok(None) => None,
            })
            .collect();
        for (key, _) in &stopped {
            if let Some(mut client) = self.clients.remove(key) {
                stop_client(&mut client);
            }
        }
        stopped.into_iter().map(|(_, message)| message).collect()
    }

    pub fn diagnostics_for(&self, path: &Path) -> Vec<LspDiagnostic> {
        let Ok(uri) = file_uri(path) else {
            return Vec::new();
        };
        self.diagnostics
            .get(&uri)
            .into_iter()
            .flatten()
            .filter(|diagnostic| diagnostic.uri == uri)
            .cloned()
            .collect()
    }

    pub fn count(&self) -> usize {
        self.clients.len()
    }

    pub fn touch(&mut self, key: &str) -> bool {
        self.clients.get_mut(key).is_some_and(|client| {
            if client.child.try_wait().ok().flatten().is_none() {
                client.last_used = Instant::now();
                true
            } else {
                false
            }
        })
    }

    pub fn shutdown_idle(&mut self, idle_for: Duration, active_keys: &[String]) -> Vec<String> {
        let now = Instant::now();
        let idle: Vec<_> = self
            .clients
            .iter()
            .filter(|(key, client)| {
                !active_keys.iter().any(|active| active == *key)
                    && now.duration_since(client.last_used) >= idle_for
            })
            .map(|(key, _)| key.clone())
            .collect();
        for key in &idle {
            if let Some(mut client) = self.clients.remove(key) {
                stop_client(&mut client);
            }
        }
        idle
    }

    pub fn shutdown_all(&mut self) {
        for mut client in std::mem::take(&mut self.clients).into_values() {
            stop_client(&mut client);
        }
    }
}

impl Drop for LspPool {
    fn drop(&mut self) {
        for client in self.clients.values_mut() {
            stop_client(client);
        }
    }
}

fn stop_client(client: &mut LspClient) {
    let _ = write_message(
        &mut client.stdin,
        &json!({"jsonrpc":"2.0","id":client.next_request_id,"method":"shutdown","params":null}),
    );
    let _ = write_message(
        &mut client.stdin,
        &json!({"jsonrpc":"2.0","method":"exit","params":null}),
    );
    let deadline = Instant::now() + Duration::from_millis(150);
    while Instant::now() < deadline {
        if client.child.try_wait().ok().flatten().is_some() {
            return;
        }
        thread::sleep(Duration::from_millis(10));
    }
    let _ = client.child.kill();
    let _ = client.child.wait();
}

fn file_uri(path: &Path) -> Result<String> {
    Url::from_file_path(path)
        .map(|url| url.to_string())
        .map_err(|_| anyhow::anyhow!("cannot convert path to file URI: {}", path.display()))
}

pub fn path_from_uri(uri: &str) -> Result<PathBuf> {
    Url::parse(uri)
        .context("invalid language server file URI")?
        .to_file_path()
        .map_err(|_| anyhow::anyhow!("language server returned a non-file URI: {uri}"))
}

fn write_message(writer: &mut impl Write, message: &Value) -> Result<()> {
    let body = serde_json::to_vec(message)?;
    write!(writer, "Content-Length: {}\r\n\r\n", body.len())?;
    writer.write_all(&body)?;
    writer.flush()?;
    Ok(())
}

fn read_messages(
    stdout: impl Read,
    diagnostics_tx: Sender<(String, Vec<LspDiagnostic>)>,
    initialized_tx: Sender<Result<(), String>>,
    responses_tx: Sender<(u64, Result<Value, String>)>,
) {
    let mut reader = BufReader::new(stdout);
    loop {
        let mut content_length = None;
        loop {
            let mut line = String::new();
            match reader.read_line(&mut line) {
                Ok(0) | Err(_) => return,
                Ok(_) if line == "\r\n" || line == "\n" => break,
                Ok(_) => {
                    if let Some((header, value)) = line.trim().split_once(':') {
                        if header.eq_ignore_ascii_case("content-length") {
                            content_length = value.trim().parse::<usize>().ok();
                        }
                    }
                }
            }
        }
        let Some(length) = content_length else {
            continue;
        };
        let mut body = vec![0; length];
        if reader.read_exact(&mut body).is_err() {
            return;
        }
        let Ok(message) = serde_json::from_slice::<Value>(&body) else {
            continue;
        };

        if message.get("id").and_then(Value::as_u64) == Some(1) {
            let response = if let Some(error) = message.get("error") {
                Err(error
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown initialize error")
                    .to_string())
            } else {
                Ok(())
            };
            let _ = initialized_tx.send(response);
        } else if let Some(id) = message.get("id").and_then(Value::as_u64) {
            let response = if let Some(error) = message.get("error") {
                Err(error
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown language server error")
                    .to_string())
            } else {
                Ok(message.get("result").cloned().unwrap_or(Value::Null))
            };
            let _ = responses_tx.send((id, response));
        }
        if message.get("method").and_then(Value::as_str) == Some("textDocument/publishDiagnostics")
        {
            if let Some(params) = message.get("params") {
                let uri = params
                    .get("uri")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                let diagnostics = params
                    .get("diagnostics")
                    .and_then(Value::as_array)
                    .map(|diagnostics| {
                        diagnostics
                            .iter()
                            .map(|diagnostic| LspDiagnostic {
                                uri: uri.clone(),
                                line: diagnostic
                                    .pointer("/range/start/line")
                                    .and_then(Value::as_u64)
                                    .unwrap_or(0) as u32
                                    + 1,
                                message: diagnostic
                                    .get("message")
                                    .and_then(Value::as_str)
                                    .unwrap_or_default()
                                    .to_string(),
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                let _ = diagnostics_tx.send((uri, diagnostics));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{file_uri, read_messages, LspPool};
    use serde_json::json;
    use std::io::Cursor;
    use std::path::Path;
    use std::sync::mpsc;

    fn frame(message: serde_json::Value, output: &mut Vec<u8>) {
        let body = serde_json::to_vec(&message).unwrap();
        output.extend_from_slice(format!("Content-Length: {}\r\n\r\n", body.len()).as_bytes());
        output.extend_from_slice(&body);
    }

    #[test]
    fn file_paths_are_encoded_as_file_uris() {
        assert!(file_uri(Path::new("/workspace/a b.rs"))
            .unwrap()
            .contains("a%20b.rs"));
    }

    #[test]
    fn pool_starts_empty() {
        assert_eq!(LspPool::new().count(), 0);
    }

    #[test]
    fn receives_initialize_response_and_replaces_diagnostics() {
        let mut input = Vec::new();
        frame(json!({"jsonrpc":"2.0","id":1,"result":{}}), &mut input);
        frame(
            json!({
                "jsonrpc":"2.0",
                "method":"textDocument/publishDiagnostics",
                "params":{
                    "uri":"file:///workspace/main.rs",
                    "diagnostics":[{
                        "range":{"start":{"line":2}},
                        "message":"first diagnostic"
                    }]
                }
            }),
            &mut input,
        );
        frame(
            json!({
                "jsonrpc":"2.0",
                "id":2,
                "result":{"items":[{"label":"println!","insertText":"println!"}]}
            }),
            &mut input,
        );
        frame(
            json!({
                "jsonrpc":"2.0",
                "method":"textDocument/publishDiagnostics",
                "params":{
                    "uri":"file:///workspace/main.rs",
                    "diagnostics":[]
                }
            }),
            &mut input,
        );

        let (diagnostics_tx, diagnostics_rx) = mpsc::channel();
        let (initialized_tx, initialized_rx) = mpsc::channel();
        let (responses_tx, responses_rx) = mpsc::channel();
        read_messages(
            Cursor::new(input),
            diagnostics_tx,
            initialized_tx,
            responses_tx,
        );

        assert_eq!(initialized_rx.try_recv().unwrap(), Ok(()));
        let (_, diagnostics) = diagnostics_rx.try_iter().last().unwrap();
        assert!(diagnostics.is_empty());
        let (id, response) = responses_rx.try_recv().unwrap();
        assert_eq!(id, 2);
        assert_eq!(response.unwrap()["items"][0]["label"], "println!");
    }
}
