'use strict';

const test = require('node:test');
const assert = require('node:assert/strict');
const { installDirFrom } = require('../../electron/windows.cjs');

test('prefers InstallLocation', () => {
  assert.equal(installDirFrom({ InstallLocation: 'C:\\Apps\\YuiTracking\\', UninstallString: '"C:\\Other\\unins000.exe"' }), 'C:\\Apps\\YuiTracking\\');
});

test('falls back to the uninstaller folder when InstallLocation is empty', () => {
  assert.equal(
    installDirFrom({ InstallLocation: '', UninstallString: '"C:\\Users\\a b\\Programs\\YuiVoiceChanger\\Uninstall YuiVoiceChanger.exe" /currentuser' }),
    'C:\\Users\\a b\\Programs\\YuiVoiceChanger',
  );
});

test('returns null when neither value is usable', () => {
  assert.equal(installDirFrom({}), null);
});
