'use strict';

const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const catalog = require('../../electron/catalog.cjs');
const { pickLatestRelease } = require('../../electron/releases.cjs');

const liveKit = catalog.find(app => app.id === 'yuilivekit');
const release = (tag, names) => ({
  tag_name: tag,
  html_url: `https://github.com/suzunayui/YuiLiveKit/releases/tag/${tag}`,
  assets: names.map(name => ({
    name,
    size: 1,
    browser_download_url: `https://github.com/suzunayui/YuiLiveKit/releases/download/${tag}/${name}`,
  })),
});

test('every catalog icon exists', () => {
  for (const app of catalog) {
    assert.ok(fs.existsSync(path.join(__dirname, '../../electron', app.icon)), app.icon);
  }
});

test('catalog ids and installed executables are unique', () => {
  assert.equal(new Set(catalog.map(app => app.id)).size, catalog.length);
  assert.equal(new Set(catalog.map(app => app.exeName)).size, catalog.length);
});

test('every app defines how to uninstall silently and what stays', () => {
  for (const app of catalog) {
    assert.ok(app.uninstallArgs.length > 0, app.id);
    assert.ok(app.uninstallNote, app.id);
  }
});

test('YuiLiveKit uses its release and skips those from its former name', () => {
  const picked = pickLatestRelease([
    release('YouTubeOverlay-2026.10.10-18', ['YouTubeOverlay-2026.10.10-18.exe']),
    release('v2026.1107.1', ['YuiLiveKit-Setup-2026.1107.1.exe', 'SHA256SUMS.txt']),
  ], liveKit);
  assert.equal(picked.version, '2026.1107.1');
  assert.equal(picked.installer.name, 'YuiLiveKit-Setup-2026.1107.1.exe');
});
