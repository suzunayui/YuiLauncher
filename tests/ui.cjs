'use strict';

// Drives the real window of a debug build against its fake machine and GitHub (see
// src-tauri/src/fixture.rs), and saves screenshots to the folder in YUI_LAUNCHER_SHOTS, if any.
const assert = require('node:assert/strict');
const { execFileSync, spawn } = require('node:child_process');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { chromium } = require('@playwright/test');
const { version } = require('../package.json');

const shots = process.env.YUI_LAUNCHER_SHOTS;
const root = path.join(__dirname, '..');
const manifest = path.join(root, 'src-tauri', 'Cargo.toml');
const exe = path.join(root, 'src-tauri', 'target', 'debug', 'yui-launcher.exe');
const PORT = 9347;

const cargoEnv = { ...process.env, Path: `${path.join(os.homedir(), '.cargo', 'bin')};${process.env.Path}` };

async function connect() {
  const deadline = Date.now() + 30000;
  for (;;) {
    try {
      const browser = await chromium.connectOverCDP(`http://127.0.0.1:${PORT}`);
      const [page] = browser.contexts()[0].pages();
      if (page) return { browser, page };
      await browser.close();
    } catch (error) {
      if (Date.now() > deadline) throw error;
    }
    await new Promise(resolve => setTimeout(resolve, 200));
  }
}

