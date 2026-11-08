// YuiLauncher: starts, updates and removes the Yui apps on Windows and macOS.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod catalog;
mod download;
#[cfg(debug_assertions)]
mod fixture;
mod releases;
mod self_update;
mod service;
mod system;
#[cfg(test)]
mod testutil;
mod version;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;

use catalog::platform_apps;
use download::{Downloader, HttpDownloader};
use releases::{GitHub, Releases};
use self_update::{SelfUpdater, SelfView, Updater};
use serde::Serialize;
use service::{Service, StateView};
use std::path::Path;
#[cfg(target_os = "macos")]
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use tauri::webview::PageLoadEvent;
use tauri::{AppHandle, Emitter, Manager, RunEvent, State, WebviewUrl, WebviewWindowBuilder, WindowEvent};
use tauri_plugin_dialog::{DialogExt, MessageDialogKind};
use system::System;

const SUPPORT_URL: &str = "https://suzunayui.fanbox.cc/";
const CHECK_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);
const RUNNING_INTERVAL: Duration = Duration::from_secs(5);
const SELF_RETRY_AFTER: Duration = Duration::from_secs(30 * 60);
const CONFIRM_TIMEOUT: Duration = Duration::from_secs(60);
const CONFIRM_INTERVAL: Duration = Duration::from_secs(1);

async fn blocking<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(work).await.map_err(|error| error.to_string())
}

#[derive(Serialize)]
struct TaskResult {
    status: &'static str,
}

fn service(app: &AppHandle) -> Arc<Service> {
    app.state::<Arc<Service>>().inner().clone()
}

fn self_updater(app: &AppHandle) -> Arc<SelfUpdater> {
    app.state::<Arc<SelfUpdater>>().inner().clone()
}

#[tauri::command]
fn get_state(service: State<'_, Arc<Service>>) -> StateView {
    service.snapshot()
}

#[tauri::command]
async fn check(app: AppHandle) -> Result<(), String> {
    let (service, updater) = (service(&app), self_updater(&app));
    blocking(move || {
        thread::scope(|scope| {
            scope.spawn(|| service.check());
            scope.spawn(|| updater.check());
        });
    })
    .await
}

#[tauri::command]
async fn open_app(app: AppHandle, id: String) -> Result<(), String> {
    let service = service(&app);
    blocking(move || service.open(&id)).await?
}

#[tauri::command]
async fn install(app: AppHandle, id: String, close_running: bool) -> Result<TaskResult, String> {
    let service = service(&app);
    let status = blocking(move || service.install(&id, close_running)).await??;
    Ok(TaskResult { status })
}

#[tauri::command]
async fn uninstall(app: AppHandle, id: String, close_running: bool) -> Result<TaskResult, String> {
    let service = service(&app);
    let status = blocking(move || service.uninstall(&id, close_running)).await??;
    Ok(TaskResult { status })
}

#[tauri::command]
fn cancel(service: State<'_, Arc<Service>>, id: String) -> Result<(), String> {
    service.cancel(&id)
}

#[tauri::command]
fn set_order(service: State<'_, Arc<Service>>, ids: Vec<String>) -> Result<(), String> {
    service.set_order(&ids)
}

#[tauri::command]
fn set_hidden(service: State<'_, Arc<Service>>, id: String, hidden: bool) -> Result<(), String> {
    service.set_hidden(&id, hidden)
}

#[tauri::command]
fn open_release_page(service: State<'_, Arc<Service>>, id: String) -> Result<(), String> {
    match service.release_page_url(&id)? {
        Some(url) => service.open_url(&url),
        None => Ok(()),
    }
}

#[tauri::command]
fn open_support_page(service: State<'_, Arc<Service>>) -> Result<(), String> {
    service.open_url(SUPPORT_URL)
}

#[tauri::command]
fn get_self_update(updater: State<'_, Arc<SelfUpdater>>) -> SelfView {
    updater.snapshot()
}

#[tauri::command]
fn install_self_update(app: AppHandle) {
    if !service(&app).is_busy() && self_updater(&app).install() && !is_fixture() {
        app.exit(0);
    }
}

#[cfg(debug_assertions)]
#[tauri::command]
fn fixture_self_update(updater: State<'_, Arc<SelfUpdater>>, state: SelfView) {
    updater.fixture_set(state);
}

