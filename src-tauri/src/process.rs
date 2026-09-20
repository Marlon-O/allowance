use sha2::{Digest, Sha256};
use std::{
    path::PathBuf,
    process::{Command, Stdio},
    time::{Duration, Instant},
};
pub fn hash(s: &str) -> String {
    format!("{:x}", Sha256::digest(s.as_bytes()))
}
pub fn executable(name: &str) -> Option<PathBuf> {
    let mut roots: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    roots.extend([
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
    ]);
    if let Some(home) = dirs::home_dir() {
        roots.extend([home.join(".local/bin"), home.join(".cargo/bin")]);
        if let Ok(entries) = std::fs::read_dir(home.join(".nvm/versions/node")) {
            let mut paths: Vec<_> = entries.flatten().map(|e| e.path().join("bin")).collect();
            paths.sort();
            roots.extend(paths.into_iter().rev());
        }
    }
    #[cfg(windows)]
    if let Some(p) = std::env::var_os("APPDATA") {
        roots.push(PathBuf::from(p).join("npm"));
    }
    for root in roots {
        for ext in if cfg!(windows) {
            vec![".exe", ".cmd", ""]
        } else {
            vec![""]
        } {
            let p = root.join(format!("{name}{ext}"));
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}
pub fn command(path: &std::path::Path) -> Command {
    #[cfg(windows)]
    let mut c = if path.extension().is_some_and(|x| x == "cmd") {
        let mut c = Command::new("cmd.exe");
        c.args(["/d", "/s", "/c"]);
        c.arg(path);
        c
    } else {
        Command::new(path)
    };
    #[cfg(not(windows))]
    let mut c = Command::new(path);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x08000000);
    }
    // npm-installed CLIs need their sibling node binary even from Finder's minimal PATH.
    let mut paths = vec![];
    if let Some(p) = path.parent() {
        paths.push(p.to_path_buf());
    }
    if let Some(node) = executable_node() {
        if let Some(p) = node.parent() {
            paths.push(p.to_path_buf());
        }
    }
    if let Some(p) = std::env::var_os("PATH") {
        paths.extend(std::env::split_paths(&p));
    }
    if let Ok(p) = std::env::join_paths(paths) {
        c.env("PATH", p);
    }
    c
}
fn executable_node() -> Option<PathBuf> {
    for p in ["/opt/homebrew/bin/node", "/usr/local/bin/node"] {
        if std::path::Path::new(p).is_file() {
            return Some(p.into());
        }
    }
    let home = dirs::home_dir()?;
    let mut ps: Vec<_> = std::fs::read_dir(home.join(".nvm/versions/node"))
        .ok()?
        .flatten()
        .map(|e| e.path().join("bin/node"))
        .filter(|p| p.is_file())
        .collect();
    ps.sort();
    ps.pop()
}
pub fn output(mut cmd: Command, timeout: Duration) -> Result<Vec<u8>, String> {
    cmd.stdout(Stdio::piped())
        .stderr(Stdio::null())
        .stdin(Stdio::null());
    let mut child = cmd
        .spawn()
        .map_err(|_| "Could not start the installed CLI.".to_string())?;
    let mut stdout = child.stdout.take().ok_or("CLI output unavailable")?;
    let reader = std::thread::spawn(move || {
        use std::io::Read;
        let mut b = vec![];
        stdout
            .by_ref()
            .take(1024 * 1024)
            .read_to_end(&mut b)
            .map(|_| b)
    });
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let bytes = reader
                    .join()
                    .map_err(|_| "CLI read failed")?
                    .map_err(|_| "CLI read failed")?;
                return if status.success() || !bytes.is_empty() {
                    Ok(bytes)
                } else {
                    Err("CLI returned an error without authentication details.".into())
                };
            }
            Ok(None) => {}
            Err(_) => return Err("CLI status unavailable".into()),
        }
        if start.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            return Err("CLI timed out. Try again.".into());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}
