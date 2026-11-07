//! What the launcher needs from macOS: app bundles in /Applications, processes and the Trash.

use crate::catalog::AppDef;
use crate::system::{Installed, System, UninstallCommand};
use objc2_app_kit::NSRunningApplication;
use objc2_foundation::{NSFileManager, NSString, NSURL};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

fn run(program: &str, args: &[&std::ffi::OsStr]) -> Result<String, String> {
    let output = Command::new(program).args(args).output().map_err(|error| format!("{program}を実行できませんでした（{error}）"))?;
    if output.status.success() {
        return Ok(String::from_utf8_lossy(&output.stdout).to_string());
    }
    let detail = String::from_utf8_lossy(&output.stderr).trim().to_string();
    Err(format!("{program}が失敗しました（{detail}）"))
}

/// The version a bundle reports, from its Info.plist.
pub fn bundle_version(bundle: &Path) -> Option<String> {
    let plist = bundle.join("Contents").join("Info.plist");
    let version = run("plutil", &["-extract".as_ref(), "CFBundleShortVersionString".as_ref(), "raw".as_ref(), "-o".as_ref(), "-".as_ref(), plist.as_os_str()]).ok()?;
    Some(version.trim().to_string()).filter(|version| !version.is_empty())
}

/// Ids of processes whose executable is inside this folder.
fn processes_in(folder: &Path) -> Vec<i32> {
    let Ok(listing) = run("ps", &["-axww".as_ref(), "-o".as_ref(), "pid=,comm=".as_ref()]) else {
        return Vec::new();
    };
    let prefix = format!("{}/", folder.display());
    listing
        .lines()
        .filter_map(|line| {
            let (pid, path) = line.trim_start().split_once(' ')?;
            path.trim().starts_with(&prefix).then(|| pid.parse().ok()).flatten()
        })
        .collect()
}

/// Copies the bundle into place beside the old one, then swaps them, so a failed copy leaves the old app working.
pub fn install_bundle(source: &Path, dest: &Path) -> Result<(), String> {
    let name = dest.file_name().ok_or("インストール先が正しくありません")?.to_string_lossy().to_string();
    let parent = dest.parent().ok_or("インストール先が正しくありません")?;
    let staging = parent.join(format!(".{name}.new"));
    let old = parent.join(format!(".{name}.old"));
    let _ = std::fs::remove_dir_all(&staging);
    let _ = std::fs::remove_dir_all(&old);
    run("ditto", &[source.as_os_str(), staging.as_os_str()])?;
    let had_old = dest.exists();
    if had_old {
        std::fs::rename(dest, &old).map_err(|error| format!("古いアプリを移動できませんでした（{error}）"))?;
    }
    if let Err(error) = std::fs::rename(&staging, dest) {
        if had_old {
            let _ = std::fs::rename(&old, dest);
        }
        let _ = std::fs::remove_dir_all(&staging);
        return Err(format!("アプリを配置できませんでした（{error}）"));
    }
    let _ = std::fs::remove_dir_all(&old);
    Ok(())
}

/// Finds the bundle at the top of an unpacked archive or disk image, or one folder down.
fn find_bundle(root: &Path, bundle: &str) -> Option<PathBuf> {
    let direct = root.join(bundle);
    if direct.is_dir() {
        return Some(direct);
    }
    std::fs::read_dir(root).ok()?.flatten().map(|entry| entry.path().join(bundle)).find(|path| path.is_dir())
}

