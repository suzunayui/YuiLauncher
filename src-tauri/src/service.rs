//! Checks, installs, updates and removes the apps in the catalog.

use crate::catalog::AppDef;
use crate::download::{DownloadError, Downloader};
use crate::releases::{parse_checksums, Release, Releases};
use crate::version::compare_versions;
use crate::windows::{System, UninstallCommand};
use serde::Serialize;
use std::cmp::Ordering;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Task {
    pub phase: &'static str,
    pub percent: Option<u64>,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct AppView {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    pub icon: &'static str,
    pub installed: bool,
    pub version: Option<String>,
    pub can_open: bool,
    pub can_uninstall: bool,
    pub uninstall_note: &'static str,
    pub hidden: bool,
    pub running: bool,
    pub latest_version: Option<String>,
    pub has_release_page: bool,
    pub update_available: bool,
    pub check_error: Option<String>,
    pub task: Option<Task>,
    pub error: Option<String>,
    pub notice: Option<String>,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct StateView {
    pub checking: bool,
    pub last_checked_at: Option<u64>,
    pub apps: Vec<AppView>,
}

#[derive(Default)]
struct Entry {
    installed: bool,
    version: Option<String>,
    exe_path: Option<PathBuf>,
    uninstaller: Option<UninstallCommand>,
    running: bool,
    release: Option<Release>,
    check_error: Option<String>,
    task: Option<Task>,
    error: Option<String>,
    notice: Option<String>,
    cancel: Option<Arc<AtomicBool>>,
}

#[derive(Default)]
struct Store {
    entries: Vec<Entry>,
    checking: bool,
    last_checked_at: Option<u64>,
}

enum Failure {
    User(String),
    Other(String),
    Cancelled,
}

impl From<String> for Failure {
    fn from(message: String) -> Self {
        Failure::Other(message)
    }
}

pub const DONE: &str = "done";
pub const BUSY: &str = "busy";
pub const NEEDS_CLOSE: &str = "needs-close";
pub const FAILED: &str = "failed";
pub const CANCELLED: &str = "cancelled";

pub type OnChange = Box<dyn Fn(StateView) + Send + Sync>;

pub struct Service {
    apps: &'static [AppDef],
    system: Arc<dyn System>,
    releases: Arc<dyn Releases>,
    downloader: Arc<dyn Downloader>,
    temp_root: PathBuf,
    on_change: OnChange,
    confirm_timeout: Duration,
    confirm_interval: Duration,
    store: Mutex<Store>,
    check_done: Condvar,
    // Indexes into apps, in the order the page shows them.
    order: Mutex<Vec<usize>>,
    // By index into apps. Hidden apps are still checked, just not shown with the others.
    hidden: Mutex<Vec<bool>>,
    settings_file: Option<PathBuf>,
}

/// Puts the known ids first, in the given order, then any app the list doesn't name.
fn order_from(apps: &[AppDef], ids: &[String]) -> Vec<usize> {
    let mut order: Vec<usize> = Vec::new();
    for id in ids {
        if let Some(index) = apps.iter().position(|app| app.id == id) {
            if !order.contains(&index) {
                order.push(index);
            }
        }
    }
    let missing: Vec<usize> = (0..apps.len()).filter(|index| !order.contains(index)).collect();
    order.extend(missing);
    order
}

fn update_available(entry: &Entry) -> bool {
    let Some(release) = &entry.release else { return false };
    if !entry.installed {
        return true;
    }
    // An unreadable local version must still be repairable by installing.
    match entry.version.as_deref().and_then(|version| compare_versions(&release.version, version)) {
        Some(order) => order == Ordering::Greater,
        None => true,
    }
}

fn unique_dir(root: &Path) -> std::io::Result<PathBuf> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    loop {
        let next = NEXT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let path = root.join(format!("yui-launcher-update-{}-{next}", std::process::id()));
        match std::fs::create_dir(&path) {
            Ok(()) => return Ok(path),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
}

impl Service {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        apps: &'static [AppDef],
        system: Arc<dyn System>,
        releases: Arc<dyn Releases>,
        downloader: Arc<dyn Downloader>,
        temp_root: PathBuf,
        on_change: OnChange,
        confirm_timeout: Duration,
        confirm_interval: Duration,
    ) -> Self {
        Self {
            apps,
            system,
            releases,
            downloader,
            temp_root,
            on_change,
            confirm_timeout,
            confirm_interval,
            store: Mutex::new(Store { entries: apps.iter().map(|_| Entry::default()).collect(), ..Store::default() }),
            check_done: Condvar::new(),
            order: Mutex::new((0..apps.len()).collect()),
            hidden: Mutex::new(vec![false; apps.len()]),
            settings_file: None,
        }
    }

    /// Keeps the order of the apps and which are hidden in this file. A missing or broken
    /// file means the catalog's order with nothing hidden.
    pub fn with_settings(mut self, file: PathBuf) -> Self {
        let settings = std::fs::read_to_string(&file)
            .ok()
            .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
            .unwrap_or_default();
        let ids = |key: &str| -> Vec<String> { serde_json::from_value(settings[key].clone()).unwrap_or_default() };
        *self.order.get_mut().unwrap() = order_from(self.apps, &ids("order"));
        let hidden = ids("hidden");
        *self.hidden.get_mut().unwrap() = self.apps.iter().map(|app| hidden.iter().any(|id| id == app.id)).collect();
        self.settings_file = Some(file);
        self
    }

    fn save_settings(&self) -> Result<(), String> {
        let Some(file) = &self.settings_file else { return Ok(()) };
        let order: Vec<&str> = self.order.lock().unwrap().iter().map(|index| self.apps[*index].id).collect();
        let hidden: Vec<&str> = {
            let hidden = self.hidden.lock().unwrap();
            self.apps.iter().zip(hidden.iter()).filter(|(_, hidden)| **hidden).map(|(app, _)| app.id).collect()
        };
        let write = || -> std::io::Result<()> {
            if let Some(parent) = file.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(file, serde_json::json!({ "order": order, "hidden": hidden }).to_string())
        };
        write().map_err(|error| format!("設定を保存できませんでした（{error}）"))
    }

    pub fn set_order(&self, ids: &[String]) -> Result<(), String> {
        *self.order.lock().unwrap() = order_from(self.apps, ids);
        self.changed();
        self.save_settings()
    }

    pub fn set_hidden(&self, id: &str, hidden: bool) -> Result<(), String> {
        let index = self.index_of(id)?;
        self.hidden.lock().unwrap()[index] = hidden;
        self.changed();
        self.save_settings()
    }

    fn index_of(&self, id: &str) -> Result<usize, String> {
        self.apps.iter().position(|app| app.id == id).ok_or_else(|| "不明なアプリです".to_string())
    }

    pub fn snapshot(&self) -> StateView {
        let order = self.order.lock().unwrap().clone();
        let hidden = self.hidden.lock().unwrap().clone();
        let store = self.store.lock().unwrap();
        StateView {
            checking: store.checking,
            last_checked_at: store.last_checked_at,
            apps: order
                .iter()
                .map(|index| (&self.apps[*index], &store.entries[*index], hidden[*index]))
                .map(|(app, entry, hidden)| AppView {
                    hidden,
                    id: app.id,
                    name: app.name,
                    description: app.description,
                    icon: app.icon,
                    installed: entry.installed,
                    version: entry.version.clone(),
                    can_open: entry.exe_path.is_some(),
                    can_uninstall: entry.uninstaller.is_some(),
                    uninstall_note: app.uninstall_note,
                    running: entry.running,
                    latest_version: entry.release.as_ref().map(|release| release.version.clone()),
                    has_release_page: entry.release.as_ref().is_some_and(|release| !release.page_url.is_empty()),
                    update_available: update_available(entry),
                    check_error: entry.check_error.clone(),
                    task: entry.task.clone(),
                    error: entry.error.clone(),
                    notice: entry.notice.clone(),
                })
                .collect(),
        }
    }

    fn changed(&self) {
        (self.on_change)(self.snapshot());
    }

    fn with<T>(&self, index: usize, change: impl FnOnce(&mut Entry) -> T) -> T {
        change(&mut self.store.lock().unwrap().entries[index])
    }

    fn exe_path(&self, index: usize) -> Option<PathBuf> {
        self.with(index, |entry| entry.exe_path.clone())
    }

    fn read_local(&self, index: usize) -> Result<(), String> {
        let installed = self.system.read_installed(&self.apps[index])?;
        self.with(index, |entry| {
            entry.installed = installed.is_some();
            entry.version = installed.as_ref().and_then(|info| info.version.clone());
            entry.exe_path = installed.as_ref().and_then(|info| info.exe_path.clone());
            entry.uninstaller = installed.as_ref().and_then(|info| info.uninstaller.clone());
        });
        Ok(())
    }

    fn is_running(&self, index: usize) -> Result<bool, String> {
        self.system.is_running(&self.apps[index], self.exe_path(index).as_deref())
    }

    fn refresh_quietly(&self) {
        thread::scope(|scope| {
            for index in 0..self.apps.len() {
                scope.spawn(move || {
                    if self.with(index, |entry| entry.task.is_some()) {
                        return;
                    }
                    let result = self.read_local(index).and_then(|()| self.is_running(index));
                    match result {
                        Ok(running) => self.with(index, |entry| entry.running = running),
                        Err(error) => eprintln!("Failed to read {}: {error}", self.apps[index].id),
                    }
                });
            }
        });
    }

    pub fn refresh_local(&self) {
        self.refresh_quietly();
        self.changed();
    }

    pub fn refresh_running(&self) {
        let mut dirty = false;
        for index in 0..self.apps.len() {
            if self.with(index, |entry| entry.task.is_some()) {
                continue;
            }
            match self.is_running(index) {
                Ok(running) => {
                    dirty |= self.with(index, |entry| std::mem::replace(&mut entry.running, running) != running);
                }
                Err(error) => eprintln!("Failed to check {}: {error}", self.apps[index].id),
            }
        }
        if dirty {
            self.changed();
        }
    }

    fn check_remote(&self) {
        thread::scope(|scope| {
            for index in 0..self.apps.len() {
                scope.spawn(move || {
                    let result = self.releases.latest(&self.apps[index]);
                    self.with(index, |entry| match result {
                        Ok(Some(release)) => {
                            entry.release = Some(release);
                            entry.check_error = None;
                        }
                        Ok(None) => {
                            entry.release = None;
                            entry.check_error = Some("Windows版のリリースが見つかりませんでした。".to_string());
                        }
                        // Keeps the last known release so an offline check doesn't hide an update.
                        Err(message) => entry.check_error = Some(message),
                    });
                });
            }
        });
    }

    /// A check already under way is waited for rather than started again.
    pub fn check(&self) {
        {
            let mut store = self.store.lock().unwrap();
            if store.checking {
                let _unused = self.check_done.wait_while(store, |store| store.checking).unwrap();
                return;
            }
            store.checking = true;
        }
        self.changed();
        thread::scope(|scope| {
            scope.spawn(|| self.refresh_quietly());
            scope.spawn(|| self.check_remote());
        });
        {
            let mut store = self.store.lock().unwrap();
            store.checking = false;
            store.last_checked_at = SystemTime::now().duration_since(UNIX_EPOCH).ok().map(|time| time.as_millis() as u64);
        }
        self.check_done.notify_all();
        self.changed();
    }

    fn set_task(&self, index: usize, phase: &'static str, percent: Option<u64>) {
        self.with(index, |entry| entry.task = Some(Task { phase, percent }));
        self.changed();
    }

    fn verified_installer(&self, index: usize, release: &Release, work_dir: &Path) -> Result<PathBuf, Failure> {
        let dest = work_dir.join(&release.installer.name);
        let cancel = Arc::new(AtomicBool::new(false));
        self.with(index, |entry| entry.cancel = Some(cancel.clone()));
        self.set_task(index, "downloading", Some(0));
        let mut last_percent = Some(0);
        let expected_size = Some(release.installer.size).filter(|size| *size > 0);
        let result = self.downloader.download(&release.installer.url, &dest, expected_size, &cancel, &mut |received, total| {
            let percent = total.map(|total| received.saturating_mul(100) / total.max(1));
            if percent != last_percent {
                last_percent = percent;
                self.set_task(index, "downloading", percent);
            }
        });
        self.with(index, |entry| entry.cancel = None);
        let sha256 = match result {
            Ok(sha256) => sha256,
            Err(DownloadError::Cancelled) => return Err(Failure::Cancelled),
            Err(DownloadError::Failed(message)) => return Err(Failure::Other(message)),
        };

        self.set_task(index, "verifying", None);
        let text = self.releases.fetch_text(&release.checksums.url).map_err(Failure::User)?;
        let checksums = parse_checksums(&text);
        let Some(expected) = checksums.get(&release.installer.name) else {
            return Err(Failure::User("チェックサムにこのインストーラーの記載がありません。".to_string()));
        };
        if *expected != sha256 {
            return Err(Failure::User("ファイルの検証に失敗しました。もう一度お試しください。".to_string()));
        }
        Ok(dest)
    }

    fn wait_for_version(&self, index: usize, version: &str) -> Result<(), Failure> {
        let deadline = Instant::now() + self.confirm_timeout;
        loop {
            self.read_local(index)?;
            let installed = self.with(index, |entry| entry.version.clone());
            if installed.is_some_and(|installed| matches!(compare_versions(&installed, version), Some(Ordering::Greater | Ordering::Equal))) {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(Failure::User("インストール後のバージョンを確認できませんでした。".to_string()));
            }
            thread::sleep(self.confirm_interval);
        }
    }

    fn relaunch_if_closed(&self, index: usize, closed: bool) {
        if !closed {
            return;
        }
        let result = self.read_local(index).and_then(|()| match self.exe_path(index) {
            Some(path) => self.system.launch(&path),
            None => Ok(()),
        });
        if let Err(error) = result {
            eprintln!("Failed to relaunch {}: {error}", self.apps[index].id);
        }
    }

    fn close_for_task(&self, index: usize) -> Result<(), Failure> {
        let exe = self.exe_path(index).unwrap_or_default();
        if self.system.close_app(&self.apps[index], &exe)? {
            return Ok(());
        }
        Err(Failure::User(format!(
            "{}を閉じられませんでした。アプリ側で閉じてから、もう一度お試しください。",
            self.apps[index].name
        )))
    }

    fn run_install(&self, index: usize, release: &Release, close_running: bool, work_dir: &Path, closed: &mut bool) -> Result<(), Failure> {
        let installer = self.verified_installer(index, release, work_dir)?;
        if self.is_running(index)? {
            if !close_running {
                return Err(Failure::User(format!("{}が起動されたため中止しました。もう一度お試しください。", self.apps[index].name)));
            }
            self.set_task(index, "closing", None);
            self.close_for_task(index)?;
            *closed = true;
        }
        self.set_task(index, "installing", None);
        let app = &self.apps[index];
        let mut args = app.silent_args.to_vec();
        if self.with(index, |entry| entry.installed) {
            args.extend_from_slice(app.update_args);
        }
        self.system.run_installer(&installer, &args)?;
        self.set_task(index, "confirming", None);
        self.wait_for_version(index, &release.version)
    }

    fn finish_task(&self, index: usize) {
        self.with(index, |entry| {
            entry.cancel = None;
            entry.task = None;
        });
        let _ = self.read_local(index);
        if let Ok(running) = self.is_running(index) {
            self.with(index, |entry| entry.running = running);
        }
        self.changed();
    }

    /// Claims the app for one task. None means another task holds it.
    fn begin(&self, index: usize, phase: &'static str) -> bool {
        self.with(index, |entry| {
            if entry.task.is_some() {
                return false;
            }
            entry.task = Some(Task { phase, percent: None });
            entry.error = None;
            entry.notice = None;
            true
        })
    }

    /// Asks whether the app may be closed, giving the claim back if it has to wait for an answer.
    fn check_may_close(&self, index: usize, close_running: bool) -> Result<Option<&'static str>, String> {
        let running = match self.is_running(index) {
            Ok(running) => running,
            Err(error) => {
                self.with(index, |entry| entry.task = None);
                return Err(error);
            }
        };
        self.with(index, |entry| entry.running = running);
        if running && !close_running {
            self.with(index, |entry| entry.task = None);
            self.changed();
            return Ok(Some(NEEDS_CLOSE));
        }
        Ok(None)
    }

    /// Downloads and verifies first so the app stays open until the last moment.
    pub fn install(&self, id: &str, close_running: bool) -> Result<&'static str, String> {
        let index = self.index_of(id)?;
        if !self.begin(index, "downloading") {
            return Ok(BUSY);
        }
        let Some(release) = self.with(index, |entry| entry.release.clone()) else {
            self.with(index, |entry| entry.task = None);
            return Err("インストールできるリリースがありません。".to_string());
        };
        if let Some(status) = self.check_may_close(index, close_running)? {
            return Ok(status);
        }

        let work_dir = unique_dir(&self.temp_root);
        let mut closed = false;
        let result = match &work_dir {
            Ok(dir) => self.run_install(index, &release, close_running, dir, &mut closed),
            Err(error) => Err(Failure::Other(error.to_string())),
        };
        let status = match result {
            Ok(()) => {
                self.with(index, |entry| entry.notice = Some(format!("{}をインストールしました。", release.version)));
                DONE
            }
            Err(Failure::Cancelled) => CANCELLED,
            Err(Failure::User(message)) => {
                self.with(index, |entry| entry.error = Some(message));
                FAILED
            }
            Err(Failure::Other(message)) => {
                self.with(index, |entry| entry.error = Some(format!("更新に失敗しました（{message}）")));
                FAILED
            }
        };
        self.relaunch_if_closed(index, closed);
        if let Ok(dir) = &work_dir {
            let _ = std::fs::remove_dir_all(dir);
        }
        self.finish_task(index);
        Ok(status)
    }

    fn wait_for_removal(&self, index: usize) -> Result<(), Failure> {
        let deadline = Instant::now() + self.confirm_timeout;
        loop {
            self.read_local(index)?;
            if !self.with(index, |entry| entry.installed) {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(Failure::User("アンインストールの完了を確認できませんでした。".to_string()));
            }
            thread::sleep(self.confirm_interval);
        }
    }

    fn run_uninstall(&self, index: usize, closed: &mut bool) -> Result<(), Failure> {
        self.read_local(index)?;
        let Some(uninstaller) = self.with(index, |entry| entry.uninstaller.clone()) else {
            return Err(Failure::User("アンインストーラーが見つかりません。".to_string()));
        };
        if self.with(index, |entry| entry.running) {
            self.set_task(index, "closing", None);
            self.close_for_task(index)?;
            *closed = true;
        }
        self.set_task(index, "uninstalling", None);
        self.system.run_uninstaller(&uninstaller)?;
        self.set_task(index, "removing", None);
        self.wait_for_removal(index)
    }

    pub fn uninstall(&self, id: &str, close_running: bool) -> Result<&'static str, String> {
        let index = self.index_of(id)?;
        if !self.begin(index, "uninstalling") {
            return Ok(BUSY);
        }
        if let Some(status) = self.check_may_close(index, close_running)? {
            return Ok(status);
        }
        let mut closed = false;
        let status = match self.run_uninstall(index, &mut closed) {
            Ok(()) => {
                self.with(index, |entry| entry.notice = Some(format!("{}をアンインストールしました。", self.apps[index].name)));
                DONE
            }
            Err(failure) => {
                let message = match failure {
                    Failure::User(message) => message,
                    Failure::Other(message) => format!("アンインストールに失敗しました（{message}）"),
                    Failure::Cancelled => String::new(),
                };
                self.with(index, |entry| entry.error = Some(message));
                self.relaunch_if_closed(index, closed);
                FAILED
            }
        };
        self.finish_task(index);
        Ok(status)
    }

    pub fn cancel(&self, id: &str) -> Result<(), String> {
        let index = self.index_of(id)?;
        self.with(index, |entry| {
            if let Some(cancel) = &entry.cancel {
                cancel.store(true, std::sync::atomic::Ordering::SeqCst);
            }
        });
        Ok(())
    }

    pub fn open(&self, id: &str) -> Result<(), String> {
        let index = self.index_of(id)?;
        if self.with(index, |entry| entry.task.is_some()) {
            return Ok(());
        }
        self.with(index, |entry| entry.error = None);
        let error = match self.exe_path(index) {
            None => Some(format!("{}の実行ファイルが見つかりません。", self.apps[index].name)),
            Some(path) => self.system.launch(&path).err().map(|error| format!("起動できませんでした（{error}）")),
        };
        self.with(index, |entry| {
            if error.is_none() {
                entry.running = true;
            }
            entry.error = error;
        });
        self.changed();
        Ok(())
    }

    pub fn release_page_url(&self, id: &str) -> Result<Option<String>, String> {
        let index = self.index_of(id)?;
        let url = self.with(index, |entry| entry.release.as_ref().map(|release| release.page_url.clone()));
        let prefix = format!("https://github.com/{}/", self.apps[index].repo);
        Ok(url.filter(|url| url.starts_with(&prefix)))
    }

    pub fn is_busy(&self) -> bool {
        self.store.lock().unwrap().entries.iter().any(|entry| entry.task.is_some())
    }

    pub fn open_url(&self, url: &str) -> Result<(), String> {
        self.system.open_url(url)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::download::Progress;
    use crate::releases::Asset;
    use crate::testutil::{temp_dir, TempDir};
    use crate::windows::Installed;
    use std::sync::atomic::Ordering as Atomic;
    use std::sync::mpsc;

    const HASH: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const OTHER: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    static DEMO: [AppDef; 1] = [AppDef {
        id: "demo",
        name: "Demo",
        description: "",
        repo: "owner/Demo",
        registry_key: "Demo",
        exe_name: "Demo.exe",
        installer_prefix: "Demo-Setup-",
        installer_suffix: ".exe",
        silent_args: &["/S"],
        update_args: &["/UPDATE"],
        uninstall_args: &["/S"],
        uninstall_note: "",
        icon: "",
    }];

    #[derive(Default)]
    struct World {
        version: Option<String>,
        running: bool,
        latest: Option<String>,
        log: Vec<String>,
        installer_ran: bool,
        checksums: Option<String>,
        online: bool,
        page_url: Option<String>,
        close_refuses: bool,
        no_uninstaller: bool,
        installer_error: Option<String>,
        installer_does_nothing: bool,
        uninstaller_error: Option<String>,
        uninstaller_does_nothing: bool,
        opens_during_download: bool,
        exe_paths: Vec<String>,
    }

    struct Fake {
        world: Mutex<World>,
        // A download waits for this when set, to hold a task open.
        gate: Mutex<Option<mpsc::Receiver<()>>>,
        started: Mutex<Option<mpsc::Sender<()>>>,
        wait_for_cancel: AtomicBool,
    }

    impl Fake {
        fn log(&self, line: String) {
            self.world.lock().unwrap().log.push(line);
        }
    }

    impl System for Fake {
        fn read_installed(&self, _: &AppDef) -> Result<Option<Installed>, String> {
            let world = self.world.lock().unwrap();
            Ok(world.version.clone().map(|version| Installed {
                version: Some(version),
                exe_path: Some(PathBuf::from(r"C:\Demo\Demo.exe")),
                uninstaller: (!world.no_uninstaller).then(|| UninstallCommand { file: PathBuf::from(r"C:\Demo\unins.exe"), args: vec!["/S".to_string()] }),
            }))
        }
        fn is_running(&self, _: &AppDef, exe_path: Option<&Path>) -> Result<bool, String> {
            let mut world = self.world.lock().unwrap();
            if let Some(path) = exe_path {
                world.exe_paths.push(path.display().to_string());
            }
            Ok(world.running)
        }
        fn close_app(&self, _: &AppDef, exe_path: &Path) -> Result<bool, String> {
            let mut world = self.world.lock().unwrap();
            world.exe_paths.push(exe_path.display().to_string());
            if world.close_refuses {
                return Ok(false);
            }
            world.log.push("close".to_string());
            world.running = false;
            Ok(true)
        }
        fn launch(&self, _: &Path) -> Result<(), String> {
            let mut world = self.world.lock().unwrap();
            world.log.push("launch".to_string());
            world.running = true;
            Ok(())
        }
        fn run_installer(&self, file: &Path, args: &[&str]) -> Result<(), String> {
            let mut world = self.world.lock().unwrap();
            if let Some(error) = world.installer_error.clone() {
                return Err(error);
            }
            assert!(file.exists(), "installer exists when it runs");
            world.log.push(format!("install:{}:{}", file.file_name().unwrap().to_string_lossy(), args.join(" ")));
            world.installer_ran = true;
            if !world.installer_does_nothing {
                world.version = world.latest.clone();
            }
            Ok(())
        }
        fn run_uninstaller(&self, command: &UninstallCommand) -> Result<(), String> {
            let mut world = self.world.lock().unwrap();
            if let Some(error) = world.uninstaller_error.clone() {
                return Err(error);
            }
            world.log.push(format!("uninstall:{}:{}", command.file.file_name().unwrap().to_string_lossy(), command.args.join(" ")));
            if !world.uninstaller_does_nothing {
                world.version = None;
            }
            Ok(())
        }
        fn open_url(&self, url: &str) -> Result<(), String> {
            self.log(format!("url:{url}"));
            Ok(())
        }
    }

    impl Releases for Fake {
        fn latest(&self, _: &AppDef) -> Result<Option<Release>, String> {
            let world = self.world.lock().unwrap();
            if !world.online {
                return Err("オフライン".to_string());
            }
            Ok(world.latest.clone().map(|version| Release {
                version,
                page_url: world.page_url.clone().unwrap_or_else(|| "https://github.com/owner/Demo/releases/tag/x".to_string()),
                installer: Asset { name: "Demo-Setup.exe".to_string(), size: 1, url: "https://github.com/owner/Demo/installer".to_string() },
                checksums: Asset { name: "SHA256SUMS.txt".to_string(), size: 1, url: "https://github.com/owner/Demo/sums".to_string() },
            }))
        }
        fn fetch_text(&self, _: &str) -> Result<String, String> {
            Ok(self.world.lock().unwrap().checksums.clone().unwrap_or_else(|| format!("{HASH}  Demo-Setup.exe\n")))
        }
    }

    impl Downloader for Fake {
        fn download(&self, _: &str, dest: &Path, _: Option<u64>, cancel: &AtomicBool, on_progress: Progress) -> Result<String, DownloadError> {
            if self.wait_for_cancel.load(Atomic::SeqCst) {
                if let Some(started) = self.started.lock().unwrap().take() {
                    let _ = started.send(());
                }
                while !cancel.load(Atomic::SeqCst) {
                    thread::sleep(Duration::from_millis(1));
                }
                return Err(DownloadError::Cancelled);
            }
            self.log("download".to_string());
            std::fs::write(dest, "x").unwrap();
            on_progress(1, Some(1));
            if let Some(started) = self.started.lock().unwrap().take() {
                let _ = started.send(());
            }
            let gate = self.gate.lock().unwrap().take();
            if let Some(gate) = gate {
                let _ = gate.recv();
            }
            let mut world = self.world.lock().unwrap();
            if world.opens_during_download {
                world.running = true;
            }
            Ok(HASH.to_string())
        }
    }

    struct Setup {
        service: Arc<Service>,
        fake: Arc<Fake>,
        temp: TempDir,
    }

    fn setup(change: impl FnOnce(&mut World)) -> Setup {
        let mut world = World {
            version: Some("1.0.0".to_string()),
            latest: Some("1.1.0".to_string()),
            online: true,
            ..World::default()
        };
        change(&mut world);
        let fake = Arc::new(Fake {
            world: Mutex::new(world),
            gate: Mutex::new(None),
            started: Mutex::new(None),
            wait_for_cancel: AtomicBool::new(false),
        });
        let temp = temp_dir();
        let service = Arc::new(Service::new(
            &DEMO,
            fake.clone(),
            fake.clone(),
            fake.clone(),
            temp.to_path_buf(),
            Box::new(|_| {}),
            Duration::from_millis(20),
            Duration::from_millis(1),
        ));
        service.check();
        Setup { service, fake, temp }
    }

    impl Setup {
        fn state(&self) -> AppView {
            self.service.snapshot().apps.remove(0)
        }
        fn log(&self) -> Vec<String> {
            self.fake.world.lock().unwrap().log.clone()
        }
        fn leftovers(&self) -> usize {
            std::fs::read_dir(&*self.temp).unwrap().count()
        }
        fn hold_download(&self) -> (mpsc::Receiver<()>, mpsc::Sender<()>) {
            let (started_tx, started_rx) = mpsc::channel();
            let (gate_tx, gate_rx) = mpsc::channel();
            *self.fake.started.lock().unwrap() = Some(started_tx);
            *self.fake.gate.lock().unwrap() = Some(gate_rx);
            (started_rx, gate_tx)
        }
    }

    #[test]
    fn shows_an_update_only_when_the_release_is_newer() {
        assert!(setup(|_| {}).state().update_available);
        assert!(!setup(|world| world.latest = Some("1.0.0".to_string())).state().update_available);
        assert!(!setup(|world| world.version = Some("2.0.0".to_string())).state().update_available);
    }

    #[test]
    fn an_unreadable_version_can_still_be_repaired() {
        assert!(setup(|world| world.version = Some("unknown".to_string())).state().update_available);
    }

    #[test]
    fn offers_install_when_the_app_is_missing() {
        let state = setup(|world| world.version = None).state();
        assert!(!state.installed);
        assert!(state.update_available);
        assert!(!state.can_open);
    }

    #[test]
    fn updates_a_closed_app_without_launching_it_then_removes_the_installer() {
        let setup = setup(|_| {});
        assert_eq!(setup.service.install("demo", false), Ok(DONE));
        assert_eq!(setup.log(), ["download", "install:Demo-Setup.exe:/S /UPDATE"]);
        let state = setup.state();
        assert_eq!(state.version.as_deref(), Some("1.1.0"));
        assert!(!state.update_available);
        assert_eq!(state.task, None);
        assert_eq!(setup.leftovers(), 0);
    }

    #[test]
    fn asks_before_closing_a_running_app_and_does_nothing_until_confirmed() {
        let setup = setup(|world| world.running = true);
        assert_eq!(setup.service.install("demo", false), Ok(NEEDS_CLOSE));
        assert!(setup.log().is_empty());
        assert_eq!(setup.state().task, None);
    }

    #[test]
    fn closes_only_after_verification_then_starts_the_app_again() {
        let setup = setup(|world| world.running = true);
        assert_eq!(setup.service.install("demo", true), Ok(DONE));
        assert_eq!(setup.log(), ["download", "close", "install:Demo-Setup.exe:/S /UPDATE", "launch"]);
    }

    #[test]
    fn a_first_install_runs_without_the_update_arguments() {
        let setup = setup(|world| world.version = None);
        assert_eq!(setup.service.install("demo", false), Ok(DONE));
        assert_eq!(setup.log(), ["download", "install:Demo-Setup.exe:/S"]);
    }

    #[test]
    fn a_bad_checksum_never_closes_the_app_or_runs_the_installer() {
        let setup = setup(|world| {
            world.running = true;
            world.checksums = Some(format!("{OTHER}  Demo-Setup.exe\n"));
        });
        assert_eq!(setup.service.install("demo", true), Ok(FAILED));
        assert_eq!(setup.log(), ["download"]);
        assert!(setup.state().error.unwrap().contains("検証に失敗"));
        assert_eq!(setup.leftovers(), 0);
    }

    #[test]
    fn a_checksum_file_without_this_installer_is_a_failure() {
        let setup = setup(|world| world.checksums = Some(format!("{HASH}  Other.exe\n")));
        assert_eq!(setup.service.install("demo", false), Ok(FAILED));
        assert_eq!(setup.log(), ["download"]);
        assert!(setup.state().error.unwrap().contains("記載がありません"));
    }

    #[test]
    fn stops_without_force_closing_when_the_app_will_not_close() {
        let setup = setup(|world| {
            world.running = true;
            world.close_refuses = true;
        });
        assert_eq!(setup.service.install("demo", true), Ok(FAILED));
        assert_eq!(setup.log(), ["download"]);
        assert!(setup.state().error.unwrap().contains("閉じられませんでした"));
    }

    #[test]
    fn stops_if_the_app_was_opened_during_an_unconfirmed_download() {
        let setup = setup(|world| world.opens_during_download = true);
        assert_eq!(setup.service.install("demo", false), Ok(FAILED));
        assert!(setup.state().error.unwrap().contains("中止"));
        assert!(!setup.fake.world.lock().unwrap().installer_ran);
    }

    #[test]
    fn reopens_the_old_version_when_the_installer_fails() {
        let setup = setup(|world| {
            world.running = true;
            world.installer_error = Some("インストーラーが終了コード2で終了しました".to_string());
        });
        assert_eq!(setup.service.install("demo", true), Ok(FAILED));
        assert_eq!(setup.log(), ["download", "close", "launch"]);
        let state = setup.state();
        assert!(state.error.unwrap().contains("終了コード2"));
        assert_eq!(state.version.as_deref(), Some("1.0.0"));
    }

    #[test]
    fn reports_failure_when_the_version_does_not_change_after_installing() {
        let setup = setup(|world| world.installer_does_nothing = true);
        assert_eq!(setup.service.install("demo", false), Ok(FAILED));
        assert!(setup.state().error.unwrap().contains("確認できませんでした"));
    }

    #[test]
    fn cancelling_a_download_is_not_an_error() {
        let setup = setup(|_| {});
        setup.fake.wait_for_cancel.store(true, Atomic::SeqCst);
        let (started_tx, started_rx) = mpsc::channel();
        *setup.fake.started.lock().unwrap() = Some(started_tx);
        let service = setup.service.clone();
        let install = thread::spawn(move || service.install("demo", false));
        started_rx.recv().unwrap();
        setup.service.cancel("demo").unwrap();
        assert_eq!(install.join().unwrap(), Ok(CANCELLED));
        let state = setup.state();
        assert_eq!(state.error, None);
        assert_eq!(state.task, None);
        assert!(setup.log().is_empty());
        assert_eq!(setup.leftovers(), 0);
    }

    #[test]
    fn refuses_a_second_install_and_an_uninstall_while_one_is_running() {
        let setup = setup(|_| {});
        let (started, gate) = setup.hold_download();
        let service = setup.service.clone();
        let first = thread::spawn(move || service.install("demo", false));
        started.recv().unwrap();
        assert!(setup.service.is_busy());
        assert_eq!(setup.service.install("demo", false), Ok(BUSY));
        assert_eq!(setup.service.uninstall("demo", false), Ok(BUSY));
        gate.send(()).unwrap();
        assert_eq!(first.join().unwrap(), Ok(DONE));
        assert!(!setup.service.is_busy());
    }

    #[test]
    fn keeps_the_last_known_release_when_a_later_check_fails() {
        let setup = setup(|_| {});
        setup.fake.world.lock().unwrap().online = false;
        setup.service.check();
        let state = setup.state();
        assert!(state.update_available);
        assert_eq!(state.check_error.as_deref(), Some("オフライン"));
    }

    #[test]
    fn never_exposes_download_urls_or_local_paths_to_the_page() {
        let setup = setup(|_| {});
        let serialized = serde_json::to_string(&setup.service.snapshot()).unwrap();
        for secret in ["https://github.com/owner/Demo/installer", "Demo.exe", "unins.exe"] {
            assert!(!serialized.contains(secret), "{secret}");
        }
    }

    #[test]
    fn checks_and_closes_the_app_by_its_installed_path() {
        let setup = setup(|world| world.running = true);
        assert_eq!(setup.service.uninstall("demo", true), Ok(DONE));
        assert!(setup.fake.world.lock().unwrap().exe_paths.iter().any(|path| path == r"C:\Demo\Demo.exe"));
        assert!(!setup.state().running, "nothing is running once the app is gone");
    }

    #[test]
    fn uninstalls_a_closed_app_without_starting_anything() {
        let setup = setup(|world| world.latest = Some("1.0.0".to_string()));
        assert!(setup.state().can_uninstall);
        assert_eq!(setup.service.uninstall("demo", false), Ok(DONE));
        assert_eq!(setup.log(), ["uninstall:unins.exe:/S"]);
        let state = setup.state();
        assert!(!state.installed);
        assert!(!state.can_uninstall);
        assert_eq!(state.task, None);
        assert!(state.update_available, "it can be installed again");
        assert!(state.notice.unwrap().contains("アンインストールしました"));
    }

    #[test]
    fn does_not_touch_a_running_app_without_confirmation() {
        let setup = setup(|world| world.running = true);
        assert_eq!(setup.service.uninstall("demo", false), Ok(NEEDS_CLOSE));
        assert!(setup.log().is_empty());
        assert!(setup.state().installed);
    }

    #[test]
    fn closes_a_running_app_first_and_does_not_start_it_again() {
        let setup = setup(|world| world.running = true);
        assert_eq!(setup.service.uninstall("demo", true), Ok(DONE));
        assert_eq!(setup.log(), ["close", "uninstall:unins.exe:/S"]);
        assert!(!setup.state().running);
    }

    #[test]
    fn stops_the_uninstall_when_the_app_will_not_close() {
        let setup = setup(|world| {
            world.running = true;
            world.close_refuses = true;
        });
        assert_eq!(setup.service.uninstall("demo", true), Ok(FAILED));
        assert!(setup.log().is_empty());
        let state = setup.state();
        assert!(state.error.unwrap().contains("閉じられませんでした"));
        assert!(state.installed);
    }

    #[test]
    fn starts_the_app_again_when_the_uninstaller_fails() {
        let setup = setup(|world| {
            world.running = true;
            world.uninstaller_error = Some("アンインストーラーが終了コード2で終了しました".to_string());
        });
        assert_eq!(setup.service.uninstall("demo", true), Ok(FAILED));
        assert_eq!(setup.log(), ["close", "launch"]);
        let state = setup.state();
        assert!(state.error.unwrap().contains("終了コード2"));
        assert!(state.installed);
    }

    #[test]
    fn reports_failure_when_the_app_is_still_registered_afterwards() {
        let setup = setup(|world| world.uninstaller_does_nothing = true);
        assert_eq!(setup.service.uninstall("demo", false), Ok(FAILED));
        let state = setup.state();
        assert!(state.error.unwrap().contains("確認できませんでした"));
        assert!(state.installed);
    }

    #[test]
    fn fails_clearly_when_no_uninstaller_is_available() {
        let setup = setup(|world| world.no_uninstaller = true);
        assert!(!setup.state().can_uninstall);
        assert_eq!(setup.service.uninstall("demo", false), Ok(FAILED));
        assert!(setup.log().is_empty());
        assert!(setup.state().error.unwrap().contains("見つかりません"));
    }

    #[test]
    fn only_returns_release_pages_from_the_app_repository() {
        let setup = setup(|world| world.page_url = Some("https://evil.example/".to_string()));
        assert_eq!(setup.service.release_page_url("demo"), Ok(None));
    }

    #[test]
    fn rejects_unknown_apps() {
        let setup = setup(|_| {});
        assert!(setup.service.install("nope", false).is_err());
        assert!(setup.service.open("nope").is_err());
    }

    fn catalog_service(fake: &Arc<Fake>, settings: &Path) -> Service {
        Service::new(
            &crate::catalog::APPS,
            fake.clone(),
            fake.clone(),
            fake.clone(),
            std::env::temp_dir(),
            Box::new(|_| {}),
            Duration::from_millis(20),
            Duration::from_millis(1),
        )
        .with_settings(settings.to_path_buf())
    }

    fn ids(service: &Service) -> Vec<&'static str> {
        service.snapshot().apps.iter().map(|app| app.id).collect()
    }

    #[test]
    fn shows_the_catalog_order_until_one_is_chosen() {
        let setup = setup(|_| {});
        let temp = temp_dir();
        let service = catalog_service(&setup.fake, &temp.join("settings.json"));
        assert_eq!(ids(&service), ["yuitracking", "yuivoicechanger", "yuilivekit"]);
    }

    #[test]
    fn keeps_a_chosen_order_across_restarts() {
        let setup = setup(|_| {});
        let temp = temp_dir();
        let file = temp.join("nested").join("settings.json");
        let service = catalog_service(&setup.fake, &file);
        service.set_order(&["yuilivekit".to_string(), "yuitracking".to_string(), "yuivoicechanger".to_string()]).unwrap();
        assert_eq!(ids(&service), ["yuilivekit", "yuitracking", "yuivoicechanger"]);
        assert_eq!(ids(&catalog_service(&setup.fake, &file)), ["yuilivekit", "yuitracking", "yuivoicechanger"]);
    }

    #[test]
    fn ignores_unknown_and_repeated_ids_and_adds_apps_the_order_leaves_out() {
        let setup = setup(|_| {});
        let temp = temp_dir();
        let service = catalog_service(&setup.fake, &temp.join("settings.json"));
        let chosen = ["nope", "yuilivekit", "yuilivekit"].map(str::to_string);
        service.set_order(&chosen).unwrap();
        assert_eq!(ids(&service), ["yuilivekit", "yuitracking", "yuivoicechanger"]);
    }

    fn hidden_ids(service: &Service) -> Vec<&'static str> {
        service.snapshot().apps.iter().filter(|app| app.hidden).map(|app| app.id).collect()
    }

    #[test]
    fn hides_and_shows_apps_and_remembers_it_with_the_order() {
        let setup = setup(|_| {});
        let temp = temp_dir();
        let file = temp.join("settings.json");
        let service = catalog_service(&setup.fake, &file);
        assert!(hidden_ids(&service).is_empty(), "nothing is hidden at first");
        service.set_order(&["yuilivekit".to_string()]).unwrap();
        service.set_hidden("yuitracking", true).unwrap();
        service.set_hidden("yuivoicechanger", true).unwrap();
        service.set_hidden("yuivoicechanger", false).unwrap();
        assert_eq!(hidden_ids(&service), ["yuitracking"]);

        let restarted = catalog_service(&setup.fake, &file);
        assert_eq!(hidden_ids(&restarted), ["yuitracking"]);
        assert_eq!(ids(&restarted), ["yuilivekit", "yuitracking", "yuivoicechanger"], "hiding keeps the order");
        assert!(setup.service.set_hidden("nope", true).is_err());
    }

    #[test]
    fn a_hidden_app_is_still_checked_for_updates() {
        let setup = setup(|_| {});
        setup.service.set_hidden("demo", true).unwrap();
        setup.service.check();
        let state = setup.state();
        assert!(state.hidden);
        assert!(state.update_available);
    }

    #[test]
    fn a_broken_settings_file_means_the_catalog_order() {
        let setup = setup(|_| {});
        let temp = temp_dir();
        let file = temp.join("settings.json");
        std::fs::write(&file, "{not json").unwrap();
        assert_eq!(ids(&catalog_service(&setup.fake, &file)), ["yuitracking", "yuivoicechanger", "yuilivekit"]);
    }

    #[test]
    fn open_runs_the_installed_app() {
        let setup = setup(|_| {});
        setup.service.open("demo").unwrap();
        assert_eq!(setup.log(), ["launch"]);
        assert!(setup.state().running);
    }
}
