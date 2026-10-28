#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use d256shell_core::service::{ApiError, Service};
use d256shell_core::terminal::session::ReadResult;
use d256shell_core::{logging::Logger, paths};
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;
use tauri::ipc::{Channel, InvokeResponseBody};
use tauri::{State, WebviewUrl, WebviewWindowBuilder};

/// The only request/response IPC entry point: everything is dispatched inside `d256shell-core`.
/// Runs on a blocking thread: the KDF and network operations must never block the UI thread.
#[tauri::command]
async fn api(
    svc: State<'_, Arc<Service>>,
    method: String,
    params: Value,
) -> Result<Value, ApiError> {
    let svc = svc.inner().clone();
    tauri::async_runtime::spawn_blocking(move || svc.call(&method, params))
        .await
        .unwrap_or_else(|e| {
            Err(ApiError {
                code: "internal",
                message: format!("Внутренняя ошибка: {e}"),
                details: None,
            })
        })
}

/// Terminal output as raw bytes over a Tauri channel (no JSON, no base64). Output is batched for a few
/// milliseconds so that `cat` of a big log does not flood the IPC. The final message is
/// `{"exit": <code>}` (JSON) when the process ended.
#[tauri::command]
fn term_stream(
    svc: State<'_, Arc<Service>>,
    term_id: String,
    on_data: Channel<InvokeResponseBody>,
) -> Result<(), ApiError> {
    let svc = svc.inner().clone();
    std::thread::spawn(move || loop {
        match svc.term_read_raw(&term_id, Duration::from_millis(100)) {
            Ok(ReadResult::Data(mut d)) => {
                std::thread::sleep(Duration::from_millis(4));
                while let Ok(ReadResult::Data(more)) = svc.term_read_raw(&term_id, Duration::ZERO) {
                    d.extend_from_slice(&more);
                    if d.len() > 512 * 1024 {
                        break;
                    }
                }
                if on_data.send(InvokeResponseBody::Raw(d)).is_err() {
                    break;
                }
            }
            Ok(ReadResult::Timeout) => {}
            Ok(ReadResult::Exited(code)) => {
                let _ = on_data.send(InvokeResponseBody::Json(format!("{{\"exit\":{code}}}")));
                break;
            }
            Err(_) => break, // terminal was closed
        }
    });
    Ok(())
}

/// Start a native drag of already staged files (`<data>/staging/...`) into Windows Explorer.
/// The call returns when the drag ends. Only paths inside the staging folder are accepted.
#[tauri::command]
fn drag_out(
    window: tauri::WebviewWindow,
    svc: State<'_, Arc<Service>>,
    paths: Vec<String>,
) -> Result<(), String> {
    let root = svc.staging_dir();
    let mut files = Vec::new();
    for p in &paths {
        let pb = std::path::PathBuf::from(p);
        if !pb.starts_with(&root) || pb.components().any(|c| c.as_os_str() == "..") || !pb.exists()
        {
            return Err("Недопустимый путь для перетаскивания.".into());
        }
        files.push(pb);
    }
    if files.is_empty() {
        return Err("Нечего перетаскивать.".into());
    }
    let icon = include_bytes!("../icons/32x32.png").to_vec();
    let dirs: Vec<std::path::PathBuf> = files
        .iter()
        .filter_map(|f| f.strip_prefix(&root).ok())
        .filter_map(|r| r.components().next())
        .map(|c| root.join(c.as_os_str()))
        .collect();
    // GTK wants its own window type, other platforms use the raw window handle
    #[cfg(target_os = "linux")]
    let handle = window.gtk_window().map_err(|e| e.to_string())?;
    #[cfg(not(target_os = "linux"))]
    let handle = window.clone();
    drag::start_drag(
        &handle,
        drag::DragItem::Files(files),
        drag::Image::Raw(icon),
        move |_result, _pos| {
            // Explorer may still be copying right after the drop: remove the staged copy a bit later
            // (anything left over is wiped at the next start).
            let dirs = dirs.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_secs(120));
                for d in dirs {
                    let _ = std::fs::remove_dir_all(d);
                }
            });
        },
        drag::Options::default(),
    )
    .map_err(|e| format!("Не удалось начать перетаскивание: {e}"))
}

fn fatal(msg: &str) -> ! {
    // GUI-subsystem builds have no console: show a native message box, otherwise the user sees nothing.
    #[cfg(windows)]
    {
        #[link(name = "user32")]
        extern "system" {
            fn MessageBoxW(hwnd: isize, text: *const u16, caption: *const u16, kind: u32) -> i32;
        }
        let wide = |s: &str| {
            s.encode_utf16()
                .chain(std::iter::once(0))
                .collect::<Vec<u16>>()
        };
        let (text, caption) = (wide(msg), wide("d256 Shell"));
        // MB_OK | MB_ICONERROR
        unsafe { MessageBoxW(0, text.as_ptr(), caption.as_ptr(), 0x10) };
    }
    eprintln!("{msg}");
    std::process::exit(1);
}

fn main() {
    // This same executable is the SSH_ASKPASS helper: it must act before anything else is initialised
    // (no data directory, no webview, no window).
    d256shell_core::terminal::askpass::run_helper_if_requested();

    let dirs = match paths::init() {
        Ok(d) => d,
        Err(e) => fatal(&e.to_string()),
    };
    let _ = paths::clear_staging(&dirs);
    if let Ok(l) = Logger::open(&dirs.logs()) {
        l.info("d256 Shell starting");
    }
    let webview_dir = dirs.webview();
    let service = Arc::new(Service::new(dirs));

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(service)
        .invoke_handler(tauri::generate_handler![api, term_stream, drag_out])
        .setup(move |app| {
            WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
                .title("d256 Shell")
                .inner_size(1200.0, 780.0)
                .min_inner_size(760.0, 460.0)
                // Windows: a custom title bar with tabs (Windows Terminal style) is drawn by the UI. Linux: the native
                // frame stays (an undecorated GTK window has no resize borders); the UI hides its own caption buttons.
                .decorations(cfg!(target_os = "linux"))
                .shadow(true)
                .data_directory(webview_dir.clone())
                .build()?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running d256 Shell");
}
