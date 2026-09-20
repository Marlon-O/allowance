use crate::{model::*, process, store::Store};
use chrono::DateTime;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};

#[derive(Clone, Serialize, Deserialize)]
struct Bridge {
    original: Option<Value>,
    installed: Value,
    settings_path: PathBuf,
    helper_path: PathBuf,
    #[serde(default)]
    generation: String,
}
pub fn config_dir() -> Result<PathBuf, String> {
    std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|p| p.join(".claude")))
        .ok_or("Home directory unavailable".into())
}
fn desktop_usage_path() -> Option<PathBuf> {
    dirs::config_dir().map(|p| p.join("Claude").join("plan-usage-history.json"))
}

fn desktop_cache_path() -> Option<PathBuf> {
    dirs::config_dir().map(|p| p.join("Claude").join("Cache").join("Cache_Data"))
}

#[derive(Debug)]
struct CachedUsageWindow {
    used_percent: f64,
    resets_at: Option<i64>,
}

fn parse_reset(value: &Value) -> Option<i64> {
    value
        .as_str()
        .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
        .map(|value| value.timestamp())
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn parse_json_prefix(bytes: &[u8]) -> Option<Value> {
    serde_json::Deserializer::from_slice(bytes)
        .into_iter::<Value>()
        .next()?
        .ok()
}

fn desktop_cached_usage_at(
    cache_dir: &Path,
    organization: &str,
) -> Option<HashMap<String, CachedUsageWindow>> {
    let endpoint = format!("/api/organizations/{organization}/usage");
    let mut files: Vec<_> = std::fs::read_dir(cache_dir)
        .ok()?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let metadata = entry.metadata().ok()?;
            (metadata.is_file() && metadata.len() <= 2 * 1024 * 1024)
                .then_some((metadata.modified().ok(), entry.path()))
        })
        .collect();
    files.sort_by_key(|(modified, _)| std::cmp::Reverse(*modified));

    for (_, path) in files {
        let Ok(bytes) = std::fs::read(path) else {
            continue;
        };
        if find_bytes(&bytes, endpoint.as_bytes()).is_none() {
            continue;
        }
        let json = if let Some(start) = find_bytes(&bytes, b"{\"five_hour\"") {
            parse_json_prefix(&bytes[start..])
        } else {
            find_bytes(&bytes, &[0x28, 0xb5, 0x2f, 0xfd])
                .and_then(|start| {
                    let frame = &bytes[start..];
                    let size = zstd::zstd_safe::find_frame_compressed_size(frame).ok()?;
                    zstd::stream::decode_all(&frame[..size]).ok()
                })
                .and_then(|decoded| parse_json_prefix(&decoded))
        };
        let Some(json) = json else { continue };
        let windows = [("five_hour", "five_hour"), ("seven_day", "seven_day")]
            .into_iter()
            .filter_map(|(id, key)| {
                let window = json.get(key)?;
                Some((
                    id.into(),
                    CachedUsageWindow {
                        used_percent: percentage(window.get("utilization")?)?,
                        resets_at: window.get("resets_at").and_then(parse_reset),
                    },
                ))
            })
            .collect::<HashMap<_, _>>();
        if !windows.is_empty() {
            return Some(windows);
        }
    }
    None
}

fn desktop_usage_at(path: &Path) -> Option<Snapshot> {
    let value: Value = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
    let sample = value["samples"].as_array()?.iter().rev().find(|sample| {
        sample["t"].as_i64().is_some()
            && sample["org"].as_str().is_some()
            && sample["u"].is_object()
    })?;
    let observed_at = sample["t"].as_i64()? / 1000;
    if observed_at <= 0 || observed_at > now() + 60 {
        return None;
    }
    let usage = &sample["u"];
    let organization = sample["org"].as_str()?;
    let cached = desktop_cache_path()
        .and_then(|path| desktop_cached_usage_at(&path, organization))
        .unwrap_or_default();
    let windows: Vec<UsageWindow> = [
        ("five_hour", "5-hour window", "fh", 300),
        ("seven_day", "Weekly window", "sd", 10080),
    ]
    .into_iter()
    .filter_map(|(id, label, key, duration_mins)| {
        let used_percent = percentage(usage.get(key)?);
        let resets_at = cached.get(id).and_then(|cached| {
            used_percent
                .is_some_and(|used| (used - cached.used_percent).abs() < 0.01)
                .then_some(cached.resets_at)
                .flatten()
        });
        Some(UsageWindow {
            id: id.into(),
            label: label.into(),
            used_percent,
            resets_at,
            duration_mins: Some(duration_mins),
        })
    })
    .filter(|window| window.used_percent.is_some())
    .collect();
    if windows.is_empty() {
        return None;
    }
    Some(Snapshot {
        provider: "claude".into(),
        state: "ready".into(),
        account_id: Some(process::hash(&format!("claude-desktop|{organization}"))),
        account_label: None,
        windows,
        observed_at: Some(observed_at),
        source: "Claude Desktop usage cache".into(),
        message: None,
    })
}

