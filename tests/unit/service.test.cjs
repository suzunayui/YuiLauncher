'use strict';

const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { createService } = require('../../electron/service.cjs');

const HASH = 'a'.repeat(64);
const APP = {
  id: 'demo',
  name: 'Demo',
  description: '',
  icon: '',
  repo: 'owner/Demo',
  exeName: 'Demo.exe',
  silentArgs: ['/S'],
};

function setup({ version = '1.0.0', running = false, latest = '1.1.0', overrides = {} } = {}) {
  const log = [];
  const world = { version, running };
  const tempRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'yl-service-'));
  const system = {
    readInstalled: async () => (world.version
      ? { version: world.version, exePath: 'C:\\Demo\\Demo.exe', uninstaller: { file: 'C:\\Demo\\unins.exe', args: ['/S'] } }
      : null),
    runUninstaller: async ({ file, args }) => {
      log.push(`uninstall:${path.win32.basename(file)}:${args.join(' ')}`);
      world.version = null;
    },
    isRunning: async () => world.running,
    closeApp: async () => {
      log.push('close');
      world.running = false;
      return true;
    },
    launch: async () => {
      log.push('launch');
      world.running = true;
    },
    runInstaller: async (file, args) => {
      log.push(`install:${path.basename(file)}:${args.join(' ')}`);
      assert.ok(fs.existsSync(file), 'installer exists when it runs');
      world.installerPath = file;
      world.version = latest;
    },
    ...overrides.system,
  };
  const releases = {
    latest: async () => (latest ? {
      version: latest,
      pageUrl: 'https://github.com/owner/Demo/releases/tag/x',
      installer: { name: 'Demo-Setup.exe', size: 1, url: 'https://github.com/owner/Demo/installer' },
      checksums: { name: 'SHA256SUMS.txt', url: 'https://github.com/owner/Demo/sums' },
    } : null),
    fetchText: async () => `${HASH}  Demo-Setup.exe\n`,
    ...overrides.releases,
  };
  const downloader = {
    download: async ({ dest, onProgress }) => {
      log.push('download');
      fs.writeFileSync(dest, 'x');
      onProgress(1, 1);
      return { sha256: HASH };
    },
    ...overrides.downloader,
  };
  const service = createService({
    catalog: [APP],
    system,
    releases,
    downloader,
    tempRoot,
    confirmTimeoutMs: 20,
    confirmIntervalMs: 1,
  });
  const appState = () => service.snapshot().apps[0];
  const leftovers = () => fs.readdirSync(tempRoot);
  return { service, log, world, appState, leftovers };
}

test('shows an update only when the release is newer', async () => {
  const newer = setup();
  await newer.service.check();
  assert.equal(newer.appState().updateAvailable, true);

  const same = setup({ latest: '1.0.0' });
  await same.service.check();
  assert.equal(same.appState().updateAvailable, false);

  const ahead = setup({ version: '2.0.0' });
  await ahead.service.check();
  assert.equal(ahead.appState().updateAvailable, false);
});

test('offers install when the app is missing', async () => {
  const { service, appState } = setup({ version: null });
  await service.check();
  assert.equal(appState().installed, false);
  assert.equal(appState().updateAvailable, true);
  assert.equal(appState().canOpen, false);
});

test('updates a closed app without launching it, then removes the installer', async () => {
  const { service, log, appState, leftovers } = setup();
  await service.check();
  assert.equal((await service.install('demo')).status, 'done');
  assert.deepEqual(log, ['download', 'install:Demo-Setup.exe:/S']);
  assert.equal(appState().version, '1.1.0');
  assert.equal(appState().updateAvailable, false);
  assert.equal(appState().task, null);
  assert.deepEqual(leftovers(), []);
});

test('asks before closing a running app and does nothing until confirmed', async () => {
  const { service, log } = setup({ running: true });
  await service.check();
  assert.equal((await service.install('demo')).status, 'needs-close');
  assert.deepEqual(log, []);
});

test('closes only after verification, then starts the app again', async () => {
  const { service, log } = setup({ running: true });
  await service.check();
  assert.equal((await service.install('demo', { closeRunning: true })).status, 'done');
  assert.deepEqual(log, ['download', 'close', 'install:Demo-Setup.exe:/S', 'launch']);
});

