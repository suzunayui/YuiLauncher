'use strict';

const test = require('node:test');
const assert = require('node:assert/strict');
const { EventEmitter } = require('node:events');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { createFileLogger, createSelfUpdater } = require('../../electron/self-update.cjs');

function setup({ check = async updater => updater.emit('checking-for-update'), retryAfterMs = 60000 } = {}) {
  const updater = new EventEmitter();
  updater.checks = 0;
  updater.installs = 0;
  updater.checkForUpdates = async () => {
    updater.checks += 1;
    await check(updater);
  };
  updater.quitAndInstall = () => { updater.installs += 1; };
  const logged = [];
  const logger = { info() {}, warn() {}, debug() {}, error: error => logged.push(error) };
  const changes = [];
  const self = createSelfUpdater({
    updater,
    version: '1.0.0',
    logger,
    retryAfterMs,
    onChange: state => changes.push(state),
  });
  return { updater, self, logged, changes };
}

test('configures the updater to download and to install on quit', () => {
  const { updater } = setup();
  assert.equal(updater.autoDownload, true);
  assert.equal(updater.autoInstallOnAppQuit, true);
  assert.ok(updater.logger);
});

test('walks through checking, downloading and ready', async () => {
  const { updater, self } = setup();
  assert.equal(self.getState().phase, 'idle');
  assert.equal(self.getState().currentVersion, '1.0.0');
  await self.check();
  assert.equal(self.getState().phase, 'checking');
  updater.emit('update-available', { version: '1.1.0' });
  assert.deepEqual(
    [self.getState().phase, self.getState().latestVersion, self.getState().percent],
    ['downloading', '1.1.0', 0],
  );
  updater.emit('download-progress', { percent: 41.7 });
  assert.equal(self.getState().percent, 41);
  updater.emit('update-downloaded', { version: '1.1.0' });
  assert.equal(self.getState().phase, 'ready');
  assert.equal(self.getState().percent, null);
});

test('reports being up to date', async () => {
  const { updater, self } = setup();
  await self.check();
  updater.emit('update-not-available', { version: '1.0.0' });
  assert.equal(self.getState().phase, 'latest');
});

test('shows a failed check once and logs it once, event and rejection together', async () => {
  const failure = new Error('network down');
  const { self, logged, changes } = setup({
    check: async updater => {
      updater.emit('checking-for-update');
      updater.emit('error', failure);
      throw failure;
    },
  });
  await self.check();
  assert.equal(self.getState().phase, 'error');
  assert.match(self.getState().error, /確認できませんでした/);
  assert.deepEqual(logged, [failure]);
  assert.equal(changes.filter(state => state.phase === 'error').length, 1);
});

test('shows a download failure that only arrives as an event', async () => {
  const { updater, self, logged } = setup();
  await self.check();
  updater.emit('update-available', { version: '1.1.0' });
  updater.emit('error', new Error('connection reset'));
  assert.equal(self.getState().phase, 'error');
  assert.match(self.getState().error, /ダウンロードできませんでした/);
  assert.equal(logged.length, 1);
});

test('a failed error message hides technical details', async () => {
  const { self } = setup({ check: async () => { throw new Error('HttpError: 404 https://secret.example/'); } });
  await self.check();
  assert.equal(self.getState().error.includes('secret.example'), false);
});

test('retries soon after a failure, and the retry clears the error', async () => {
  let fail = true;
  const { self, updater } = setup({
    retryAfterMs: 5,
    check: async target => {
      target.emit('checking-for-update');
      if (fail) throw new Error('offline');
      target.emit('update-not-available', {});
    },
  });
  await self.check();
  assert.equal(self.getState().phase, 'error');
  fail = false;
  await new Promise(resolve => setTimeout(resolve, 40));
  assert.equal(updater.checks, 2);
  assert.equal(self.getState().phase, 'latest');
  assert.equal(self.getState().error, null);
});

test('checking again recovers from an error right away', async () => {
  let fail = true;
  const { self } = setup({
    check: async target => {
      target.emit('checking-for-update');
      if (fail) throw new Error('offline');
      target.emit('update-not-available', {});
    },
  });
  await self.check();
  fail = false;
  await self.check();
  assert.equal(self.getState().phase, 'latest');
});

test('does not start a second check while one runs or a download is under way', async () => {
  let finish;
  const { updater, self } = setup({ check: () => new Promise(resolve => { finish = resolve; }) });
  const first = self.check();
  self.check();
  assert.equal(updater.checks, 1);
  finish();
  await first;
  updater.emit('update-available', { version: '1.1.0' });
  await self.check();
  assert.equal(updater.checks, 1);
});

test('keeps a ready update ready when checked again', async () => {
  const { updater, self } = setup();
  await self.check();
  updater.emit('update-downloaded', { version: '1.1.0' });
  await self.check();
  assert.equal(updater.checks, 1);
  assert.equal(self.getState().phase, 'ready');
});

test('installs only when an update is ready', async () => {
  const { updater, self } = setup();
  assert.equal(self.install(), false);
  assert.equal(updater.installs, 0);
  updater.emit('update-downloaded', { version: '1.1.0' });
  assert.equal(self.install(), true);
  assert.equal(updater.installs, 1);
});

test('does nothing when running from source', async () => {
  const self = createSelfUpdater({ updater: null, version: '1.0.0' });
  assert.equal(self.getState().phase, 'unavailable');
  await self.check();
  assert.equal(self.install(), false);
});

test('the log records messages and errors, skips range noise and rotates', () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'yl-log-'));
  try {
    const file = path.join(dir, 'logs', 'self-update.log');
    const logger = createFileLogger(file, { maxBytes: 200 });
    logger.info('Found version 1.1.0');
    logger.info('download range: bytes=1-2');
    logger.debug('hidden');
    logger.error(new Error('boom'));
    const first = fs.readFileSync(file, 'utf8');
    assert.match(first, /INFO Found version 1\.1\.0/);
    assert.match(first, /ERROR Error: boom/);
    assert.equal(first.includes('download range'), false);
    assert.equal(first.includes('hidden'), false);

    logger.info('x'.repeat(300));
    logger.info('after rotation');
    assert.ok(fs.existsSync(`${file}.old`));
    assert.match(fs.readFileSync(file, 'utf8'), /after rotation/);
    assert.equal(fs.readFileSync(file, 'utf8').includes('boom'), false);
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});