fn desktop_usage() -> Option<Snapshot> {
    desktop_usage_path().and_then(|path| desktop_usage_at(&path))
}
fn read_config(p: &Path) -> Result<Value, String> {
    if !p.exists() {
        return Ok(json!({}));
    }
    let v: Value = serde_json::from_slice(&std::fs::read(p).map_err(|e| e.to_string())?)
        .map_err(|_| "Claude settings contain invalid JSON; nothing was changed.")?;
    if !v.is_object() {
        return Err("Claude settings must be a JSON object".into());
    }
    Ok(v)
}
fn atomic_json(p: &Path, v: &Value) -> Result<(), String> {
    let tmp = p.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)
            .map_err(|e| e.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            f.set_permissions(std::fs::Permissions::from_mode(0o600))
                .map_err(|e| e.to_string())?;
        }
        f.write_all(serde_json::to_string_pretty(v).unwrap().as_bytes())
            .map_err(|e| e.to_string())?;
        f.sync_all().map_err(|e| e.to_string())?;
        #[cfg(windows)]
        {
            if p.exists() {
                // ReplaceFileW preserves an existing settings file atomically.
                use std::os::windows::ffi::OsStrExt;
                #[link(name = "Kernel32")]
                extern "system" {
                    fn ReplaceFileW(
                        replaced: *const u16,
                        replacement: *const u16,
                        backup: *const u16,
                        flags: u32,
                        exclude: *mut std::ffi::c_void,
                        reserved: *mut std::ffi::c_void,
                    ) -> i32;
                }
                let a: Vec<u16> = p.as_os_str().encode_wide().chain(Some(0)).collect();
                let b: Vec<u16> = tmp.as_os_str().encode_wide().chain(Some(0)).collect();
                if unsafe {
                    ReplaceFileW(
                        a.as_ptr(),
                        b.as_ptr(),
                        std::ptr::null(),
                        0,
                        std::ptr::null_mut(),
                        std::ptr::null_mut(),
                    )
                } == 0
                {
                    return Err(std::io::Error::last_os_error().to_string());
                }
                return Ok(());
            }
        }
        std::fs::rename(&tmp, p).map_err(|e| e.to_string())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(tmp);
    }
    result
}
fn bridge(db: &Store) -> Option<Bridge> {
    db.conn
        .query_row("SELECT json FROM bridge WHERE id=1", [], |r| {
            r.get::<_, String>(0)
        })
        .ok()
        .and_then(|v| serde_json::from_str(&v).ok())
}
pub fn maintain_bridge(db: &Store) -> Result<(), String> {
    maintain_bridge_at(db, &std::env::current_exe().map_err(|e| e.to_string())?)
}
fn maintain_bridge_at(db: &Store, exe: &Path) -> Result<(), String> {
    let Some(mut b) = bridge(db) else {
        return Ok(());
    };
    let mut settings = read_config(&b.settings_path)?;
    if settings.get("statusLine") != Some(&b.installed) {
        return Ok(());
    }
    let dir = b.helper_path.parent().ok_or("Unsupported helper path")?;
    let helper = install_helper(exe, dir)?;
    let command = installed_command(&helper, dir)?;
    if b.installed.get("refreshInterval").and_then(Value::as_i64) == Some(5)
        && b.installed.get("command").and_then(Value::as_str) == Some(command.as_str())
        && b.helper_path == helper
    {
        return Ok(());
    }
    let previous = b.clone();
    b.helper_path = helper.clone();
    b.generation = uuid::Uuid::new_v4().to_string();
    b.installed["command"] = json!(command);
    b.installed["refreshInterval"] = json!(5);
    settings["statusLine"] = b.installed.clone();
    db.conn
        .execute(
            "INSERT OR REPLACE INTO bridge VALUES(1,?)",
            [serde_json::to_string(&b).unwrap()],
        )
        .map_err(|e| e.to_string())?;
    if let Err(error) = atomic_json(&b.settings_path, &settings) {
        let _ = db.conn.execute(
            "INSERT OR REPLACE INTO bridge VALUES(1,?)",
            [serde_json::to_string(&previous).unwrap()],
        );
        if helper != previous.helper_path {
            let _ = std::fs::remove_file(helper);
        }
        return Err(error);
    }
    if previous.helper_path != b.helper_path {
        let _ = std::fs::remove_file(previous.helper_path);
    }
    Ok(())
}
pub fn connected(db: &Store) -> bool {
    bridge(db).is_some_and(|b| {
        read_config(&b.settings_path)
            .ok()
            .is_some_and(|v| v.get("statusLine") == Some(&b.installed))
    })
}
fn quote_unix(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}
fn installed_command(exe: &Path, dir: &Path) -> Result<String, String> {
    let exe = exe.to_str().ok_or("Unsupported helper path")?;
    let dir = dir.to_str().ok_or("Unsupported data path")?;
    if cfg!(windows) {
        // A PowerShell EncodedCommand works whether Claude invokes Git Bash or PowerShell.
        // It receives original stdin unchanged and uses literal single-quoted paths.
        let args = format!("--claude-statusline \"{}\"", dir.replace('\\', "/"));
        let script = format!(
            "$p=New-Object System.Diagnostics.Process; $p.StartInfo.FileName='{}'; $p.StartInfo.Arguments='{}'; $p.StartInfo.UseShellExecute=$false; $p.StartInfo.RedirectStandardInput=$true; $null=$p.Start(); $p.StandardInput.Write([Console]::In.ReadToEnd()); $p.StandardInput.Close(); $p.WaitForExit(); exit $p.ExitCode",
            exe.replace('\'', "''"), args.replace('\'', "''")
        );
        let bytes: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
        Ok(format!(
            "powershell.exe -NoProfile -NonInteractive -EncodedCommand {}",
            base64(&bytes)
        ))
    } else {
        Ok(format!(
            "{} --claude-statusline {}",
            quote_unix(exe),
            quote_unix(dir)
        ))
    }
}
fn file_digest(path: &Path) -> Result<String, String> {
    let mut file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(format!("{:x}", digest.finalize()))
}
fn install_helper(exe: &Path, dir: &Path) -> Result<PathBuf, String> {
    let digest = file_digest(exe)?;
    let suffix = if cfg!(windows) { ".exe" } else { "" };
    let mut helper = dir.join(format!("allowance-helper-{}{suffix}", &digest[..12]));
    if helper.exists() {
        if file_digest(&helper).ok().as_deref() == Some(digest.as_str()) {
            return Ok(helper);
        }
        helper = dir.join(format!(
            "allowance-helper-{}-{}{suffix}",
            &digest[..12],
            uuid::Uuid::new_v4()
        ));
    }
    std::fs::copy(exe, &helper).map_err(|e| format!("Could not install helper: {e}"))?;
    Ok(helper)
}
fn base64(bytes: &[u8]) -> String {
    let chars = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::new();
    for c in bytes.chunks(3) {
        let n = ((c[0] as u32) << 16)
            | ((c.get(1).copied().unwrap_or(0) as u32) << 8)
            | (c.get(2).copied().unwrap_or(0) as u32);
        s.push(chars[((n >> 18) & 63) as usize] as char);
        s.push(chars[((n >> 12) & 63) as usize] as char);
        s.push(if c.len() > 1 {
            chars[((n >> 6) & 63) as usize] as char
        } else {
            '='
        });
        s.push(if c.len() > 2 {
            chars[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    s
}
pub fn connect(dir: &Path, db: &Store) -> Result<(), String> {
    let config = config_dir()?;
    std::fs::create_dir_all(&config).map_err(|e| e.to_string())?;
    connect_at(
        dir,
        db,
        &config.join("settings.json"),
        &std::env::current_exe().map_err(|e| e.to_string())?,
    )
}
fn connect_at(dir: &Path, db: &Store, path: &Path, exe: &Path) -> Result<(), String> {
    if connected(db) {
        return Ok(());
    }
    if bridge(db).is_some() {
        return Err("Claude status line changed outside Allowance. Disconnect first to preserve that change, then connect again.".into());
    }
    let mut settings = read_config(path)?;
    if settings["disableAllHooks"] == true {
        return Err(
            "Claude hooks are disabled. Enable them in Claude settings before connecting.".into(),
        );
    }
    let helper = install_helper(exe, dir)?;
    let original = settings.get("statusLine").cloned();
    let mut installed = original
        .clone()
        .filter(Value::is_object)
        .unwrap_or_else(|| json!({}));
    installed["type"] = json!("command");
    installed["command"] = json!(installed_command(&helper, dir)?);
    // Desktop sessions can have fewer UI-driven status-line updates than the CLI.
    // A short supported refresh interval keeps local Desktop readings current.
    installed["refreshInterval"] = json!(5);
    let b = Bridge {
        original,
        installed: installed.clone(),
        settings_path: path.into(),
        helper_path: helper.clone(),
        generation: uuid::Uuid::new_v4().to_string(),
    };
    // Back up only the setting being changed, not unrelated settings or credentials.
    db.conn
        .execute(
            "INSERT OR REPLACE INTO bridge VALUES(1,?)",
            [serde_json::to_string(&b).unwrap()],
        )
        .map_err(|e| e.to_string())?;
    settings["statusLine"] = installed;
    if let Err(e) = atomic_json(path, &settings) {
        let _ = db.conn.execute("DELETE FROM bridge", []);
        let _ = std::fs::remove_file(helper);
        return Err(e);
    }
    Ok(())
}
pub fn disconnect(db: &Store) -> Result<String, String> {
    let Some(b) = bridge(db) else {
        return Ok("Claude Code is disconnected.".into());
    };
    let mut settings = read_config(&b.settings_path)?;
    let owned = settings.get("statusLine") == Some(&b.installed);
    if owned {
        if let Some(original) = b.original {
            settings["statusLine"] = original
        } else {
            settings.as_object_mut().unwrap().remove("statusLine");
        }
        atomic_json(&b.settings_path, &settings)?;
    }
    db.conn
        .execute_batch("DELETE FROM bridge; DELETE FROM sessions;")
        .map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(b.helper_path);
    Ok(if owned {
        "Claude status line restored."
    } else {
        "Your newer Claude status line was preserved."
    }
    .into())
}
pub fn auth() -> Result<(String, Option<String>), String> {
    let exe = process::executable("claude").ok_or("missing_cli")?;
    let mut cmd = process::command(&exe);
    cmd.args(["auth", "status", "--json"]);
    let bytes = process::output(cmd, Duration::from_secs(4))?;
    let v: Value = serde_json::from_slice(&bytes)
        .map_err(|_| "Claude returned unreadable authentication status")?;
    if v["loggedIn"] != true {
        return Err("signed_out".into());
    }
    if v["authMethod"].as_str().is_some_and(|m| m != "claude.ai") {
        return Err("unsupported".into());
    }
    let email = v["email"].as_str();
    let id = v["accountId"].as_str().or(v["orgId"].as_str());
    if email.is_none() && id.is_none() {
        return Err("Claude did not return an account identity".into());
    }
    Ok((
        process::hash(&format!("{}|{}", id.unwrap_or(""), email.unwrap_or(""))),
        email.map(str::to_owned),
    ))
}
pub fn read(db: &Store) -> Snapshot {
    // Claude Desktop maintains this local, credential-free cache using its own
    // usage probe. Prefer it because Desktop does not consistently render a
    // Claude Code status line. A newer CLI observation still wins.
    let desktop = desktop_usage();
    let status_line = read_status_line(db, desktop.as_ref());
    match desktop {
        Some(mut snapshot)
            if snapshot.observed_at.unwrap_or_default()
                >= status_line.observed_at.unwrap_or_default() =>
        {
            merge_reset_times(&mut snapshot, &status_line);
            snapshot
        }
        _ => status_line,
    }
}

fn desktop_identity(snapshot: Option<&Snapshot>) -> Option<String> {
    snapshot.and_then(|snapshot| snapshot.account_id.clone())
}

fn claude_identity(
    desktop: Option<&Snapshot>,
    authenticated: Result<(String, Option<String>), String>,
) -> Result<(String, Option<String>), String> {
    match (desktop_identity(desktop), authenticated) {
        (Some(account), authenticated) => {
            Ok((account, authenticated.ok().and_then(|(_, label)| label)))
        }
        (None, authenticated) => authenticated,
    }
}

fn merge_reset_times(desktop: &mut Snapshot, status_line: &Snapshot) {
    if desktop.account_id != status_line.account_id {
        return;
    }
    let observed_at = desktop.observed_at.unwrap_or_default();
    for window in &mut desktop.windows {
        let status_line_reset = status_line
            .windows
            .iter()
            .find(|candidate| candidate.id == window.id)
            .and_then(|candidate| candidate.resets_at)
            .filter(|reset| *reset > observed_at);
        if status_line_reset.is_some() {
            window.resets_at = status_line_reset;
        }
    }
    if desktop.account_label.is_none() {
        desktop.account_label = status_line.account_label.clone();
    }
}

fn read_status_line(db: &Store, desktop: Option<&Snapshot>) -> Snapshot {
    if !connected(db) {
        return Snapshot::empty(
            "claude",
            "disconnected",
            "Connect Claude Code to receive usage readings.",
        );
    }
    let (account, label) = match claude_identity(desktop, auth()) {
        Ok(identity) => identity,
        Err(e) => {
            if e == "signed_out" {
                let _ = sync_identity(db, "signed-out");
            }
            return Snapshot::empty(
                "claude",
                match e.as_str() {
                    "missing_cli" => "missing_cli",
                    "signed_out" => "signed_out",
                    "unsupported" => "unsupported",
                    _ => "error",
                },
                match e.as_str() {
                    "missing_cli" => "Install Claude Code, then refresh.",
                    "signed_out" => "Sign in through Claude Code, then refresh.",
                    "unsupported" => "Connect Claude Code with your Claude subscription.",
                    _ => "Could not read Claude account status.",
                },
            );
        }
    };
    if sync_identity(db, &account).is_err() {
        return Snapshot::empty("claude", "error", "Could not scope account readings.");
    }
    let saved: Option<String> = db
        .conn
        .query_row(
            "SELECT snapshot FROM sessions WHERE account=? AND revoked=0 ORDER BY at DESC LIMIT 1",
            [&account],
            |r| r.get(0),
        )
        .optional()
        .ok()
        .flatten();
    if let Some(s) = saved.and_then(|s| serde_json::from_str::<Snapshot>(&s).ok()) {
        return s;
    }
    let mut s=Snapshot::empty("claude","waiting","Waiting for Claude Code activity. Restart existing sessions after connecting or switching accounts.");
    s.account_id = Some(account);
    s.account_label = label;
    s
}
fn sync_identity(db: &Store, account: &str) -> Result<(), String> {
    db.conn
        .execute_batch("BEGIN IMMEDIATE")
        .map_err(|e| e.to_string())?;
    let result = (|| {
        let prior: Option<String> = db
            .conn
            .query_row(
                "SELECT account FROM account_guard WHERE provider='claude'",
                [],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        if prior.as_deref().is_some_and(|p| p != account) {
            db.conn
                .execute("UPDATE sessions SET revoked=1", [])
                .map_err(|e| e.to_string())?;
        }
        db.conn
            .execute(
                "INSERT OR REPLACE INTO account_guard VALUES('claude',?)",
                [account],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    })();
    let _ = db
        .conn
        .execute_batch(if result.is_ok() { "COMMIT" } else { "ROLLBACK" });
    result
}
pub fn helper_main() {
    let Some(dir) = std::env::args_os().nth(2).map(PathBuf::from) else {
        return;
    };
    let mut input = vec![];
    if std::io::stdin()
        .take(1024 * 1024)
        .read_to_end(&mut input)
        .is_err()
    {
        return;
    }
    let Ok(db) = Store::open(&dir) else { return };
    let Some(b) = bridge(&db) else { return };
    let original = b.original.as_ref().and_then(|v| v["command"].as_str());
    // Forward the exact original JSON and working directory; never store the full input.
    let mut child = original.and_then(|s| {
        let mut cmd = shell(s);
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit());
        let mut child = cmd.spawn().ok()?;
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(&input);
        }
        Some(child)
    });
    if original.is_none() {
        print!("Claude Code");
        let _ = std::io::stdout().flush();
    }
    let _ = capture(&db, &input, &b.generation);
    if let Some(ref mut c) = child {
        let _ = c.wait();
    }
}
fn shell(s: &str) -> std::process::Command {
    #[cfg(windows)]
    {
        if let Some(bash) = process::executable("bash") {
            let mut c = process::command(&bash);
            c.args(["-c", s]);
            c
        } else {
            let mut c = std::process::Command::new("powershell.exe");
            c.args(["-NoProfile", "-Command", s]);
            c
        }
    }
    #[cfg(not(windows))]
    {
        let mut c = std::process::Command::new("/bin/sh");
        c.args(["-c", s]);
        c
    }
}
fn capture(db: &Store, bytes: &[u8], generation: &str) -> Result<(), String> {
    let v: Value = serde_json::from_slice(bytes).map_err(|_| "invalid input")?;
    if v["session_id"].as_str().is_none() || claude_windows(&v["rate_limits"]).is_empty() {
        return Ok(());
    }
    let desktop = desktop_usage();
    let (account, label) = claude_identity(desktop.as_ref(), auth())?;
    capture_for(db, &v, &account, label, generation, now())
}
fn capture_for(
    db: &Store,
    v: &Value,
    account: &str,
    label: Option<String>,
    generation: &str,
    t: i64,
) -> Result<(), String> {
    sync_identity(db, account)?;
    let session = process::hash(&format!(
        "{generation}|{}",
        v["session_id"].as_str().ok_or("No session ID")?
    ));
    let windows = claude_windows(&v["rate_limits"]);
    if windows.is_empty() {
        return Ok(());
    }
    let digest = process::hash(&v["rate_limits"].to_string());
    let s = Snapshot {
        provider: "claude".into(),
        state: "ready".into(),
        account_id: Some(account.into()),
        account_label: label,
        windows,
        observed_at: Some(t),
        source: "Claude Code status line".into(),
        message: None,
    };
    // A session binds once. Never relabel an old session after a login changes.
    // A repeated tick is still proof that this is the provider's current reading.
    db.conn.execute("INSERT INTO sessions(session,account,digest,at,snapshot) VALUES(?,?,?,?,?) ON CONFLICT(session) DO UPDATE SET digest=excluded.digest,at=excluded.at,snapshot=excluded.snapshot WHERE sessions.account=excluded.account AND sessions.revoked=0",params![session,account,digest,t,serde_json::to_string(&s).unwrap()]).map_err(|e|e.to_string())?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn setup_restore_and_external_edits() {
        let d = tempfile::tempdir().unwrap();
        let db = Store::open(d.path()).unwrap();
        let p = d.path().join("settings with spaces.json");
        let e = d.path().join("fake executable");
        std::fs::write(&e, b"test").unwrap();
        let orig = json!({"theme":"dark","statusLine":{"type":"command","command":"printf 'hello'","padding":2}});
        atomic_json(&p, &orig).unwrap();
        connect_at(d.path(), &db, &p, &e).unwrap();
        assert!(connected(&db));
        disconnect(&db).unwrap();
        assert_eq!(read_config(&p).unwrap(), orig);
        connect_at(d.path(), &db, &p, &e).unwrap();
        let changed = json!({"statusLine":{"command":"echo new"},"theme":"light"});
        atomic_json(&p, &changed).unwrap();
        disconnect(&db).unwrap();
        assert_eq!(read_config(&p).unwrap(), changed);
    }
    #[test]
    fn account_switch_and_repeated_ticks() {
        let d = tempfile::tempdir().unwrap();
        let db = Store::open(d.path()).unwrap();
        let mut v = json!({"session_id":"one","rate_limits":{"five_hour":{"used_percentage":25,"resets_at":4000}}});
        capture_for(&db, &v, "A", None, "g", 1000).unwrap();
        capture_for(&db, &v, "A", None, "g", 1200).unwrap();
        capture_for(&db, &v, "B", None, "g", 1300).unwrap();
        let row: (String, i64) = db
            .conn
            .query_row("SELECT account,at FROM sessions", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap();
        assert_eq!(row, ("A".into(), 1200));
        v["session_id"] = json!("two");
        capture_for(&db, &v, "B", None, "g", 1400).unwrap();
        assert_eq!(
            db.conn
                .query_row("SELECT COUNT(*) FROM sessions", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            2
        );
    }
    #[test]
    fn malformed_config_is_unchanged() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("settings.json");
        std::fs::write(&p, "bad").unwrap();
        assert!(read_config(&p).is_err());
        assert_eq!(std::fs::read_to_string(p).unwrap(), "bad");
    }
    #[test]
    fn shell_paths_are_quoted() {
        assert_eq!(quote_unix("/a b/c'd"), "'/a b/c'\\''d'");
        assert_eq!(base64(b"hello"), "aGVsbG8=");
    }
    #[test]
    fn concurrent_sessions_are_atomic() {
        let d = tempfile::tempdir().unwrap();
        let db = Store::open(d.path()).unwrap();
        let joins:Vec<_>=(0..8).map(|i|{let p=d.path().to_owned();std::thread::spawn(move||{let db=Store::open(&p).unwrap();let v=json!({"session_id":format!("s{i}"),"rate_limits":{"five_hour":{"used_percentage":i,"resets_at":9000}}});capture_for(&db,&v,"A",None,"g",1000+i).unwrap();})}).collect();
        for j in joins {
            j.join().unwrap()
        }
        assert_eq!(
            db.conn
                .query_row("SELECT COUNT(*) FROM sessions", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            8
        );
    }
    #[test]
    fn existing_bridge_gets_desktop_refresh_interval_and_current_helper() {
        let d = tempfile::tempdir().unwrap();
        let db = Store::open(d.path()).unwrap();
        let p = d.path().join("settings.json");
        let old_exe = d.path().join("old executable");
        let new_exe = d.path().join("new executable");
        std::fs::write(&old_exe, b"old").unwrap();
        std::fs::write(&new_exe, b"new").unwrap();
        connect_at(d.path(), &db, &p, &old_exe).unwrap();
        let mut b = bridge(&db).unwrap();
        let old_helper = b.helper_path.clone();
        b.installed
            .as_object_mut()
            .unwrap()
            .remove("refreshInterval");
        let mut settings = read_config(&p).unwrap();
        settings["statusLine"] = b.installed.clone();
        atomic_json(&p, &settings).unwrap();
        db.conn
            .execute(
                "INSERT OR REPLACE INTO bridge VALUES(1,?)",
                [serde_json::to_string(&b).unwrap()],
            )
            .unwrap();
        maintain_bridge_at(&db, &new_exe).unwrap();
        let upgraded = bridge(&db).unwrap();
        assert_eq!(read_config(&p).unwrap()["statusLine"]["refreshInterval"], 5);
        assert_ne!(upgraded.helper_path, old_helper);
        assert_eq!(std::fs::read(&upgraded.helper_path).unwrap(), b"new");
        assert!(!old_helper.exists());
        assert!(read_config(&p).unwrap()["statusLine"]["command"]
            .as_str()
            .unwrap()
            .contains(upgraded.helper_path.file_name().unwrap().to_str().unwrap()));
        assert!(connected(&db));
    }
    #[test]
    fn reads_latest_claude_desktop_usage_without_cli_auth() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("plan-usage-history.json");
        let t = now() * 1000;
        std::fs::write(
            &p,
            serde_json::to_vec(&json!({
                "version": 2,
                "samples": [
                    {"org":"old-org","t":t-900_000,"u":{"fh":20,"sd":30}},
                    {"org":"current-org","t":t,"u":{"fh":75,"sd":35}}
                ]
            }))
            .unwrap(),
        )
        .unwrap();
        let snapshot = desktop_usage_at(&p).unwrap();
        assert_eq!(snapshot.source, "Claude Desktop usage cache");
        assert_eq!(snapshot.observed_at, Some(t / 1000));
        assert_eq!(snapshot.windows[0].used_percent, Some(75.0));
        assert_eq!(snapshot.windows[1].used_percent, Some(35.0));
        assert_eq!(snapshot.account_label, None);
        assert_ne!(snapshot.account_id.as_deref(), Some("current-org"));

        let identity = claude_identity(Some(&snapshot), Err("signed_out".into())).unwrap();
        assert_eq!(identity.0, snapshot.account_id.unwrap());
        assert_eq!(identity.1, None);
    }
    #[test]
    fn reads_reset_times_from_claude_desktop_http_cache() {
        let d = tempfile::tempdir().unwrap();
        let payload = json!({
            "five_hour": {"utilization": 0.0, "resets_at": null},
            "seven_day": {
                "utilization": 20.0,
                "resets_at": "2026-09-24T05:59:59.526391+00:00"
            }
        });
        let mut cached = b"cache metadata /api/organizations/current-org/usage\0".to_vec();
        cached.extend(
            zstd::stream::encode_all(serde_json::to_vec(&payload).unwrap().as_slice(), 0).unwrap(),
        );
        cached.extend(b"trailing cache metadata");
        std::fs::write(d.path().join("response_0"), cached).unwrap();

        let usage = desktop_cached_usage_at(d.path(), "current-org").unwrap();

        assert_eq!(usage["five_hour"].used_percent, 0.0);
        assert_eq!(usage["five_hour"].resets_at, None);
        assert_eq!(usage["seven_day"].used_percent, 20.0);
        assert_eq!(usage["seven_day"].resets_at, Some(1_790_229_599));
    }
    #[test]
    #[ignore = "Uses the installed Claude Desktop cache; run explicitly on your computer"]
    fn live_claude_desktop_cache_includes_active_reset_times() {
        let snapshot = desktop_usage().expect("Claude Desktop usage cache");
        for window in snapshot
            .windows
            .iter()
            .filter(|window| window.used_percent.is_some_and(|used| used > 0.0))
        {
            assert!(
                window.resets_at.is_some(),
                "active {} is missing its reset time",
                window.id
            );
        }
    }
    #[test]
    fn desktop_reading_keeps_current_status_line_reset_times() {
        let mut desktop = Snapshot {
            provider: "claude".into(),
            state: "ready".into(),
            account_id: Some("account".into()),
            account_label: None,
            windows: vec![UsageWindow {
                id: "five_hour".into(),
                label: "5-hour window".into(),
                used_percent: Some(40.0),
                resets_at: None,
                duration_mins: Some(300),
            }],
            observed_at: Some(2_000),
            source: "Claude Desktop usage cache".into(),
            message: None,
        };
        let mut status_line = desktop.clone();
        status_line.account_label = Some("person@example.com".into());
        status_line.windows[0].resets_at = Some(3_000);
        status_line.observed_at = Some(1_900);
        status_line.source = "Claude Code status line".into();

        merge_reset_times(&mut desktop, &status_line);

        assert_eq!(desktop.windows[0].resets_at, Some(3_000));
        assert_eq!(desktop.account_label.as_deref(), Some("person@example.com"));

        status_line.windows[0].resets_at = Some(1_999);
        merge_reset_times(&mut desktop, &status_line);
        assert_eq!(desktop.windows[0].resets_at, Some(3_000));

        status_line.account_id = Some("different-account".into());
        status_line.windows[0].resets_at = Some(4_000);
        merge_reset_times(&mut desktop, &status_line);
        assert_eq!(desktop.windows[0].resets_at, Some(3_000));
    }
    #[test]
    fn missing_status_line_reset_does_not_erase_desktop_reset() {
        let mut desktop = Snapshot {
            provider: "claude".into(),
            state: "ready".into(),
            account_id: Some("account".into()),
            account_label: None,
            windows: vec![UsageWindow {
                id: "seven_day".into(),
                label: "Weekly window".into(),
                used_percent: Some(20.0),
                resets_at: Some(3_000),
                duration_mins: Some(10_080),
            }],
            observed_at: Some(2_000),
            source: "Claude Desktop usage cache".into(),
            message: None,
        };
        let mut status_line = desktop.clone();
        status_line.windows.clear();
        status_line.observed_at = None;
        status_line.source = "Claude Code status line".into();

        merge_reset_times(&mut desktop, &status_line);

        assert_eq!(desktop.windows[0].resets_at, Some(3_000));
    }
    #[test]
    fn returning_to_account_requires_new_session() {
        let d = tempfile::tempdir().unwrap();
        let db = Store::open(d.path()).unwrap();
        let v = json!({"session_id":"old","rate_limits":{"five_hour":{"used_percentage":25}}});
        capture_for(&db, &v, "A", None, "g", 1000).unwrap();
        sync_identity(&db, "B").unwrap();
        sync_identity(&db, "A").unwrap();
        let changed =
            json!({"session_id":"old","rate_limits":{"five_hour":{"used_percentage":30}}});
        capture_for(&db, &changed, "A", None, "g", 1300).unwrap();
        assert_eq!(
            db.conn
                .query_row("SELECT COUNT(*) FROM sessions WHERE revoked=0", [], |r| r
                    .get::<_, i64>(
                    0
                ))
                .unwrap(),
            0
        );
    }
}
