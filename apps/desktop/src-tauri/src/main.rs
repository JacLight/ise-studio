//! Iṣẹ́ Studio in its own window.
//!
//! The window shows the same engine the browser does (http://127.0.0.1:7420),
//! so both work at once on the same projects. The app only starts the engine
//! when nothing answers there, and leaves it running when it quits, so open
//! browser tabs and connected assistants keep working: the engine is its own
//! program, the window is one more view of it.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::webview::{DownloadEvent, NewWindowResponse, PageLoadEvent};
use tauri::{AppHandle, Manager, Url, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

/// The checkout this app was built from: where it finds the engine, the web
/// app and the projects, exactly as `de-server` run from the repository does.
fn repo() -> PathBuf {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..");
    root.canonicalize().unwrap_or(root)
}

/// `ISE_PORT` and `ISE_PROJECT` pick another engine (as `de-server --port/--project` do).
fn port() -> u16 {
    std::env::var("ISE_PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(7420)
}

fn engine_url() -> Url {
    Url::parse(&format!("http://127.0.0.1:{}/", port())).unwrap()
}

/// What answers on the engine's port.
enum Answer {
    Engine,
    Other(String),
    Nothing,
}

/// Asks for the project list, which only the engine serves.
fn probe() -> Answer {
    let address = SocketAddr::from(([127, 0, 0, 1], port()));
    let Ok(mut stream) = TcpStream::connect_timeout(&address, Duration::from_millis(400)) else {
        return Answer::Nothing;
    };
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    if stream.write_all(b"GET /projects HTTP/1.0\r\nHost: 127.0.0.1\r\n\r\n").is_err() {
        return Answer::Nothing;
    }
    let mut reply = Vec::new();
    let _ = stream.take(64 * 1024).read_to_end(&mut reply);
    let reply = String::from_utf8_lossy(&reply);
    let status = reply.lines().next().unwrap_or("").to_string();
    let body = reply.split("\r\n\r\n").nth(1).unwrap_or("").trim_start();
    if status.contains(" 200 ") && (body.starts_with('[') || body.starts_with('{')) {
        Answer::Engine
    } else {
        Answer::Other(status)
    }
}

/// The engine program: `ISE_ENGINE`, one beside this app, or the checkout's release build.
fn engine_binary() -> Option<PathBuf> {
    let name = if cfg!(windows) { "de-server.exe" } else { "de-server" };
    let beside = std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.join(name)));
    std::env::var_os("ISE_ENGINE")
        .map(PathBuf::from)
        .into_iter()
        .chain(beside)
        .chain([repo().join("target/release").join(name)])
        .find(|p| p.is_file())
}

