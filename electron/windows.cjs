'use strict';

const { execFile, spawn } = require('node:child_process');
const fs = require('node:fs');
const path = require('node:path');

const SYSTEM32 = path.join(process.env.SystemRoot || 'C:\\Windows', 'System32');
const POWERSHELL = path.join(SYSTEM32, 'WindowsPowerShell', 'v1.0', 'powershell.exe');
const TASKLIST = path.join(SYSTEM32, 'tasklist.exe');

const UNINSTALL_ROOTS = [
  'HKCU:\\Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall',
  'HKLM:\\Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall',
  'HKLM:\\Software\\WOW6432Node\\Microsoft\\Windows\\CurrentVersion\\Uninstall',
];

function exec(file, args, timeout = 15000) {
  return new Promise((resolve, reject) => {
    execFile(file, args, { windowsHide: true, timeout, encoding: 'utf8' }, (error, stdout) => {
      if (error) reject(error);
      else resolve(stdout);
    });
  });
}

// exit 0: a missing registry key or process otherwise makes PowerShell exit with 1.
async function powershell(script) {
  const prelude = '[Console]::OutputEncoding=[Text.Encoding]::UTF8;';
  try {
    return await exec(POWERSHELL, ['-NoProfile', '-NonInteractive', '-Command', `${prelude}${script};exit 0`]);
  } catch (error) {
    throw new Error(`Windowsの情報を読み取れませんでした（${error.code}）`);
  }
}

const psQuote = value => `'${String(value).replace(/'/g, "''")}'`;

// electron-builder's NSIS installer leaves InstallLocation empty, so fall back
// to the folder of the uninstaller.
function installDirFrom({ InstallLocation, UninstallString }) {
  if (InstallLocation) return InstallLocation;
  const match = /^"([^"]+)"/.exec(UninstallString || '');
  return match ? path.dirname(match[1]) : null;
}

// Splits a registry command like: "C:\App\Uninstall App.exe" /currentuser
function parseCommand(command) {
  const match = /^"([^"]+)"\s*(.*)$/.exec(command || '');
  if (!match) return null;
  return { file: match[1], args: match[2].split(/\s+/).filter(Boolean) };
}

// Only runs an uninstaller that sits in the app's own install folder.
function uninstallerFrom(entry, app, exists = fs.existsSync) {
  const command = parseCommand(entry.UninstallString);
  const dir = installDirFrom(entry);
  if (!command || !dir || path.extname(command.file).toLowerCase() !== '.exe') return null;
  if (path.dirname(path.resolve(command.file)).toLowerCase() !== path.resolve(dir).toLowerCase()) return null;
  if (!exists(command.file)) return null;
  return { file: command.file, args: [...new Set([...command.args, ...app.uninstallArgs])] };
}

async function readInstalled(app) {
  const keys = UNINSTALL_ROOTS.map(root => `'${root}\\${app.registryKey}'`).join(',');
  const script = `foreach($k in ${keys}){$i=Get-ItemProperty -LiteralPath $k -ErrorAction SilentlyContinue;`
    + 'if($i){$i|Select-Object DisplayVersion,InstallLocation,UninstallString|ConvertTo-Json -Compress;break}}';
  const output = (await powershell(script)).trim();
  if (!output) return null;
  const entry = JSON.parse(output);
  const dir = installDirFrom(entry);
  const exePath = dir ? path.join(dir, app.exeName) : null;
  return {
    version: entry.DisplayVersion || null,
    exePath: exePath && fs.existsSync(exePath) ? exePath : null,
    uninstaller: uninstallerFrom(entry, app),
  };
}

// Matches by full path: a development build elsewhere can share the exe name
// and must never count as the installed app, or be closed by the launcher.
function processesAt(app, exePath) {
  const name = path.basename(app.exeName, '.exe');
  return `Get-Process -Name ${psQuote(name)} -ErrorAction SilentlyContinue|Where-Object {$_.Path -eq ${psQuote(exePath)}}`;
}

async function isRunning(app, exePath) {
  if (!exePath) return false;
  // tasklist is much faster than PowerShell, so it settles the common case.
  const output = await exec(TASKLIST, ['/FI', `IMAGENAME eq ${app.exeName}`, '/FO', 'CSV', '/NH']);
  if (!output.toLowerCase().includes(`"${app.exeName.toLowerCase()}"`)) return false;
  const matches = await powershell(`(@(${processesAt(app, exePath)})).Count`);
  return Number(matches.trim()) > 0;
}

// Asks the app to close the same way the window's X button does and never
// force-kills it: it may be streaming or holding unsaved work.
async function closeApp(app, exePath, { timeoutMs = 20000, intervalMs = 500 } = {}) {
  await powershell(`${processesAt(app, exePath)}|Where-Object {$_.MainWindowHandle -ne 0}|ForEach-Object {[void]$_.CloseMainWindow()}`);
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if (!(await isRunning(app, exePath))) return true;
    await new Promise(resolve => setTimeout(resolve, intervalMs));
  }
  return false;
}

function launch(exePath) {
  return new Promise((resolve, reject) => {
    const child = spawn(exePath, [], { cwd: path.dirname(exePath), detached: true, stdio: 'ignore' });
    child.once('error', reject);
    child.once('spawn', () => {
      child.unref();
      resolve();
    });
  });
}

function runToExit(file, args, label) {
  return new Promise((resolve, reject) => {
    const child = spawn(file, args, { windowsHide: true, stdio: 'ignore' });
    child.once('error', reject);
    child.once('exit', code => {
      if (code === 0) resolve();
      else reject(new Error(`${label}が終了コード${code}で終了しました`));
    });
  });
}

const runInstaller = (installerPath, args) => runToExit(installerPath, args, 'インストーラー');
const runUninstaller = ({ file, args }) => runToExit(file, args, 'アンインストーラー');

module.exports = {
  installDirFrom,
  parseCommand,
  uninstallerFrom,
  readInstalled,
  isRunning,
  closeApp,
  launch,
  runInstaller,
  runUninstaller,
};
