//! Fake machine and GitHub for the UI test: no network, no real installs.
//! Only debug builds contain this.

use crate::catalog::{AppDef, APPS};
use crate::download::{DownloadError, Downloader, Progress};
use crate::releases::{Asset, Release, Releases};
use crate::system::{Installed, System, UninstallCommand};
use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

const HASH: &str = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";

struct FakeApp {
    version: Option<String>,
    running: bool,
    latest: &'static str,
    installer: &'static str,
}

#[derive(Serialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct Recorded {
    pub self_checks: u32,
    pub self_installs: u32,
    pub opened_urls: Vec<String>,
}

pub struct Fixture {
    apps: Mutex<HashMap<&'static str, FakeApp>>,
    recorded: Mutex<Recorded>,
}

impl Fixture {
    pub fn new() -> Self {
        let app = |version: &str, running, latest, installer| FakeApp { version: Some(version.to_string()), running, latest, installer };
        Self {
            apps: Mutex::new(HashMap::from([
                ("yuitracking", app("2026.1106.5", true, "2026.1107.1", "YuiTracking-Setup-2026.1107.1-x64.exe")),
                ("yuivoicechanger", app("2026.1104.31", false, "2026.1104.31", "YuiVoiceChanger-Setup-2026.1104.31.exe")),
                ("yuilivekit", app("2026.1107.1", false, "2026.1107.1", "YuiLiveKit-Setup-2026.1107.1.exe")),
                ("yuiclock", app("2026.1108.1", false, "2026.1108.1", "YuiClock-Setup-2026.1108.1.exe")),
            ])),
            recorded: Mutex::new(Recorded::default()),
        }
    }

    pub fn recorded(&self) -> Recorded {
        self.recorded.lock().unwrap().clone()
    }

    pub fn record_self_install(&self) {
        self.recorded.lock().unwrap().self_installs += 1;
    }
}

impl System for Fixture {
    fn read_installed(&self, app: &AppDef) -> Result<Option<Installed>, String> {
        let apps = self.apps.lock().unwrap();
        Ok(apps[app.id].version.clone().map(|version| Installed {
            version: Some(version),
            exe_path: Some(Path::new(r"C:\Fake").join(app.exe_name)),
            uninstaller: Some(UninstallCommand { file: PathBuf::from(r"C:\Fake\unins.exe"), args: vec![app.id.to_string()] }),
        }))
    }

    fn is_running(&self, app: &AppDef, _: Option<&Path>) -> Result<bool, String> {
        Ok(self.apps.lock().unwrap()[app.id].running)
    }

    fn close_app(&self, app: &AppDef, _: &Path) -> Result<bool, String> {
        thread::sleep(Duration::from_millis(100));
        self.apps.lock().unwrap().get_mut(app.id).unwrap().running = false;
        Ok(true)
    }

    fn launch(&self, exe_path: &Path) -> Result<(), String> {
        let app = APPS.iter().find(|app| exe_path.ends_with(app.exe_name)).ok_or("unknown exe")?;
        self.apps.lock().unwrap().get_mut(app.id).unwrap().running = true;
        Ok(())
    }

    fn run_installer(&self, _: &AppDef, file: &Path, _: &[&str]) -> Result<(), String> {
        thread::sleep(Duration::from_millis(200));
        let name = file.file_name().unwrap_or_default().to_string_lossy().to_string();
        let mut apps = self.apps.lock().unwrap();
        let app = apps.values_mut().find(|app| app.installer == name).ok_or("unknown installer")?;
        app.version = Some(app.latest.to_string());
        Ok(())
    }

    fn run_uninstaller(&self, command: &UninstallCommand) -> Result<(), String> {
        thread::sleep(Duration::from_millis(200));
        self.apps.lock().unwrap().get_mut(command.args[0].as_str()).unwrap().version = None;
        Ok(())
    }

    fn open_url(&self, url: &str) -> Result<(), String> {
        self.recorded.lock().unwrap().opened_urls.push(url.to_string());
        Ok(())
    }
}

impl Releases for Fixture {
    fn latest(&self, app: &AppDef) -> Result<Option<Release>, String> {
        if app.id == "yuilauncher" {
            self.recorded.lock().unwrap().self_checks += 1;
            return Ok(None);
        }
        let apps = self.apps.lock().unwrap();
        let fake = &apps[app.id];
        Ok(Some(Release {
            version: fake.latest.to_string(),
            page_url: format!("https://github.com/{}/releases/tag/v{}", app.repo, fake.latest),
            installer: Asset { name: fake.installer.to_string(), size: 100, url: "https://example.invalid/installer".to_string() },
            checksums: vec![Asset { name: "SHA256SUMS.txt".to_string(), size: 1, url: "https://example.invalid/sums".to_string() }],
        }))
    }

    fn fetch_text(&self, _: &str) -> Result<String, String> {
        let apps = self.apps.lock().unwrap();
        Ok(apps.values().map(|app| format!("{HASH}  {}\n", app.installer)).collect())
    }
}

impl Downloader for Fixture {
    fn download(&self, _: &str, dest: &Path, _: Option<u64>, _: &AtomicBool, on_progress: Progress) -> Result<String, DownloadError> {
        for received in [10, 42, 70, 100] {
            on_progress(received, Some(100));
            thread::sleep(Duration::from_millis(350));
        }
        std::fs::write(dest, "fake").map_err(|error| DownloadError::Failed(error.to_string()))?;
        Ok(HASH.to_string())
    }
}
