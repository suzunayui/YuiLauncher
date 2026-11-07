//! What the launcher needs from Windows: the registry, processes and installers.

use crate::catalog::AppDef;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::{CloseHandle, BOOL, HWND, INVALID_HANDLE_VALUE, LPARAM};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, CREATE_NO_WINDOW, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindow, GetWindowThreadProcessId, IsWindowVisible, PostMessageW, GW_OWNER, WM_CLOSE,
};
use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
use winreg::RegKey;

pub use crate::system::{Installed, System, UninstallCommand};

/// electron-builder's NSIS installer leaves InstallLocation empty, so this falls back
/// to the folder of the uninstaller. Tauri's writes it with quotes around it.
pub fn install_dir_from(location: &str, uninstall: &str) -> Option<PathBuf> {
    let location = location.trim().trim_matches('"');
    if !location.is_empty() {
        return Some(PathBuf::from(location));
    }
    parse_command(uninstall)?.file.parent().map(Path::to_path_buf)
}

/// Splits a registry command like: "C:\App\Uninstall App.exe" /currentuser
pub fn parse_command(command: &str) -> Option<UninstallCommand> {
    let rest = command.strip_prefix('"')?;
    let (file, args) = rest.split_once('"')?;
    Some(UninstallCommand {
        file: PathBuf::from(file),
        args: args.split_whitespace().map(str::to_string).collect(),
    })
}

fn same_folder(a: &Path, b: &Path) -> bool {
    let normalize = |path: &Path| path.to_string_lossy().trim_end_matches(['\\', '/']).to_lowercase();
    normalize(a) == normalize(b)
}

/// Only runs an uninstaller that sits in the app's own install folder.
pub fn uninstaller_from(location: &str, uninstall: &str, app: &AppDef, exists: &dyn Fn(&Path) -> bool) -> Option<UninstallCommand> {
    let mut command = parse_command(uninstall)?;
    let dir = install_dir_from(location, uninstall)?;
    let is_exe = command.file.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("exe"));
    if !is_exe || !same_folder(command.file.parent()?, &dir) || !exists(&command.file) {
        return None;
    }
    for arg in app.uninstall_args {
        if !command.args.iter().any(|existing| existing == arg) {
            command.args.push((*arg).to_string());
        }
    }
    Some(command)
}

/// Process ids running this exact file. Matching by full path keeps a development
/// build elsewhere that shares the exe name from counting as the installed app.
fn processes_at(exe_path: &Path) -> Vec<u32> {
    let Some(exe_name) = exe_path.file_name().map(|name| name.to_string_lossy().to_string()) else {
        return Vec::new();
    };
    let wanted = exe_path.to_string_lossy().to_lowercase();
    let mut found = Vec::new();
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snapshot == INVALID_HANDLE_VALUE {
            return found;
        }
        let mut process: PROCESSENTRY32W = std::mem::zeroed();
        process.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut more = Process32FirstW(snapshot, &mut process) != 0;
        while more {
            let length = process.szExeFile.iter().position(|c| *c == 0).unwrap_or(process.szExeFile.len());
            if String::from_utf16_lossy(&process.szExeFile[..length]).eq_ignore_ascii_case(&exe_name) {
                let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, process.th32ProcessID);
                if !handle.is_null() {
                    let mut buffer = [0u16; 1024];
                    let mut size = buffer.len() as u32;
                    if QueryFullProcessImageNameW(handle, 0, buffer.as_mut_ptr(), &mut size) != 0
                        && String::from_utf16_lossy(&buffer[..size as usize]).to_lowercase() == wanted
                    {
                        found.push(process.th32ProcessID);
                    }
                    CloseHandle(handle);
                }
            }
            more = Process32NextW(snapshot, &mut process) != 0;
        }
        CloseHandle(snapshot);
    }
    found
}

unsafe extern "system" fn close_main_windows(window: HWND, targets: LPARAM) -> BOOL {
    let targets = &*(targets as *const Vec<u32>);
    let mut process = 0;
    GetWindowThreadProcessId(window, &mut process);
    if targets.contains(&process) && IsWindowVisible(window) != 0 && GetWindow(window, GW_OWNER).is_null() {
        PostMessageW(window, WM_CLOSE, 0, 0);
    }
    1
}

