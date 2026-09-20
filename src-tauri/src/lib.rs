pub mod claude;
mod codex;
mod model;
mod process;
mod store;
use model::*;
use serde::Serialize;
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex,
    },
};
use store::Store;
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Emitter, Manager,
};
use tauri_plugin_autostart::ManagerExt as AutostartExt;
use tauri_plugin_notification::NotificationExt;

struct Core {
    db: Store,
    snapshots: Vec<Snapshot>,
}
struct AppState {
    core: Mutex<Core>,
    dir: PathBuf,
    refresh: tokio::sync::mpsc::Sender<()>,
    last_focus_loss: AtomicU64,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct View {
    settings: Settings,
    providers: Vec<Snapshot>,
    autostart: bool,
}
fn lock(state: &AppState) -> Result<std::sync::MutexGuard<'_, Core>, String> {
    state
        .core
        .lock()
        .map_err(|_| "Application state unavailable".into())
}
fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
#[tauri::command]
fn get_state(app: tauri::AppHandle, state: tauri::State<AppState>) -> Result<View, String> {
    let c = lock(&state)?;
    Ok(View {
        settings: c.db.settings(),
        providers: c.snapshots.clone(),
        autostart: app.autolaunch().is_enabled().unwrap_or(false),
    })
}
#[tauri::command]
fn refresh(state: tauri::State<AppState>) {
    let _ = state.refresh.try_send(());
}
#[tauri::command]
fn update_preferences(
    theme: String,
    show_used: bool,
    alerts: bool,
    state: tauri::State<AppState>,
) -> Result<(), String> {
    if !["system", "dark", "light"].contains(&theme.as_str()) {
        return Err("Invalid theme".into());
    }
    let c = lock(&state)?;
    let mut s = c.db.settings();
    s.theme = theme;
    s.show_used = show_used;
    s.alerts = alerts;
    c.db.save_settings(&s)
}
#[tauri::command]
fn set_autostart(enabled: bool, app: tauri::AppHandle) -> Result<(), String> {
    if enabled {
        app.autolaunch().enable()
    } else {
        app.autolaunch().disable()
    }
    .map_err(|e| e.to_string())
}
#[tauri::command]
async fn connect_provider(
    provider: String,
    enabled: bool,
    app: tauri::AppHandle,
) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move||{
        let state=app.state::<AppState>();let mut c=lock(&state)?;let mut s=c.db.settings();
        let message=match provider.as_str(){"claude"=>{let m=if enabled {if process::executable("claude").is_none(){return Err("Install Claude Code before connecting.".into())}claude::connect(&state.dir,&c.db)?;"Claude Code connected. Restart existing Claude Code sessions to begin receiving readings.".into()}else{claude::disconnect(&c.db)?};s.claude_enabled=enabled;m},"codex"=>{s.codex_enabled=enabled;if enabled{"Codex connected."}else{"Codex disconnected from Allowance. Your Codex login is unchanged."}.into()},_=>return Err("Unknown provider".into())};
        c.db.save_settings(&s)?;
        if let Some(snapshot)=c.snapshots.iter_mut().find(|s|s.provider==provider){*snapshot=Snapshot::empty(&provider,if enabled{"waiting"}else{"disconnected"},if enabled{"Waiting for a fresh reading."}else{"Provider disconnected."});}
        drop(c);let _=state.refresh.try_send(());let _=app.emit("usage-updated",());Ok(message)
    }).await.map_err(|e|e.to_string())?
}
#[tauri::command]
fn quit(app: tauri::AppHandle) {
    app.exit(0)
}

