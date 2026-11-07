//! The launcher's own update. It is found, downloaded and verified the same way as
//! the apps' updates, and the installer runs when the launcher restarts or quits.

use crate::catalog::LAUNCHER;
use crate::download::Downloader;
use crate::releases::{parse_checksums, Release, Releases};
use crate::version::compare_versions;
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const MAX_LOG_BYTES: u64 = 1024 * 1024;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SelfView {
    pub current_version: String,
    pub phase: String,
    pub latest_version: Option<String>,
    pub percent: Option<u64>,
    pub error: Option<String>,
}

/// Starts an installer without waiting for it.
pub type RunInstaller = Box<dyn Fn(&Path, &[&str]) -> Result<(), String> + Send + Sync>;
pub type OnChange = Box<dyn Fn(SelfView) + Send + Sync>;

pub struct Updater {
    pub releases: Arc<dyn Releases>,
    pub downloader: Arc<dyn Downloader>,
    pub run_installer: RunInstaller,
}

struct State {
    view: SelfView,
    installer: Option<PathBuf>,
    // Bumped by each check, so a retry timer from an older failure does nothing.
    generation: u64,
}

pub struct SelfUpdater {
    updater: Option<Updater>,
    dir: PathBuf,
    log_file: PathBuf,
    retry_after: Duration,
    on_change: OnChange,
    state: Mutex<State>,
}

fn timestamp() -> String {
    let seconds = SystemTime::now().duration_since(UNIX_EPOCH).map(|time| time.as_secs()).unwrap_or(0);
    let (days, time) = ((seconds / 86400) as i64, seconds % 86400);
    // Howard Hinnant's civil_from_days.
    let shifted = days + 719468;
    let era = shifted.div_euclid(146097);
    let day_of_era = shifted.rem_euclid(146097);
    let year_of_era = (day_of_era - day_of_era / 1460 + day_of_era / 36524 - day_of_era / 146096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_part = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_part + 2) / 5 + 1;
    let month = if month_part < 10 { month_part + 3 } else { month_part - 9 };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z", time / 3600, time / 60 % 60, time % 60)
}

/// Writes to a file so a failed update can be traced later. Over max_bytes, it moves to .old.
pub fn write_log(file: &Path, max_bytes: u64, level: &str, message: &str) {
    let write = || -> std::io::Result<()> {
        if let Some(parent) = file.parent() {
            std::fs::create_dir_all(parent)?;
        }
        if std::fs::metadata(file).is_ok_and(|info| info.len() > max_bytes) {
            let mut old = file.as_os_str().to_os_string();
            old.push(".old");
            std::fs::rename(file, old)?;
        }
        let mut output = std::fs::OpenOptions::new().create(true).append(true).open(file)?;
        writeln!(output, "{} {level} {message}", timestamp())
    };
    if let Err(error) = write() {
        eprintln!("Failed to write the update log: {error}");
    }
}

impl SelfUpdater {
    /// updater is None when running from source, where there is nothing to update.
    pub fn new(version: String, updater: Option<Updater>, dir: PathBuf, log_file: PathBuf, retry_after: Duration, on_change: OnChange) -> Arc<Self> {
        let phase = if updater.is_some() { "idle" } else { "unavailable" };
        Arc::new(Self {
            updater,
            dir,
            log_file,
            retry_after,
            on_change,
            state: Mutex::new(State {
                view: SelfView { current_version: version, phase: phase.to_string(), latest_version: None, percent: None, error: None },
                installer: None,
                generation: 0,
            }),
        })
    }

    pub fn snapshot(&self) -> SelfView {
        self.state.lock().unwrap().view.clone()
    }

    fn log(&self, level: &str, message: &str) {
        write_log(&self.log_file, MAX_LOG_BYTES, level, message);
    }

    fn set(&self, change: impl FnOnce(&mut SelfView)) {
        let view = {
            let mut state = self.state.lock().unwrap();
            change(&mut state.view);
            state.view.clone()
        };
        (self.on_change)(view);
    }

