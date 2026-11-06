'use strict';

const test = require('node:test');
const assert = require('node:assert/strict');
const { compareVersions, isVersion } = require('../../electron/version.cjs');

test('compares each part as a number, not as text', () => {
  assert.equal(compareVersions('2026.1021.3', '2026.103.1'), 1);
  assert.equal(compareVersions('2026.1107.10', '2026.1107.9'), 1);
  assert.equal(compareVersions('2026.1107.1', '2026.1107.1'), 0);
  assert.equal(compareVersions('2026.1106.5', '2026.1107.1'), -1);
});

test('treats missing parts as zero', () => {
  assert.equal(compareVersions('1.2', '1.2.0'), 0);
  assert.equal(compareVersions('1.2.1', '1.2'), 1);
});

test('rejects tags that are not plain versions', () => {
  assert.equal(isVersion('2026.1103.34-mac.1'), false);
  assert.equal(isVersion('0.3.0-beta.7'), false);
  assert.equal(isVersion(null), false);
  assert.throws(() => compareVersions('1.0-beta', '1.0'));
});
