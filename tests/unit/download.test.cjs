'use strict';

const test = require('node:test');
const assert = require('node:assert/strict');
const crypto = require('node:crypto');
const fs = require('node:fs');
const http = require('node:http');
const os = require('node:os');
const path = require('node:path');
const { download } = require('../../electron/download.cjs');

const BODY = crypto.randomBytes(256 * 1024);

async function withServer(run) {
  const server = http.createServer((_req, res) => res.end(BODY));
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'yl-download-'));
  try {
    await run(`http://127.0.0.1:${server.address().port}/`, path.join(dir, 'file.bin'));
  } finally {
    server.close();
    fs.rmSync(dir, { recursive: true, force: true });
  }
}

test('writes the file, reports progress and returns its hash', () => withServer(async (url, dest) => {
  const seen = [];
  const { sha256 } = await download({ url, dest, expectedSize: BODY.length, onProgress: received => seen.push(received) });
  assert.equal(sha256, crypto.createHash('sha256').update(BODY).digest('hex'));
  assert.deepEqual(fs.readFileSync(dest), BODY);
  assert.equal(seen.at(-1), BODY.length);
}));

test('fails when the size differs from the release', () => withServer(async (url, dest) => {
  await assert.rejects(download({ url, dest, expectedSize: BODY.length + 1 }), /サイズ/);
}));

test('stops when aborted', () => withServer(async (url, dest) => {
  const controller = new AbortController();
  controller.abort();
  await assert.rejects(download({ url, dest, signal: controller.signal }), { name: 'AbortError' });
}));