/// Unpacks a downloaded .zip or .dmg and puts the bundle it holds at dest.
pub fn install_from(file: &Path, bundle: &str, dest: &Path) -> Result<(), String> {
    let work = file.with_extension("unpacked");
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work).map_err(|error| error.to_string())?;
    let name = file.file_name().unwrap_or_default().to_string_lossy().to_lowercase();
    let result = if name.ends_with(".zip") {
        run("ditto", &["-x".as_ref(), "-k".as_ref(), file.as_os_str(), work.as_os_str()]).and_then(|_| {
            let source = find_bundle(&work, bundle).ok_or_else(|| format!("{bundle}が見つかりません"))?;
            install_bundle(&source, dest)
        })
    } else if name.ends_with(".dmg") {
        let mount = work.join("mount");
        let attach = ["attach".as_ref(), "-nobrowse".as_ref(), "-readonly".as_ref(), "-noautoopen".as_ref(), "-mountpoint".as_ref(), mount.as_os_str(), file.as_os_str()];
        run("hdiutil", &attach).and_then(|_| {
            let result = find_bundle(&mount, bundle).ok_or_else(|| format!("{bundle}が見つかりません")).and_then(|source| install_bundle(&source, dest));
            if run("hdiutil", &["detach".as_ref(), mount.as_os_str()]).is_err() {
                let _ = run("hdiutil", &["detach".as_ref(), "-force".as_ref(), mount.as_os_str()]);
            }
            result
        })
    } else {
        Err(format!("対応していない形式です（{name}）"))
    };
    let _ = std::fs::remove_dir_all(&work);
    result
}

pub struct MacSystem {
    apps_dir: PathBuf,
}

impl MacSystem {
    pub fn new() -> Self {
        Self::with_apps_dir(PathBuf::from("/Applications"))
    }

    pub fn with_apps_dir(apps_dir: PathBuf) -> Self {
        Self { apps_dir }
    }

    fn bundle_path(&self, app: &AppDef) -> Option<PathBuf> {
        app.mac.as_ref().map(|mac| self.apps_dir.join(mac.bundle))
    }
}

impl System for MacSystem {
    fn read_installed(&self, app: &AppDef) -> Result<Option<Installed>, String> {
        let Some(bundle) = self.bundle_path(app).filter(|path| path.is_dir()) else {
            return Ok(None);
        };
        Ok(Some(Installed {
            version: bundle_version(&bundle),
            exe_path: Some(bundle.clone()),
            uninstaller: Some(UninstallCommand { file: bundle, args: Vec::new() }),
        }))
    }

    fn is_running(&self, _app: &AppDef, bundle: Option<&Path>) -> Result<bool, String> {
        Ok(bundle.is_some_and(|bundle| !processes_in(bundle).is_empty()))
    }

    // Asks the app to quit the way its Quit menu item does and never force-kills it:
    // it may be streaming or holding unsaved work.
    fn close_app(&self, _app: &AppDef, bundle: &Path) -> Result<bool, String> {
        for pid in processes_in(&bundle.join("Contents").join("MacOS")) {
            if let Some(running) = NSRunningApplication::runningApplicationWithProcessIdentifier(pid) {
                running.terminate();
            }
        }
        let deadline = Instant::now() + Duration::from_secs(20);
        while Instant::now() < deadline {
            if processes_in(bundle).is_empty() {
                return Ok(true);
            }
            thread::sleep(Duration::from_millis(500));
        }
        Ok(false)
    }

    fn launch(&self, bundle: &Path) -> Result<(), String> {
        run("open", &[bundle.as_os_str()]).map(|_| ())
    }

    fn run_installer(&self, app: &AppDef, file: &Path, _args: &[&str]) -> Result<(), String> {
        let mac = app.mac.as_ref().ok_or("このアプリにはMac版がありません")?;
        install_from(file, mac.bundle, &self.apps_dir.join(mac.bundle))
    }

    /// Moves the bundle to the Trash, so it can still be put back.
    fn run_uninstaller(&self, command: &UninstallCommand) -> Result<(), String> {
        let bundle = &command.file;
        let inside_apps = bundle.parent() == Some(self.apps_dir.as_path());
        if !inside_apps || bundle.extension().is_none_or(|extension| extension != "app") {
            return Err("アプリケーションフォルダの外にあるものは削除しません".to_string());
        }
        let url = NSURL::fileURLWithPath(&NSString::from_str(&bundle.to_string_lossy()));
        NSFileManager::defaultManager()
            .trashItemAtURL_resultingItemURL_error(&url, None)
            .map_err(|error| format!("ゴミ箱に移動できませんでした（{}）", error.localizedDescription()))
    }

