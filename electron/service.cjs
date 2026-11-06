'use strict';

const fs = require('node:fs');
const path = require('node:path');
const { compareVersions, isVersion } = require('./version.cjs');
const { parseChecksums } = require('./releases.cjs');

class UserError extends Error {}

function createService({
  catalog,
  system,
  releases,
  downloader,
  tempRoot,
  onChange = () => {},
  confirmTimeoutMs = 60000,
  confirmIntervalMs = 1000,
}) {
  const entries = new Map(catalog.map(app => [app.id, {
    app,
    installed: false,
    version: null,
    exePath: null,
    running: false,
    release: null,
    checkError: null,
    task: null,
    error: null,
    notice: null,
    abort: null,
  }]));
  let checking = false;
  let lastCheckedAt = null;
  let inFlightCheck = null;

  function entryFor(id) {
    const entry = entries.get(id);
    if (!entry) throw new UserError('不明なアプリです');
    return entry;
  }

  function updateAvailable(entry) {
    if (!entry.release) return false;
    if (!entry.installed) return true;
    // An unreadable local version must still be repairable by installing.
    if (!isVersion(entry.version)) return true;
    return compareVersions(entry.release.version, entry.version) > 0;
  }

  function snapshot() {
    return {
      checking,
      lastCheckedAt,
      apps: [...entries.values()].map(entry => ({
        id: entry.app.id,
        name: entry.app.name,
        description: entry.app.description,
        icon: entry.app.icon,
        installed: entry.installed,
        version: entry.version,
        canOpen: Boolean(entry.exePath),
        running: entry.running,
        latestVersion: entry.release ? entry.release.version : null,
        hasReleasePage: Boolean(entry.release && entry.release.pageUrl),
        updateAvailable: updateAvailable(entry),
        checkError: entry.checkError,
        task: entry.task,
        error: entry.error,
        notice: entry.notice,
      })),
    };
  }

  const changed = () => onChange(snapshot());

  async function readLocal(entry) {
    const installed = await system.readInstalled(entry.app);
    entry.installed = Boolean(installed);
    entry.version = installed ? installed.version : null;
    entry.exePath = installed ? installed.exePath : null;
  }

  async function refreshLocal() {
    await Promise.all([...entries.values()].map(async entry => {
      if (entry.task) return;
      try {
        await readLocal(entry);
        entry.running = await system.isRunning(entry.app);
      } catch (error) {
        console.error(`Failed to read ${entry.app.id}`, error);
      }
    }));
    changed();
  }

  async function refreshRunning() {
    let dirty = false;
    await Promise.all([...entries.values()].map(async entry => {
      if (entry.task) return;
      try {
        const running = await system.isRunning(entry.app);
        if (running !== entry.running) {
          entry.running = running;
          dirty = true;
        }
      } catch (error) {
        console.error(`Failed to check ${entry.app.id}`, error);
      }
    }));
    if (dirty) changed();
  }

  async function checkRemote() {
    await Promise.all([...entries.values()].map(async entry => {
      try {
        entry.release = await releases.latest(entry.app);
        entry.checkError = entry.release ? null : 'Windows版のリリースが見つかりませんでした。';
      } catch (error) {
        // Keep the last known release so an offline check doesn't hide an update.
        entry.checkError = error.message;
      }
    }));
  }

  function check() {
    if (inFlightCheck) return inFlightCheck;
    inFlightCheck = (async () => {
      checking = true;
      changed();
      try {
        await Promise.all([refreshLocal(), checkRemote()]);
        lastCheckedAt = Date.now();
      } finally {
        checking = false;
        inFlightCheck = null;
        changed();
      }
    })();
    return inFlightCheck;
  }

  function setTask(entry, phase, percent = null) {
    entry.task = { phase, percent };
    changed();
  }

  async function verifiedInstaller(entry, release, workDir) {
    const dest = path.join(workDir, release.installer.name);
    entry.abort = new AbortController();
    setTask(entry, 'downloading', 0);
    let lastPercent = 0;
    const { sha256 } = await downloader.download({
      url: release.installer.url,
      dest,
      expectedSize: release.installer.size,
      signal: entry.abort.signal,
      onProgress(received, total) {
        const percent = total ? Math.floor((received / total) * 100) : null;
        if (percent !== lastPercent) {
          lastPercent = percent;
          setTask(entry, 'downloading', percent);
        }
      },
    });
    entry.abort = null;

    setTask(entry, 'verifying');
    const expected = parseChecksums(await releases.fetchText(release.checksums.url)).get(release.installer.name);
    if (!expected) throw new UserError('チェックサムにこのインストーラーの記載がありません。');
    if (expected !== sha256) throw new UserError('ファイルの検証に失敗しました。もう一度お試しください。');
    return dest;
  }

  async function waitForVersion(entry, version) {
    const deadline = Date.now() + confirmTimeoutMs;
    for (;;) {
      await readLocal(entry);
      if (isVersion(entry.version) && compareVersions(entry.version, version) >= 0) return;
      if (Date.now() >= deadline) throw new UserError('インストール後のバージョンを確認できませんでした。');
      await new Promise(resolve => setTimeout(resolve, confirmIntervalMs));
    }
  }

  // Downloads and verifies first so the app stays open until the last moment.
  async function install(id, { closeRunning = false } = {}) {
    const entry = entryFor(id);
    if (entry.task) return { status: 'busy' };
    const release = entry.release;
    if (!release) throw new UserError('インストールできるリリースがありません。');

    entry.error = null;
    entry.notice = null;
    entry.running = await system.isRunning(entry.app);
    if (entry.running && !closeRunning) {
      changed();
      return { status: 'needs-close' };
    }

    let workDir = null;
    let closedApp = false;
    try {
      workDir = await fs.promises.mkdtemp(path.join(tempRoot, 'update-'));
      const installer = await verifiedInstaller(entry, release, workDir);

      if (await system.isRunning(entry.app)) {
        if (!closeRunning) throw new UserError(`${entry.app.name}が起動されたため中止しました。もう一度お試しください。`);
        setTask(entry, 'closing');
        if (!(await system.closeApp(entry.app))) {
          throw new UserError(`${entry.app.name}を閉じられませんでした。アプリ側で閉じてから、もう一度お試しください。`);
        }
        closedApp = true;
      }

      setTask(entry, 'installing');
      await system.runInstaller(installer, entry.app.silentArgs);
      setTask(entry, 'confirming');
      await waitForVersion(entry, release.version);

      entry.notice = `${release.version}をインストールしました。`;
      await relaunchIfClosed(entry, closedApp);
      return { status: 'done' };
    } catch (error) {
      if (error.name === 'AbortError') return { status: 'cancelled' };
      entry.error = error instanceof UserError ? error.message : `更新に失敗しました（${error.message}）`;
      await relaunchIfClosed(entry, closedApp);
      return { status: 'failed' };
    } finally {
      entry.abort = null;
      entry.task = null;
      if (workDir) await fs.promises.rm(workDir, { recursive: true, force: true }).catch(() => {});
      await readLocal(entry).catch(() => {});
      entry.running = await system.isRunning(entry.app).catch(() => entry.running);
      changed();
    }
  }

  async function relaunchIfClosed(entry, closedApp) {
    if (!closedApp) return;
    try {
      await readLocal(entry);
      if (entry.exePath) await system.launch(entry.exePath);
    } catch (error) {
      console.error(`Failed to relaunch ${entry.app.id}`, error);
    }
  }

  function cancel(id) {
    const entry = entryFor(id);
    if (entry.abort) entry.abort.abort();
  }

  async function open(id) {
    const entry = entryFor(id);
    if (entry.task) return;
    entry.error = null;
    try {
      if (!entry.exePath) throw new UserError(`${entry.app.name}の実行ファイルが見つかりません。`);
      await system.launch(entry.exePath);
      entry.running = true;
    } catch (error) {
      entry.error = error instanceof UserError ? error.message : `起動できませんでした（${error.message}）`;
    }
    changed();
  }

  function releasePageUrl(id) {
    const entry = entryFor(id);
    const url = entry.release && entry.release.pageUrl;
    return url && url.startsWith(`https://github.com/${entry.app.repo}/`) ? url : null;
  }

  const isBusy = () => [...entries.values()].some(entry => entry.task);

  return { snapshot, check, refreshLocal, refreshRunning, install, cancel, open, releasePageUrl, isBusy };
}

module.exports = { createService, UserError };
