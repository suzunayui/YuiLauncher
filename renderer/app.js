'use strict';

const PHASES = {
  downloading: 'ダウンロード中',
  verifying: 'ファイルを検証中',
  closing: 'アプリを閉じています',
  installing: 'インストール中',
  confirming: 'インストールを確認中',
  uninstalling: 'アンインストール中',
  removing: 'アンインストールを確認中',
};

const elements = {
  apps: document.getElementById('apps'),
  summary: document.getElementById('summary'),
  check: document.getElementById('check'),
  selfUpdate: document.getElementById('self-update'),
  selfUpdateText: document.getElementById('self-update-text'),
  selfUpdateInstall: document.getElementById('self-update-install'),
  dialog: document.getElementById('close-dialog'),
  dialogTitle: document.getElementById('close-title'),
  dialogBody: document.getElementById('close-body'),
  dialogCancel: document.getElementById('close-cancel'),
  dialogConfirm: document.getElementById('close-confirm'),
};

let pendingConfirm = null;
const icons = new Map();

// Cards are rebuilt on every state change; reusing the image keeps it from flickering.
function iconFor(app) {
  if (!icons.has(app.id)) {
    const icon = el('img', 'icon');
    icon.src = app.icon;
    icon.alt = '';
    icons.set(app.id, icon);
  }
  return icons.get(app.id);
}

function el(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
}

function button(label, key, onClick, { primary = false, disabled = false } = {}) {
  const node = el('button', primary ? 'button primary' : 'button', label);
  node.dataset.key = key;
  node.disabled = disabled;
  node.addEventListener('click', onClick);
  return node;
}

function confirmAction({ title, body, confirmLabel, danger = false, onConfirm }) {
  pendingConfirm = onConfirm;
  elements.dialogTitle.textContent = title;
  elements.dialogBody.textContent = body;
  elements.dialogConfirm.textContent = confirmLabel;
  elements.dialogConfirm.classList.toggle('danger', danger);
  elements.dialog.showModal();
}

async function startInstall(app, closeRunning = false) {
  const result = await window.launcher.install(app.id, closeRunning);
  if (result && result.status === 'needs-close') {
    confirmAction({
      title: `${app.name}を閉じて更新しますか？`,
      body: 'ダウンロードと検証が終わると、アプリを閉じて更新し、自動で起動し直します。配信や作業の途中なら、終わってから更新してください。',
      confirmLabel: '閉じて更新',
      onConfirm: () => startInstall(app, true),
    });
  }
}

function startUninstall(app) {
  confirmAction({
    title: `${app.name}をアンインストールしますか？`,
    body: `${app.uninstallNote} 起動中の場合は、アプリを閉じてからアンインストールします。`,
    confirmLabel: 'アンインストール',
    danger: true,
    onConfirm: () => window.launcher.uninstall(app.id, true),
  });
}

function versionLine(app) {
  const line = el('p', 'version');
  if (!app.installed) {
    line.append('未インストール');
    if (app.latestVersion) line.append(` ・ 最新 ${app.latestVersion}`);
  } else if (app.updateAvailable) {
    line.append(`${app.version || '不明'} → `);
    line.append(el('strong', '', app.latestVersion));
  } else {
    line.append(app.latestVersion ? `${app.version} ・ 最新です` : app.version || 'バージョン不明');
  }
  if (app.hasReleasePage && app.updateAvailable) {
    line.append(' ・ ');
    const link = el('button', 'link', '更新内容');
    link.dataset.key = `${app.id}:notes`;
    link.addEventListener('click', () => window.launcher.openReleasePage(app.id));
    line.append(link);
  }
  // Kept on the left, away from the main buttons, so it can't be hit by mistake.
  if (app.canUninstall) {
    line.append(' ・ ');
    const remove = el('button', 'link danger', 'アンインストール');
    remove.dataset.key = `${app.id}:uninstall`;
    remove.disabled = Boolean(app.task);
    remove.addEventListener('click', () => startUninstall(app));
    line.append(remove);
  }
  return line;
}

function progress(app) {
  const wrapper = el('div', 'progress');
  const row = el('div', 'progress-row');
  const { phase, percent } = app.task;
  const determinate = phase === 'downloading' && percent !== null;
  row.append(el('span', '', determinate ? `${PHASES[phase]} ${percent}%` : PHASES[phase]));
  if (phase === 'downloading') {
    row.append(button('キャンセル', `${app.id}:cancel`, () => window.launcher.cancel(app.id)));
  }
  const track = el('div', 'progress-track');
  const bar = el('div', determinate ? 'progress-bar' : 'progress-bar indeterminate');
  if (determinate) bar.style.width = `${percent}%`;
  track.append(bar);
  wrapper.append(row, track);
  return wrapper;
}

function card(app) {
  const node = el('article', 'card');
  node.dataset.app = app.id;

  const main = el('div', 'card-main');
  const info = el('div', 'info');
  const titleRow = el('div', 'title-row');
  titleRow.append(el('h2', 'name', app.name));
  if (app.running) titleRow.append(el('span', 'running', '起動中'));
  info.append(titleRow, el('p', 'description', app.description), versionLine(app));

  const actions = el('div', 'actions');
  const busy = Boolean(app.task);
  if (app.updateAvailable) {
    actions.append(button(app.installed ? 'アップデート' : 'インストール', `${app.id}:install`, () => startInstall(app), { primary: true, disabled: busy }));
  }
  if (app.installed) {
    const primary = !app.updateAvailable;
    actions.append(button('開く', `${app.id}:open`, () => window.launcher.open(app.id), { primary, disabled: busy || !app.canOpen }));
  }
  main.append(iconFor(app), info, actions);
  node.append(main);

  if (app.task) node.append(progress(app));
  if (app.error) node.append(el('p', 'message error', app.error));
  else if (app.checkError) node.append(el('p', 'message warning', app.checkError));
  else if (app.notice) node.append(el('p', 'message notice', app.notice));
  return node;
}

function summaryText(state) {
  if (state.checking) return '更新を確認しています…';
  const count = state.apps.filter(app => app.installed && app.updateAvailable).length;
  const checkedAt = state.lastCheckedAt
    ? `最終確認 ${new Date(state.lastCheckedAt).toLocaleTimeString('ja-JP', { hour: '2-digit', minute: '2-digit' })}`
    : '';
  const updates = count > 0 ? `${count}件のアップデートがあります` : '';
  return [updates, checkedAt].filter(Boolean).join(' ・ ');
}

function render(state) {
  const focusKey = document.activeElement && document.activeElement.dataset ? document.activeElement.dataset.key : null;
  elements.apps.replaceChildren(...state.apps.map(card));
  elements.summary.textContent = summaryText(state);
  elements.check.disabled = state.checking;
  if (focusKey) {
    const target = elements.apps.querySelector(`[data-key="${CSS.escape(focusKey)}"]`);
    if (target) target.focus();
  }
}

elements.check.addEventListener('click', () => window.launcher.check());
elements.dialogCancel.addEventListener('click', () => {
  pendingConfirm = null;
  elements.dialog.close();
});
elements.dialogConfirm.addEventListener('click', () => {
  const onConfirm = pendingConfirm;
  pendingConfirm = null;
  elements.dialog.close();
  if (onConfirm) onConfirm();
});
elements.selfUpdateInstall.addEventListener('click', () => window.launcher.installSelfUpdate());

window.launcher.onState(render);
window.launcher.onSelfUpdate(({ version }) => {
  elements.selfUpdateText.textContent = `YuiLauncher ${version} の準備ができました。`;
  elements.selfUpdate.hidden = false;
});
window.launcher.getState().then(render);