    fn open_url(&self, url: &str) -> Result<(), String> {
        run("open", &[url.as_ref()]).map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::APPS;
    use crate::testutil::temp_dir;

    fn make_bundle(dir: &Path, name: &str, version: &str) -> PathBuf {
        let bundle = dir.join(name);
        std::fs::create_dir_all(bundle.join("Contents").join("MacOS")).unwrap();
        let plist = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<plist version=\"1.0\"><dict><key>CFBundleShortVersionString</key><string>{version}</string></dict></plist>\n"
        );
        std::fs::write(bundle.join("Contents").join("Info.plist"), plist).unwrap();
        bundle
    }

    #[test]
    fn reads_an_installed_bundle_and_its_version() {
        let temp = temp_dir();
        let system = MacSystem::with_apps_dir(temp.to_path_buf());
        assert!(system.read_installed(&APPS[0]).unwrap().is_none());
        let bundle = make_bundle(&temp, "YuiTracking.app", "2026.1021.3");
        let installed = system.read_installed(&APPS[0]).unwrap().unwrap();
        assert_eq!(installed.version.as_deref(), Some("2026.1021.3"));
        assert_eq!(installed.exe_path.as_deref(), Some(bundle.as_path()));
        assert!(system.read_installed(&APPS[2]).unwrap().is_none(), "no Mac version, never installed");
    }

    #[test]
    fn installs_from_a_zip_over_an_old_version() {
        let temp = temp_dir();
        let apps = temp.join("Applications");
        std::fs::create_dir_all(&apps).unwrap();
        make_bundle(&apps, "YuiTracking.app", "1.0");
        let source = temp.join("source");
        make_bundle(&source, "YuiTracking.app", "2.0");
        let zip = temp.join("YuiTracking-2.0-arm64-mac.zip");
        run("ditto", &["-c".as_ref(), "-k".as_ref(), "--keepParent".as_ref(), source.join("YuiTracking.app").as_os_str(), zip.as_os_str()]).unwrap();

        let system = MacSystem::with_apps_dir(apps.clone());
        system.run_installer(&APPS[0], &zip, &[]).unwrap();
        assert_eq!(bundle_version(&apps.join("YuiTracking.app")).as_deref(), Some("2.0"));
        let left: Vec<_> = std::fs::read_dir(&apps).unwrap().flatten().map(|entry| entry.file_name()).collect();
        assert_eq!(left, ["YuiTracking.app"], "no staging folders are left");
        assert!(!temp.join("YuiTracking-2.0-arm64-mac.unpacked").exists());
    }

    #[test]
    fn installs_from_a_disk_image() {
        let temp = temp_dir();
        let apps = temp.join("Applications");
        std::fs::create_dir_all(&apps).unwrap();
        let source = temp.join("source");
        make_bundle(&source, "YuiVoiceChanger.app", "3.0");
        let dmg = temp.join("YuiVoiceChanger-3.0-mac-arm64.dmg");
        run("hdiutil", &["create".as_ref(), "-quiet".as_ref(), "-srcfolder".as_ref(), source.as_os_str(), "-format".as_ref(), "UDZO".as_ref(), dmg.as_os_str()]).unwrap();

        MacSystem::with_apps_dir(apps.clone()).run_installer(&APPS[1], &dmg, &[]).unwrap();
        assert_eq!(bundle_version(&apps.join("YuiVoiceChanger.app")).as_deref(), Some("3.0"));
    }