async function run(page) {
  const shot = async name => {
    if (shots) await page.screenshot({ path: path.join(shots, `${name}.png`) });
  };
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  page.on('console', message => {
    if (message.type() === 'error') errors.push(`${message.text()} ${message.location().url}`);
  });
  const invoke = (command, args) => page.evaluate(([name, values]) => window.__TAURI__.core.invoke(name, values), [command, args]);
  const recorded = () => invoke('fixture_recorded');

  await page.waitForLoadState();
  const tracking = page.locator('[data-app="yuitracking"]');
  const voice = page.locator('[data-app="yuivoicechanger"]');
  const liveKit = page.locator('[data-app="yuilivekit"]');

  await tracking.getByRole('button', { name: 'アップデート' }).waitFor();
  await page.getByText('1件のアップデートがあります').waitFor();
  assert.ok(await tracking.getByText('起動中').isVisible());
  assert.ok(await voice.getByText('最新です').isVisible());
  assert.equal(await voice.getByRole('button', { name: 'アップデート' }).count(), 0);
  assert.ok(await liveKit.getByText('最新です').isVisible());
  const iconsLoaded = await page.locator('.icon').evaluateAll(icons => icons.every(icon => icon.naturalWidth > 0));
  assert.ok(iconsLoaded, 'app icons load');
  const overflow = await page.evaluate(() => document.documentElement.scrollWidth > window.innerWidth);
  assert.equal(overflow, false, 'no horizontal scroll');
  await shot('1-update-available');

  const setSelf = (phase, changes = {}) => invoke('fixture_self_update', {
    state: { currentVersion: version, phase, latestVersion: null, percent: null, error: null, ...changes },
  });
  const banner = page.locator('#self-update');
  const ownVersion = page.locator('#own-version');
  await ownVersion.getByText(`${version} ・ 最新です`).waitFor();
  assert.equal(await banner.isVisible(), false, 'no banner while up to date');

  const checksBefore = (await recorded()).selfChecks;
  await page.getByRole('button', { name: '更新を確認' }).click();
  await page.waitForFunction(() => !document.getElementById('check').disabled);
  for (let tries = 0; (await recorded()).selfChecks === checksBefore && tries < 50; tries += 1) {
    await page.waitForTimeout(100);
  }
  assert.equal((await recorded()).selfChecks, checksBefore + 1, 'the button checks the launcher too');

  await setSelf('checking');
  await page.waitForFunction(() => !document.getElementById('own-version').textContent.includes('最新です'));
  await setSelf('error', { error: 'YuiLauncherの更新を確認できませんでした。しばらくしてからもう一度お試しください。' });
  await banner.getByText('YuiLauncherの更新を確認できませんでした').waitFor();
  assert.equal(await page.locator('#self-update-install').isVisible(), false, 'nothing to install after a failure');
  await shot('7-self-update-error');

  await setSelf('checking');
  await banner.waitFor({ state: 'hidden' });
  await setSelf('downloading', { latestVersion: '2099.1.1', percent: 42 });
  await banner.getByText('YuiLauncher 2099.1.1 をダウンロード中 42%').waitFor();
  assert.equal(await page.locator('#self-update-install').isVisible(), false, 'not installable yet');
  await shot('8-self-update-downloading');

  await setSelf('ready', { latestVersion: '2099.1.1' });
  await banner.getByText('YuiLauncher 2099.1.1 の準備ができました。').waitFor();
  await shot('9-self-update-ready');
  await page.getByRole('button', { name: '再起動して更新' }).click();
  for (let tries = 0; (await recorded()).selfInstalls === 0 && tries < 50; tries += 1) {
    await page.waitForTimeout(100);
  }
  assert.equal((await recorded()).selfInstalls, 1);

  await page.reload();
  await page.locator('#self-update').getByText('YuiLauncher 2099.1.1 の準備ができました。').waitFor();
  await tracking.getByRole('button', { name: 'アップデート' }).waitFor();

  const support = page.getByRole('button', { name: 'FANBOXで応援する' });
  const lastCard = await page.locator('.card').last().boundingBox();
  assert.ok((await support.boundingBox()).y > lastCard.y + lastCard.height, 'the link is below the apps');
  await support.click();
  for (let tries = 0; (await recorded()).openedUrls.length === 0 && tries < 50; tries += 1) {
    await page.waitForTimeout(100);
  }
  assert.deepEqual((await recorded()).openedUrls, ['https://suzunayui.fanbox.cc/']);

  for (const width of [744, 680, 620]) {
    await page.setViewportSize({ width, height: 563 });
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
  await page.setViewportSize({ width: 744, height: 563 });

  const order = () => page.locator('.card').evaluateAll(cards => cards.map(card => card.dataset.app));
  const waitForOrder = async expected => {
    for (let tries = 0; (await order()).join() !== expected.join() && tries < 50; tries += 1) await page.waitForTimeout(50);
    assert.deepEqual(await order(), expected);
  };
  assert.deepEqual(await order(), ['yuitracking', 'yuivoicechanger', 'yuilivekit']);
  const handle = card => card.locator('.handle');
  const handleBox = await handle(tracking).boundingBox();
  const liveKitBox = await liveKit.boundingBox();
  await page.mouse.move(handleBox.x + handleBox.width / 2, handleBox.y + handleBox.height / 2);
  await page.mouse.down();
  await page.mouse.move(handleBox.x + handleBox.width / 2, liveKitBox.y + liveKitBox.height * 0.8, { steps: 12 });
  assert.ok(await tracking.evaluate(card => card.classList.contains('dragging')), 'the card follows the pointer');
  await shot('10-dragging');
  await page.mouse.up();
  await waitForOrder(['yuivoicechanger', 'yuilivekit', 'yuitracking']);
  assert.equal(await tracking.evaluate(card => card.style.transform), '', 'the card settles after the drop');
  await page.reload();
  await waitForOrder(['yuivoicechanger', 'yuilivekit', 'yuitracking']);

  await handle(tracking).focus();
  await page.keyboard.press('ArrowUp');
  await waitForOrder(['yuivoicechanger', 'yuitracking', 'yuilivekit']);
  assert.equal(await page.evaluate(() => document.activeElement.dataset.key), 'yuitracking:move', 'focus stays on the moved card');
  await page.keyboard.press('ArrowUp');
  await page.keyboard.press('ArrowUp');
  await waitForOrder(['yuitracking', 'yuivoicechanger', 'yuilivekit']);

  // A click on the handle alone changes nothing.
  await handle(voice).click();
  await waitForOrder(['yuitracking', 'yuivoicechanger', 'yuilivekit']);

  const hiddenApps = page.locator('#hidden-apps');
  const hiddenRow = id => page.locator(`.hidden-row[data-app="${id}"]`);
  assert.equal(await hiddenApps.isVisible(), false, 'no hidden list while nothing is hidden');
  await tracking.getByRole('button', { name: '非表示', exact: true }).click();
  await waitForOrder(['yuivoicechanger', 'yuilivekit']);
  await hiddenApps.getByText('非表示のアプリ（1）').waitFor();
  assert.equal(await page.getByText('件のアップデートがあります').count(), 0, 'a hidden app is not counted');
  await hiddenApps.locator('summary').click();
  assert.ok(await hiddenRow('yuitracking').getByText('アップデートあり').isVisible(), 'its update still shows in the list');
  await shot('11-hidden');

  // Moving the shown apps keeps the hidden one hidden.
  await handle(liveKit).focus();
  await page.keyboard.press('ArrowUp');
  await waitForOrder(['yuilivekit', 'yuivoicechanger']);
  await page.reload();
  await waitForOrder(['yuilivekit', 'yuivoicechanger']);
  await hiddenApps.getByText('非表示のアプリ（1）').waitFor();

  for (const card of [liveKit, voice]) await card.getByRole('button', { name: '非表示', exact: true }).click();
  await page.getByText('すべてのアプリを非表示にしています。').waitFor();
  await hiddenApps.getByText('非表示のアプリ（3）').waitFor();
  await shot('12-all-hidden');

  await hiddenApps.locator('summary').click();
  for (const id of ['yuitracking', 'yuivoicechanger', 'yuilivekit']) await hiddenRow(id).getByRole('button', { name: '表示する' }).click();
  await waitForOrder(['yuilivekit', 'yuivoicechanger', 'yuitracking']);
  assert.equal(await hiddenApps.isVisible(), false);
  assert.equal(await page.getByText('すべてのアプリを非表示にしています。').isVisible(), false);
  await page.getByText('1件のアップデートがあります').waitFor();
  await handle(tracking).focus();
  await page.keyboard.press('ArrowUp');
  await page.keyboard.press('ArrowUp');
  await waitForOrder(['yuitracking', 'yuilivekit', 'yuivoicechanger']);
  await handle(voice).focus();
  await page.keyboard.press('ArrowUp');
  await waitForOrder(['yuitracking', 'yuivoicechanger', 'yuilivekit']);

  await tracking.getByRole('button', { name: 'アップデート' }).click();
  const dialog = page.locator('#close-dialog');
  await dialog.waitFor();
  await shot('2-confirm-close');
  await dialog.getByRole('button', { name: 'やめる' }).click();
  assert.equal(await tracking.locator('.progress').count(), 0, 'cancel leaves the app alone');

  await tracking.getByRole('button', { name: 'アップデート' }).click();
  await dialog.getByRole('button', { name: '閉じて更新' }).click();
  await tracking.getByText(/ダウンロード中 \d+%/).waitFor();
  await tracking.getByRole('button', { name: 'キャンセル' }).waitFor();
  assert.ok(await tracking.getByRole('button', { name: '開く' }).isDisabled(), 'open is disabled during the update');
  await shot('3-downloading');

  await tracking.getByText('2026.1107.1をインストールしました。').waitFor({ timeout: 15000 });
  assert.ok(await tracking.getByText('最新です').isVisible());
  assert.equal(await tracking.getByRole('button', { name: 'アップデート' }).count(), 0);
  assert.ok(await tracking.getByText('起動中').isVisible(), 'closed app is started again');
  await shot('4-updated');

  const confirm = dialog.getByRole('button', { name: 'アンインストール' });
  await liveKit.getByRole('button', { name: 'アンインストール' }).click();
  await dialog.getByText('YuiLiveKitをアンインストールしますか？').waitFor();
  assert.ok(await dialog.getByText(/コメント、設定/).isVisible(), 'says which data stays');
  assert.equal(await page.evaluate(() => document.activeElement.textContent), 'やめる', 'the safe choice has focus');
  await shot('5-confirm-uninstall');
  await dialog.getByRole('button', { name: 'やめる' }).click();
  assert.ok(await liveKit.getByText('最新です').isVisible(), 'cancel keeps the app');

  await liveKit.getByRole('button', { name: 'アンインストール' }).click();
  await confirm.click();
  await liveKit.getByText('YuiLiveKitをアンインストールしました。').waitFor({ timeout: 15000 });
  assert.ok(await liveKit.getByText(/未インストール/).isVisible());
  assert.equal(await liveKit.getByRole('button', { name: '開く' }).count(), 0);
  assert.equal(await liveKit.getByRole('button', { name: 'アンインストール' }).count(), 0);
  assert.ok(await liveKit.getByRole('button', { name: 'インストール' }).isVisible(), 'can install again');
  await shot('6-uninstalled');

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
}

async function main() {
  execFileSync(process.execPath, [path.join(root, 'scripts', 'prepare-tauri-frontend.cjs')], { stdio: 'inherit' });
  execFileSync('cargo', ['build', '--manifest-path', manifest], { stdio: 'inherit', env: cargoEnv });

  // A separate profile keeps this away from an installed launcher's data.
  const profile = fs.mkdtempSync(path.join(os.tmpdir(), 'yui-launcher-test-'));
  const app = spawn(exe, [], {
    stdio: 'ignore',
    env: {
      ...process.env,
      YUI_LAUNCHER_FIXTURE: '1',
      WEBVIEW2_USER_DATA_FOLDER: profile,
      WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${PORT}`,
    },
  });
  let browser = null;
  try {
    const connection = await connect();
    browser = connection.browser;
    await run(connection.page);
    console.log('UI test passed');
  } finally {
    if (browser) await browser.close().catch(() => {});
    app.kill();
    await new Promise(resolve => setTimeout(resolve, 1000));
    fs.rmSync(profile, { recursive: true, force: true, maxRetries: 5, retryDelay: 300 });
  }
}

main().catch(error => {
  console.error(error);
  process.exitCode = 1;
});
