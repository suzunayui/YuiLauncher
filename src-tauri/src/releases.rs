//! Finds each app's newest Windows release on GitHub.

use crate::catalog::AppDef;
use crate::version::{compare_versions, is_version};
use serde_json::Value;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

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
    /// Every checksum file in the release: some keep Windows and Mac in separate files.
    pub checksums: Vec<Asset>,
}

pub trait Releases: Send + Sync {
    fn latest(&self, app: &AppDef) -> Result<Option<Release>, String>;
    fn fetch_text(&self, url: &str) -> Result<String, String>;
    /// How long until GitHub takes requests again, when it has refused them.
    fn retry_after(&self) -> Option<Duration> {
        None
    }
}

/// How every rate limit message starts, so callers can show it as it is.
pub const RATE_LIMITED: &str = "GitHubの確認回数の上限に達しました。";

fn rate_limited_message(until: Option<SystemTime>) -> String {
    match until.and_then(|until| until.duration_since(SystemTime::now()).ok()) {
        Some(wait) => format!("{RATE_LIMITED}約{}分後にもう一度お試しください。", wait.as_secs().div_ceil(60).max(1)),
        None => format!("{RATE_LIMITED}しばらくしてからもう一度お試しください。"),
    }
}

/// When a refused request may be sent again: Retry-After in seconds, or the epoch second in X-RateLimit-Reset.
fn limit_lifts_at(response: &ureq::Response) -> Option<SystemTime> {
    if let Some(seconds) = response.header("retry-after").and_then(|value| value.trim().parse::<u64>().ok()) {
        return Some(SystemTime::now() + Duration::from_secs(seconds));
    }
    let reset = response.header("x-ratelimit-reset")?.trim().parse::<u64>().ok()?;
    Some(UNIX_EPOCH + Duration::from_secs(reset))
}