test('a bad checksum never closes the app or runs the installer', async () => {
  const { service, log, appState, leftovers } = setup({
    running: true,
    overrides: { releases: { fetchText: async () => `${'b'.repeat(64)}  Demo-Setup.exe\n` } },
  });
  await service.check();
  assert.equal((await service.install('demo', { closeRunning: true })).status, 'failed');
  assert.deepEqual(log, ['download']);
  assert.match(appState().error, /検証に失敗/);
  assert.deepEqual(leftovers(), []);
});

test('a checksum file without this installer is a failure', async () => {
  const { service, log, appState } = setup({
    overrides: { releases: { fetchText: async () => `${HASH}  Other.exe\n` } },
  });
  await service.check();
  assert.equal((await service.install('demo')).status, 'failed');
  assert.deepEqual(log, ['download']);
  assert.match(appState().error, /記載がありません/);
});

test('stops without force-closing when the app will not close', async () => {
  const { service, log, appState } = setup({
    running: true,
    overrides: { system: { closeApp: async () => false } },
  });
  await service.check();
  assert.equal((await service.install('demo', { closeRunning: true })).status, 'failed');
  assert.deepEqual(log, ['download']);
  assert.match(appState().error, /閉じられませんでした/);
});

test('stops if the app was opened during an unconfirmed download', async () => {
  const context = setup({
    overrides: {
      downloader: {
        download: async ({ dest }) => {
          fs.writeFileSync(dest, 'x');
          context.world.running = true;
          return { sha256: HASH };
        },
      },
    },
  });
  await context.service.check();
  assert.equal((await context.service.install('demo')).status, 'failed');
  assert.match(context.appState().error, /中止/);
  assert.deepEqual(context.log, []);
});

test('reopens the old version when the installer fails', async () => {
  const { service, log, appState } = setup({
    running: true,
    overrides: { system: { runInstaller: async () => { throw new Error('インストーラーが終了コード2で終了しました'); } } },
  });
  await service.check();
  assert.equal((await service.install('demo', { closeRunning: true })).status, 'failed');
  assert.deepEqual(log, ['download', 'close', 'launch']);
  assert.match(appState().error, /終了コード2/);
  assert.equal(appState().version, '1.0.0');
});

test('reports failure when the version does not change after installing', async () => {
  const { service, appState } = setup({
    overrides: { system: { runInstaller: async () => {} } },
  });
  await service.check();
  assert.equal((await service.install('demo')).status, 'failed');
  assert.match(appState().error, /確認できませんでした/);
});

test('cancelling a download is not an error', async () => {
  const context = setup({
    overrides: {
      downloader: {
        download: ({ signal }) => new Promise((_resolve, reject) => {
          signal.addEventListener('abort', () => reject(Object.assign(new Error('aborted'), { name: 'AbortError' })));
          setImmediate(() => context.service.cancel('demo'));
        }),
      },
    },
  });
  await context.service.check();
  assert.equal((await context.service.install('demo')).status, 'cancelled');
  assert.equal(context.appState().error, null);
  assert.equal(context.appState().task, null);
  assert.deepEqual(context.log, []);
});

test('refuses a second install while one is running', async () => {
  let release;
  const { service } = setup({
    overrides: {
      downloader: {
        download: async ({ dest }) => {
          fs.writeFileSync(dest, 'x');
          await new Promise(resolve => { release = resolve; });
          return { sha256: HASH };
        },
      },
    },
  });
  await service.check();
  const first = service.install('demo');
  while (!release) await new Promise(resolve => setImmediate(resolve));
  assert.equal(service.isBusy(), true);
  assert.equal((await service.install('demo')).status, 'busy');
  release();
  assert.equal((await first).status, 'done');
  assert.equal(service.isBusy(), false);
});

test('keeps the last known release when a later check fails', async () => {
  let online = true;
  const { service, appState } = setup({
    overrides: {
      releases: {
        latest: async () => {
          if (!online) throw new Error('オフライン');
          return { version: '1.1.0', installer: {}, checksums: {} };
        },
      },
    },
  });
  await service.check();
  online = false;
  await service.check();
  assert.equal(appState().updateAvailable, true);
  assert.equal(appState().checkError, 'オフライン');
});

test('never exposes download URLs or local paths to the page', async () => {
  const { service } = setup();
  await service.check();
  const serialized = JSON.stringify(service.snapshot());
  assert.equal(serialized.includes('https://github.com/owner/Demo/installer'), false);
  assert.equal(serialized.includes('Demo.exe'), false);
  assert.equal(serialized.includes('unins.exe'), false);
});