fn run_to_exit(file: &Path, args: &[String], label: &str) -> Result<(), String> {
    let status = Command::new(file)
        .args(args)
        .creation_flags(CREATE_NO_WINDOW)
        .status()
        .map_err(|error| format!("{label}を起動できませんでした（{error}）"))?;
    if status.success() {
        return Ok(());
    }
    match status.code() {
        Some(code) => Err(format!("{label}が終了コード{code}で終了しました")),
        None => Err(format!("{label}が途中で終了しました")),
    }
}

pub struct RealSystem;

impl System for RealSystem {
    fn read_installed(&self, app: &AppDef) -> Result<Option<Installed>, String> {
        const ROOTS: [(winreg::HKEY, &str); 3] = [
            (HKEY_CURRENT_USER, r"Software\Microsoft\Windows\CurrentVersion\Uninstall"),
            (HKEY_LOCAL_MACHINE, r"Software\Microsoft\Windows\CurrentVersion\Uninstall"),
            (HKEY_LOCAL_MACHINE, r"Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall"),
        ];
        for (hive, root) in ROOTS {
            let Ok(key) = RegKey::predef(hive).open_subkey(format!(r"{root}\{}", app.registry_key)) else {
                continue;
            };
            let location: String = key.get_value("InstallLocation").unwrap_or_default();
            let uninstall: String = key.get_value("UninstallString").unwrap_or_default();
            return Ok(Some(Installed {
                version: key.get_value("DisplayVersion").ok(),
                exe_path: install_dir_from(&location, &uninstall)
                    .map(|dir| dir.join(app.exe_name))
                    .filter(|path| path.exists()),
                uninstaller: uninstaller_from(&location, &uninstall, app, &|path| path.exists()),
            }));
        }
        Ok(None)
    }

    fn is_running(&self, _app: &AppDef, exe_path: Option<&Path>) -> Result<bool, String> {
        Ok(exe_path.is_some_and(|path| !processes_at(path).is_empty()))
    }

    // Asks the app to close the same way the window's X button does and never
    // force-kills it: it may be streaming or holding unsaved work.
    fn close_app(&self, _app: &AppDef, exe_path: &Path) -> Result<bool, String> {
        let targets = processes_at(exe_path);
        if targets.is_empty() {
            return Ok(true);
        }
        unsafe {
            EnumWindows(Some(close_main_windows), &targets as *const Vec<u32> as LPARAM);
        }
        let deadline = Instant::now() + Duration::from_secs(20);
        while Instant::now() < deadline {
            if processes_at(exe_path).is_empty() {
                return Ok(true);
            }
            thread::sleep(Duration::from_millis(500));
        }
        Ok(false)
    }

    fn launch(&self, exe_path: &Path) -> Result<(), String> {
        let dir = exe_path.parent().map(Path::to_path_buf).unwrap_or_default();
        Command::new(exe_path).current_dir(dir).spawn().map(|_| ()).map_err(|error| error.to_string())
    }

    fn run_installer(&self, _app: &AppDef, file: &Path, args: &[&str]) -> Result<(), String> {
        let args: Vec<String> = args.iter().map(|arg| (*arg).to_string()).collect();
        run_to_exit(file, &args, "インストーラー")
    }

    fn run_uninstaller(&self, command: &UninstallCommand) -> Result<(), String> {
        run_to_exit(&command.file, &command.args, "アンインストーラー")
    }

