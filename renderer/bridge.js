'use strict';

// The page's only way to reach the app.
(() => {
  const { invoke } = window.__TAURI__.core;
  const { listen } = window.__TAURI__.event;

  window.launcher = {
    getState: () => invoke('get_state'),
    check: () => invoke('check'),
    open: id => invoke('open_app', { id }),
    install: (id, closeRunning) => invoke('install', { id, closeRunning: closeRunning === true }),
    uninstall: (id, closeRunning) => invoke('uninstall', { id, closeRunning: closeRunning === true }),
    cancel: id => invoke('cancel', { id }),
    openReleasePage: id => invoke('open_release_page', { id }),
    openSupportPage: () => invoke('open_support_page'),
    getSelfUpdate: () => invoke('get_self_update'),
    installSelfUpdate: () => invoke('install_self_update'),
    onState: listener => listen('state', event => listener(event.payload)),
    onSelfUpdate: listener => listen('self-update', event => listener(event.payload)),
  };
})();
