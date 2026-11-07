'use strict';

const test = require('node:test');
const assert = require('node:assert/strict');
const path = require('node:path');
const { installDirFrom, parseCommand, uninstallerFrom, readInstalled, isRunning, closeApp } = require('../../electron/windows.cjs');

// These use the real registry and process list, read-only.
test('a missing registry key means not installed, not an error', async () => {
  assert.equal(await readInstalled({ registryKey: 'YuiLauncherTestNoSuchKey', exeName: 'x.exe', uninstallArgs: [] }), null);
});

test('a process with the same name elsewhere is not the installed app', async () => {
  const node = { exeName: path.basename(process.execPath) };
  assert.equal(await isRunning(node, process.execPath), true);
  assert.equal(await isRunning(node, 'C:\\NoSuchFolder\\node.exe'), false);
  assert.equal(await isRunning(node, "C:\\It's\\node.exe"), false, 'quotes in a path are safe');
  assert.equal(await isRunning(node, null), false);
});

test('closing ignores a process with the same name elsewhere', async () => {
  const node = { exeName: path.basename(process.execPath) };
  assert.equal(await closeApp(node, 'C:\\NoSuchFolder\\node.exe', { timeoutMs: 100 }), true);
  assert.equal(await isRunning(node, process.execPath), true);
});

const exists = () => true;

test('parses quoted uninstall commands', () => {
  assert.deepEqual(parseCommand('"C:\\a b\\Uninstall App.exe" /currentuser'), { file: 'C:\\a b\\Uninstall App.exe', args: ['/currentuser'] });
  assert.deepEqual(parseCommand('"C:\\App\\unins000.exe"'), { file: 'C:\\App\\unins000.exe', args: [] });
  assert.equal(parseCommand('C:\\App\\unins000.exe'), null);
  assert.equal(parseCommand(undefined), null);
});

test('builds the real command shape for NSIS apps, keeping /currentuser', () => {
  const uninstaller = uninstallerFrom(
    { InstallLocation: '', UninstallString: '"C:\\P\\YuiVoiceChanger\\Uninstall YuiVoiceChanger.exe" /currentuser' },
    { uninstallArgs: ['/S'] },
    exists,
  );
  assert.deepEqual(uninstaller.args, ['/currentuser', '/S']);
});

test('builds the real command shape for Inno Setup apps', () => {
  const uninstaller = uninstallerFrom(
    { InstallLocation: 'C:\\P\\YuiTracking\\', UninstallString: '"C:\\P\\YuiTracking\\unins000.exe"' },
    { uninstallArgs: ['/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART'] },
    exists,
  );
  assert.equal(uninstaller.file, 'C:\\P\\YuiTracking\\unins000.exe');
  assert.deepEqual(uninstaller.args, ['/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART']);
});

test('rejects uninstallers outside the install folder, non-exe files and missing files', () => {
  const app = { uninstallArgs: ['/S'] };
  assert.equal(uninstallerFrom({ InstallLocation: 'C:\\P\\App', UninstallString: '"C:\\Temp\\evil.exe"' }, app, exists), null);
  assert.equal(uninstallerFrom({ InstallLocation: 'C:\\P\\App', UninstallString: '"C:\\P\\App\\run.cmd"' }, app, exists), null);
  assert.equal(uninstallerFrom({ InstallLocation: 'C:\\P\\App', UninstallString: '"C:\\P\\App\\unins.exe"' }, app, () => false), null);
  assert.equal(uninstallerFrom({ InstallLocation: 'C:\\P\\App' }, app, exists), null);
});

test('prefers InstallLocation', () => {
  assert.equal(installDirFrom({ InstallLocation: 'C:\\Apps\\YuiTracking\\', UninstallString: '"C:\\Other\\unins000.exe"' }), 'C:\\Apps\\YuiTracking\\');
});

test('falls back to the uninstaller folder when InstallLocation is empty', () => {
  assert.equal(
    installDirFrom({ InstallLocation: '', UninstallString: '"C:\\Users\\a b\\Programs\\YuiVoiceChanger\\Uninstall YuiVoiceChanger.exe" /currentuser' }),
    'C:\\Users\\a b\\Programs\\YuiVoiceChanger',
  );
});

test('returns null when neither value is usable', () => {
  assert.equal(installDirFrom({}), null);
});

// The values Tauri's NSIS installer really writes.
const tauriEntry = {
  InstallLocation: '"C:\\Users\\a b\\AppData\\Local\\YuiVoiceChanger"',
  UninstallString: '"C:\\Users\\a b\\AppData\\Local\\YuiVoiceChanger\\uninstall.exe"',
};

test('removes the quotes Tauri puts around InstallLocation', () => {
  assert.equal(installDirFrom(tauriEntry), 'C:\\Users\\a b\\AppData\\Local\\YuiVoiceChanger');
});

test('accepts the uninstaller of a Tauri install', () => {
  assert.deepEqual(uninstallerFrom(tauriEntry, { uninstallArgs: ['/S'] }, exists), {
    file: 'C:\\Users\\a b\\AppData\\Local\\YuiVoiceChanger\\uninstall.exe',
    args: ['/S'],
  });
});
