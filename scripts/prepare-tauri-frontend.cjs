'use strict';

// Builds src-tauri/frontend, the folder embedded in the app.
const fs = require('node:fs');
const path = require('node:path');

const root = path.join(__dirname, '..');
const output = path.join(root, 'src-tauri', 'frontend');

fs.rmSync(output, { recursive: true, force: true });
fs.cpSync(path.join(root, 'renderer'), path.join(output, 'renderer'), { recursive: true });
fs.cpSync(path.join(root, 'assets', 'apps'), path.join(output, 'assets', 'apps'), { recursive: true });
fs.copyFileSync(path.join(root, 'assets', 'icon.ico'), path.join(output, 'assets', 'icon.ico'));
console.log('Prepared src-tauri/frontend');
