'use strict';

const { app, BrowserWindow, Menu, dialog, ipcMain, nativeTheme, shell } = require('electron');
const path = require('node:path');
const catalog = require('./catalog.cjs');
const { createService } = require('./service.cjs');

const CHECK_INTERVAL_MS = 6 * 60 * 60 * 1000;
const RUNNING_INTERVAL_MS = 5000;
const SUPPORT_URL = 'https://suzunayui.fanbox.cc/';

let window = null;
let service = null;
let selfUpdateReady = false;

function send(channel, payload) {
  if (window && !window.isDestroyed()) window.webContents.send(channel, payload);
}

function dependencies() {
  // Lets the UI tests run the flow without touching GitHub or real installs.
  const fixture = process.env.YUI_LAUNCHER_FIXTURE;
  if (fixture && !app.isPackaged) return require(path.resolve(fixture)).create();
  const { createReleaseClient } = require('./releases.cjs');
  return {
    catalog,
    system: require('./windows.cjs'),
    releases: createReleaseClient(),
    downloader: require('./download.cjs'),
  };
}

function handle(channel, listener) {
  ipcMain.handle(channel, (event, ...args) => {
    if (!window || event.senderFrame !== window.webContents.mainFrame) throw new Error('Untrusted sender');
    return listener(...args);
  });
}

function registerIpc() {
  handle('state:get', () => service.snapshot());
  handle('state:check', () => service.check());
  handle('app:open', id => service.open(id));
  handle('app:install', (id, closeRunning) => service.install(id, { closeRunning: closeRunning === true }));
  handle('app:uninstall', (id, closeRunning) => service.uninstall(id, { closeRunning: closeRunning === true }));
  handle('app:cancel', id => service.cancel(id));
  handle('app:release-page', id => {
    const url = service.releasePageUrl(id);
    return url ? shell.openExternal(url) : undefined;
  });
  handle('support:open', () => shell.openExternal(SUPPORT_URL));
  handle('self:install', () => {
    if (!selfUpdateReady || service.isBusy()) return;
    require('electron-updater').autoUpdater.quitAndInstall();
  });
}

function createWindow() {
  window = new BrowserWindow({
    width: 760,
    height: 520,
    minWidth: 520,
    minHeight: 400,
    title: 'YuiLauncher',
    icon: path.join(__dirname, '..', 'assets', 'icon.ico'),
    backgroundColor: nativeTheme.shouldUseDarkColors ? '#15181b' : '#f5f6f7',
    show: false,
    webPreferences: {
      preload: path.join(__dirname, 'preload.cjs'),
      contextIsolation: true,
      sandbox: true,
      nodeIntegration: false,
    },
  });
  window.webContents.on('will-navigate', event => event.preventDefault());
  window.webContents.setWindowOpenHandler(() => ({ action: 'deny' }));
  window.once('ready-to-show', () => window.show());
  window.on('focus', () => service.refreshLocal());
  window.on('close', event => {
    if (!service.isBusy()) return;
    event.preventDefault();
    dialog.showMessageBox(window, {
      type: 'info',
      title: 'YuiLauncher',
      message: '処理が終わるまでお待ちください。',
      detail: 'ダウンロード中であれば、キャンセルしてから閉じられます。',
    });
  });
  window.on('closed', () => {
    window = null;
  });
  window.loadFile(path.join(__dirname, '..', 'renderer', 'index.html'));
}

// The launcher's own update. Failures stay silent: the launcher keeps working,
// and the update is retried at the next check.
function checkSelfUpdate() {
  if (!app.isPackaged) return;
  const { autoUpdater } = require('electron-updater');
  autoUpdater.autoDownload = true;
  autoUpdater.autoInstallOnAppQuit = true;
  autoUpdater.removeAllListeners('update-downloaded');
  autoUpdater.on('update-downloaded', info => {
    selfUpdateReady = true;
    send('self-update', { version: info.version });
  });
  autoUpdater.removeAllListeners('error');
  autoUpdater.on('error', error => console.error('Self update failed', error));
  autoUpdater.checkForUpdates().catch(error => console.error('Self update check failed', error));
}

if (!app.requestSingleInstanceLock()) {
  app.quit();
} else {
  app.on('second-instance', () => {
    if (!window) return;
    if (window.isMinimized()) window.restore();
    window.focus();
  });

  app.whenReady().then(() => {
    app.setAppUserModelId('com.suzunayui.yuilauncher');
    Menu.setApplicationMenu(null);
    const deps = dependencies();
    service = createService({
      catalog: deps.catalog,
      system: deps.system,
      releases: deps.releases,
      downloader: deps.downloader,
      tempRoot: app.getPath('temp'),
      onChange: state => send('state', state),
    });
    registerIpc();
    createWindow();
    service.check();
    checkSelfUpdate();
    setInterval(() => {
      service.check();
      checkSelfUpdate();
    }, CHECK_INTERVAL_MS);
    setInterval(() => service.refreshRunning(), RUNNING_INTERVAL_MS);
  });

  app.on('window-all-closed', () => app.quit());
}
