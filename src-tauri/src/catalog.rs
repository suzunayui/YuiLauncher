//! Apps managed by the launcher. Add an entry to APPS to manage another app.

pub struct AppDef {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    pub repo: &'static str,
    /// The app's key under ...\CurrentVersion\Uninstall.
    pub registry_key: &'static str,
    pub exe_name: &'static str,
    pub installer_prefix: &'static str,
    pub installer_suffix: &'static str,
    pub silent_args: &'static [&'static str],
    /// Added when updating. Tauri's installer needs /UPDATE to not bring back a removed desktop shortcut.
    pub update_args: &'static [&'static str],
    pub uninstall_args: &'static [&'static str],
    pub uninstall_note: &'static str,
    pub icon: &'static str,
}

impl AppDef {
    /// The installer's name is the prefix, a version, then the suffix.
    pub fn is_installer(&self, name: &str) -> bool {
        name.strip_prefix(self.installer_prefix)
            .and_then(|rest| rest.strip_suffix(self.installer_suffix))
            .is_some_and(|version| !version.is_empty() && version.chars().all(|c| c.is_ascii_digit() || c == '.'))
    }

    pub fn is_checksums(&self, name: &str) -> bool {
        name.starts_with("SHA256SUMS") && name.ends_with(".txt")
    }
}

pub static APPS: [AppDef; 3] = [
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
    },
    AppDef {
        id: "yuilivekit",
        name: "YuiLiveKit",
        description: "OBS向けのコメント表示と音声字幕",
        repo: "suzunayui/YuiLiveKit",
        registry_key: "54a05e14-5bce-5486-8d4f-9dc5271d69b3",
        exe_name: "YuiLiveKit.exe",
        installer_prefix: "YuiLiveKit-Setup-",
        installer_suffix: ".exe",
        silent_args: &["/S"],
        update_args: &[],
        uninstall_args: &["/S"],
        uninstall_note: "コメント、設定、字幕用のモデルなどのデータは残ります。",
        icon: "../assets/apps/YuiLiveKit.ico",
    },
];

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
    fn ids_and_executables_are_unique() {
        assert_eq!(APPS.iter().map(|app| app.id).collect::<HashSet<_>>().len(), APPS.len());
        assert_eq!(APPS.iter().map(|app| app.exe_name).collect::<HashSet<_>>().len(), APPS.len());
    }

    #[test]
    fn every_app_defines_how_to_uninstall_and_what_stays() {
        for app in &APPS {
            assert!(!app.uninstall_args.is_empty(), "{}", app.id);
            assert!(!app.uninstall_note.is_empty(), "{}", app.id);
        }
    }

    #[test]
    fn recognises_installer_names() {
        assert!(APPS[0].is_installer("YuiTracking-Setup-2026.1107.1-x64.exe"));
        assert!(!APPS[0].is_installer("YuiTracking-Setup-2026.1107.1.exe"));
        assert!(!APPS[1].is_installer("YuiVoiceChanger-Setup-.exe"));
        assert!(!APPS[1].is_installer("YuiVoiceChanger-Setup-1.0-beta.exe"));
        assert!(LAUNCHER.is_installer("YuiLauncher-Setup-2026.1107.10.exe"));
        assert!(!LAUNCHER.is_installer("YuiLauncher-Setup-2026.1107.10.exe.blockmap"));
    }
}
