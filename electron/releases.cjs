'use strict';

const { isVersion, compareVersions } = require('./version.cjs');

const API_HEADERS = {
  Accept: 'application/vnd.github+json',
  'User-Agent': 'YuiLauncher',
  'X-GitHub-Api-Version': '2022-11-28',
};

class ReleaseError extends Error {}

// Picks the newest stable release holding this app's Windows installer.
// A Mac-only release or a -mac.1 / -beta tag must never hide the Windows one.
function pickLatestRelease(releases, app) {
  let best = null;
  for (const release of releases) {
    if (release.draft || release.prerelease) continue;
    const version = String(release.tag_name || '').replace(/^v/, '');
    if (!isVersion(version)) continue;
    const assets = release.assets || [];
    const installer = assets.find(asset => app.installerPattern.test(asset.name));
    const checksums = assets.find(asset => app.checksumPattern.test(asset.name));
    if (!installer || !checksums) continue;
    if (!isTrustedAssetUrl(installer.browser_download_url, app)) continue;
    if (!isTrustedAssetUrl(checksums.browser_download_url, app)) continue;
    if (best && compareVersions(version, best.version) <= 0) continue;
    best = {
      version,
      pageUrl: release.html_url,
      installer: { name: installer.name, size: installer.size, url: installer.browser_download_url },
      checksums: { name: checksums.name, url: checksums.browser_download_url },
    };
  }
  return best;
}

function isTrustedAssetUrl(url, app) {
  return typeof url === 'string' && url.startsWith(`https://github.com/${app.repo}/releases/download/`);
}

function parseChecksums(text) {
  const checksums = new Map();
  for (const line of text.replace(/^﻿/, '').split(/\r?\n/)) {
    const match = /^([a-fA-F0-9]{64})\s+\*?(.+?)\s*$/.exec(line);
    if (match) checksums.set(match[2], match[1].toLowerCase());
  }
  return checksums;
}

function createReleaseClient({ fetchImpl = fetch } = {}) {
  // Conditional requests that return 304 do not count against GitHub's limit.
  const cache = new Map();

  async function latest(app) {
    const url = `https://api.github.com/repos/${app.repo}/releases?per_page=30`;
    const cached = cache.get(url);
    let response;
    try {
      response = await fetchImpl(url, {
        headers: { ...API_HEADERS, ...(cached ? { 'If-None-Match': cached.etag } : {}) },
        signal: AbortSignal.timeout(20000),
      });
    } catch {
      throw new ReleaseError('更新を確認できませんでした。インターネット接続を確認してください。');
    }
    if (response.status === 304 && cached) return pickLatestRelease(cached.releases, app);
    if (response.status === 403 || response.status === 429) {
      throw new ReleaseError('GitHubの確認回数の上限に達しました。しばらくしてからもう一度お試しください。');
    }
    if (!response.ok) throw new ReleaseError(`更新を確認できませんでした（GitHub ${response.status}）。`);
    const releases = await response.json();
    const etag = response.headers.get('etag');
    if (etag) cache.set(url, { etag, releases });
    return pickLatestRelease(releases, app);
  }

  async function fetchText(url) {
    const response = await fetchImpl(url, {
      headers: { 'User-Agent': API_HEADERS['User-Agent'] },
      signal: AbortSignal.timeout(30000),
    });
    if (!response.ok) throw new ReleaseError(`チェックサムを取得できませんでした（${response.status}）。`);
    return response.text();
  }

  return { latest, fetchText };
}

module.exports = { ReleaseError, pickLatestRelease, parseChecksums, createReleaseClient };
