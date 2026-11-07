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
  ownVersion: document.getElementById('own-version'),
  dialog: document.getElementById('close-dialog'),
  dialogTitle: document.getElementById('close-title'),
  dialogBody: document.getElementById('close-body'),
  dialogCancel: document.getElementById('close-cancel'),
  dialogConfirm: document.getElementById('close-confirm'),
};

let pendingConfirm = null;
const icons = new Map();
// While a card is dragged, state changes wait here so the card isn't rebuilt under the pointer.
let drag = null;
let pendingState = null;

const GRIP = '<svg width="10" height="16" viewBox="0 0 10 16" fill="currentColor" aria-hidden="true">'
  + '<circle cx="2" cy="2" r="1.5"/><circle cx="8" cy="2" r="1.5"/>'
  + '<circle cx="2" cy="8" r="1.5"/><circle cx="8" cy="8" r="1.5"/>'
  + '<circle cx="2" cy="14" r="1.5"/><circle cx="8" cy="14" r="1.5"/></svg>';

const shownIds = () => [...elements.apps.children].map(card => card.dataset.app);

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

// Moves the neighbours rather than the dragged card: taking it out of the page would drop the pointer.
function followPointer(event) {
  const { card, grab } = drag;
  card.style.transform = '';
  const top = event.clientY - grab;
  let previous = card.previousElementSibling;
  while (previous && top < previous.getBoundingClientRect().top + previous.offsetHeight / 2) {
    elements.apps.insertBefore(previous, card.nextElementSibling);
    previous = card.previousElementSibling;
  }
  let next = card.nextElementSibling;
  while (next && top + card.offsetHeight > next.getBoundingClientRect().top + next.offsetHeight / 2) {
    elements.apps.insertBefore(next, card);
    next = card.nextElementSibling;
  }
  card.style.transform = `translateY(${top - card.getBoundingClientRect().top}px)`;
}

function endDrag() {
  const { card, handle, before } = drag;
  handle.removeEventListener('pointermove', followPointer);
  handle.removeEventListener('pointerup', endDrag);
  handle.removeEventListener('pointercancel', endDrag);
  card.style.transform = '';
  card.classList.remove('dragging');
  document.body.classList.remove('dragging');
  drag = null;
  const after = shownIds();
  const waiting = pendingState;
  pendingState = null;
  // A new order comes back as a state change, so the old one waiting here would flash.
  if (after.join() !== before.join()) window.launcher.setOrder(after);
  else if (waiting) render(waiting);
}

function startDrag(event) {
  if (event.button !== 0 || drag) return;
  event.preventDefault();
  const handle = event.currentTarget;
  const card = handle.closest('.card');
  handle.setPointerCapture(event.pointerId);
  drag = { card, handle, before: shownIds(), grab: event.clientY - card.getBoundingClientRect().top };
  card.classList.add('dragging');
  document.body.classList.add('dragging');
  handle.addEventListener('pointermove', followPointer);
  handle.addEventListener('pointerup', endDrag);
  handle.addEventListener('pointercancel', endDrag);
}

function moveByKey(event, app) {
  const step = { ArrowUp: -1, ArrowDown: 1 }[event.key];
  if (!step || drag) return;
  event.preventDefault();
  const ids = shownIds();
  const from = ids.indexOf(app.id);
  const to = from + step;
  if (to < 0 || to >= ids.length) return;
  [ids[from], ids[to]] = [ids[to], ids[from]];
  window.launcher.setOrder(ids);
}

function handleFor(app) {
  const node = el('button', 'handle');
  node.type = 'button';
  node.innerHTML = GRIP;
  node.dataset.key = `${app.id}:move`;
  node.title = 'ドラッグして並べ替え';
  node.setAttribute('aria-label', `${app.name}を移動（上下の矢印キーで並べ替え）`);
  node.addEventListener('pointerdown', startDrag);
  node.addEventListener('keydown', event => moveByKey(event, app));
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
  main.append(handleFor(app), iconFor(app), info, actions);
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

function renderSelfUpdate(state) {
  const { phase, currentVersion, latestVersion, percent, error } = state;
  elements.ownVersion.textContent = phase === 'latest' ? `${currentVersion} ・ 最新です` : currentVersion;
  const messages = {
    downloading: `YuiLauncher ${latestVersion} をダウンロード中 ${percent ?? 0}%`,
    ready: `YuiLauncher ${latestVersion} の準備ができました。`,
    error,
  };
  const message = messages[phase];
  elements.selfUpdate.hidden = !message;
  elements.selfUpdate.classList.toggle('error', phase === 'error');
  elements.selfUpdateText.textContent = message || '';
  elements.selfUpdateInstall.hidden = phase !== 'ready';
}

function render(state) {
  if (drag) {
    pendingState = state;
    return;
  }
  elements.selfUpdateInstall.disabled = state.apps.some(app => app.task);
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
document.getElementById('support').addEventListener('click', () => window.launcher.openSupportPage());
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
window.launcher.onSelfUpdate(renderSelfUpdate);
window.launcher.getState().then(render);
// Asks once on load: a cached update can be reported before this page is listening.
window.launcher.getSelfUpdate().then(renderSelfUpdate);
