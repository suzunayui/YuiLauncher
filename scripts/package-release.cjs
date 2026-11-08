'use strict';

// Collects what a GitHub Release needs into release/: the installer under its public
// name, one checksum file, and latest.yml for Electron launchers still out there, whose
// updater reads that file to find this installer.
//
// The Mac zip from scripts/package-mac.sh goes in release/ first. Its hash joins the
// installer's in the same checksum file: launchers up to 2026.1107.12 read only the first
// checksum file a release lists, which by name would be a separate Mac one.
const crypto = require('node:crypto');
const fs = require('node:fs');
const path = require('node:path');
const { version } = require('../package.json');

const root = path.join(__dirname, '..');
const built = path.join(root, 'src-tauri', 'target', 'release', 'bundle', 'nsis', `YuiLauncher_${version}_x64-setup.exe`);
const output = path.join(root, 'release');
const name = `YuiLauncher-Setup-${version}.exe`;
const macName = `YuiLauncher-${version}-mac-arm64.zip`;

fs.mkdirSync(output, { recursive: true });
for (const file of fs.readdirSync(output)) {
  if (file !== macName) fs.rmSync(path.join(output, file), { recursive: true, force: true });
}
fs.copyFileSync(built, path.join(output, name));

const bytes = fs.readFileSync(built);
const sha256 = data => crypto.createHash('sha256').update(data).digest('hex');
const sha512 = crypto.createHash('sha512').update(bytes).digest('base64');
const sums = [`${sha256(bytes)}  ${name}`];
const macPath = path.join(output, macName);
if (fs.existsSync(macPath)) {
  sums.push(`${sha256(fs.readFileSync(macPath))}  ${macName}`);
} else {
  console.warn(`No ${macName} in release/: this release will have no Mac version.`);
}
fs.writeFileSync(path.join(output, `SHA256SUMS-${version}.txt`), `${sums.join('\n')}\n`);
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
console.log(`Packaged ${name} (${(bytes.length / 1024 / 1024).toFixed(2)}MB)${fs.existsSync(macPath) ? ` and ${macName}` : ''}`);
