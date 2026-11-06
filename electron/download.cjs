'use strict';

const crypto = require('node:crypto');
const fs = require('node:fs');
const { once } = require('node:events');

// Streams url to dest and returns the SHA-256 of what was written.
async function download({ url, dest, expectedSize, signal, onProgress = () => {}, fetchImpl = fetch }) {
  const response = await fetchImpl(url, { headers: { 'User-Agent': 'YuiLauncher' }, signal });
  if (!response.ok || !response.body) throw new Error(`ダウンロードに失敗しました（${response.status}）`);

  const hash = crypto.createHash('sha256');
  const file = fs.createWriteStream(dest);
  let received = 0;
  try {
    const reader = response.body.getReader();
    for (;;) {
      const { done, value } = await reader.read();
      if (done) break;
      hash.update(value);
      received += value.length;
      if (!file.write(value)) await once(file, 'drain');
      onProgress(received, expectedSize);
    }
    file.end();
    await once(file, 'finish');
  } catch (error) {
    file.destroy();
    throw error;
  }
  if (expectedSize && received !== expectedSize) {
    throw new Error('ダウンロードしたファイルのサイズが一致しません');
  }
  return { sha256: hash.digest('hex') };
}

module.exports = { download };
