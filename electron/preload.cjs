'use strict';

const { contextBridge, ipcRenderer } = require('electron');

contextBridge.exposeInMainWorld('launcher', {
  getState: () => ipcRenderer.invoke('state:get'),
  check: () => ipcRenderer.invoke('state:check'),
  open: id => ipcRenderer.invoke('app:open', id),
  install: (id, closeRunning) => ipcRenderer.invoke('app:install', id, closeRunning),
  uninstall: (id, closeRunning) => ipcRenderer.invoke('app:uninstall', id, closeRunning),
  cancel: id => ipcRenderer.invoke('app:cancel', id),
  openReleasePage: id => ipcRenderer.invoke('app:release-page', id),
  installSelfUpdate: () => ipcRenderer.invoke('self:install'),
  onState: listener => ipcRenderer.on('state', (_event, state) => listener(state)),
  onSelfUpdate: listener => ipcRenderer.on('self-update', (_event, state) => listener(state)),
});
