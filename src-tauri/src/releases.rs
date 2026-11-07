//! Finds each app's newest Windows release on GitHub.

use crate::catalog::AppDef;
use crate::version::{compare_versions, is_version};
use serde_json::Value;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Clone, Debug)]
pub struct Asset {
    pub name: String,
    pub size: u64,
    pub url: String,
}

#[derive(Clone, Debug)]
pub struct Release {
    pub version: String,
    pub page_url: String,
    pub installer: Asset,
    pub checksums: Asset,
}

pub trait Releases: Send + Sync {
    fn latest(&self, app: &AppDef) -> Result<Option<Release>, String>;
    fn fetch_text(&self, url: &str) -> Result<String, String>;
}

pub fn agent(read_timeout: Duration) -> Result<ureq::Agent, String> {
    let tls = native_tls::TlsConnector::new().map_err(|error| error.to_string())?;
    Ok(ureq::AgentBuilder::new()
        .tls_connector(Arc::new(tls))
        .timeout_connect(Duration::from_secs(20))
        .timeout_read(read_timeout)
        .build())
}

fn trusted_asset(asset: &Value, app: &AppDef) -> Option<Asset> {
    let url = asset["browser_download_url"].as_str()?;
    if !url.starts_with(&format!("https://github.com/{}/releases/download/", app.repo)) {
        return None;
    }
    Some(Asset {
        name: asset["name"].as_str()?.to_string(),
        size: asset["size"].as_u64().unwrap_or(0),
        url: url.to_string(),
    })
}

/// Picks the newest stable release holding this app's Windows installer.
/// A Mac-only release or a -mac.1 / -beta tag must never hide the Windows one.
pub fn pick_latest_release(releases: &Value, app: &AppDef) -> Option<Release> {
    let mut best: Option<Release> = None;
    for release in releases.as_array().into_iter().flatten() {
        if release["draft"].as_bool().unwrap_or(false) || release["prerelease"].as_bool().unwrap_or(false) {
            continue;
        }
        let tag = release["tag_name"].as_str().unwrap_or_default();
        let version = tag.strip_prefix('v').unwrap_or(tag);
        if !is_version(version) {
            continue;
        }
        let assets = release["assets"].as_array().map(Vec::as_slice).unwrap_or_default();
        let named = |test: &dyn Fn(&str) -> bool| {
            assets.iter().find(|asset| test(asset["name"].as_str().unwrap_or_default()))
        };
        let (Some(installer), Some(checksums)) = (named(&|name| app.is_installer(name)), named(&|name| app.is_checksums(name))) else {
            continue;
        };
        let (Some(installer), Some(checksums)) = (trusted_asset(installer, app), trusted_asset(checksums, app)) else {
            continue;
        };
        if best.as_ref().is_some_and(|current| compare_versions(version, &current.version) != Some(Ordering::Greater)) {
            continue;
        }
        best = Some(Release {
            version: version.to_string(),
            page_url: release["html_url"].as_str().unwrap_or_default().to_string(),
            installer,
            checksums,
        });
    }
    best
}

/// Reads `<sha256>  <name>` and `<sha256> *<name>` lines. Names map to lower-case hashes.
pub fn parse_checksums(text: &str) -> HashMap<String, String> {
    let mut checksums = HashMap::new();
    for line in text.trim_start_matches('\u{feff}').lines() {
        let Some((hash, rest)) = line.split_at_checked(64) else { continue };
        if !hash.chars().all(|c| c.is_ascii_hexdigit()) || !rest.starts_with(char::is_whitespace) {
            continue;
        }
        let name = rest.trim_start();
        let name = name.strip_prefix('*').unwrap_or(name).trim_end();
        if !name.is_empty() {
            checksums.insert(name.to_string(), hash.to_lowercase());
        }
    }
    checksums
}

pub struct GitHub {
    api_root: String,
    agent: ureq::Agent,
    // Conditional requests that return 304 do not count against GitHub's limit.
    cache: Mutex<HashMap<String, (String, Value)>>,
}

impl GitHub {
    pub fn new() -> Result<Self, String> {
        Self::with_root("https://api.github.com")
    }