    fn fail(self: &Arc<Self>, detail: &str, downloading: bool) {
        self.log("ERROR", detail);
        // The page sees a plain message: the details can hold URLs and paths.
        let message = if downloading {
            "YuiLauncherの更新をダウンロードできませんでした。しばらくしてからもう一度お試しください。"
        } else {
            "YuiLauncherの更新を確認できませんでした。しばらくしてからもう一度お試しください。"
        };
        self.set(|view| {
            view.phase = "error".to_string();
            view.percent = None;
            view.error = Some(message.to_string());
        });
        let generation = self.state.lock().unwrap().generation;
        let this = self.clone();
        thread::spawn(move || {
            thread::sleep(this.retry_after);
            if this.state.lock().unwrap().generation == generation {
                this.check();
            }
        });
    }

    fn download(&self, updater: &Updater, release: &Release) -> Result<PathBuf, String> {
        let _ = std::fs::remove_dir_all(&self.dir);
        std::fs::create_dir_all(&self.dir).map_err(|error| error.to_string())?;
        let dest = self.dir.join(&release.installer.name);
        let mut last_percent = Some(0);
        let expected_size = Some(release.installer.size).filter(|size| *size > 0);
        let sha256 = updater
            .downloader
            .download(&release.installer.url, &dest, expected_size, &AtomicBool::new(false), &mut |received, total| {
                let percent = total.map(|total| received.saturating_mul(100) / total.max(1));
                if percent != last_percent {
                    last_percent = percent;
                    self.set(|view| view.percent = percent);
                }
            })
            .map_err(|error| format!("{error:?}"))?;
        let text = updater.releases.fetch_text(&release.checksums.url)?;
        match parse_checksums(&text).get(&release.installer.name) {
            Some(expected) if *expected == sha256 => Ok(dest),
            Some(_) => Err(format!("checksum mismatch for {}", release.installer.name)),
            None => Err(format!("no checksum for {}", release.installer.name)),
        }
    }

    /// Never fails. A ready update stays ready, and a running one isn't started twice.
    pub fn check(self: &Arc<Self>) {
        let Some(updater) = &self.updater else { return };
        {
            let mut state = self.state.lock().unwrap();
            if matches!(state.view.phase.as_str(), "ready" | "downloading" | "checking") {
                return;
            }
            state.generation += 1;
        }
        self.set(|view| {
            view.phase = "checking".to_string();
            view.error = None;
        });
        let release = match updater.releases.latest(&LAUNCHER) {
            Ok(release) => release,
            Err(error) => return self.fail(&error, false),
        };
        let current = self.snapshot().current_version;
        let Some(release) = release.filter(|release| compare_versions(&release.version, &current) == Some(Ordering::Greater)) else {
            // Also clears the installer the last update ran from.
            let _ = std::fs::remove_dir_all(&self.dir);
            return self.set(|view| {
                view.phase = "latest".to_string();
                view.latest_version = None;
                view.percent = None;
            });
        };
        self.log("INFO", &format!("Found version {}", release.version));
        self.set(|view| {
            view.phase = "downloading".to_string();
            view.latest_version = Some(release.version.clone());
            view.percent = Some(0);
        });
        match self.download(updater, &release) {
            Ok(installer) => {
                self.log("INFO", &format!("Downloaded and verified {}", release.installer.name));
                self.state.lock().unwrap().installer = Some(installer);
                self.set(|view| {
                    view.phase = "ready".to_string();
                    view.percent = None;
                });
            }
            Err(error) => self.fail(&error, true),
        }
    }

    fn start_installer(&self, args: &[&str]) -> bool {
        let Some(updater) = &self.updater else { return false };
        let installer = {
            let mut state = self.state.lock().unwrap();
            if state.view.phase != "ready" {
                return false;
            }
            state.installer.take()
        };
        let Some(installer) = installer else { return false };
        match (updater.run_installer)(&installer, args) {
            Ok(()) => true,
            Err(error) => {
                self.log("ERROR", &format!("Could not start the installer: {error}"));
                self.state.lock().unwrap().installer = Some(installer);
                false
            }
        }
    }

