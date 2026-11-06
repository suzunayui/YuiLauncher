'use strict';

const test = require('node:test');
const assert = require('node:assert/strict');
const catalog = require('../../electron/catalog.cjs');
const { pickLatestRelease, parseChecksums, createReleaseClient } = require('../../electron/releases.cjs');

const [tracking, voice] = catalog;

function release(tag, names, extra = {}, repo = voice.repo) {
  return {
    tag_name: tag,
    html_url: `https://github.com/${repo}/releases/tag/${tag}`,
    draft: false,
    prerelease: false,
    assets: names.map(name => ({
      name,
      size: 10,
      browser_download_url: `https://github.com/${repo}/releases/download/${tag}/${name}`,
    })),
    ...extra,
  };
}

const voiceAssets = version => [`YuiVoiceChanger-Setup-${version}.exe`, 'SHA256SUMS.txt'];

test('skips a newer Mac-only release', () => {
  const picked = pickLatestRelease([
    release('v2026.1110.1', ['YuiVoiceChanger-2026.1110.1-mac-arm64.dmg', 'SHA256SUMS.txt']),
    release('v2026.1104.31', voiceAssets('2026.1104.31')),
  ], voice);
  assert.equal(picked.version, '2026.1104.31');
});

test('skips Mac tags, betas, drafts and prereleases', () => {
  const picked = pickLatestRelease([
    release('v2026.1103.34-mac.1', voiceAssets('2026.1103.34')),
    release('v0.3.0-beta.7', voiceAssets('0.3.0')),
    release('v2026.1200.1', voiceAssets('2026.1200.1'), { draft: true }),
    release('v2026.1201.1', voiceAssets('2026.1201.1'), { prerelease: true }),
    release('v2026.1103.34', voiceAssets('2026.1103.34')),
  ], voice);
  assert.equal(picked.version, '2026.1103.34');
});

test('picks the highest version regardless of order', () => {
  const picked = pickLatestRelease([
    release('v2026.103.1', voiceAssets('2026.103.1')),
    release('v2026.1021.3', voiceAssets('2026.1021.3')),
  ], voice);
  assert.equal(picked.version, '2026.1021.3');
});

test('finds the YuiTracking installer among Mac assets and versioned checksums', () => {
  const picked = pickLatestRelease([release('v2026.1107.1', [
    'SHA256SUMS-2026.1107.1.txt',
    'YuiTracking-2026.1107.1-arm64-mac.zip',
    'YuiTracking-2026.1107.1-arm64.dmg',
    'YuiTracking-Setup-2026.1107.1-x64.exe',
  ], {}, tracking.repo)], tracking);
  assert.equal(picked.installer.name, 'YuiTracking-Setup-2026.1107.1-x64.exe');
  assert.equal(picked.checksums.name, 'SHA256SUMS-2026.1107.1.txt');
});

test('requires checksums and rejects assets hosted elsewhere', () => {
  assert.equal(pickLatestRelease([release('v1.0.0', ['YuiVoiceChanger-Setup-1.0.0.exe'])], voice), null);
  const foreign = release('v1.0.0', voiceAssets('1.0.0'));
  foreign.assets[0].browser_download_url = 'https://example.com/YuiVoiceChanger-Setup-1.0.0.exe';
  assert.equal(pickLatestRelease([foreign], voice), null);
});

test('parses checksum files in both common formats', () => {
  const a = 'A'.repeat(64);
  const b = 'b'.repeat(64);
  const parsed = parseChecksums(`﻿${a}  one.exe\r\n${b} *two three.exe\nnot a checksum\n`);
  assert.equal(parsed.get('one.exe'), 'a'.repeat(64));
  assert.equal(parsed.get('two three.exe'), b);
  assert.equal(parsed.size, 2);
});

test('reuses the cached list on 304 and sends no personal data', async () => {
  const calls = [];
  const fetchImpl = async (_url, options) => {
    calls.push(options.headers);
    if (calls.length === 1) {
      return new Response(JSON.stringify([release('v1.0.0', voiceAssets('1.0.0'))]), { headers: { etag: '"abc"' } });
    }
    return new Response(null, { status: 304 });
  };
  const client = createReleaseClient({ fetchImpl });
  assert.equal((await client.latest(voice)).version, '1.0.0');
  assert.equal((await client.latest(voice)).version, '1.0.0');
  assert.equal(calls[1]['If-None-Match'], '"abc"');
  assert.equal(calls[0]['User-Agent'], 'YuiLauncher');
});

test('explains rate limits and network failures', async () => {
  const limited = createReleaseClient({ fetchImpl: async () => new Response('', { status: 403 }) });
  await assert.rejects(limited.latest(voice), /上限/);
  const offline = createReleaseClient({ fetchImpl: async () => { throw new TypeError('fetch failed'); } });
  await assert.rejects(offline.latest(voice), /インターネット/);
});