    fn with_root(api_root: &str) -> Result<Self, String> {
        Ok(Self {
            api_root: api_root.to_string(),
            agent: agent(Duration::from_secs(20))?,
            cache: Mutex::new(HashMap::new()),
        })
    }
}

impl Releases for GitHub {
    fn latest(&self, app: &AppDef) -> Result<Option<Release>, String> {
        const NETWORK: &str = "更新を確認できませんでした。インターネット接続を確認してください。";
        let url = format!("{}/repos/{}/releases?per_page=30", self.api_root, app.repo);
        let etag = self.cache.lock().unwrap().get(&url).map(|(etag, _)| etag.clone());
        let mut request = self
            .agent
            .get(&url)
            .set("Accept", "application/vnd.github+json")
            .set("User-Agent", "YuiLauncher")
            .set("X-GitHub-Api-Version", "2022-11-28");
        if let Some(etag) = &etag {
            request = request.set("If-None-Match", etag);
        }
        let response = request.call().map_err(|error| match error {
            ureq::Error::Status(403 | 429, _) => {
                "GitHubの確認回数の上限に達しました。しばらくしてからもう一度お試しください。".to_string()
            }
            ureq::Error::Status(code, _) => format!("更新を確認できませんでした（GitHub {code}）。"),
            ureq::Error::Transport(_) => NETWORK.to_string(),
        })?;
        if response.status() == 304 {
            let cache = self.cache.lock().unwrap();
            if let Some((_, releases)) = cache.get(&url) {
                return Ok(pick_latest_release(releases, app));
            }
        }
        let new_etag = response.header("etag").map(str::to_string);
        let releases: Value = response.into_json().map_err(|_| NETWORK.to_string())?;
        let picked = pick_latest_release(&releases, app);
        if let Some(new_etag) = new_etag {
            self.cache.lock().unwrap().insert(url, (new_etag, releases));
        }
        Ok(picked)
    }

