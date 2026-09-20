// Exercises the actual native executable's helper branch, without touching user settings.
#[test]
#[cfg(unix)]
fn original_statusline_receives_exact_input_and_working_directory() {
    use std::{
        io::Write,
        process::{Command, Stdio},
    };
    let dir = tempfile::Builder::new()
        .prefix("allowance helper spaces ")
        .tempdir()
        .unwrap();
    let db = rusqlite::Connection::open(dir.path().join("usage.sqlite3")).unwrap();
    db.execute_batch("CREATE TABLE bridge(id INTEGER PRIMARY KEY,json TEXT NOT NULL);")
        .unwrap();
    let bridge = serde_json::json!({"original":{"type":"command","command":"printf 'original-output:'; cat; printf '\\n'; pwd"},"installed":{"command":"test"},"settings_path":dir.path().join("settings.json"),"helper_path":dir.path().join("helper"),"generation":"fixture"});
    db.execute("INSERT INTO bridge VALUES(1,?)", [bridge.to_string()])
        .unwrap();
    // No session ID means no auth call. This content must never be saved.
    let raw = "{\"private_fixture\":\"not-for-storage\"}";
    let mut child = Command::new(env!("CARGO_BIN_EXE_ai-usage-widget"))
        .arg("--claude-statusline")
        .arg(dir.path())
        .current_dir(dir.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(raw.as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success());
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.starts_with(&format!("original-output:{raw}\n")));
    assert!(text.contains(dir.path().to_str().unwrap()));
    let count: i64 = db
        .query_row("SELECT COUNT(*) FROM sessions", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 0);
}
