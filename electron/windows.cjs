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

function powershell(script) {
  const prelude = '[Console]::OutputEncoding=[Text.Encoding]::UTF8;';
  return exec(POWERSHELL, ['-NoProfile', '-NonInteractive', '-Command', prelude + script]);
}

// electron-builder's NSIS installer leaves InstallLocation empty, so fall back
// to the folder of the uninstaller.
function installDirFrom({ InstallLocation, UninstallString }) {
  if (InstallLocation) return InstallLocation;
  const match = /^"([^"]+)"/.exec(UninstallString || '');
  return match ? path.dirname(match[1]) : null;
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
  };
}

async function isRunning(app) {
  const output = await exec(TASKLIST, ['/FI', `IMAGENAME eq ${app.exeName}`, '/FO', 'CSV', '/NH']);
  return output.toLowerCase().includes(`"${app.exeName.toLowerCase()}"`);
}

// Asks the app to close the same way the window's X button does and never
// force-kills it: it may be streaming or holding unsaved work.
async function closeApp(app, { timeoutMs = 20000, intervalMs = 500 } = {}) {
  const name = path.basename(app.exeName, '.exe');
  await powershell(`Get-Process -Name '${name}' -ErrorAction SilentlyContinue|`
    + 'Where-Object {$_.MainWindowHandle -ne 0}|ForEach-Object {[void]$_.CloseMainWindow()}');
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if (!(await isRunning(app))) return true;
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

function runInstaller(installerPath, args) {
  return new Promise((resolve, reject) => {
    const child = spawn(installerPath, args, { windowsHide: true, stdio: 'ignore' });
    child.once('error', reject);
    child.once('exit', code => {
      if (code === 0) resolve();
      else reject(new Error(`インストーラーが終了コード${code}で終了しました`));
    });
  });
}

module.exports = { installDirFrom, readInstalled, isRunning, closeApp, launch, runInstaller };