    /// For the restart button. /R makes the installer start the new version. True means quit now.
    pub fn install(&self) -> bool {
        self.start_installer(&["/S", "/UPDATE", "/R"])
    }

    /// A downloaded update goes in when the launcher quits, without starting it again.
    pub fn install_on_quit(&self) {
        self.start_installer(&["/S", "/UPDATE"]);
    }

    /// For the UI test, which shows each state without a real update.
    #[cfg(debug_assertions)]
    pub fn fixture_set(&self, view: SelfView) {
        {
            let mut state = self.state.lock().unwrap();
            state.installer = (view.phase == "ready").then(|| PathBuf::from("fixture-installer.exe"));
            state.view = view;
        }
        (self.on_change)(self.snapshot());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::AppDef;
    use crate::download::{DownloadError, Progress};
    use crate::releases::Asset;
    use crate::testutil::{temp_dir, TempDir};
    use std::sync::mpsc;

    const HASH: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    #[derive(Default)]
    struct World {
        latest: Option<String>,
        check_error: Option<String>,
        download_error: Option<String>,
        checksums: Option<String>,
        checks: u32,
        downloads: u32,
        runs: Vec<Vec<String>>,
        run_error: Option<String>,
        hold_check: Option<(mpsc::Sender<()>, mpsc::Receiver<()>)>,
    }

    struct Fake(Mutex<World>);

    impl Releases for Fake {
        fn latest(&self, app: &AppDef) -> Result<Option<Release>, String> {
            assert_eq!(app.id, "yuilauncher");
            let held = {
                let mut world = self.0.lock().unwrap();
                world.checks += 1;
                world.hold_check.take()
            };
            if let Some((started, gate)) = held {
                started.send(()).unwrap();
                let _ = gate.recv();
            }
            let world = self.0.lock().unwrap();
            if let Some(error) = &world.check_error {
                return Err(error.clone());
            }
            Ok(world.latest.clone().map(|version| Release {
                version,
                page_url: String::new(),
                installer: Asset { name: "YuiLauncher-Setup-1.1.0.exe".to_string(), size: 4, url: "https://secret.example/setup".to_string() },
                checksums: Asset { name: "SHA256SUMS.txt".to_string(), size: 1, url: "https://secret.example/sums".to_string() },
            }))
        }
        fn fetch_text(&self, _: &str) -> Result<String, String> {
            Ok(self.0.lock().unwrap().checksums.clone().unwrap_or_else(|| format!("{HASH}  YuiLauncher-Setup-1.1.0.exe\n")))
        }
    }

    impl Downloader for Fake {
        fn download(&self, _: &str, dest: &Path, _: Option<u64>, _: &AtomicBool, on_progress: Progress) -> Result<String, DownloadError> {
            let mut world = self.0.lock().unwrap();
            world.downloads += 1;
            if let Some(error) = &world.download_error {
                return Err(DownloadError::Failed(error.clone()));
            }
            std::fs::write(dest, "fake").unwrap();
            drop(world);
            on_progress(1, Some(4));
            on_progress(4, Some(4));
            Ok(HASH.to_string())
        }
    }

    struct Setup {
        fake: Arc<Fake>,
        updater: Arc<SelfUpdater>,
        changes: Arc<Mutex<Vec<SelfView>>>,
        temp: TempDir,
    }

    fn setup(retry_after: Duration, change: impl FnOnce(&mut World)) -> Setup {
        let mut world = World::default();
        change(&mut world);
        let fake = Arc::new(Fake(Mutex::new(world)));
        let temp = temp_dir();
        let changes = Arc::new(Mutex::new(Vec::new()));
        let recorded = changes.clone();
        let runs = fake.clone();
        let updater = SelfUpdater::new(
            "1.0.0".to_string(),
            Some(Updater {
                releases: fake.clone(),
                downloader: fake.clone(),
                run_installer: Box::new(move |file, args| {
                    assert!(file.exists(), "the installer is still there");
                    let mut world = runs.0.lock().unwrap();
                    if let Some(error) = &world.run_error {
                        return Err(error.clone());
                    }
                    world.runs.push(args.iter().map(|arg| (*arg).to_string()).collect());
                    Ok(())
                }),
            }),
            temp.join("update"),
            temp.join("logs").join("self-update.log"),
            retry_after,
            Box::new(move |view| recorded.lock().unwrap().push(view)),
        );
        Setup { fake, updater, changes, temp }
    }

    fn long() -> Duration {
        Duration::from_secs(600)
    }

    fn newer(world: &mut World) {
        world.latest = Some("1.1.0".to_string());
    }

    #[test]
    fn walks_through_checking_downloading_and_ready() {
        let setup = setup(long(), newer);
        assert_eq!(setup.updater.snapshot().phase, "idle");
        assert_eq!(setup.updater.snapshot().current_version, "1.0.0");
        setup.updater.check();
        let phases: Vec<String> = setup.changes.lock().unwrap().iter().map(|view| view.phase.clone()).collect();
        assert_eq!(phases[0], "checking");
        assert!(phases.contains(&"downloading".to_string()));
        let percents: Vec<Option<u64>> = setup.changes.lock().unwrap().iter().filter(|view| view.phase == "downloading").map(|view| view.percent).collect();
        assert_eq!(percents, [Some(0), Some(25), Some(100)]);
        let view = setup.updater.snapshot();
        assert_eq!(view.phase, "ready");
        assert_eq!(view.latest_version.as_deref(), Some("1.1.0"));
        assert_eq!(view.percent, None);
    }

    #[test]
    fn reports_being_up_to_date_and_clears_the_last_installer() {
        let setup = setup(long(), |world| world.latest = Some("1.0.0".to_string()));
        std::fs::create_dir_all(setup.temp.join("update")).unwrap();
        setup.updater.check();
        assert_eq!(setup.updater.snapshot().phase, "latest");
        assert_eq!(setup.fake.0.lock().unwrap().downloads, 0);
        assert!(!setup.temp.join("update").exists());
    }

    #[test]
    fn a_missing_release_counts_as_up_to_date() {
        let setup = setup(long(), |_| {});
        setup.updater.check();
        assert_eq!(setup.updater.snapshot().phase, "latest");
    }

    #[test]
    fn a_failed_check_shows_a_plain_message_and_logs_the_details() {
        let setup = setup(long(), |world| world.check_error = Some("HttpError https://secret.example/".to_string()));
        setup.updater.check();
        let view = setup.updater.snapshot();
        assert_eq!(view.phase, "error");
        assert!(view.error.as_ref().unwrap().contains("確認できませんでした"));
        assert!(!view.error.unwrap().contains("secret.example"));
        let log = std::fs::read_to_string(setup.temp.join("logs").join("self-update.log")).unwrap();
        assert!(log.contains("ERROR HttpError https://secret.example/"));
        assert_eq!(setup.changes.lock().unwrap().iter().filter(|view| view.phase == "error").count(), 1);
    }

    #[test]
    fn a_failed_download_says_so() {
        let setup = setup(long(), |world| {
            newer(world);
            world.download_error = Some("connection reset".to_string());
        });
        setup.updater.check();
        let view = setup.updater.snapshot();
        assert_eq!(view.phase, "error");
        assert!(view.error.unwrap().contains("ダウンロードできませんでした"));
    }

    #[test]
    fn a_bad_checksum_is_never_ready_to_install() {
        let setup = setup(long(), |world| {
            newer(world);
            world.checksums = Some(format!("{}  YuiLauncher-Setup-1.1.0.exe\n", "b".repeat(64)));
        });
        setup.updater.check();
        assert_eq!(setup.updater.snapshot().phase, "error");
        assert!(!setup.updater.install());
        assert!(setup.fake.0.lock().unwrap().runs.is_empty());
    }

    #[test]
    fn retries_soon_after_a_failure_and_the_retry_clears_the_error() {
        let setup = setup(Duration::from_millis(5), |world| world.check_error = Some("offline".to_string()));
        setup.updater.check();
        assert_eq!(setup.updater.snapshot().phase, "error");
        setup.fake.0.lock().unwrap().check_error = None;
        for _ in 0..400 {
            if setup.updater.snapshot().phase == "latest" {
                break;
            }
            thread::sleep(Duration::from_millis(5));
        }
        let view = setup.updater.snapshot();
        assert_eq!(view.phase, "latest");
        assert_eq!(view.error, None);
    }

    #[test]
    fn checking_again_recovers_from_an_error_right_away() {
        let setup = setup(long(), |world| world.check_error = Some("offline".to_string()));
        setup.updater.check();
        setup.fake.0.lock().unwrap().check_error = None;
        setup.updater.check();
        assert_eq!(setup.updater.snapshot().phase, "latest");
    }

    #[test]
    fn does_not_start_a_second_check_while_one_runs() {
        let (started_tx, started_rx) = mpsc::channel();
        let (gate_tx, gate_rx) = mpsc::channel();
        let setup = setup(long(), |world| world.hold_check = Some((started_tx, gate_rx)));
        let updater = setup.updater.clone();
        let first = thread::spawn(move || updater.check());
        started_rx.recv().unwrap();
        setup.updater.check();
        assert_eq!(setup.fake.0.lock().unwrap().checks, 1);
        gate_tx.send(()).unwrap();
        first.join().unwrap();
    }

    #[test]
    fn keeps_a_ready_update_ready_when_checked_again() {
        let setup = setup(long(), newer);
        setup.updater.check();
        setup.updater.check();
        assert_eq!(setup.fake.0.lock().unwrap().checks, 1);
        assert_eq!(setup.updater.snapshot().phase, "ready");
    }

    #[test]
    fn installs_only_when_an_update_is_ready_and_restarts_the_launcher() {
        let setup = setup(long(), newer);
        assert!(!setup.updater.install());
        setup.updater.check();
        assert!(setup.updater.install());
        assert_eq!(setup.fake.0.lock().unwrap().runs, [["/S", "/UPDATE", "/R"]]);
    }

    #[test]
    fn installs_a_ready_update_on_quit_without_restarting() {
        let setup = setup(long(), newer);
        setup.updater.check();
        setup.updater.install_on_quit();
        assert_eq!(setup.fake.0.lock().unwrap().runs, [["/S", "/UPDATE"]]);
    }

    #[test]
    fn the_restart_button_keeps_quitting_from_running_the_installer_twice() {
        let setup = setup(long(), newer);
        setup.updater.check();
        assert!(setup.updater.install());
        setup.updater.install_on_quit();
        assert_eq!(setup.fake.0.lock().unwrap().runs.len(), 1);
    }

    #[test]
    fn stays_open_and_can_try_again_when_the_installer_will_not_start() {
        let setup = setup(long(), |world| {
            newer(world);
            world.run_error = Some("blocked".to_string());
        });
        setup.updater.check();
        assert!(!setup.updater.install());
        setup.fake.0.lock().unwrap().run_error = None;
        assert!(setup.updater.install());
    }

    #[test]
    fn does_nothing_when_running_from_source() {
        let temp = temp_dir();
        let updater = SelfUpdater::new("1.0.0".to_string(), None, temp.join("update"), temp.join("log"), long(), Box::new(|_| {}));
        assert_eq!(updater.snapshot().phase, "unavailable");
        updater.check();
        assert_eq!(updater.snapshot().phase, "unavailable");
        assert!(!updater.install());
    }

    #[test]
    fn the_log_records_messages_and_rotates() {
        let temp = temp_dir();
        let file = temp.join("logs").join("self-update.log");
        write_log(&file, 200, "INFO", "Found version 1.1.0");
        write_log(&file, 200, "ERROR", "boom");
        let first = std::fs::read_to_string(&file).unwrap();
        assert!(first.contains("INFO Found version 1.1.0"));
        assert!(first.contains("ERROR boom"));
        assert!(first.starts_with("20"), "starts with a date: {first}");

        write_log(&file, 200, "INFO", &"x".repeat(300));
        write_log(&file, 200, "INFO", "after rotation");
        assert!(temp.join("logs").join("self-update.log.old").exists());
        let second = std::fs::read_to_string(&file).unwrap();
        assert!(second.contains("after rotation"));
        assert!(!second.contains("boom"));
    }
}
