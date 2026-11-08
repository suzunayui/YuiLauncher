//! Apps managed by the launcher. Add an entry to APPS to manage another app.

/// How an app is installed on a Mac. An app without one isn't shown there.
#[derive(Clone, Copy)]
pub struct MacDef {
    /// The bundle's folder name in /Applications.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub bundle: &'static str,
    /// The release asset is the prefix, a version, then the suffix: a .zip or .dmg holding the bundle.
    pub installer_prefix: &'static str,
    pub installer_suffix: &'static str,
    pub uninstall_note: &'static str,
}

pub struct AppDef {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    pub repo: &'static str,
    /// The app's key under ...\CurrentVersion\Uninstall.
    #[cfg_attr(not(windows), allow(dead_code))]
    pub registry_key: &'static str,
    #[cfg_attr(not(windows), allow(dead_code))]
    pub exe_name: &'static str,
    pub installer_prefix: &'static str,
    pub installer_suffix: &'static str,
    pub silent_args: &'static [&'static str],
    /// Added when updating. Tauri's installer needs /UPDATE to not bring back a removed desktop shortcut.
    pub update_args: &'static [&'static str],
    #[cfg_attr(not(windows), allow(dead_code))]
    pub uninstall_args: &'static [&'static str],
    pub uninstall_note: &'static str,
    pub icon: &'static str,
    pub mac: Option<MacDef>,
}

pub const ON_MAC: bool = cfg!(target_os = "macos");

impl AppDef {
    fn installer_affixes(&self, mac: bool) -> Option<(&'static str, &'static str)> {
        if mac {
            self.mac.as_ref().map(|mac| (mac.installer_prefix, mac.installer_suffix))
        } else {
            Some((self.installer_prefix, self.installer_suffix))
        }
    }

    /// The installer's name is the prefix, a version, then the suffix.
    pub fn is_installer_for(&self, name: &str, mac: bool) -> bool {
        let Some((prefix, suffix)) = self.installer_affixes(mac) else { return false };
        name.strip_prefix(prefix)
            .and_then(|rest| rest.strip_suffix(suffix))
            .is_some_and(|version| !version.is_empty() && version.chars().all(|c| c.is_ascii_digit() || c == '.'))
    }

    /// What this version's installer is called, if the app has one for this platform.
    pub fn installer_name_for(&self, version: &str, mac: bool) -> Option<String> {
        self.installer_affixes(mac).map(|(prefix, suffix)| format!("{prefix}{version}{suffix}"))
    }

    pub fn is_checksums(&self, name: &str) -> bool {
        name.starts_with("SHA256SUMS") && name.ends_with(".txt")
    }

    /// The checksum files a release may hold, most specific first: the apps name them differently.
    pub fn checksum_names_for(&self, version: &str, mac: bool) -> Vec<String> {
        let mut names = Vec::new();
        if mac {
            names.push(format!("SHA256SUMS-{version}-mac-arm64.txt"));
            names.push("SHA256SUMS-mac-arm64.txt".to_string());
        }
        names.push(format!("SHA256SUMS-{version}.txt"));
        names.push("SHA256SUMS.txt".to_string());
        names
    }

    pub fn uninstall_note(&self) -> &'static str {
        match (&self.mac, ON_MAC) {
            (Some(mac), true) => mac.uninstall_note,
            _ => self.uninstall_note,
        }
    }
}

pub static APPS: [AppDef; 4] = [
    AppDef {
        id: "yuitracking",
        name: "YuiTracking",
        description: "VRMアバターのトラッキングと仮想カメラ",
        repo: "suzunayui/YuiTracking",
        registry_key: "{A4A68FE9-BAEA-4CBB-A7DD-877B688187BD}_is1",
        exe_name: "YuiTracking.exe",
        installer_prefix: "YuiTracking-Setup-",
        installer_suffix: "-x64.exe",
        silent_args: &["/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART"],
        update_args: &[],
        uninstall_args: &["/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART"],
        uninstall_note: "設定、VRMの履歴、ログは残ります。仮想カメラの登録は削除されます。",
        icon: "../assets/apps/YuiTracking.ico",
        mac: Some(MacDef {
            bundle: "YuiTracking.app",
            installer_prefix: "YuiTracking-",
            installer_suffix: "-arm64-mac.zip",
            uninstall_note: "アプリはゴミ箱に移動します。設定、VRMの履歴、ログは残ります。",
        }),
    },
    AppDef {
        id: "yuivoicechanger",
        name: "YuiVoiceChanger",
        description: "リアルタイムボイスチェンジャー",
        repo: "suzunayui/YuiVoiceChanger",
        registry_key: "YuiVoiceChanger",
        exe_name: "yui-voice-changer.exe",
        installer_prefix: "YuiVoiceChanger-Setup-",
        installer_suffix: ".exe",
        silent_args: &["/S"],
        update_args: &["/UPDATE"],
        uninstall_args: &["/S"],
        uninstall_note: "設定とPython環境（ホームの.YuiVoiceChanger）、音声モデル、OBSプラグインは残ります。",
        icon: "../assets/apps/YuiVoiceChanger.ico",
        mac: Some(MacDef {
            bundle: "YuiVoiceChanger.app",
            installer_prefix: "YuiVoiceChanger-",
            installer_suffix: "-mac-arm64.dmg",
            uninstall_note: "アプリはゴミ箱に移動します。設定とPython環境（ホームの.YuiVoiceChanger）、音声モデルは残ります。",
        }),
    },
    AppDef {
        id: "yuilivekit",
        name: "YuiLiveKit",
        description: "OBS向けのコメント表示と音声字幕",
        repo: "suzunayui/YuiLiveKit",
        // The Tauri version's NSIS installer, like YuiVoiceChanger. The Electron version used
        // "54a05e14-5bce-5486-8d4f-9dc5271d69b3" and is uninstalled separately.
        registry_key: "YuiLiveKit",
        exe_name: "YuiLiveKit.exe",
        installer_prefix: "YuiLiveKit-Setup-",
        installer_suffix: ".exe",
        silent_args: &["/S"],
        update_args: &["/UPDATE"],
        uninstall_args: &["/S"],
        uninstall_note: "コメント履歴、設定、字幕と翻訳の追加機能とモデル（AppData の com.suzunayui.yuilivekit フォルダ）は残ります。",
        icon: "../assets/apps/YuiLiveKit.ico",
        mac: None,
    },
    AppDef {
        id: "yuiclock",
        name: "YuiClock",
        description: "デスクトップに置いておけるデジタル時計",
        repo: "suzunayui/YuiClock",
        registry_key: "YuiClock",
        exe_name: "YuiClock.exe",
        installer_prefix: "YuiClock-Setup-",
        installer_suffix: ".exe",
        silent_args: &["/S"],
        update_args: &["/UPDATE"],
        uninstall_args: &["/S"],
        uninstall_note: "位置、色、フォントなどの設定は残ります。",
        icon: "../assets/apps/YuiClock.ico",
        mac: None,
    },
];

