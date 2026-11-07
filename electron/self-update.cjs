'use strict';

const fs = require('node:fs');
const path = require('node:path');

const RETRY_AFTER_FAILURE_MS = 30 * 60 * 1000;
const MAX_LOG_BYTES = 1024 * 1024;

// Writes what the updater reports to a file, so a failed update can be traced later.
function createFileLogger(file, { maxBytes = MAX_LOG_BYTES } = {}) {
  function write(level, message) {
    const text = message instanceof Error ? message.stack || message.message : String(message);
    // A differential download logs thousands of these.
    if (text.startsWith('download range')) return;
    try {
      fs.mkdirSync(path.dirname(file), { recursive: true });
      if (fs.existsSync(file) && fs.statSync(file).size > maxBytes) {
        fs.renameSync(file, `${file}.old`);
      }
      fs.appendFileSync(file, `${new Date().toISOString()} ${level} ${text}\n`);
    } catch (error) {
      console.error('Failed to write the update log', error);
    }
  }
  return {
    info: message => write('INFO', message),
    warn: message => write('WARN', message),
    error: message => write('ERROR', message),
    debug: () => {},
  };
}

// Tracks the launcher's own update. updater is electron-updater's autoUpdater,
// or null when running from source, where there is nothing to update.
function createSelfUpdater({
  updater,
  version,
  logger,
  onChange = () => {},
  retryAfterMs = RETRY_AFTER_FAILURE_MS,
}) {
  const state = {
    currentVersion: version,
    phase: updater ? 'idle' : 'unavailable',
    latestVersion: null,
    percent: null,
    error: null,
  };
  let retryTimer = null;
  let inFlight = null;

  const snapshot = () => ({ ...state });

  function set(changes) {
    Object.assign(state, changes);
    onChange(snapshot());
  }

  // A failed check arrives both as an error event and as a rejected promise.
  function fail(error) {
    if (state.phase !== 'error') {
      if (logger) logger.error(error);
      const message = state.phase === 'downloading'
        ? 'YuiLauncherの更新をダウンロードできませんでした。しばらくしてからもう一度お試しください。'
        : 'YuiLauncherの更新を確認できませんでした。しばらくしてからもう一度お試しください。';
      set({ phase: 'error', percent: null, error: message });
    }
    clearTimeout(retryTimer);
    retryTimer = setTimeout(check, retryAfterMs);
    if (retryTimer.unref) retryTimer.unref();
  }

  if (updater) {
    updater.autoDownload = true;
    updater.autoInstallOnAppQuit = true;
    updater.logger = logger;
    updater.on('checking-for-update', () => set({ phase: 'checking', error: null }));
    updater.on('update-not-available', () => set({ phase: 'latest', latestVersion: null, percent: null, error: null }));
    updater.on('update-available', info => set({ phase: 'downloading', latestVersion: info.version, percent: 0, error: null }));
    updater.on('download-progress', progress => {
      const percent = Math.floor(progress.percent);
      if (state.phase === 'downloading' && percent !== state.percent) set({ percent });
    });
    updater.on('update-downloaded', info => set({ phase: 'ready', latestVersion: info.version, percent: null, error: null }));
    // A failed download only arrives here: checkForUpdates has already resolved by then.
    updater.on('error', fail);
  }

  // Never rejects. A ready update stays ready, and a running one isn't started twice.
  function check() {
    if (!updater || state.phase === 'ready' || state.phase === 'downloading') return Promise.resolve();
    if (inFlight) return inFlight;
    clearTimeout(retryTimer);
    inFlight = (async () => {
      try {
        await updater.checkForUpdates();
      } catch (error) {
        fail(error);
      } finally {
        inFlight = null;
      }
    })();
    return inFlight;
  }

  function install() {
    if (!updater || state.phase !== 'ready') return false;
    updater.quitAndInstall();
    return true;
  }

  return { getState: snapshot, check, install };
}

module.exports = { createFileLogger, createSelfUpdater };
