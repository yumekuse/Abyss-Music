// Auto-update (GitHub Releases) and announcements (announcements.json in the repo).
//
// Updates:  checks https://api.github.com/repos/<REPO>/releases/latest a few seconds after start
//           and every 6 hours. Windows: downloads the new setup, checks its SHA-256, runs it
//           silently; the installer closes abyss, updates it and opens it again.
//           Linux AppImage: downloads the new AppImage, checks it, swaps it in place, restarts.
//           Other Linux installs (deb / AUR / tarball) get a notice with a link.
// News:     https://raw.githubusercontent.com/<REPO>/main/announcements.json, every 3 hours.
//           Shown as a banner; dismissed ones are remembered.
//
// For testing, ABYSS_UPDATE_URL and ABYSS_ANNOUNCE_URL point the app at other URLs.
use crate::app::{App, UserEvent};
use crate::util::{self, e, log, Res};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub const REPO: &str = "yumekuse/Abyss-Music";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
const UA: &str = concat!("abyss-music/", env!("CARGO_PKG_VERSION"), " (+https://github.com/yumekuse/Abyss-Music)");

fn release_url() -> String {
    std::env::var("ABYSS_UPDATE_URL").unwrap_or_else(|_| format!("https://api.github.com/repos/{REPO}/releases/latest"))
}
fn news_url() -> String {
    std::env::var("ABYSS_ANNOUNCE_URL").unwrap_or_else(|_| format!("https://raw.githubusercontent.com/{REPO}/main/announcements.json"))
}
pub fn platform() -> &'static str {
    if cfg!(windows) {
        "windows"
    } else if cfg!(target_os = "linux") {
        "linux"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else {
        "other"
    }
}

/// "v1.2.3" / "1.2.3-beta" → (1,2,3); unknown parts are 0
pub fn ver(s: &str) -> (u64, u64, u64) {
    let core = s.trim().trim_start_matches(['v', 'V']).split(['-', '+']).next().unwrap_or("");
    let mut p = core.split('.').map(|x| x.parse::<u64>().unwrap_or(0));
    (p.next().unwrap_or(0), p.next().unwrap_or(0), p.next().unwrap_or(0))
}
pub fn newer(a: &str, b: &str) -> bool {
    ver(a) > ver(b)
}

pub struct Updater {
    state: Mutex<Value>,
    busy: AtomicBool,
    news: Mutex<Value>,
}