#[cfg(debug_assertions)]
#[tauri::command]
fn fixture_recorded(fixture: State<'_, Arc<fixture::Fixture>>) -> fixture::Recorded {
    fixture.recorded()
}

#[cfg(debug_assertions)]
fn is_fixture() -> bool {
    std::env::var_os("YUI_LAUNCHER_FIXTURE").is_some()
}

#[cfg(not(debug_assertions))]
fn is_fixture() -> bool {
    false
}

/// Starts the installer on its own, so it carries on after the launcher quits. /R makes it
/// start the new version, and /UPDATE keeps it from bringing back a removed desktop shortcut.
#[cfg(windows)]
fn start_detached(file: &Path, restart: bool) -> Result<(), String> {
    let args: &[&str] = if restart { &["/S", "/UPDATE", "/R"] } else { &["/S", "/UPDATE"] };
    Command::new(file).args(args).spawn().map(|_| ()).map_err(|error| error.to_string())
}

/// Swaps the running bundle for the new one: macOS keeps the old one open until it quits.
/// To restart, a shell waits for this process to end and then opens the new bundle.
#[cfg(target_os = "macos")]
fn start_detached(file: &Path, restart: bool) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|error| error.to_string())?;
    let bundle = exe.ancestors().nth(3).filter(|path| path.extension().is_some_and(|extension| extension == "app"));
    let bundle = bundle.ok_or("YuiLauncher.app から起動していません")?.to_path_buf();
    macos::install_from(file, "YuiLauncher.app", &bundle)?;
    if restart {
        let log = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_default()
            .join("Library/Logs/com.suzunayui.yuilauncher/restart.log");
        let script = "exec >>\"$3\" 2>&1; echo \"$(date) waiting for $1\"; \
            while kill -0 \"$1\" 2>/dev/null; do sleep 0.2; done; \
            echo \"$(date) opening $2\"; open \"$2\"; echo \"$(date) open exited with $?\"; \
            sleep 3; if ! pgrep -f \"$2/Contents/MacOS/\" >/dev/null; then \
            echo \"$(date) not running, opening a new instance\"; open -n \"$2\"; echo \"$(date) open -n exited with $?\"; fi";
        Command::new("sh")
            .args(["-c", script, "sh", &std::process::id().to_string()])
            .arg(&bundle)
            .arg(&log)
            .stdin(std::process::Stdio::null())
            .spawn()
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

/// For reproducing the restart in a debug build without a newer release:
/// open --env YUI_LAUNCHER_RESTART_PROBE=<downloaded zip> YuiLauncher.app
#[cfg(all(debug_assertions, target_os = "macos"))]
fn restart_probe(app: &AppHandle) {
    let Some(zip) = std::env::var_os("YUI_LAUNCHER_RESTART_PROBE") else { return };
    let handle = app.clone();
    thread::spawn(move || {
        thread::sleep(Duration::from_secs(3));
        match start_detached(Path::new(&zip), true) {
            Ok(()) => handle.exit(0),
            Err(error) => eprintln!("restart probe failed: {error}"),
        }
    });
}

#[cfg(windows)]
fn real_system() -> Arc<dyn System> {
    Arc::new(windows::RealSystem)
}

#[cfg(target_os = "macos")]
fn real_system() -> Arc<dyn System> {
    Arc::new(macos::MacSystem::new())
}

struct Backend {
    system: Arc<dyn System>,
    releases: Arc<dyn Releases>,
    downloader: Arc<dyn Downloader>,
}

fn real_backend() -> Backend {
    let releases = GitHub::new().expect("failed to set up TLS");
    Backend { system: real_system(), releases: Arc::new(releases), downloader: Arc::new(HttpDownloader) }
}

fn set_up(app: &AppHandle) -> tauri::Result<()> {
    #[cfg(debug_assertions)]
    let (backend, recorder) = if is_fixture() {
        let fake = Arc::new(fixture::Fixture::new());
        app.manage(fake.clone());
        (Backend { system: fake.clone(), releases: fake.clone(), downloader: fake.clone() }, Some(fake))
    } else {
        (real_backend(), None)
    };
    #[cfg(not(debug_assertions))]
    let backend = real_backend();

    let handle = app.clone();
    let apps_service = Service::new(
        platform_apps(),
        backend.system.clone(),
        backend.releases.clone(),
        backend.downloader.clone(),
        std::env::temp_dir(),
        Box::new(move |state| {
            let _ = handle.emit("state", state);
        }),
        CONFIRM_TIMEOUT,
        CONFIRM_INTERVAL,
    );
    // A fixture run keeps its order in memory, away from the real settings.
    let apps_service = if is_fixture() {
        apps_service
    } else {
        apps_service.with_settings(app.path().app_config_dir()?.join("settings.json"))
    };
    app.manage(Arc::new(apps_service));

    // A debug build isn't installed, so it has nothing of its own to update.
    let updater = if cfg!(debug_assertions) && !is_fixture() {
        None
    } else {
        #[cfg(debug_assertions)]
        let run_installer: self_update::RunInstaller = match recorder {
            Some(fake) => Box::new(move |_, _| {
                fake.record_self_install();
                Ok(())
            }),
            None => Box::new(start_detached),
        };
        #[cfg(not(debug_assertions))]
        let run_installer: self_update::RunInstaller = Box::new(start_detached);
        Some(Updater { releases: backend.releases.clone(), downloader: backend.downloader.clone(), run_installer })
    };
    let handle = app.clone();
    app.manage(SelfUpdater::new(
        app.package_info().version.to_string(),
        updater,
        std::env::temp_dir().join("YuiLauncherUpdate"),
        app.path().app_log_dir()?.join("self-update.log"),
        SELF_RETRY_AFTER,
        Box::new(move |state| {
            let _ = handle.emit("self-update", state);
        }),
    ));

    let window = WebviewWindowBuilder::new(app, "main", WebviewUrl::App("renderer/index.html".into()))
        .title("YuiLauncher")
        .inner_size(744.0, 563.0)
        .min_inner_size(520.0, 400.0)
        .visible(false)
        // Only the launcher's own page ever loads here.
        .on_navigation(|url| url.host_str() == Some("tauri.localhost") || url.scheme() == "tauri")
        .on_page_load(|window, payload| {
            if payload.event() == PageLoadEvent::Finished {
                let _ = window.show();
            }
        })
        .build()?;
    let handle = app.clone();
    window.on_window_event(move |event| match event {
        WindowEvent::Focused(true) => {
            let service = service(&handle);
            thread::spawn(move || service.refresh_local());
        }
        WindowEvent::CloseRequested { api, .. } if service(&handle).is_busy() => {
            api.prevent_close();
            handle
                .dialog()
                .message("ダウンロード中であれば、キャンセルしてから閉じられます。")
                .title("処理が終わるまでお待ちください。")
                .kind(MessageDialogKind::Info)
                .show(|_| {});
        }
        _ => {}
    });

    let handle = app.clone();
    thread::spawn(move || loop {
        let (service, updater) = (service(&handle), self_updater(&handle));
        thread::scope(|scope| {
            scope.spawn(|| service.check());
            scope.spawn(|| updater.check());
        });
        thread::sleep(CHECK_INTERVAL);
    });
    let handle = app.clone();
    thread::spawn(move || loop {
        thread::sleep(RUNNING_INTERVAL);
        service(&handle).refresh_running();
    });
    #[cfg(all(debug_assertions, target_os = "macos"))]
    restart_probe(app);
    Ok(())
}

macro_rules! handlers {
    ($($extra:ident),*) => {
        tauri::generate_handler![
            get_state,
            check,
            open_app,
            install,
            uninstall,
            cancel,
            set_order,
            set_hidden,
            open_release_page,
            open_support_page,
            get_self_update,
            install_self_update
            $(, $extra)*
        ]
    };
}

fn main() {
    let mut builder = tauri::Builder::default().plugin(tauri_plugin_dialog::init());
    // A fixture run must not be blocked by an installed launcher that is already open.
    if !is_fixture() {
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, _, _| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
        }));
    }
    #[cfg(debug_assertions)]
    let builder = builder.invoke_handler(handlers![fixture_self_update, fixture_recorded]);
    #[cfg(not(debug_assertions))]
    let builder = builder.invoke_handler(handlers![]);
    builder
        .setup(|app| {
            set_up(app.handle())?;
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("failed to start YuiLauncher")
        .run(|app, event| {
            if let RunEvent::Exit = event {
                self_updater(app).install_on_quit();
            }
        });
}