/// The apps this platform can install: on a Mac, only those with a Mac version.
pub fn platform_apps() -> Vec<&'static AppDef> {
    APPS.iter().filter(|app| !ON_MAC || app.mac.is_some()).collect()
}

/// The launcher itself, for finding its own releases. It is not listed as an app.
pub static LAUNCHER: AppDef = AppDef {
    id: "yuilauncher",
    name: "YuiLauncher",
    description: "",
    repo: "suzunayui/YuiLauncher",
    registry_key: "YuiLauncher",
    exe_name: "YuiLauncher.exe",
    installer_prefix: "YuiLauncher-Setup-",
    installer_suffix: ".exe",
    silent_args: &["/S"],
    update_args: &["/UPDATE"],
    uninstall_args: &["/S"],
    uninstall_note: "",
    icon: "",
    mac: Some(MacDef {
        bundle: "YuiLauncher.app",
        installer_prefix: "YuiLauncher-",
        installer_suffix: "-mac-arm64.zip",
        uninstall_note: "",
    }),
};

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::path::Path;

    #[test]
    fn every_icon_exists() {
        let renderer = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("renderer");
        for app in &APPS {
            assert!(renderer.join(app.icon).exists(), "{}", app.icon);
        }
    }

    #[test]
    fn ids_executables_and_bundles_are_unique() {
        assert_eq!(APPS.iter().map(|app| app.id).collect::<HashSet<_>>().len(), APPS.len());
        assert_eq!(APPS.iter().map(|app| app.exe_name).collect::<HashSet<_>>().len(), APPS.len());
        let bundles: Vec<_> = APPS.iter().filter_map(|app| app.mac.as_ref().map(|mac| mac.bundle)).collect();
        assert_eq!(bundles.iter().collect::<HashSet<_>>().len(), bundles.len());
        assert!(bundles.iter().all(|bundle| bundle.ends_with(".app")));
    }

    #[test]
    fn every_app_defines_how_to_uninstall_and_what_stays() {
        for app in &APPS {
            assert!(!app.uninstall_args.is_empty(), "{}", app.id);
            assert!(!app.uninstall_note.is_empty(), "{}", app.id);
            if let Some(mac) = &app.mac {
                assert!(!mac.uninstall_note.is_empty(), "{}", app.id);
            }
        }
    }

    #[test]
    fn recognises_windows_installer_names() {
        assert!(APPS[0].is_installer_for("YuiTracking-Setup-2026.1107.1-x64.exe", false));
        assert!(!APPS[0].is_installer_for("YuiTracking-Setup-2026.1107.1.exe", false));
        assert!(!APPS[0].is_installer_for("YuiTracking-2026.1107.1-arm64-mac.zip", false));
        assert!(!APPS[1].is_installer_for("YuiVoiceChanger-Setup-.exe", false));
        assert!(!APPS[1].is_installer_for("YuiVoiceChanger-Setup-1.0-beta.exe", false));
        assert!(LAUNCHER.is_installer_for("YuiLauncher-Setup-2026.1107.10.exe", false));
        assert!(!LAUNCHER.is_installer_for("YuiLauncher-Setup-2026.1107.10.exe.blockmap", false));
    }

    #[test]
    fn recognises_mac_installer_names() {
        assert!(APPS[0].is_installer_for("YuiTracking-2026.1107.1-arm64-mac.zip", true));
        assert!(!APPS[0].is_installer_for("YuiTracking-2026.1107.1-arm64.dmg", true));
        assert!(!APPS[0].is_installer_for("YuiTracking-Setup-2026.1107.1-x64.exe", true));
        assert!(APPS[1].is_installer_for("YuiVoiceChanger-2026.1103.34-mac-arm64.dmg", true));
        assert!(!APPS[1].is_installer_for("YuiVoiceChanger-0.3.0-beta.7-mac-arm64-preview.zip", true));
        assert!(!APPS[2].is_installer_for("YuiLiveKit-Setup-2026.1107.1.exe", true), "no Mac version");
        assert!(LAUNCHER.is_installer_for("YuiLauncher-2026.1107.13-mac-arm64.zip", true));
    }
}