test('checks and closes the app by its installed path', async () => {
  const paths = [];
  const context = setup({
    running: true,
    overrides: {
      system: {
        isRunning: async (_app, exePath) => {
          paths.push(exePath);
          return context.world.running;
        },
        closeApp: async (_app, exePath) => {
          paths.push(exePath);
          context.world.running = false;
          return true;
        },
      },
    },
  });
  await context.service.check();
  await context.service.uninstall('demo', { closeRunning: true });
  assert.ok(paths.includes('C:\\Demo\\Demo.exe'));
  assert.equal(context.appState().running, false, 'nothing is running once the app is gone');
});

test('uninstalls a closed app without starting anything', async () => {
  const { service, log, appState } = setup({ latest: '1.0.0' });
  await service.check();
  assert.equal(appState().canUninstall, true);
  assert.equal((await service.uninstall('demo')).status, 'done');
  assert.deepEqual(log, ['uninstall:unins.exe:/S']);
  assert.equal(appState().installed, false);
  assert.equal(appState().canUninstall, false);
  assert.equal(appState().task, null);
  assert.equal(appState().updateAvailable, true, 'it can be installed again');
  assert.match(appState().notice, /アンインストールしました/);
});

test('does not touch a running app without confirmation', async () => {
  const { service, log, appState } = setup({ running: true });
  await service.check();
  assert.equal((await service.uninstall('demo')).status, 'needs-close');
  assert.deepEqual(log, []);
  assert.equal(appState().installed, true);
});

test('closes a running app first and does not start it again', async () => {
  const { service, log, appState } = setup({ running: true });
  await service.check();
  assert.equal((await service.uninstall('demo', { closeRunning: true })).status, 'done');
  assert.deepEqual(log, ['close', 'uninstall:unins.exe:/S']);
  assert.equal(appState().running, false);
});

test('stops the uninstall when the app will not close', async () => {
  const { service, log, appState } = setup({
    running: true,
    overrides: { system: { closeApp: async () => false } },
  });
  await service.check();
  assert.equal((await service.uninstall('demo', { closeRunning: true })).status, 'failed');
  assert.deepEqual(log, []);
  assert.match(appState().error, /閉じられませんでした/);
  assert.equal(appState().installed, true);
});

test('starts the app again when the uninstaller fails', async () => {
  const { service, log, appState } = setup({
    running: true,
    overrides: { system: { runUninstaller: async () => { throw new Error('アンインストーラーが終了コード2で終了しました'); } } },
  });
  await service.check();
  assert.equal((await service.uninstall('demo', { closeRunning: true })).status, 'failed');
  assert.deepEqual(log, ['close', 'launch']);
  assert.match(appState().error, /終了コード2/);
  assert.equal(appState().installed, true);
});

test('reports failure when the app is still registered afterwards', async () => {
  const { service, appState } = setup({
    overrides: { system: { runUninstaller: async () => {} } },
  });
  await service.check();
  assert.equal((await service.uninstall('demo')).status, 'failed');
  assert.match(appState().error, /確認できませんでした/);
  assert.equal(appState().installed, true);
});

test('fails clearly when no uninstaller is available', async () => {
  const { service, log, appState } = setup({
    overrides: { system: { readInstalled: async () => ({ version: '1.0.0', exePath: null, uninstaller: null }) } },
  });
  await service.check();
  assert.equal(appState().canUninstall, false);
  assert.equal((await service.uninstall('demo')).status, 'failed');
  assert.deepEqual(log, []);
  assert.match(appState().error, /見つかりません/);
});

test('refuses to uninstall while an update is running', async () => {
  let release;
  const { service } = setup({
    overrides: {
      downloader: {
        download: async ({ dest }) => {
          fs.writeFileSync(dest, 'x');
          await new Promise(resolve => { release = resolve; });
          return { sha256: HASH };
        },
      },
    },
  });
  await service.check();
  const update = service.install('demo');
  while (!release) await new Promise(resolve => setImmediate(resolve));
  assert.equal((await service.uninstall('demo')).status, 'busy');
  release();
  assert.equal((await update).status, 'done');
});

test('only returns release pages from the app repository', async () => {
  const { service } = setup({
    overrides: {
      releases: {
        latest: async () => ({ version: '1.1.0', pageUrl: 'https://evil.example/', installer: {}, checksums: {} }),
      },
    },
  });
  await service.check();
  assert.equal(service.releasePageUrl('demo'), null);
});