/// Keeps only what picking a release reads, so the saved cache stays small.
fn trimmed(releases: &Value) -> Value {
    let releases: Vec<Value> = releases
        .as_array()
        .into_iter()
        .flatten()
        .map(|release| {
            let assets: Vec<Value> = release["assets"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|asset| serde_json::json!({"name": asset["name"], "size": asset["size"], "browser_download_url": asset["browser_download_url"]}))
                .collect();
            serde_json::json!({
                "tag_name": release["tag_name"],
                "html_url": release["html_url"],
                "draft": release["draft"],
                "prerelease": release["prerelease"],
                "assets": assets,
            })
        })
        .collect();
    Value::Array(releases)
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

/// Picks the newest stable release holding this app's installer for this platform.
/// A release for the other platform or a -mac.1 / -beta tag must never hide it.
pub fn pick_latest_release(releases: &Value, app: &AppDef) -> Option<Release> {
    pick_latest_release_for(releases, app, crate::catalog::ON_MAC)
}

pub fn pick_latest_release_for(releases: &Value, app: &AppDef, mac: bool) -> Option<Release> {
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
        let name_of = |asset: &Value| asset["name"].as_str().unwrap_or_default().to_string();
        let Some(installer) = assets.iter().find(|asset| app.is_installer_for(&name_of(asset), mac)).and_then(|asset| trusted_asset(asset, app)) else {
            continue;
        };
        let checksums: Vec<Asset> = assets
            .iter()
            .filter(|asset| app.is_checksums(&name_of(asset)))
            .filter_map(|asset| trusted_asset(asset, app))
            .collect();
        if checksums.is_empty() {
            continue;
        }
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

/// The installer's hash from whichever checksum file lists it. None when none does.
pub fn expected_checksum(releases: &dyn Releases, release: &Release) -> Result<Option<String>, String> {
    let mut failure = None;
    for asset in &release.checksums {
        match releases.fetch_text(&asset.url) {
            Ok(text) => {
                if let Some(hash) = parse_checksums(&text).remove(&release.installer.name) {
                    return Ok(Some(hash));
                }
            }
            Err(error) => {
                failure.get_or_insert(error);
            }
        }
    }
    failure.map_or(Ok(None), Err)
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

/// What the release pages on github.com tell without the API.
enum WebAnswer {
    Found(Release),
    NoRelease,
    /// The latest release has no installer for this platform, so older ones must be searched.
    NeedsApi,
}

pub struct GitHub {
    api_root: String,
    web_root: String,
    agent: ureq::Agent,
    // Reads where a redirect points instead of following it.
    no_redirect: ureq::Agent,
    // Conditional requests that return 304 do not count against GitHub's limit. Kept in a
    // file, they keep working after a restart, so starting the launcher again costs nothing.
    cache: Mutex<HashMap<String, (String, Value)>>,
    cache_file: Option<PathBuf>,
    // While GitHub refuses requests, none are sent: they would only be refused again.
    limited_until: Mutex<Option<SystemTime>>,
}

impl GitHub {
    pub fn new() -> Result<Self, String> {
        Self::with_roots("https://api.github.com", "https://github.com")
    }

    #[cfg(test)]
    fn with_root(api_root: &str) -> Result<Self, String> {
        // A web root that refuses connections sends every lookup to the API.
        Self::with_roots(api_root, "http://127.0.0.1:9")
    }

    fn with_roots(api_root: &str, web_root: &str) -> Result<Self, String> {
        let tls = native_tls::TlsConnector::new().map_err(|error| error.to_string())?;
        let no_redirect = ureq::AgentBuilder::new()
            .tls_connector(Arc::new(tls))
            .timeout_connect(Duration::from_secs(20))
            .timeout_read(Duration::from_secs(20))
            .redirects(0)
            .build();
        Ok(Self {
            api_root: api_root.to_string(),
            web_root: web_root.trim_end_matches('/').to_string(),
            agent: agent(Duration::from_secs(20))?,
            no_redirect,
            cache: Mutex::new(HashMap::new()),
            cache_file: None,
            limited_until: Mutex::new(None),
        })
    }

    /// Loads earlier answers from this file and saves new ones to it. A missing or broken file is an empty cache.
    pub fn with_cache_file(mut self, file: PathBuf) -> Self {
        let saved = std::fs::read_to_string(&file).ok().and_then(|text| serde_json::from_str::<Value>(&text).ok());
        let entries = saved.as_ref().and_then(Value::as_object).into_iter().flatten();
        let loaded = entries.filter_map(|(url, entry)| Some((url.clone(), (entry["etag"].as_str()?.to_string(), entry["releases"].clone()))));
        *self.cache.get_mut().unwrap() = loaded.collect();
        self.cache_file = Some(file);
        self
    }

    /// Finds the latest release from github.com's own pages, which the API's hourly limit
    /// doesn't cover: /releases/latest redirects to the latest tag, and the installer's
    /// download URL follows from its name.
    fn latest_from_web(&self, app: &AppDef) -> Result<WebAnswer, String> {
        let page = format!("{}/{}/releases/latest", self.web_root, app.repo);
        let response = match self.no_redirect.head(&page).set("User-Agent", "YuiLauncher").call() {
            Ok(response) => response,
            Err(ureq::Error::Status(404, _)) => return Ok(WebAnswer::NoRelease),
            Err(error) => return Err(error.to_string()),
        };
        let Some(tag) = response.header("location").and_then(|location| location.rsplit_once("/releases/tag/")).map(|(_, tag)| tag.to_string()) else {
            return Ok(WebAnswer::NeedsApi);
        };
        let version = tag.strip_prefix('v').unwrap_or(&tag).to_string();
        let mac = crate::catalog::ON_MAC;
        let Some(installer_name) = app.installer_name_for(&version, mac).filter(|_| is_version(&version)) else {
            return Ok(WebAnswer::NeedsApi);
        };
        let download = |name: &str| format!("https://github.com/{}/releases/download/{tag}/{name}", app.repo);
        let probe = format!("{}/{}/releases/download/{tag}/{installer_name}", self.web_root, app.repo);
        let size = match self.agent.head(&probe).set("User-Agent", "YuiLauncher").call() {
            Ok(response) => response.header("content-length").and_then(|value| value.parse().ok()).unwrap_or(0),
            Err(ureq::Error::Status(404, _)) => return Ok(WebAnswer::NeedsApi),
            Err(error) => return Err(error.to_string()),
        };
        let checksums = app
            .checksum_names_for(&version, mac)
            .into_iter()
            .map(|name| Asset { url: download(&name), name, size: 0 })
            .collect();
        Ok(WebAnswer::Found(Release {
            page_url: format!("https://github.com/{}/releases/tag/{tag}", app.repo),
            installer: Asset { url: download(&installer_name), name: installer_name, size },
            checksums,
            version,
        }))
    }

    fn save_cache(&self) {
        let Some(file) = &self.cache_file else { return };
        let saved: serde_json::Map<String, Value> = self
            .cache
            .lock()
            .unwrap()
            .iter()
            .map(|(url, (etag, releases))| (url.clone(), serde_json::json!({"etag": etag, "releases": releases})))
            .collect();
        let write = || -> std::io::Result<()> {
            if let Some(parent) = file.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let partial = file.with_extension("json.partial");
            std::fs::write(&partial, Value::Object(saved).to_string())?;
            std::fs::rename(&partial, file)
        };
        if let Err(error) = write() {
            eprintln!("Failed to save the release cache: {error}");
        }
    }
}

impl Releases for GitHub {
    fn latest(&self, app: &AppDef) -> Result<Option<Release>, String> {
        match self.latest_from_web(app) {
            Ok(WebAnswer::Found(release)) => Ok(Some(release)),
            Ok(WebAnswer::NoRelease) => Ok(None),
            // The API may still answer when github.com's pages can't, and it searches older releases.
            Ok(WebAnswer::NeedsApi) | Err(_) => self.latest_from_api(app),
        }
    }

    fn fetch_text(&self, url: &str) -> Result<String, String> {
        let response = self.agent.get(url).set("User-Agent", "YuiLauncher").call().map_err(|error| match error {
            ureq::Error::Status(code, _) => format!("チェックサムを取得できませんでした（{code}）。"),
            ureq::Error::Transport(_) => "チェックサムを取得できませんでした。".to_string(),
        })?;
        response.into_string().map_err(|_| "チェックサムを取得できませんでした。".to_string())
    }

    fn retry_after(&self) -> Option<Duration> {
        let until = (*self.limited_until.lock().unwrap())?;
        until.duration_since(SystemTime::now()).ok()
    }
}

impl GitHub {
    /// Searches the latest 30 releases through the API, which counts against its hourly limit
    /// unless the answer is unchanged since the cached one.
    fn latest_from_api(&self, app: &AppDef) -> Result<Option<Release>, String> {
        const NETWORK: &str = "更新を確認できませんでした。インターネット接続を確認してください。";
        let until = *self.limited_until.lock().unwrap();
        if until.is_some_and(|until| until > SystemTime::now()) {
            return Err(rate_limited_message(until));
        }
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
            ureq::Error::Status(403 | 429, response) => {
                let until = limit_lifts_at(&response);
                *self.limited_until.lock().unwrap() = until;
                rate_limited_message(until)
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
            self.cache.lock().unwrap().insert(url, (new_etag, trimmed(&releases)));
            self.save_cache();
        }
        Ok(picked)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::APPS;
    use crate::testutil::serve;
    use serde_json::json;

    fn pick_windows(releases: &Value, app: &AppDef) -> Option<Release> {
        pick_latest_release_for(releases, app, false)
    }

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
        release_of(
            tag,
            &[&format!("YuiVoiceChanger-Setup-{version}.exe"), &format!("YuiVoiceChanger-{version}-mac-arm64.dmg"), "SHA256SUMS.txt"],
            voice().repo,
        )
    }

    #[test]
    fn skips_a_newer_mac_only_release() {
        let picked = pick_windows(
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
        let picked = pick_windows(
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
        let picked = pick_windows(&json!([voice_release("v2026.103.1", "2026.103.1"), voice_release("v2026.1021.3", "2026.1021.3")]), voice());
        assert_eq!(picked.unwrap().version, "2026.1021.3");
    }

    #[test]
    fn finds_the_tracking_installer_among_mac_assets_and_versioned_checksums() {
        let tracking = &APPS[0];
        let picked = pick_windows(
            &json!([release_of(
                "v2026.1107.1",
                &["SHA256SUMS-2026.1107.1.txt", "YuiTracking-2026.1107.1-arm64-mac.zip", "YuiTracking-2026.1107.1-arm64.dmg", "YuiTracking-Setup-2026.1107.1-x64.exe"],
                tracking.repo,
            )]),
            tracking,
        )
        .unwrap();
        assert_eq!(picked.installer.name, "YuiTracking-Setup-2026.1107.1-x64.exe");
        assert_eq!(picked.checksums[0].name, "SHA256SUMS-2026.1107.1.txt");
    }

    #[test]
    fn livekit_skips_releases_from_its_former_name() {
        let live_kit = &APPS[2];
        let picked = pick_windows(
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
        assert!(pick_windows(&json!([release_of("v1.0.0", &["YuiVoiceChanger-Setup-1.0.0.exe"], voice().repo)]), voice()).is_none());
        let mut foreign = voice_release("v1.0.0", "1.0.0");
        foreign["assets"][0]["browser_download_url"] = json!("https://example.com/YuiVoiceChanger-Setup-1.0.0.exe");
        assert!(pick_windows(&json!([foreign]), voice()).is_none());
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
    fn says_when_the_limit_lifts_and_sends_nothing_until_then() {
        let reset = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() + 600;
        let refused = format!("HTTP/1.1 403 Forbidden\r\nX-RateLimit-Remaining: 0\r\nX-RateLimit-Reset: {reset}\r\nContent-Length: 0\r\n\r\n");
        let (root, requests) = serve(vec![refused.into_bytes()]);
        let client = GitHub::with_root(&root).unwrap();
        let message = client.latest(voice()).unwrap_err();
        assert!(message.starts_with(RATE_LIMITED), "{message}");
        assert!(message.contains("約10分後"), "{message}");
        let again = client.latest(&APPS[0]).unwrap_err();
        assert!(again.starts_with(RATE_LIMITED), "{again}");
        assert_eq!(requests.lock().unwrap().len(), 1, "no request while the limit holds");
        let wait = client.retry_after().unwrap();
        assert!(wait > Duration::from_secs(590) && wait <= Duration::from_secs(600), "{wait:?}");
    }

    #[test]
    fn keeps_answers_in_a_file_so_a_restart_asks_conditionally() {
        let temp = crate::testutil::temp_dir();
        let file = temp.join("cache").join("github-releases.json");
        let mut release = voice_release("v1.0.0", "1.0.0");
        release["uploader"] = json!({"login": "someone", "avatar_url": "https://example.com/a.png"});
        let body = json!([release]).to_string();
        let (root, requests) = serve(vec![
            format!("HTTP/1.1 200 OK\r\nETag: \"abc\"\r\nContent-Length: {}\r\n\r\n{body}", body.len()).into_bytes(),
            b"HTTP/1.1 304 Not Modified\r\nContent-Length: 0\r\n\r\n".to_vec(),
        ]);
        let first = GitHub::with_root(&root).unwrap().with_cache_file(file.clone());
        assert_eq!(first.latest(voice()).unwrap().unwrap().version, "1.0.0");
        let saved = std::fs::read_to_string(&file).unwrap();
        assert!(!saved.contains("someone"), "only what picking reads is saved");

        let restarted = GitHub::with_root(&root).unwrap().with_cache_file(file.clone());
        assert_eq!(restarted.latest(voice()).unwrap().unwrap().version, "1.0.0");
        assert!(requests.lock().unwrap()[1].to_lowercase().contains("if-none-match: \"abc\""));
    }

    #[test]
    fn a_broken_cache_file_is_an_empty_cache() {
        let temp = crate::testutil::temp_dir();
        let file = temp.join("github-releases.json");
        std::fs::write(&file, "{not json").unwrap();
        let client = GitHub::with_root("http://127.0.0.1:9").unwrap().with_cache_file(file);
        assert!(client.cache.lock().unwrap().is_empty());
    }

    fn web_client(web_root: &str) -> GitHub {
        // An API root that refuses connections shows any lookup that reaches it.
        GitHub::with_roots("http://127.0.0.1:9", web_root).unwrap()
    }

    #[test]
    fn finds_the_latest_release_without_the_api() {
        let (root, requests) = serve(vec![
            b"HTTP/1.1 302 Found\r\nLocation: https://github.com/suzunayui/YuiVoiceChanger/releases/tag/v2026.1108.3\r\nContent-Length: 0\r\n\r\n".to_vec(),
            b"HTTP/1.1 200 OK\r\nContent-Length: 4242\r\n\r\n".to_vec(),
        ]);
        let release = web_client(&root).latest(voice()).unwrap().unwrap();
        assert_eq!(release.version, "2026.1108.3");
        let installer = if crate::catalog::ON_MAC { "YuiVoiceChanger-2026.1108.3-mac-arm64.dmg" } else { "YuiVoiceChanger-Setup-2026.1108.3.exe" };
        assert_eq!(release.installer.name, installer);
        assert_eq!(release.installer.size, 4242);
        assert_eq!(release.installer.url, format!("https://github.com/suzunayui/YuiVoiceChanger/releases/download/v2026.1108.3/{installer}"));
        assert_eq!(release.page_url, "https://github.com/suzunayui/YuiVoiceChanger/releases/tag/v2026.1108.3");
        assert!(release.checksums.iter().any(|asset| asset.name == "SHA256SUMS.txt"));
        assert!(release.checksums.iter().all(|asset| asset.url.starts_with("https://github.com/suzunayui/YuiVoiceChanger/releases/download/v2026.1108.3/")));
        let requests = requests.lock().unwrap();
        assert!(requests[0].starts_with("HEAD /suzunayui/YuiVoiceChanger/releases/latest "), "{}", requests[0]);
        assert!(requests[1].starts_with(&format!("HEAD /suzunayui/YuiVoiceChanger/releases/download/v2026.1108.3/{installer} ")), "{}", requests[1]);
    }

    #[test]
    fn a_repository_without_releases_has_none() {
        let (root, _) = serve(vec![b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n".to_vec()]);
        assert!(web_client(&root).latest(voice()).unwrap().is_none());
    }

    #[test]
    fn searches_older_releases_through_the_api_when_the_latest_lacks_this_platform() {
        let (web, _) = serve(vec![
            b"HTTP/1.1 302 Found\r\nLocation: https://github.com/suzunayui/YuiVoiceChanger/releases/tag/v2026.1108.3\r\nContent-Length: 0\r\n\r\n".to_vec(),
            b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n".to_vec(),
        ]);
        let body = json!([voice_release("v2026.1103.34", "2026.1103.34")]).to_string();
        let (api, api_requests) = serve(vec![format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{body}", body.len()).into_bytes()]);
        let client = GitHub::with_roots(api.trim_end_matches('/'), &web).unwrap();
        assert_eq!(client.latest(voice()).unwrap().unwrap().version, "2026.1103.34");
        assert_eq!(api_requests.lock().unwrap().len(), 1);
    }

    #[test]
    fn a_tag_that_is_not_a_version_goes_to_the_api() {
        let (web, _) = serve(vec![b"HTTP/1.1 302 Found\r\nLocation: https://github.com/suzunayui/YuiLiveKit/releases/tag/YouTubeOverlay-2026.10.10-18\r\nContent-Length: 0\r\n\r\n".to_vec()]);
        let body = json!([release_of("v2026.1107.1", &["YuiLiveKit-Setup-2026.1107.1.exe", "SHA256SUMS.txt"], APPS[2].repo)]).to_string();
        let (api, _) = serve(vec![format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{body}", body.len()).into_bytes()]);
        let client = GitHub::with_roots(api.trim_end_matches('/'), &web).unwrap();
        let picked = client.latest(&APPS[2]).unwrap();
        if crate::catalog::ON_MAC {
            assert!(picked.is_none(), "no Mac version");
        } else {
            assert_eq!(picked.unwrap().version, "2026.1107.1");
        }
    }

    #[test]
    fn explains_rate_limits_and_network_failures() {
        let (root, _) = serve(vec![b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\n\r\n".to_vec()]);
        assert!(GitHub::with_root(&root).unwrap().latest(voice()).unwrap_err().contains("上限"));
        let offline = GitHub::with_root("http://127.0.0.1:9").unwrap();
        assert!(offline.latest(voice()).unwrap_err().contains("インターネット"));
    }

    #[test]
    fn a_mac_picks_its_own_installer_and_every_checksum_file() {
        let tracking = &APPS[0];
        let picked = pick_latest_release_for(
            &json!([release_of(
                "v2026.1107.1",
                &["SHA256SUMS-2026.1107.1.txt", "YuiTracking-2026.1107.1-arm64-mac.zip", "YuiTracking-2026.1107.1-arm64.dmg", "YuiTracking-Setup-2026.1107.1-x64.exe"],
                tracking.repo,
            )]),
            tracking,
            true,
        )
        .unwrap();
        assert_eq!(picked.installer.name, "YuiTracking-2026.1107.1-arm64-mac.zip");

        let voice_mac = pick_latest_release_for(
            &json!([release_of(
                "v2026.1103.34",
                &["SHA256SUMS-mac-arm64.txt", "SHA256SUMS.txt", "YuiVoiceChanger-2026.1103.34-mac-arm64.dmg", "YuiVoiceChanger-Setup-2026.1103.34.exe"],
                voice().repo,
            )]),
            voice(),
            true,
        )
        .unwrap();
        assert_eq!(voice_mac.installer.name, "YuiVoiceChanger-2026.1103.34-mac-arm64.dmg");
        assert_eq!(voice_mac.checksums.len(), 2);
    }

    #[test]
    fn a_mac_skips_windows_only_releases_and_apps_without_a_mac_version() {
        let newer_windows_only = release_of("v2026.1107.7", &["YuiVoiceChanger-Setup-2026.1107.7.exe", "SHA256SUMS.txt"], voice().repo);
        let picked = pick_latest_release_for(&json!([newer_windows_only, voice_release("v2026.1103.34", "2026.1103.34")]), voice(), true);
        assert_eq!(picked.unwrap().version, "2026.1103.34");
        let live_kit = &APPS[2];
        let release = release_of("v2026.1107.1", &["YuiLiveKit-Setup-2026.1107.1.exe", "SHA256SUMS.txt"], live_kit.repo);
        assert!(pick_latest_release_for(&json!([release]), live_kit, true).is_none());
    }

    struct Texts(Vec<(&'static str, Result<String, String>)>);

    impl Releases for Texts {
        fn latest(&self, _: &AppDef) -> Result<Option<Release>, String> {
            Ok(None)
        }
        fn fetch_text(&self, url: &str) -> Result<String, String> {
            self.0.iter().find(|(name, _)| url.ends_with(name)).map(|(_, text)| text.clone()).unwrap()
        }
    }

    fn release_with_sums(names: &[&str]) -> Release {
        let asset = |name: &str| Asset { name: name.to_string(), size: 1, url: format!("https://github.com/x/releases/download/v1/{name}") };
        Release { version: "1".to_string(), page_url: String::new(), installer: asset("App.dmg"), checksums: names.iter().map(|name| asset(name)).collect() }
    }

    #[test]
    fn finds_the_hash_in_whichever_checksum_file_lists_the_installer() {
        let hash = "a".repeat(64);
        let texts = Texts(vec![
            ("SHA256SUMS.txt", Ok(format!("{}  App.exe
", "b".repeat(64)))),
            ("SHA256SUMS-mac.txt", Ok(format!("{hash}  App.dmg
"))),
        ]);
        let release = release_with_sums(&["SHA256SUMS.txt", "SHA256SUMS-mac.txt"]);
        assert_eq!(expected_checksum(&texts, &release), Ok(Some(hash)));

        let unlisted = Texts(vec![("SHA256SUMS.txt", Ok("nothing".to_string()))]);
        assert_eq!(expected_checksum(&unlisted, &release_with_sums(&["SHA256SUMS.txt"])), Ok(None));

        let failing = Texts(vec![("SHA256SUMS.txt", Err("down".to_string()))]);
        assert_eq!(expected_checksum(&failing, &release_with_sums(&["SHA256SUMS.txt"])), Err("down".to_string()));
    }
}

#[cfg(test)]
mod live {
    use super::*;

    /// Looks up every app on the real GitHub: cargo test live_ -- --ignored --nocapture
    #[test]
    #[ignore]
    fn live_latest_releases() {
        let github = GitHub::new().unwrap();
        for app in crate::catalog::platform_apps().into_iter().chain([&crate::catalog::LAUNCHER]) {
            match github.latest(app) {
                Ok(Some(release)) => {
                    let sums = expected_checksum(&github, &release);
                    eprintln!("{}: {} {} ({} bytes) checksum={:?}", app.id, release.version, release.installer.name, release.installer.size, sums.map(|hash| hash.is_some()));
                }
                other => eprintln!("{}: {other:?}", app.id),
            }
        }
    }
}