    #[test]
    fn a_broken_download_keeps_the_old_app() {
        let temp = temp_dir();
        let apps = temp.join("Applications");
        std::fs::create_dir_all(&apps).unwrap();
        make_bundle(&apps, "YuiTracking.app", "1.0");
        let zip = temp.join("YuiTracking-2.0-arm64-mac.zip");
        std::fs::write(&zip, "not a zip").unwrap();
        assert!(MacSystem::with_apps_dir(apps.clone()).run_installer(&APPS[0], &zip, &[]).is_err());
        assert_eq!(bundle_version(&apps.join("YuiTracking.app")).as_deref(), Some("1.0"));
    }

    #[test]
    fn refuses_to_remove_anything_outside_the_apps_folder() {
        let temp = temp_dir();
        let apps = temp.join("Applications");
        std::fs::create_dir_all(&apps).unwrap();
        let elsewhere = make_bundle(&temp, "Other.app", "1.0");
        let system = MacSystem::with_apps_dir(apps.clone());
        assert!(system.run_uninstaller(&UninstallCommand { file: elsewhere.clone(), args: Vec::new() }).is_err());
        assert!(elsewhere.exists());
        let not_app = apps.join("notes");
        std::fs::create_dir_all(&not_app).unwrap();
        assert!(system.run_uninstaller(&UninstallCommand { file: not_app.clone(), args: Vec::new() }).is_err());
        assert!(not_app.exists());
    }

    /// Reads the real /Applications and GitHub, and installs the real YuiTracking release into a
    /// temporary folder: cargo test -- --ignored real_
    #[test]
    #[ignore]
    fn real_install_of_the_tracking_release_into_a_temporary_folder() {
        use crate::download::HttpDownloader;
        use crate::releases::{GitHub, Releases};
        use crate::service::Service;
        use std::sync::Arc;

        let installed = MacSystem::new().read_installed(&APPS[0]).unwrap();
        eprintln!("installed in /Applications: {:?}", installed.as_ref().map(|info| &info.version));
        let github = Arc::new(GitHub::new().unwrap());
        for app in crate::catalog::platform_apps() {
            let release = github.latest(app).unwrap();
            eprintln!("{}: {:?}", app.id, release.map(|release| (release.version, release.installer.name)));
        }

        let temp = temp_dir();
        let apps = temp.join("Applications");
        std::fs::create_dir_all(&apps).unwrap();
        let service = Service::new(
            crate::catalog::platform_apps(),
            Arc::new(MacSystem::with_apps_dir(apps.clone())),
            github,
            Arc::new(HttpDownloader),
            temp.to_path_buf(),
            Box::new(|_| {}),
            Duration::from_secs(60),
            Duration::from_secs(1),
        );
        service.check();
        let before = service.snapshot().apps.into_iter().find(|app| app.id == "yuitracking").unwrap();
        assert!(!before.installed && before.update_available);
        assert_eq!(service.install("yuitracking", false), Ok("done"));
        let after = service.snapshot().apps.into_iter().find(|app| app.id == "yuitracking").unwrap();
        eprintln!("after: {:?} {:?} {:?}", after.version, after.notice, after.error);
        assert_eq!(after.version, after.latest_version);
        assert!(after.can_open && after.can_uninstall);
        assert_eq!(service.uninstall("yuitracking", false), Ok("done"));
        assert!(!apps.join("YuiTracking.app").exists(), "moved to the Trash");
    }

    #[test]
    fn finds_this_test_process_by_its_folder_only() {
        let own = std::env::current_exe().unwrap();
        let folder = own.parent().unwrap();
        let system = MacSystem::new();
        assert!(system.is_running(&APPS[0], Some(folder)).unwrap());
        assert!(!system.is_running(&APPS[0], Some(Path::new("/NoSuchFolder.app"))).unwrap());
        assert!(!system.is_running(&APPS[0], None).unwrap());
        assert!(system.close_app(&APPS[0], Path::new("/NoSuchFolder.app")).unwrap());
    }
}
