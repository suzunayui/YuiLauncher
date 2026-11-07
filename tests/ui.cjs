'use strict';

// Drives the real Electron window against tests/fixture.cjs and saves
// screenshots to the folder given in YUI_LAUNCHER_SHOTS, if any.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { _electron: electron } = require('@playwright/test');

const shots = process.env.YUI_LAUNCHER_SHOTS;

async function main() {
  const env = { ...process.env, YUI_LAUNCHER_FIXTURE: path.join(__dirname, 'fixture.cjs') };
  delete env.ELECTRON_RUN_AS_NODE;
  // A separate profile keeps an installed, running launcher from blocking this one.
  const profile = fs.mkdtempSync(path.join(os.tmpdir(), 'yui-launcher-test-'));
  const app = await electron.launch({ args: [`--user-data-dir=${profile}`, path.join(__dirname, '..')], env });
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
    assert.ok(await page.locator('[data-app="yuilivekit"]').getByText('最新です').isVisible());
    const iconsLoaded = await page.locator('.icon').evaluateAll(icons => icons.every(icon => icon.naturalWidth > 0));
    assert.ok(iconsLoaded, 'app icons load');
    const overflow = await page.evaluate(() => document.documentElement.scrollWidth > window.innerWidth);
    assert.equal(overflow, false, 'no horizontal scroll');
    await shot(page, '1-update-available');

    // Records the URL instead of opening a real browser.
    await app.evaluate(({ shell }) => {
      global.openedUrls = [];
      shell.openExternal = async url => { global.openedUrls.push(url); };
    });
    const support = page.getByRole('button', { name: 'FANBOXで応援する' });
    const lastCard = await page.locator('.card').last().boundingBox();
    assert.ok((await support.boundingBox()).y > lastCard.y + lastCard.height, 'the link is below the apps');
    await support.click();
    assert.deepEqual(await app.evaluate(() => global.openedUrls), ['https://suzunayui.fanbox.cc/']);

    const setWidth = width => app.evaluate(({ BrowserWindow }, value) => {
      const [window] = BrowserWindow.getAllWindows();
      window.setContentSize(value, window.getContentSize()[1]);
    }, width);
    for (const width of [744, 680, 620]) {
      await setWidth(width);
      await page.waitForFunction(value => window.innerWidth === value, width);
      for (const card of [tracking, voice]) {
        const buttons = card.locator('.actions .button');
        assert.equal(await buttons.last().textContent(), '開く', 'open is the rightmost button');
        const remove = await card.getByRole('button', { name: 'アンインストール' }).boundingBox();
        const nearest = await buttons.first().boundingBox();
        const gap = nearest.x - (remove.x + remove.width);
        assert.ok(gap >= 40, `uninstall is ${gap}px from the main buttons at width ${width}`);
      }
    }
    await setWidth(744);
    await page.waitForFunction(() => window.innerWidth === 744);

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

    const liveKit = page.locator('[data-app="yuilivekit"]');
    const confirm = dialog.getByRole('button', { name: 'アンインストール' });
    await liveKit.getByRole('button', { name: 'アンインストール' }).click();
    await dialog.getByText('YuiLiveKitをアンインストールしますか？').waitFor();
    assert.ok(await dialog.getByText(/コメント、設定/).isVisible(), 'says which data stays');
    assert.equal(await page.evaluate(() => document.activeElement.textContent), 'やめる', 'the safe choice has focus');
    await shot(page, '5-confirm-uninstall');
    await dialog.getByRole('button', { name: 'やめる' }).click();
    assert.ok(await liveKit.getByText('最新です').isVisible(), 'cancel keeps the app');

    await liveKit.getByRole('button', { name: 'アンインストール' }).click();
    await confirm.click();
    await liveKit.getByText('YuiLiveKitをアンインストールしました。').waitFor({ timeout: 15000 });
    assert.ok(await liveKit.getByText(/未インストール/).isVisible());
    assert.equal(await liveKit.getByRole('button', { name: '開く' }).count(), 0);
    assert.equal(await liveKit.getByRole('button', { name: 'アンインストール' }).count(), 0);
    assert.ok(await liveKit.getByRole('button', { name: 'インストール' }).isVisible(), 'can install again');
    await shot(page, '6-uninstalled');

    await tracking.getByRole('button', { name: 'アンインストール' }).click();
    await dialog.getByText('YuiTrackingをアンインストールしますか？').waitFor();
    await confirm.click();
    await tracking.getByText('YuiTrackingをアンインストールしました。').waitFor({ timeout: 15000 });
    assert.equal(await tracking.getByText('起動中').count(), 0, 'a running app is closed and not started again');

    await liveKit.getByRole('button', { name: 'インストール' }).click();
    await liveKit.getByText('2026.1107.1をインストールしました。').waitFor({ timeout: 15000 });
    await tracking.getByRole('button', { name: 'インストール' }).click();
    await tracking.getByText('2026.1107.1をインストールしました。').waitFor({ timeout: 15000 });

    assert.deepEqual(errors, []);
    console.log('UI test passed');
  } finally {
    await app.close();
    fs.rmSync(profile, { recursive: true, force: true, maxRetries: 5, retryDelay: 200 });
  }
}

main().catch(error => {
  console.error(error);
  process.exitCode = 1;
});