impl Updater {
    pub fn new() -> Self {
        let news = std::fs::read(util::local_dir().join("news.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or(json!([]));
        Updater {
            state: Mutex::new(json!({ "current": VERSION, "platform": platform(), "phase": "idle", "latest": null, "error": null, "progress": 0, "canInstall": false })),
            busy: AtomicBool::new(false),
            news: Mutex::new(news),
        }
    }
    pub fn state(&self) -> Value {
        self.state.lock().unwrap().clone()
    }
    pub fn news(&self) -> Value {
        self.news.lock().unwrap().clone()
    }
    fn set(&self, app: &App, f: impl FnOnce(&mut Value)) {
        let st = {
            let mut s = self.state.lock().unwrap();
            f(&mut s);
            s.clone()
        };
        app.send(UserEvent::Eval(format!("window.__abyssUpdate&&window.__abyssUpdate({st})")));
    }

    /* ---------------- update check ---------------- */
    pub async fn check(app: &Arc<App>) -> Res<Value> {
        let u = &app.updater;
        let r = app.yt.http.get(release_url()).header("User-Agent", UA).header("Accept", "application/vnd.github+json").timeout(Duration::from_secs(15)).send().await.map_err(e)?;
        if r.status().as_u16() == 404 {
            // no release published yet (or the repo is still private)
            u.set(app, |s| {
                s["latest"] = Value::Null;
                s["error"] = Value::Null;
                s["checkedAt"] = json!(util::now_ms());
            });
            return Ok(u.state());
        }
        if !r.status().is_success() {
            let m = format!("update check failed (HTTP {})", r.status().as_u16());
            u.set(app, |s| s["error"] = json!(m));
            return Err(m);
        }
        let j: Value = r.json().await.map_err(e)?;
        let tag = j["tag_name"].as_str().unwrap_or("").to_string();
        let assets: Vec<Value> = j["assets"].as_array().cloned().unwrap_or_default();
        let pick = |pred: &dyn Fn(&str) -> bool| assets.iter().find(|a| pred(&a["name"].as_str().unwrap_or("").to_lowercase())).cloned();
        let appimage = std::env::var_os("APPIMAGE").is_some();
        let asset = match platform() {
            "windows" => pick(&|n| n.ends_with(".exe") && n.contains("setup")),
            "linux" if appimage => pick(&|n| n.ends_with(".appimage") && (n.contains("x86_64") || n.contains("amd64"))),
            _ => None,
        };
        let asset = asset.map(|a| {
            json!({
                "name": a["name"], "url": a["browser_download_url"], "size": a["size"],
                "sha256": a["digest"].as_str().and_then(|d| d.strip_prefix("sha256:")).map(String::from),
            })
        });
        let is_new = !tag.is_empty() && newer(&tag, VERSION);
        let latest = json!({
            "version": tag.trim_start_matches(['v', 'V']), "name": j["name"], "notes": j["body"].as_str().unwrap_or("").chars().take(6000).collect::<String>(),
            "url": j["html_url"], "published": j["published_at"], "asset": asset, "newer": is_new,
        });
        let can = is_new && latest["asset"].is_object();
        u.set(app, |s| {
            s["latest"] = latest;
            s["error"] = Value::Null;
            s["checkedAt"] = json!(util::now_ms());
            s["canInstall"] = json!(can);
        });
        if is_new {
            log(format!("update available: {tag} (this is {VERSION})"));
        }
        Ok(u.state())
    }

    /* ---------------- install ---------------- */
    pub async fn install(app: &Arc<App>) -> Res<Value> {
        let u = &app.updater;
        let st = u.state();
        let a = st["latest"]["asset"].clone();
        if !st["latest"]["newer"].as_bool().unwrap_or(false) || !a.is_object() {
            return Err("no update to install".into());
        }
        if u.busy.swap(true, Ordering::SeqCst) {
            return Err("already updating".into());
        }
        let r = Self::download_and_run(app, &a).await;
        if let Err(x) = &r {
            log(format!("update failed: {x}"));
            u.set(app, |s| {
                s["phase"] = json!("error");
                s["error"] = json!(x);
            });
            u.busy.store(false, Ordering::SeqCst);
        }
        r
    }
    async fn download_and_run(app: &Arc<App>, a: &Value) -> Res<Value> {
        let u = &app.updater;
        let url = a["url"].as_str().ok_or("bad download link")?;
        if !(url.starts_with("https://") || std::env::var_os("ABYSS_UPDATE_URL").is_some()) {
            return Err("refusing a non-https download".into());
        }
        let name = a["name"].as_str().unwrap_or("abyss-update").replace(['/', '\\'], "_");
        u.set(app, |s| {
            s["phase"] = json!("downloading");
            s["progress"] = json!(0);
            s["error"] = Value::Null;
        });
        let mut r = app.yt.http.get(url).header("User-Agent", UA).timeout(Duration::from_secs(600)).send().await.map_err(e)?;
        if !r.status().is_success() {
            return Err(format!("download failed (HTTP {})", r.status().as_u16()));
        }
        let total = r.content_length().or(a["size"].as_u64()).unwrap_or(0);
        let mut buf: Vec<u8> = Vec::with_capacity(total as usize);
        let mut last = 0u64;
        while let Some(chunk) = r.chunk().await.map_err(e)? {
            buf.extend_from_slice(&chunk);
            let pct = if total > 0 { buf.len() as u64 * 100 / total } else { 0 };
            if pct != last {
                last = pct;
                u.set(app, |s| s["progress"] = json!(pct));
            }
        }
        if let Some(want) = a["sha256"].as_str() {
            let got: String = Sha256::digest(&buf).iter().map(|b| format!("{b:02x}")).collect();
            if !got.eq_ignore_ascii_case(want) {
                return Err("the download was damaged (checksum mismatch) — try again".into());
            }
        } else if buf.len() < 1_000_000 {
            return Err("the download looks incomplete".into());
        }
        u.set(app, |s| {
            s["phase"] = json!("installing");
            s["progress"] = json!(100);
        });
        Self::run_new(app, &name, buf)?;
        Ok(u.state())
    }

    #[cfg(windows)]
    fn run_new(app: &Arc<App>, name: &str, buf: Vec<u8>) -> Res<()> {
        use std::os::windows::process::CommandExt;
        let dir = std::env::temp_dir().join("abyss-update");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join(name);
        std::fs::write(&path, &buf).map_err(e)?;
        log(format!("starting installer {}", path.display()));
        app.store_flush();
        // DETACHED_PROCESS: the installer outlives abyss; it closes it, updates, and opens it again
        std::process::Command::new(&path)
            .args(["/SILENT", "/SUPPRESSMSGBOXES", "/NORESTART", "/CLOSEAPPLICATIONS", "/MERGETASKS=!removeold", "/UPDATE=1"])
            .creation_flags(0x0000_0008)
            .spawn()
            .map_err(|x| format!("couldn't start the installer: {x}"))?;
        let a = app.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(1500));
            a.send(UserEvent::Quit);
        });
        Ok(())
    }
    #[cfg(not(windows))]
    fn run_new(app: &Arc<App>, _name: &str, buf: Vec<u8>) -> Res<()> {
        use std::os::unix::fs::PermissionsExt;
        let target = std::path::PathBuf::from(std::env::var_os("APPIMAGE").ok_or("only the AppImage can update itself")?);
        let tmp = target.with_extension("new");
        std::fs::write(&tmp, &buf).map_err(e)?;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755)).map_err(e)?;
        std::fs::rename(&tmp, &target).map_err(|x| format!("couldn't replace the AppImage ({x}) — is the folder writable?"))?;
        log("AppImage replaced, restarting");
        app.store_flush();
        // start the new one after this one has let go of its port
        std::process::Command::new("sh").arg("-c").arg("sleep 1.5; exec \"$0\"").arg(&target).spawn().map_err(e)?;
        let a = app.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(400));
            a.send(UserEvent::Quit);
        });
        Ok(())
    }

    /* ---------------- announcements ---------------- */
    pub async fn fetch_news(app: &Arc<App>) -> Res<Value> {
        let sep = if news_url().contains('?') { '&' } else { '?' };
        let r = app.yt.http.get(format!("{}{sep}t={}", news_url(), util::now_ms() / 60_000)).header("User-Agent", UA).timeout(Duration::from_secs(15)).send().await.map_err(e)?;
        if !r.status().is_success() {
            return Err(format!("announcements unavailable (HTTP {})", r.status().as_u16()));
        }
        let j: Value = serde_json::from_str(&r.text().await.map_err(e)?).map_err(|x| format!("announcements.json is not valid JSON: {x}"))?;
        let list = filter_news(&j);
        let _ = util::write_atomic(&util::local_dir().join("news.json"), &serde_json::to_vec(&list).unwrap_or_default());
        let changed = *app.updater.news.lock().unwrap() != list;
        *app.updater.news.lock().unwrap() = list.clone();
        if changed {
            app.send(UserEvent::Eval(format!("window.__abyssNews&&window.__abyssNews({list})")));
        }
        Ok(list)
    }

    /// checks at start, then every few hours
    pub async fn run(app: Arc<App>) {
        if std::env::var_os("ABYSS_NO_UPDATE").is_some() {
            return;
        }
        tokio::time::sleep(Duration::from_secs(4)).await;
        let mut n = 0u64;
        loop {
            if n % 2 == 0 {
                if let Err(x) = Self::check(&app).await {
                    log(x);
                }
            }
            if let Err(x) = Self::fetch_news(&app).await {
                log(x);
            }
            n += 1;
            tokio::time::sleep(Duration::from_secs(3 * 3600)).await;
        }
    }
}