/// The PATH a terminal would have. Apps opened from the Finder get a bare one,
/// and the engine looks there for Claude Code (Iṣẹ́ AI) and other tools.
fn login_path() -> Option<String> {
    if cfg!(windows) {
        return None;
    }
    let shell = std::env::var("SHELL").ok()?;
    let mut child = Command::new(shell)
        .args(["-l", "-c", "printf %s \"$PATH\""])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let start = Instant::now();
    while child.try_wait().ok()?.is_none() {
        if start.elapsed() > Duration::from_secs(3) {
            let _ = child.kill();
            return None;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let mut path = String::new();
    child.stdout.take()?.read_to_string(&mut path).ok()?;
    (!path.trim().is_empty()).then(|| path.trim().to_string())
}

/// The engine this app started, if it started one (it never stops one it didn't start).
#[derive(Default)]
struct Engine {
    child: Mutex<Option<Child>>,
    starting: AtomicBool,
}

/// Makes sure the engine answers, starting it if nothing does. `say` reports progress.
fn ensure_engine(app: &AppHandle, say: &dyn Fn(&str)) -> Result<(), String> {
    match probe() {
        Answer::Engine => return Ok(()),
        Answer::Other(status) => {
            return Err(format!(
                "Something else is using port {} (it answered “{status}”).\nClose it, or start the app with ISE_PORT set to the engine's port.",
                port()
            ));
        }
        Answer::Nothing => {}
    }
    let repo = repo();
    let Some(binary) = engine_binary() else {
        return Err(format!(
            "The engine isn't built yet. In a terminal:\n\ncd {}\ncargo build --release -p de-server\n\nThen try again (or set ISE_ENGINE to the de-server program).",
            repo.display()
        ));
    };
    let web = repo.join("apps/web/dist");
    if !web.join("index.html").is_file() {
        return Err(format!(
            "The web app isn't built yet. In a terminal:\n\ncd {}\nnpm install && npm run build\n\nThen try again.",
            repo.join("apps/web").display()
        ));
    }
    let project = std::env::var_os("ISE_PROJECT").map(PathBuf::from).unwrap_or_else(|| repo.join("projects/default.jsonl"));
    say("Starting the engine…");
    let logs = app.path().app_log_dir().unwrap_or_else(|_| std::env::temp_dir());
    let _ = std::fs::create_dir_all(&logs);
    let log_path = logs.join("engine.log");
    let log = std::fs::OpenOptions::new().create(true).append(true).open(&log_path).map_err(|e| format!("Can't write {}: {e}", log_path.display()))?;
    let mut command = Command::new(&binary);
    command
        .args(["--project", &project.to_string_lossy()])
        .args(["--web", &web.to_string_lossy()])
        .args(["--port", &port().to_string()])
        .current_dir(&repo)
        .stdin(Stdio::null())
        .stdout(log.try_clone().map_err(|e| e.to_string())?)
        .stderr(log);
    if let Some(path) = login_path() {
        command.env("PATH", path);
    }
    // Its own process group: quitting the app (or Ctrl-C on `npm run dev`) leaves it running.
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut command, 0);
    #[cfg(windows)]
    std::os::windows::process::CommandExt::creation_flags(&mut command, 0x0800_0000 | 0x0000_0200); // no console, new group
    let child = command.spawn().map_err(|e| format!("Couldn't start {}: {e}", binary.display()))?;
    let engine = app.state::<Engine>();
    *engine.child.lock().unwrap() = Some(child);

    let start = Instant::now();
    loop {
        std::thread::sleep(Duration::from_millis(250));
        if let Some(status) = engine.child.lock().unwrap().as_mut().and_then(|c| c.try_wait().ok().flatten()) {
            engine.child.lock().unwrap().take();
            let text = std::fs::read_to_string(&log_path).unwrap_or_default();
            let tail: Vec<&str> = text.lines().rev().take(12).collect();
            let tail: Vec<&str> = tail.into_iter().rev().collect();
            return Err(format!("The engine stopped ({status}). The end of {}:\n\n{}", log_path.display(), tail.join("\n")));
        }
        if let Answer::Engine = probe() {
            return Ok(());
        }
        if start.elapsed() > Duration::from_secs(120) {
            return Err(format!("The engine didn't answer within two minutes. Its log: {}", log_path.display()));
        }
    }
}

/// Opens a link in the computer's browser.
fn open_in_browser(url: &str) {
    #[cfg(target_os = "macos")]
    let _ = Command::new("open").arg(url).spawn();
    #[cfg(windows)]
    let _ = Command::new("cmd").args(["/C", "start", "", url]).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let _ = Command::new("xdg-open").arg(url).spawn();
}

/// The engine's own pages (and this app's start page) stay in the app; any
/// other site opens in the browser.
fn is_ours(url: &Url) -> bool {
    match url.scheme() {
        "tauri" | "about" | "blob" | "data" => true,
        "http" | "https" => is_engine(url) || is_start_page(url),
        _ => false,
    }
}

fn is_engine(url: &Url) -> bool {
    matches!(url.host_str(), Some("127.0.0.1" | "localhost")) && url.port_or_known_default() == Some(port())
}

/// This app's own start page (tauri://localhost, or http://tauri.localhost on Windows).
fn is_start_page(url: &Url) -> bool {
    url.scheme() == "tauri" || url.host_str() == Some("tauri.localhost")
}

static WINDOWS: AtomicUsize = AtomicUsize::new(0);

/// A window on the engine (or on the start page), handling what a browser
/// would: links to other sites, new tabs, downloads, dropped files, the title.
fn window(app: &AppHandle, url: WebviewUrl) -> tauri::Result<WebviewWindow> {
    let n = WINDOWS.fetch_add(1, Ordering::SeqCst);
    let label = if n == 0 { "main".to_string() } else { format!("window-{n}") };
    let opener = app.clone();
    // Where each download went, to say so when it finishes (macOS doesn't report it).
    let saving: Mutex<HashMap<String, PathBuf>> = Mutex::new(HashMap::new());
    let builder = WebviewWindowBuilder::new(app, label, url)
        .title("Iṣẹ́ Studio")
        .inner_size(1440.0, 900.0)
        .min_inner_size(720.0, 480.0)
        .center()
        // The web app takes dropped files itself (pictures, STEP, KiCad…).
        .disable_drag_drop_handler()
        .on_document_title_changed(|window, title| {
            let _ = window.set_title(if title.trim().is_empty() { "Iṣẹ́ Studio" } else { &title });
        })
        .on_navigation(|url| {
            if is_ours(url) {
                return true;
            }
            open_in_browser(url.as_str());
            false
        })
        .on_new_window(move |url, _features| {
            if is_ours(&url) && url.scheme() != "about" {
                // A "new tab" of the engine becomes another window; built off this callback's thread.
                let app = opener.clone();
                std::thread::spawn(move || {
                    let _ = window(&app, WebviewUrl::External(url));
                });
            } else if url.scheme() != "about" {
                open_in_browser(url.as_str());
            }
            NewWindowResponse::Deny
        })
        .on_download(move |webview, event| {
            match event {
                DownloadEvent::Requested { url, destination } => {
                    saving.lock().unwrap().insert(url.to_string(), destination.clone());
                }
                DownloadEvent::Finished { url, success, .. } => {
                    let path = saving.lock().unwrap().remove(url.as_str());
                    let name = path.as_ref().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                    let text = if success { format!("Saved “{name}” in Downloads") } else { format!("Couldn't save “{name}”") };
                    let _ = webview.eval(toast(&text));
                }
                _ => {}
            }
            true
        });
    builder.build()
}

/// A short note at the bottom of the page (the web app has no download bar of its own).
fn toast(text: &str) -> String {
    let text = serde_json::to_string(text).unwrap();
    format!(
        r#"(() => {{
  const n = document.createElement("div");
  n.textContent = {text};
  n.setAttribute("role", "status");
  n.style.cssText = "position:fixed;left:50%;bottom:28px;transform:translateX(-50%);z-index:2147483647;padding:9px 16px;border-radius:10px;background:rgba(28,24,40,.92);color:#fff;font:500 13px/1.3 'DM Sans',system-ui,sans-serif;box-shadow:0 6px 24px rgba(0,0,0,.25);pointer-events:none;transition:opacity .3s";
  document.body.appendChild(n);
  setTimeout(() => {{ n.style.opacity = "0"; setTimeout(() => n.remove(), 400); }}, 3200);
}})();"#
    )
}

