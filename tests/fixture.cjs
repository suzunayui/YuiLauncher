'use strict';

// Fake machine and GitHub for the UI test: no network, no real installs.
const fs = require('node:fs');

const HASH = 'c'.repeat(64);
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));

function create() {
  const realCatalog = require('../electron/catalog.cjs');
  const world = {
    yuitracking: { version: '2026.1106.5', running: true, latest: '2026.1107.1' },
    yuivoicechanger: { version: '2026.1104.31', running: false, latest: '2026.1104.31' },
    yuilivekit: { version: '2026.1107.1', running: false, latest: '2026.1107.1' },
  };
  const installerNames = {
    yuitracking: 'YuiTracking-Setup-2026.1107.1-x64.exe',
    yuivoicechanger: 'YuiVoiceChanger-Setup-2026.1104.31.exe',
    yuilivekit: 'YuiLiveKit-Setup-2026.1107.1.exe',
  };
  const catalog = realCatalog;
  // The UI test drives this from the main process to show each update state.
  const updater = new (require('node:events').EventEmitter)();
  updater.checks = 0;
  updater.installs = 0;
  updater.checkForUpdates = async () => {
    updater.checks += 1;
    updater.emit('checking-for-update');
    updater.emit('update-not-available', {});
  };
  updater.quitAndInstall = () => { updater.installs += 1; };
  global.fixtureUpdater = updater;
  return {
    catalog,
    updater,
    system: {
      readInstalled: async app => (world[app.id].version
        ? {
          version: world[app.id].version,
          exePath: `C:\\Fake\\${app.exeName}`,
          uninstaller: { file: 'C:\\Fake\\unins.exe', args: [app.id] },
        }
        : null),
      runUninstaller: async ({ args }) => {
        await sleep(200);
        world[args[0]].version = null;
      },
      isRunning: async app => world[app.id].running,
      closeApp: async app => {
        await sleep(100);
        world[app.id].running = false;
        return true;
      },
      launch: async exePath => {
        const app = catalog.find(item => exePath.endsWith(item.exeName));
        world[app.id].running = true;
      },
      runInstaller: async (file, _args) => {
        await sleep(200);
        const app = catalog.find(item => item.installerPattern.test(require('node:path').basename(file)));
        world[app.id].version = world[app.id].latest;
      },
    },
    releases: {
      latest: async app => ({
        version: world[app.id].latest,
        pageUrl: `https://github.com/${app.repo}/releases/tag/v${world[app.id].latest}`,
        installer: {
          name: installerNames[app.id],
          size: 100,
          url: 'https://example.invalid/installer',
        },
        checksums: { name: 'SHA256SUMS.txt', url: 'https://example.invalid/sums' },
      }),
      fetchText: async () => Object.values(installerNames).map(name => `${HASH}  ${name}`).join('\n'),
    },
    downloader: {
      download: async ({ dest, onProgress }) => {
        for (const received of [10, 42, 70, 100]) {
          onProgress(received, 100);
          await sleep(350);
        }
        fs.writeFileSync(dest, 'fake');
        return { sha256: HASH };
      },
    },
  };
}

module.exports = { create };
