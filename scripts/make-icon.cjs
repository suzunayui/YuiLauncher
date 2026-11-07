'use strict';

// Writes assets/icon.ico and assets/icon.png from assets/icon-source.webp.
// Pass a folder to also save each size as a PNG for checking.
const fs = require('node:fs');
const path = require('node:path');
const { _electron: electron } = require('@playwright/test');

const SIZES = [256, 128, 64, 48, 32, 24, 16];

// Halving in steps keeps fine lines cleaner than one big downscale.
const RENDER = `async (sizes) => {
  const image = document.getElementById('source');
  await image.decode();
  const scaled = target => {
    let canvas = document.createElement('canvas');
    canvas.width = image.naturalWidth;
    canvas.height = image.naturalHeight;
    canvas.getContext('2d').drawImage(image, 0, 0);
    while (canvas.width > target) {
      const next = document.createElement('canvas');
      next.width = Math.max(target, Math.ceil(canvas.width / 2));
      next.height = next.width;
      const context = next.getContext('2d');
      context.imageSmoothingQuality = 'high';
      context.drawImage(canvas, 0, 0, next.width, next.height);
      canvas = next;
    }
    return canvas;
  };
  const full = document.createElement('canvas');
  full.width = image.naturalWidth;
  full.height = image.naturalHeight;
  const context = full.getContext('2d');
  context.drawImage(image, 0, 0);
  return {
    width: image.naturalWidth,
    height: image.naturalHeight,
    cornerAlpha: context.getImageData(0, 0, 1, 1).data[3],
    pngs: sizes.map(size => scaled(size).toDataURL('image/png')),
  };
}`;

async function main() {
  const root = path.join(__dirname, '..');
  const source = fs.readFileSync(path.join(root, 'assets', 'icon-source.webp')).toString('base64');
  const previewDir = process.argv[2];
  const env = { ...process.env, YUI_LAUNCHER_FIXTURE: path.join(root, 'tests', 'fixture.cjs') };
  delete env.ELECTRON_RUN_AS_NODE;
  const app = await electron.launch({ args: [root], env });
  let result;
  try {
    await app.firstWindow();
    result = await app.evaluate(async ({ BrowserWindow }, { source: webp, sizes, render }) => {
      const window = new BrowserWindow({ show: false, webPreferences: { offscreen: true } });
      const html = `<!doctype html><img id="source" src="data:image/webp;base64,${webp}">`;
      await window.loadURL(`data:text/html;base64,${Buffer.from(html).toString('base64')}`);
      const output = await window.webContents.executeJavaScript(`(${render})(${JSON.stringify(sizes)})`);
      window.destroy();
      return output;
    }, { source, sizes: SIZES, render: RENDER });
  } finally {
    await app.close();
  }

  console.log(`Source ${result.width}x${result.height}, corner alpha ${result.cornerAlpha}`);
  if (result.width !== result.height) throw new Error('The source image must be square');
  const pngs = result.pngs.map(url => Buffer.from(url.split(',')[1], 'base64'));
  const { default: pngToIco } = await import('png-to-ico');
  fs.writeFileSync(path.join(root, 'assets', 'icon.ico'), await pngToIco(pngs));
  fs.writeFileSync(path.join(root, 'assets', 'icon.png'), pngs[0]);
  if (previewDir) {
    SIZES.forEach((size, index) => fs.writeFileSync(path.join(previewDir, `icon-${size}.png`), pngs[index]));
  }
  console.log('Wrote assets/icon.ico');
}

main().catch(error => {
  console.error(error);
  process.exitCode = 1;
});