/// Shows progress or a problem on the start page.
fn say(window: &WebviewWindow, text: &str, problem: bool) {
    let text = serde_json::to_string(text).unwrap();
    let _ = window.eval(format!("window.iseStatus && window.iseStatus({text}, {problem})"));
}

/// Once the start page is showing: reach (or start) the engine, then open it.
fn connect(window: WebviewWindow) {
    let app = window.app_handle().clone();
    let engine = app.state::<Engine>();
    if engine.starting.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(move || {
        let result = ensure_engine(&app, &|text| say(&window, text, false));
        app.state::<Engine>().starting.store(false, Ordering::SeqCst);
        match result {
            Ok(()) => {
                let _ = window.navigate(engine_url());
                if let Some(stop) = app.try_state::<StopItem>() {
                    let _ = stop.0.set_enabled(app.state::<Engine>().child.lock().unwrap().is_some());
                }
            }
            Err(problem) => say(&window, &problem, true),
        }
    });
}

/// Stops the engine this app started (gracefully, so it finishes writing).
fn stop_engine(app: &AppHandle) {
    let Some(mut child) = app.state::<Engine>().child.lock().unwrap().take() else {
        return;
    };
    #[cfg(unix)]
    let _ = Command::new("kill").args(["-TERM", &child.id().to_string()]).status();
    #[cfg(not(unix))]
    let _ = child.kill();
    let start = Instant::now();
    while child.try_wait().ok().flatten().is_none() && start.elapsed() < Duration::from_secs(8) {
        std::thread::sleep(Duration::from_millis(100));
    }
    let _ = child.kill();
}

/// "Stop Engine and Quit", enabled once the app has started an engine.
struct StopItem(MenuItem<tauri::Wry>);

