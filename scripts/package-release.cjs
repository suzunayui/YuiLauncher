'use strict';

// Collects what a GitHub Release needs into release/: the installer under its public
// name, its checksum for the launcher, and latest.yml for Electron launchers still
// out there, whose updater reads that file to find this installer.
const crypto = require('node:crypto');
const fs = require('node:fs');
const path = require('node:path');
const { version } = require('../package.json');

const root = path.join(__dirname, '..');
const built = path.join(root, 'src-tauri', 'target', 'release', 'bundle', 'nsis', `YuiLauncher_${version}_x64-setup.exe`);
const output = path.join(root, 'release');
const name = `YuiLauncher-Setup-${version}.exe`;

fs.rmSync(output, { recursive: true, force: true });
fs.mkdirSync(output, { recursive: true });
fs.copyFileSync(built, path.join(output, name));

const bytes = fs.readFileSync(built);
const sha256 = crypto.createHash('sha256').update(bytes).digest('hex');
const sha512 = crypto.createHash('sha512').update(bytes).digest('base64');
fs.writeFileSync(path.join(output, `SHA256SUMS-${version}.txt`), `${sha256}  ${name}\n`);
fs.writeFileSync(path.join(output, 'latest.yml'), [
  `version: ${version}`,
  'files:',
  `  - url: ${name}`,
  `    sha512: ${sha512}`,
  `    size: ${bytes.length}`,
  `path: ${name}`,
  `sha512: ${sha512}`,
  `releaseDate: '${new Date().toISOString()}'`,
  '',
].join('\n'));
console.log(`Packaged ${name} (${(bytes.length / 1024 / 1024).toFixed(2)}MB)`);
