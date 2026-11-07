//! What the launcher needs from the operating system. windows.rs and macos.rs provide it.

use crate::catalog::AppDef;
use std::path::{Path, PathBuf};

/// How to remove an app: an uninstaller to run on Windows, the bundle to move to the Trash on a Mac.
#[derive(Clone, Debug, PartialEq)]
pub struct UninstallCommand {
    pub file: PathBuf,
    pub args: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct Installed {
    pub version: Option<String>,
    /// What opening the app runs: an exe on Windows, the .app bundle on a Mac.
    pub exe_path: Option<PathBuf>,
    pub uninstaller: Option<UninstallCommand>,
}

pub trait System: Send + Sync {
    fn read_installed(&self, app: &AppDef) -> Result<Option<Installed>, String>;
    fn is_running(&self, app: &AppDef, exe_path: Option<&Path>) -> Result<bool, String>;
    fn close_app(&self, app: &AppDef, exe_path: &Path) -> Result<bool, String>;
    fn launch(&self, exe_path: &Path) -> Result<(), String>;
    fn run_installer(&self, app: &AppDef, file: &Path, args: &[&str]) -> Result<(), String>;
    fn run_uninstaller(&self, command: &UninstallCommand) -> Result<(), String>;
    fn open_url(&self, url: &str) -> Result<(), String>;
}