fn show(app: &tauri::AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.set_focus();
        if let Some(s) = app.try_state::<AppState>() {
            let _ = s.refresh.try_send(());
        }
    }
}
fn show_at(app: &tauri::AppHandle, point: tauri::PhysicalPosition<f64>) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    if let (Ok(size), Ok(monitors)) = (window.outer_size(), window.available_monitors()) {
        let px = point.x.round() as i32;
        let py = point.y.round() as i32;
        if let Some(monitor) = monitors.iter().find(|monitor| {
            let origin = monitor.position();
            let bounds = monitor.size();
            px >= origin.x
                && py >= origin.y
                && px < origin.x + bounds.width as i32
                && py < origin.y + bounds.height as i32
        }) {
            let origin = monitor.position();
            let bounds = monitor.size();
            let width = size.width as i32;
            let height = size.height as i32;
            let min_x = origin.x + 8;
            let max_x = (origin.x + bounds.width as i32 - width - 8).max(min_x);
            let min_y = origin.y + 8;
            let max_y = (origin.y + bounds.height as i32 - height - 8).max(min_y);
            let x = (px - width / 2).clamp(min_x, max_x);
            let opens_down = py < origin.y + bounds.height as i32 / 2;
            let y = (if opens_down {
                py + 10
            } else {
                py - height - 10
            })
            .clamp(min_y, max_y);
            let _ = window.set_position(tauri::PhysicalPosition::new(x, y));
        }
    }
    show(app);
}
fn publish(app: &tauri::AppHandle, snapshot: Snapshot) {
    let state = app.state::<AppState>();
    let Ok(mut c) = lock(&state) else { return };
    // Re-check enable state after a background read completes to avoid reconnect races.
    let settings = c.db.settings();
    if (snapshot.provider == "claude" && !settings.claude_enabled)
        || (snapshot.provider == "codex" && !settings.codex_enabled)
    {
        return;
    }
    if settings.alerts && fresh(&snapshot, now()) {
        for w in &snapshot.windows {
            if let Some(threshold) = alert_threshold(w, now()) {
                if c.db.claim_alert(&snapshot, w, threshold).unwrap_or(false) {
                    let provider = if snapshot.provider == "codex" {
                        "Codex"
                    } else {
                        "Claude Code"
                    };
                    if app
                        .notification()
                        .builder()
                        .title(format!("{provider}: allowance running low"))
                        .body(format!(
                            "{}: {:.0}% remaining.",
                            w.label,
                            remaining(w).unwrap_or_default()
                        ))
                        .show()
                        .is_err()
                    {
                        c.db.unclaim_alert(&snapshot, w, threshold);
                    }
                }
            }
        }
    }
    if let Some(s) = c
        .snapshots
        .iter_mut()
        .find(|s| s.provider == snapshot.provider)
    {
        *s = snapshot;
    }
    drop(c);
    let _ = app.emit("usage-updated", ());
}
async fn monitor(app: tauri::AppHandle, mut rx: tokio::sync::mpsc::Receiver<()>) {
    let mut rpc: Option<codex::Codex> = None;
    let mut last_codex = 0i64;
    let mut next_codex = 0i64;
    let mut failures = 0u32;
    let mut last_claude = 0i64;
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(5));
    loop {
        let requested =
            tokio::select! {_ = interval.tick()=>false,v=rx.recv()=>{if v.is_none(){break}true}};
        let state = app.state::<AppState>();
        let settings = match lock(&state) {
            Ok(c) => c.db.settings(),
            Err(_) => continue,
        };
        let t = now();
        if settings.claude_enabled && (t - last_claude >= 30 || requested) {
            last_claude = t;
            let handle = app.clone();
            if let Ok(s) = tauri::async_runtime::spawn_blocking(move || {
                let state = handle.state::<AppState>();
                match Store::open(&state.dir) {
                    Ok(db) => claude::read(&db),
                    Err(_) => Snapshot::empty("claude", "error", "Local database unavailable."),
                }
            })
            .await
            {
                publish(&app, s);
            }
        }
        if !settings.codex_enabled {
            rpc = None;
            next_codex = 0;
            last_codex = 0;
            continue;
        }
        let dirty = rpc.as_mut().is_some_and(|r| r.take_dirty());
        if (t >= next_codex || (requested && failures == 0) || dirty) && t - last_codex >= 5 {
            last_codex = t;
            let result = async {
                if rpc.is_none() {
                    rpc = Some(codex::Codex::start(&state.dir).await?);
                }
                rpc.as_mut().unwrap().read().await
            }
            .await;
            match result {
                Ok(s) => {
                    failures = 0;
                    next_codex = now() + 120;
                    publish(&app, s)
                }
                Err(e) => {
                    rpc = None;
                    failures = (failures + 1).min(5);
                    next_codex = now() + (15 * 2i64.pow(failures - 1)).min(300);
                    let s = Snapshot::empty(
                        "codex",
                        if e == "missing_cli" {
                            "missing_cli"
                        } else {
                            "error"
                        },
                        if e == "missing_cli" {
                            "Install Codex CLI, then refresh."
                        } else {
                            &e
                        },
                    );
                    publish(&app, s);
                }
            }
        }
    }
}
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _, _| show(app)))
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec!["--background"]),
        ))
        .plugin(tauri_plugin_notification::init())
        .invoke_handler(tauri::generate_handler![
            get_state,
            refresh,
            update_preferences,
            set_autostart,
            connect_provider,
            quit
        ])
        .setup(|app| {
            let dir = app.path().app_data_dir()?;
            let db = Store::open(&dir).map_err(std::io::Error::other)?;
            db.housekeeping(now()).map_err(std::io::Error::other)?;
            let settings = db.settings();
            if settings.claude_enabled {
                let _ = claude::maintain_bridge(&db);
            }
            let (tx, rx) = tokio::sync::mpsc::channel(1);
            let snapshots = vec![
                Snapshot::empty(
                    "claude",
                    if settings.claude_enabled {
                        "waiting"
                    } else {
                        "disconnected"
                    },
                    "Connect Claude Code to receive usage readings.",
                ),
                Snapshot::empty(
                    "codex",
                    if settings.codex_enabled {
                        "waiting"
                    } else {
                        "disconnected"
                    },
                    "Waiting for Codex usage.",
                ),
            ];
            app.manage(AppState {
                core: Mutex::new(Core { db, snapshots }),
                dir,
                refresh: tx,
                last_focus_loss: AtomicU64::new(0),
            });
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);
            let refresh = MenuItem::with_id(app, "refresh", "Refresh usage", true, None::<&str>)?;
            let exit = MenuItem::with_id(app, "quit", "Quit Allowance", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&refresh, &exit])?;
            let icon = tauri::image::Image::from_bytes(include_bytes!("../icons/tray.png"))?;
            TrayIconBuilder::new()
                .icon(icon)
                .icon_as_template(true)
                .tooltip("Allowance · AI usage")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, e| match e.id.as_ref() {
                    "refresh" => {
                        if let Some(state) = app.try_state::<AppState>() {
                            let _ = state.refresh.try_send(());
                        }
                    }
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, e| {
                    if let TrayIconEvent::Click {
                        position,
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = e
                    {
                        let app = tray.app_handle();
                        if let Some(w) = app.get_webview_window("main") {
                            if w.is_visible().unwrap_or(false) {
                                let _ = w.hide();
                            } else if now_millis().saturating_sub(
                                app.state::<AppState>()
                                    .last_focus_loss
                                    .load(Ordering::Relaxed),
                            ) > 300
                            {
                                show_at(app, position)
                            }
                        }
                    }
                })
                .build(app)?;
            let w = app
                .get_webview_window("main")
                .ok_or("Main window missing")?;
            w.set_always_on_top(true)?;
            let handle = app.handle().clone();
            w.on_window_event(move |event| match event {
                tauri::WindowEvent::CloseRequested { api, .. } => {
                    api.prevent_close();
                    if let Some(w) = handle.get_webview_window("main") {
                        let _ = w.hide();
                    }
                }
                tauri::WindowEvent::Focused(false) => {
                    handle
                        .state::<AppState>()
                        .last_focus_loss
                        .store(now_millis(), Ordering::Relaxed);
                    if let Some(w) = handle.get_webview_window("main") {
                        let _ = w.hide();
                    }
                }
                _ => {}
            });
            tauri::async_runtime::spawn(monitor(app.handle().clone(), rx));
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("Could not start Allowance")
        .run(|app, event| {
            #[cfg(target_os = "macos")]
            if matches!(event, tauri::RunEvent::Reopen { .. }) {
                show(app);
            }
            #[cfg(not(target_os = "macos"))]
            let _ = (app, event);
        });
}
