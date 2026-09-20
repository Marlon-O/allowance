use crate::{model::*, process};
use serde_json::{json, Value};
use std::process::Stdio;
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin},
    sync::mpsc,
    time::{timeout, Duration},
};

pub struct Codex {
    child: Child,
    input: ChildStdin,
    lines: mpsc::Receiver<Value>,
    id: u64,
    pub dirty: bool,
}
impl Drop for Codex {
    fn drop(&mut self) {
        let _ = self.child.start_kill();
    }
}
impl Codex {
    pub async fn start(data_dir: &std::path::Path) -> Result<Self, String> {
        let runtime_dir = data_dir.join("codex-runtime");
        std::fs::create_dir_all(&runtime_dir).map_err(|e| e.to_string())?;
        let path = process::executable("codex").ok_or("missing_cli")?;
        let mut base = process::command(&path);
        base.args(["app-server", "--listen", "stdio://"]);
        base.arg("-c").arg(format!(
            "sqlite_home={}",
            serde_json::to_string(&runtime_dir.to_string_lossy()).unwrap()
        ));
        base.arg("-c").arg("analytics.enabled=false");
        // App server owns authentication; this process never reads auth files or tokens.
        let mut cmd = tokio::process::Command::from(base);
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        let mut child = cmd
            .spawn()
            .map_err(|_| "Could not start Codex app server")?;
        let input = child.stdin.take().ok_or("Codex input unavailable")?;
        let stdout = child.stdout.take().ok_or("Codex output unavailable")?;
        let (tx, rx) = mpsc::channel(128);
        tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if line.len() > 2 * 1024 * 1024 {
                    break;
                }
                if let Ok(v) = serde_json::from_str(&line) {
                    if tx.send(v).await.is_err() {
                        break;
                    }
                }
            }
        });
        let mut rpc = Self {
            child,
            input,
            lines: rx,
            id: 0,
            dirty: false,
        };
        rpc.request("initialize",json!({"clientInfo":{"name":"allowance_widget","title":"Allowance","version":env!("CARGO_PKG_VERSION")},"capabilities":{"experimentalApi":false}})).await?;
        rpc.write(json!({"method":"initialized"})).await?;
        Ok(rpc)
    }
    async fn write(&mut self, v: Value) -> Result<(), String> {
        self.input
            .write_all(format!("{v}\n").as_bytes())
            .await
            .map_err(|_| "Codex connection closed".to_string())
    }
    fn notification(&mut self, v: &Value) {
        if matches!(
            v["method"].as_str(),
            Some("account/rateLimits/updated" | "account/updated")
        ) {
            self.dirty = true;
        }
    }
    pub fn take_dirty(&mut self) -> bool {
        while let Ok(v) = self.lines.try_recv() {
            self.notification(&v)
        }
        std::mem::take(&mut self.dirty)
    }
    async fn request(&mut self, method: &str, params: Value) -> Result<Value, String> {
        self.id += 1;
        let id = self.id;
        self.write(json!({"id":id,"method":method,"params":params}))
            .await?;
        timeout(Duration::from_secs(15),async {
            loop {let v=self.lines.recv().await.ok_or("Codex connection closed")?;
                if v["id"].as_u64()==Some(id){if v.get("error").is_some(){return Err(format!("Codex could not complete {method}. Check your login and CLI version."))}return Ok(v["result"].clone())}
                // Reject requests we cannot handle, without logging potentially sensitive payloads.
                if v.get("id").is_some() && v.get("method").is_some(){let _=self.write(json!({"id":v["id"],"error":{"code":-32601,"message":"Read-only usage client"}})).await;}
                self.notification(&v);
            }
        }).await.map_err(|_|"Codex usage request timed out".to_string())?
    }
    pub async fn read(&mut self) -> Result<Snapshot, String> {
        let account = self
            .request("account/read", json!({"refreshToken":false}))
            .await?;
        let a = &account["account"];
        if a.is_null() {
            return Ok(Snapshot::empty(
                "codex",
                "signed_out",
                "Sign in through Codex, then refresh.",
            ));
        }
        let kind = a["type"].as_str().unwrap_or("");
        if kind == "apiKey" || kind == "apikey" {
            return Ok(Snapshot::empty(
                "codex",
                "unsupported",
                "API billing is not a subscription allowance. Sign in to Codex with ChatGPT.",
            ));
        }
        let identity = account_identity(a)
            .ok_or("Codex did not return an account identity. Update the CLI and sign in again.")?;
        let limits = self.request("account/rateLimits/read", json!({})).await?;
        let after = self
            .request("account/read", json!({"refreshToken":false}))
            .await?;
        if account_identity(&after["account"]) != Some(identity.clone()) {
            return Ok(Snapshot::empty(
                "codex",
                "waiting",
                "Account changed. Waiting for a fresh reading.",
            ));
        }
        let windows = codex_windows(&limits);
        Ok(Snapshot {
            provider: "codex".into(),
            state: if windows.is_empty() {
                "unavailable"
            } else {
                "ready"
            }
            .into(),
            account_id: Some(identity),
            account_label: a["email"].as_str().map(str::to_owned),
            windows,
            observed_at: Some(now()),
            source: "Codex app server".into(),
            message: None,
        })
    }
}
fn account_identity(v: &Value) -> Option<String> {
    let email = v["email"].as_str();
    let id = v["accountId"].as_str().or(v["id"].as_str());
    if email.is_none() && id.is_none() {
        return None;
    }
    Some(process::hash(&format!(
        "{}|{}",
        id.unwrap_or(""),
        email.unwrap_or("")
    )))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn identity_accounts_differ() {
        assert_ne!(
            account_identity(&json!({"email":"a","accountId":"1"})),
            account_identity(&json!({"email":"a","accountId":"2"}))
        );
        assert!(account_identity(&json!({"type":"chatgpt"})).is_none());
    }
}

#[cfg(test)]
mod live_tests {
    use super::*;
    #[tokio::test]
    #[ignore = "Uses the installed CLI and signed-in account; run explicitly on your computer"]
    async fn live_codex_read_only() {
        let dir = tempfile::tempdir().unwrap();
        let mut rpc = Codex::start(dir.path())
            .await
            .expect("start installed Codex");
        let s = rpc.read().await.expect("read usage");
        println!("state={}, windows={}", s.state, s.windows.len());
        for w in &s.windows {
            println!(
                "{}: used={:?}, reset={:?}",
                w.id, w.used_percent, w.resets_at
            );
        }
        assert_eq!(s.state, "ready");
        assert!(!s.windows.is_empty());
    }
}