fn menu(app: &AppHandle) -> tauri::Result<Menu<tauri::Wry>> {
    let new_window = MenuItem::with_id(app, "new-window", "New Window", true, Some("CmdOrCtrl+N"))?;
    let in_browser = MenuItem::with_id(app, "open-in-browser", "Open in Browser", true, None::<&str>)?;
    let stop = MenuItem::with_id(app, "stop-engine", "Stop Engine and Quit", false, None::<&str>)?;
    app.manage(StopItem(stop.clone()));
    let reload = MenuItem::with_id(app, "reload", "Reload", true, Some("CmdOrCtrl+R"))?;
    let docs = MenuItem::with_id(app, "docs", "Iṣẹ́ Studio Documentation", true, None::<&str>)?;
    let separator = || PredefinedMenuItem::separator(app);

    #[cfg(target_os = "macos")]
    let file = Submenu::with_items(app, "File", true, &[&new_window, &in_browser, &separator()?, &PredefinedMenuItem::close_window(app, None)?])?;
    #[cfg(not(target_os = "macos"))]
    let file = Submenu::with_items(app, "File", true, &[&new_window, &in_browser, &separator()?, &stop, &PredefinedMenuItem::quit(app, None)?])?;
    let edit = Submenu::with_items(
        app,
        "Edit",
        true,
        &[
            &PredefinedMenuItem::undo(app, None)?,
            &PredefinedMenuItem::redo(app, None)?,
            &separator()?,
            &PredefinedMenuItem::cut(app, None)?,
            &PredefinedMenuItem::copy(app, None)?,
            &PredefinedMenuItem::paste(app, None)?,
            &PredefinedMenuItem::select_all(app, None)?,
        ],
    )?;
    let view = Submenu::with_items(app, "View", true, &[&reload, &separator()?, &PredefinedMenuItem::fullscreen(app, None)?])?;
    let window = Submenu::with_items(app, "Window", true, &[&PredefinedMenuItem::minimize(app, None)?, &PredefinedMenuItem::maximize(app, None)?])?;
    let help = Submenu::with_items(app, "Help", true, &[&docs])?;

    #[cfg(target_os = "macos")]
    {
        let about = PredefinedMenuItem::about(app, None, None)?;
        let ise = Submenu::with_items(
            app,
            "Iṣẹ́ Studio",
            true,
            &[
                &about,
                &separator()?,
                &PredefinedMenuItem::services(app, None)?,
                &separator()?,
                &PredefinedMenuItem::hide(app, None)?,
                &PredefinedMenuItem::hide_others(app, None)?,
                &PredefinedMenuItem::show_all(app, None)?,
                &separator()?,
                &stop,
                &PredefinedMenuItem::quit(app, None)?,
            ],
        )?;
        Menu::with_items(app, &[&ise, &file, &edit, &view, &window, &help])
    }
    #[cfg(not(target_os = "macos"))]
    Menu::with_items(app, &[&file, &edit, &view, &window, &help])
}

/// The window in front, if any.
fn focused(app: &AppHandle) -> Option<WebviewWindow> {
    let windows = app.webview_windows();
    windows.values().find(|w| w.is_focused().unwrap_or(false)).or_else(|| windows.values().next()).cloned()
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .manage(Engine::default())
        .menu(menu)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "new-window" => {
                let _ = window(app, WebviewUrl::External(engine_url()));
            }
            "open-in-browser" => {
                // The page in front, as it is (its project and view are in the address).
                let url = focused(app).and_then(|w| w.url().ok()).filter(is_engine).unwrap_or_else(engine_url);
                open_in_browser(url.as_str());
            }
            "reload" => {
                if let Some(w) = focused(app) {
                    let _ = w.reload();
                }
            }
            "docs" => open_in_browser("https://github.com/JacLight/ise-studio/tree/main/docs"),
            "stop-engine" => {
                stop_engine(app);
                app.exit(0);
            }
            _ => {}
        })
        .on_page_load(|webview, payload| {
            // The start page (the app's own) asks for the engine each time it loads, so "Try again" is a reload.
            if payload.event() == PageLoadEvent::Finished && is_start_page(payload.url()) {
                if let Some(window) = webview.app_handle().get_webview_window(webview.label()) {
                    connect(window);
                }
            }
            // Debug builds only: ISE_DESKTOP_EVAL=<file.js> runs a script in the first engine page, for checks.
            #[cfg(debug_assertions)]
            if payload.event() == PageLoadEvent::Finished && is_engine(payload.url()) && webview.label() == "main" {
                static RAN: AtomicBool = AtomicBool::new(false);
                if let Some(script) = std::env::var_os("ISE_DESKTOP_EVAL").and_then(|f| std::fs::read_to_string(f).ok())
                    && !RAN.swap(true, Ordering::SeqCst)
                {
                    let _ = webview.eval(script);
                }
            }
        })
        .setup(|app| {
            window(app.handle(), WebviewUrl::App("index.html".into()))?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("Iṣẹ́ Studio couldn't open its window");
}
