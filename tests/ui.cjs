'use strict';

// Drives the real Electron window against tests/fixture.cjs and saves
// screenshots to the folder given in YUI_LAUNCHER_SHOTS, if any.
const assert = require('node:assert/strict');
const path = require('node:path');
const { _electron: electron } = require('@playwright/test');

const shots = process.env.YUI_LAUNCHER_SHOTS;

async function main() {
  const env = { ...process.env, YUI_LAUNCHER_FIXTURE: path.join(__dirname, 'fixture.cjs') };
  delete env.ELECTRON_RUN_AS_NODE;
  const app = await electron.launch({ args: [path.join(__dirname, '..')], env });
  const shot = async (page, name) => {
    if (shots) await page.screenshot({ path: path.join(shots, `${name}.png`) });
  };
  try {
    const page = await app.firstWindow();
    const errors = [];
    page.on('pageerror', error => errors.push(error.message));
    page.on('console', message => {
      if (message.type() === 'error') errors.push(message.text());
    });

    const tracking = page.locator('[data-app="yuitracking"]');
    const voice = page.locator('[data-app="yuivoicechanger"]');

    await tracking.getByRole('button', { name: 'アップデート' }).waitFor();
    await page.getByText('1件のアップデートがあります').waitFor();
    assert.ok(await tracking.getByText('起動中').isVisible());
    assert.ok(await voice.getByText('最新です').isVisible());
    assert.equal(await voice.getByRole('button', { name: 'アップデート' }).count(), 0);
    const iconsLoaded = await page.locator('.icon').evaluateAll(icons => icons.every(icon => icon.naturalWidth > 0));
    assert.ok(iconsLoaded, 'app icons load');
    const overflow = await page.evaluate(() => document.documentElement.scrollWidth > window.innerWidth);
    assert.equal(overflow, false, 'no horizontal scroll');
    await shot(page, '1-update-available');

    await tracking.getByRole('button', { name: 'アップデート' }).click();
    const dialog = page.locator('#close-dialog');
    await dialog.waitFor();
    await shot(page, '2-confirm-close');
    await dialog.getByRole('button', { name: 'やめる' }).click();
    assert.equal(await tracking.locator('.progress').count(), 0, 'cancel leaves the app alone');

    await tracking.getByRole('button', { name: 'アップデート' }).click();
    await dialog.getByRole('button', { name: '閉じて更新' }).click();
    await tracking.getByText(/ダウンロード中 \d+%/).waitFor();
    await tracking.getByRole('button', { name: 'キャンセル' }).waitFor();
    assert.ok(await tracking.getByRole('button', { name: '開く' }).isDisabled(), 'open is disabled during the update');
    await shot(page, '3-downloading');

    await tracking.getByText('2026.1107.1をインストールしました。').waitFor({ timeout: 15000 });
    assert.ok(await tracking.getByText('最新です').isVisible());
    assert.equal(await tracking.getByRole('button', { name: 'アップデート' }).count(), 0);
    assert.ok(await tracking.getByText('起動中').isVisible(), 'closed app is started again');
    await shot(page, '4-updated');

    assert.deepEqual(errors, []);
    console.log('UI test passed');
  } finally {
    await app.close();
  }
}

main().catch(error => {
  console.error(error);
  process.exitCode = 1;
});