    fn open_url(&self, url: &str) -> Result<(), String> {
        Command::new("rundll32")
            .args(["url.dll,FileProtocolHandler", url])
            .spawn()
            .map(|_| ())
            .map_err(|error| error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{APPS, LAUNCHER};

    const VOICE_DIR: &str = r"C:\Users\a b\AppData\Local\YuiVoiceChanger";

    fn exists(_: &Path) -> bool {
        true
    }

    #[test]
    fn a_missing_registry_key_means_not_installed() {
        let missing = AppDef { registry_key: "YuiLauncherTestNoSuchKey", ..LAUNCHER };
        assert!(RealSystem.read_installed(&missing).unwrap().is_none());
    }

    #[test]
    fn a_process_with_the_same_name_elsewhere_is_not_the_installed_app() {
        let own = std::env::current_exe().unwrap();
        let name = own.file_name().unwrap();
        let elsewhere = Path::new(r"C:\NoSuchFolder").join(name);
        assert!(RealSystem.is_running(&LAUNCHER, Some(&own)).unwrap());
        assert!(!RealSystem.is_running(&LAUNCHER, Some(&elsewhere)).unwrap());
        assert!(!RealSystem.is_running(&LAUNCHER, None).unwrap());
    }

    #[test]
    fn closing_ignores_a_process_with_the_same_name_elsewhere() {
        let own = std::env::current_exe().unwrap();
        let elsewhere = Path::new(r"C:\NoSuchFolder").join(own.file_name().unwrap());
        assert!(RealSystem.close_app(&LAUNCHER, &elsewhere).unwrap());
        assert!(RealSystem.is_running(&LAUNCHER, Some(&own)).unwrap());
    }

    #[test]
    fn parses_quoted_uninstall_commands() {
        assert_eq!(
            parse_command(r#""C:\a b\Uninstall App.exe" /currentuser"#),
            Some(UninstallCommand { file: PathBuf::from(r"C:\a b\Uninstall App.exe"), args: vec!["/currentuser".to_string()] })
        );
        assert_eq!(parse_command(r#""C:\App\unins000.exe""#).unwrap().args, Vec::<String>::new());
        assert_eq!(parse_command(r"C:\App\unins000.exe"), None);
        assert_eq!(parse_command(""), None);
    }

    #[test]
    fn keeps_currentuser_for_electron_builder_apps() {
        let command = uninstaller_from("", r#""C:\P\YuiLiveKit\Uninstall YuiLiveKit.exe" /currentuser"#, &APPS[2], &exists).unwrap();
        assert_eq!(command.args, ["/currentuser", "/S"]);
    }

    #[test]
    fn builds_the_command_for_inno_setup_apps() {
        let command = uninstaller_from(r"C:\P\YuiTracking\", r#""C:\P\YuiTracking\unins000.exe""#, &APPS[0], &exists).unwrap();
        assert_eq!(command.file, PathBuf::from(r"C:\P\YuiTracking\unins000.exe"));
        assert_eq!(command.args, ["/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART"]);
    }

    #[test]
    fn rejects_uninstallers_outside_the_folder_non_exe_files_and_missing_files() {
        let app = &APPS[1];
        assert_eq!(uninstaller_from(r"C:\P\App", r#""C:\Temp\evil.exe""#, app, &exists), None);
        assert_eq!(uninstaller_from(r"C:\P\App", r#""C:\P\App\..\evil.exe""#, app, &exists), None);
        assert_eq!(uninstaller_from(r"C:\P\App", r#""C:\P\App\run.cmd""#, app, &exists), None);
        assert_eq!(uninstaller_from(r"C:\P\App", r#""C:\P\App\unins.exe""#, app, &|_| false), None);
        assert_eq!(uninstaller_from(r"C:\P\App", "", app, &exists), None);
    }

    #[test]
    fn install_dir_prefers_install_location() {
        assert_eq!(install_dir_from(r"C:\Apps\YuiTracking\", r#""C:\Other\unins000.exe""#), Some(PathBuf::from(r"C:\Apps\YuiTracking\")));
    }

    #[test]
    fn install_dir_falls_back_to_the_uninstaller_folder() {
        assert_eq!(
            install_dir_from("", r#""C:\Users\a b\Programs\App\Uninstall App.exe" /currentuser"#),
            Some(PathBuf::from(r"C:\Users\a b\Programs\App"))
        );
        assert_eq!(install_dir_from("", ""), None);
    }

    #[test]
    fn handles_what_tauris_installer_writes() {
        let location = format!("\"{VOICE_DIR}\"");
        let uninstall = format!("\"{VOICE_DIR}\\uninstall.exe\"");
        assert_eq!(install_dir_from(&location, &uninstall), Some(PathBuf::from(VOICE_DIR)));
        let command = uninstaller_from(&location, &uninstall, &APPS[1], &exists).unwrap();
        assert_eq!(command.file, Path::new(VOICE_DIR).join("uninstall.exe"));
        assert_eq!(command.args, ["/S"]);
    }
}