    fn fetch_text(&self, url: &str) -> Result<String, String> {
        let response = self.agent.get(url).set("User-Agent", "YuiLauncher").call().map_err(|error| match error {
            ureq::Error::Status(code, _) => format!("チェックサムを取得できませんでした（{code}）。"),
            ureq::Error::Transport(_) => "チェックサムを取得できませんでした。".to_string(),
        })?;
        response.into_string().map_err(|_| "チェックサムを取得できませんでした。".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::APPS;
    use crate::testutil::serve;
    use serde_json::json;

    fn voice() -> &'static AppDef {
        &APPS[1]
    }

    fn release_of(tag: &str, names: &[&str], repo: &str) -> Value {
        let assets: Vec<Value> = names
            .iter()
            .map(|name| json!({"name": name, "size": 10, "browser_download_url": format!("https://github.com/{repo}/releases/download/{tag}/{name}")}))
            .collect();
        json!({"tag_name": tag, "html_url": format!("https://github.com/{repo}/releases/tag/{tag}"), "draft": false, "prerelease": false, "assets": assets})
    }

    fn voice_release(tag: &str, version: &str) -> Value {
        release_of(tag, &[&format!("YuiVoiceChanger-Setup-{version}.exe"), "SHA256SUMS.txt"], voice().repo)
    }

    #[test]
    fn skips_a_newer_mac_only_release() {
        let picked = pick_latest_release(
            &json!([
                release_of("v2026.1110.1", &["YuiVoiceChanger-2026.1110.1-mac-arm64.dmg", "SHA256SUMS.txt"], voice().repo),
                voice_release("v2026.1104.31", "2026.1104.31"),
            ]),
            voice(),
        );
        assert_eq!(picked.unwrap().version, "2026.1104.31");
    }

    #[test]
    fn skips_mac_tags_betas_drafts_and_prereleases() {
        let mut draft = voice_release("v2026.1200.1", "2026.1200.1");
        draft["draft"] = json!(true);
        let mut prerelease = voice_release("v2026.1201.1", "2026.1201.1");
        prerelease["prerelease"] = json!(true);
        let picked = pick_latest_release(
            &json!([
                voice_release("v2026.1103.34-mac.1", "2026.1103.34"),
                voice_release("v0.3.0-beta.7", "0.3.0"),
                draft,
                prerelease,
                voice_release("v2026.1103.34", "2026.1103.34"),
            ]),
            voice(),
        );
        assert_eq!(picked.unwrap().version, "2026.1103.34");
    }

    #[test]
    fn picks_the_highest_version_regardless_of_order() {
        let picked = pick_latest_release(&json!([voice_release("v2026.103.1", "2026.103.1"), voice_release("v2026.1021.3", "2026.1021.3")]), voice());
        assert_eq!(picked.unwrap().version, "2026.1021.3");
    }

    #[test]
    fn finds_the_tracking_installer_among_mac_assets_and_versioned_checksums() {
        let tracking = &APPS[0];
        let picked = pick_latest_release(
            &json!([release_of(
                "v2026.1107.1",
                &["SHA256SUMS-2026.1107.1.txt", "YuiTracking-2026.1107.1-arm64-mac.zip", "YuiTracking-2026.1107.1-arm64.dmg", "YuiTracking-Setup-2026.1107.1-x64.exe"],
                tracking.repo,
            )]),
            tracking,
        )
        .unwrap();
        assert_eq!(picked.installer.name, "YuiTracking-Setup-2026.1107.1-x64.exe");
        assert_eq!(picked.checksums.name, "SHA256SUMS-2026.1107.1.txt");
    }

    #[test]
    fn livekit_skips_releases_from_its_former_name() {
        let live_kit = &APPS[2];
        let picked = pick_latest_release(
            &json!([
                release_of("YouTubeOverlay-2026.10.10-18", &["YouTubeOverlay-2026.10.10-18.exe"], live_kit.repo),
                release_of("v2026.1107.1", &["YuiLiveKit-Setup-2026.1107.1.exe", "SHA256SUMS.txt"], live_kit.repo),
            ]),
            live_kit,
        )
        .unwrap();
        assert_eq!(picked.version, "2026.1107.1");
    }

    #[test]
    fn requires_checksums_and_rejects_assets_hosted_elsewhere() {
        assert!(pick_latest_release(&json!([release_of("v1.0.0", &["YuiVoiceChanger-Setup-1.0.0.exe"], voice().repo)]), voice()).is_none());
        let mut foreign = voice_release("v1.0.0", "1.0.0");
        foreign["assets"][0]["browser_download_url"] = json!("https://example.com/YuiVoiceChanger-Setup-1.0.0.exe");
        assert!(pick_latest_release(&json!([foreign]), voice()).is_none());
    }

    #[test]
    fn parses_checksum_files_in_both_common_formats() {
        let (a, b) = ("A".repeat(64), "b".repeat(64));
        let parsed = parse_checksums(&format!("\u{feff}{a}  one.exe\r\n{b} *two three.exe\nnot a checksum\n"));
        assert_eq!(parsed["one.exe"], "a".repeat(64));
        assert_eq!(parsed["two three.exe"], b);
        assert_eq!(parsed.len(), 2);
    }

    #[test]
    fn reuses_the_cached_list_on_304_and_sends_no_personal_data() {
        let body = json!([voice_release("v1.0.0", "1.0.0")]).to_string();
        let (root, requests) = serve(vec![
            format!("HTTP/1.1 200 OK\r\nETag: \"abc\"\r\nContent-Length: {}\r\n\r\n{body}", body.len()).into_bytes(),
            b"HTTP/1.1 304 Not Modified\r\nContent-Length: 0\r\n\r\n".to_vec(),
        ]);
        let client = GitHub::with_root(&root).unwrap();
        assert_eq!(client.latest(voice()).unwrap().unwrap().version, "1.0.0");
        assert_eq!(client.latest(voice()).unwrap().unwrap().version, "1.0.0");
        let requests = requests.lock().unwrap();
        assert!(requests[0].contains("YuiLauncher"));
        assert!(requests[1].to_lowercase().contains("if-none-match: \"abc\""));
    }

    #[test]
    fn explains_rate_limits_and_network_failures() {
        let (root, _) = serve(vec![b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\n\r\n".to_vec()]);
        assert!(GitHub::with_root(&root).unwrap().latest(voice()).unwrap_err().contains("上限"));
        let offline = GitHub::with_root("http://127.0.0.1:9").unwrap();
        assert!(offline.latest(voice()).unwrap_err().contains("インターネット"));
    }
}
