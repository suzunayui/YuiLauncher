'use strict';

// Versions look like 2026.1107.1, where the middle part has no leading zero
// (2026.103.1 is January 3), so each part has to be compared as a number.
const VERSION = /^\d+(\.\d+)*$/;

function isVersion(value) {
  return typeof value === 'string' && VERSION.test(value);
}

function compareVersions(a, b) {
  if (!isVersion(a) || !isVersion(b)) throw new Error(`Invalid version: ${a} / ${b}`);
  const left = a.split('.').map(Number);
  const right = b.split('.').map(Number);
  for (let i = 0; i < Math.max(left.length, right.length); i += 1) {
    const difference = (left[i] || 0) - (right[i] || 0);
    if (difference !== 0) return Math.sign(difference);
  }
  return 0;
}

module.exports = { isVersion, compareVersions };
