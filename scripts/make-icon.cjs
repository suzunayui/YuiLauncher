'use strict';

// Renders the launcher icon with Electron and writes assets/icon.ico.
const fs = require('node:fs');
const path = require('node:path');
const { _electron: electron } = require('@playwright/test');

const SVG = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 256 256" width="256" height="256">
  <defs><linearGradient id="g" x1="0" y1="0" x2="1" y2="1">
    <stop offset="0" stop-color="#14918a"/><stop offset="1" stop-color="#0a5a56"/>
  </linearGradient></defs>
  <rect x="8" y="8" width="240" height="240" rx="56" fill="url(#g)"/>
  <rect x="52" y="52" width="66" height="66" rx="16" fill="#fff"/>
  <rect x="138" y="52" width="66" height="66" rx="16" fill="#fff" opacity=".6"/>
  <rect x="52" y="138" width="66" height="66" rx="16" fill="#fff" opacity=".6"/>
  <path d="M171 204v-52m-23 23 23-23 23 23" fill="none" stroke="#fff" stroke-width="15" stroke-linecap="round" stroke-linejoin="round"/>
</svg>`;

async function main() {
  const root = path.join(__dirname, '..');
  const app = await electron.launch({ args: [root], env: { ...process.env, YUI_LAUNCHER_FIXTURE: path.join(root, 'tests', 'fixture.cjs') } });
  const pngs = [];
  try {
    const page = await app.firstWindow();
    await page.waitForLoadState();
    for (const size of [256, 64, 48, 32, 16]) {
      const png = await app.evaluate(async ({ BrowserWindow, nativeImage }, { svg, size: target }) => {
        const window = new BrowserWindow({ show: false, width: 300, height: 300, webPreferences: { offscreen: true } });
        const html = `<!doctype html><body style="margin:0;background:transparent"><img id="i" src="data:image/svg+xml;base64,${Buffer.from(svg).toString('base64')}" width="${target}" height="${target}" style="display:block"></body>`;
        await window.loadURL(`data:text/html;base64,${Buffer.from(html).toString('base64')}`);
        const dataUrl = await window.webContents.executeJavaScript(`(async()=>{const i=document.getElementById('i');await i.decode();const c=document.createElement('canvas');c.width=${target};c.height=${target};c.getContext('2d').drawImage(i,0,0,${target},${target});return c.toDataURL('image/png');})()`);
        window.destroy();
        return nativeImage.createFromDataURL(dataUrl).toPNG().toString('base64');
      }, { svg: SVG, size });
      pngs.push(Buffer.from(png, 'base64'));
    }
  } finally {
    await app.close();
  }
  const { default: pngToIco } = await import('png-to-ico');
  fs.writeFileSync(path.join(root, 'assets', 'icon.ico'), await pngToIco(pngs));
  fs.writeFileSync(path.join(root, 'assets', 'icon.png'), pngs[0]);
  console.log('Wrote assets/icon.ico');
}

main().catch(error => {
  console.error(error);
  process.exitCode = 1;
});