fn today() -> String {
    crate::auth::ms_iso(util::now_ms())[..10].to_string()
}
/// keeps announcements meant for this version / platform / date; drops anything malformed
pub fn filter_news(j: &Value) -> Value {
    let list = j["announcements"].as_array().or(j.as_array()).cloned().unwrap_or_default();
    let day = today();
    let ok: Vec<Value> = list
        .into_iter()
        .filter_map(|a| {
            let id = a["id"].as_str()?.trim();
            let title = a["title"].as_str().unwrap_or("").trim();
            let body = a["body"].as_str().unwrap_or("").trim();
            if id.is_empty() || (title.is_empty() && body.is_empty()) {
                return None;
            }
            if let Some(min) = a["minVersion"].as_str() {
                if newer(min, VERSION) {
                    return None;
                }
            }
            if let Some(max) = a["maxVersion"].as_str() {
                if newer(VERSION, max) {
                    return None;
                }
            }
            if let Some(p) = a["platforms"].as_array() {
                if !p.iter().any(|x| x.as_str() == Some(platform())) {
                    return None;
                }
            }
            if a["start"].as_str().is_some_and(|s| s.get(..10).unwrap_or(s) > day.as_str()) {
                return None;
            }
            if a["end"].as_str().is_some_and(|s| s.get(..10).unwrap_or(s) < day.as_str()) {
                return None;
            }
            let link = a["link"]["url"].as_str().filter(|u| u.starts_with("https://")).map(|u| json!({ "url": u, "label": a["link"]["label"].as_str().unwrap_or("open") }));
            let level = match a["level"].as_str() {
                Some(l @ ("info" | "update" | "warning" | "important")) => l,
                _ => "info",
            };
            Some(json!({
                "id": id.chars().take(80).collect::<String>(), "title": title.chars().take(140).collect::<String>(), "body": body.chars().take(1200).collect::<String>(),
                "level": level, "link": link, "date": a["date"].as_str().or(a["start"].as_str()).unwrap_or(""), "dismissible": a["dismissible"].as_bool().unwrap_or(true),
            }))
        })
        .collect();
    Value::Array(ok)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn versions() {
        assert!(newer("v0.3.0", "0.2.9"));
        assert!(newer("1.0.0", "0.99.99"));
        assert!(!newer("0.2.0", "0.2.0"));
        assert!(!newer("v0.2.0-beta", "0.2.0"));
        assert!(newer("0.10.0", "0.9.0"));
    }
    #[test]
    fn news_filter() {
        let j = json!({ "announcements": [
            { "id": "a", "title": "hello", "body": "x" },
            { "id": "b", "title": "future", "start": "2999-01-01" },
            { "id": "c", "title": "old", "end": "2000-01-01" },
            { "id": "d", "title": "new app only", "minVersion": "99.0.0" },
            { "id": "e", "title": "elsewhere", "platforms": ["nowhere"] },
            { "id": "f", "title": "bad link", "link": { "url": "javascript:alert(1)" } },
            { "title": "no id" },
        ]});
        let v = filter_news(&j);
        let ids: Vec<&str> = v.as_array().unwrap().iter().map(|x| x["id"].as_str().unwrap()).collect();
        assert_eq!(ids, ["a", "f"]);
        assert!(v[1]["link"].is_null());
    }
}
