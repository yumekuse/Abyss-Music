// Sign-in: Google device code (the TV flow) or a pasted music.youtube.com cookie.
// Credentials are stored encrypted with Windows DPAPI (on Linux: a file only you can read). Logins saved by the old Electron
// build (Chromium os_crypt / safeStorage) are read once and moved over automatically.
use crate::util::{self, e, log, Res};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha1::{Digest, Sha1};
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::{Mutex, RwLock};

#[derive(Clone, Serialize, Deserialize, Default)]
pub struct ClientId {
    pub client_id: String,
    pub client_secret: String,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Tokens {
    pub access_token: String,
    pub refresh_token: String,
    pub expiry_date: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client: Option<ClientId>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Saved {
    Oauth { tokens: Tokens },
    Cookie { cookie: String },
}

pub struct Auth {
    pub state: RwLock<Option<Saved>>,
    refresh_lock: Mutex<()>,
    pub pending: AtomicU64,
    pub code: std::sync::Mutex<Option<Value>>,
}

pub const TV_UA: &str = "Mozilla/5.0 (ChromiumStylePlatform) Cobalt/Version";

impl Auth {
    pub fn new() -> Self {
        Auth { state: RwLock::new(None), refresh_lock: Mutex::new(()), pending: AtomicU64::new(0), code: std::sync::Mutex::new(None) }
    }
    pub async fn kind(&self) -> Option<&'static str> {
        match &*self.state.read().await {
            Some(Saved::Oauth { .. }) => Some("oauth"),
            Some(Saved::Cookie { .. }) => Some("cookie"),
            None => None,
        }
    }
    pub async fn signed_in(&self) -> bool {
        self.state.read().await.is_some()
    }
    pub async fn set(&self, s: Option<Saved>) {
        if let Some(x) = &s {
            save(x);
        }
        *self.state.write().await = s;
    }

    /// headers that sign a request in (refreshes the OAuth token when it ran out)
    pub async fn headers(&self, http: &reqwest::Client) -> Res<Vec<(&'static str, String)>> {
        let cur = self.state.read().await.clone();
        match cur {
            None => Err("sign in with google first".into()),
            Some(Saved::Cookie { cookie }) => {
                let mut h = vec![("Cookie", cookie.clone()), ("X-Goog-AuthUser", "0".into()), ("X-Origin", "https://www.youtube.com".into())];
                if let Some(sid) = cookie_val(&cookie, "SAPISID").or_else(|| cookie_val(&cookie, "__Secure-3PAPISID")) {
                    h.push(("Authorization", sid_auth(&sid)));
                }
                Ok(h)
            }
            Some(Saved::Oauth { tokens }) => {
                if iso_ms(&tokens.expiry_date) > util::now_ms() + 60_000 {
                    return Ok(vec![("Authorization", format!("Bearer {}", tokens.access_token))]);
                }
                let _g = self.refresh_lock.lock().await;
                // someone else may have refreshed while we waited
                if let Some(Saved::Oauth { tokens: t }) = &*self.state.read().await {
                    if iso_ms(&t.expiry_date) > util::now_ms() + 60_000 {
                        return Ok(vec![("Authorization", format!("Bearer {}", t.access_token))]);
                    }
                }
                let t = refresh(http, tokens).await?;
                let h = vec![("Authorization", format!("Bearer {}", t.access_token))];
                self.set(Some(Saved::Oauth { tokens: t })).await;
                Ok(h)
            }
        }
    }
}

fn cookie_val(c: &str, name: &str) -> Option<String> {
    c.split(';').find_map(|p| {
        let (k, v) = p.trim().split_once('=')?;
        (k == name).then(|| v.to_string())
    })
}
fn sid_auth(sid: &str) -> String {
    let ts = util::now_ms() / 1000;
    let h = Sha1::digest(format!("{ts} {sid} https://www.youtube.com").as_bytes());
    let hex: String = h.iter().map(|b| format!("{b:02x}")).collect();
    format!("SAPISIDHASH {ts}_{hex}")
}

/// pasted text can be a raw cookie string or copied request headers
pub fn parse_cookie(text: &str) -> Res<String> {
    let t = text.trim();
    let line = t.lines().find(|l| l.trim_start().to_lowercase().starts_with("cookie:"));
    let raw = line.unwrap_or(t);
    let mut c = raw.trim_start();
    if c.to_lowercase().starts_with("cookie:") {
        c = &c[7..];
    }
    let c = c.trim().trim_matches(|ch| ch == '"' || ch == '\'').trim().to_string();
    if cookie_val(&c, "SAPISID").is_none() && cookie_val(&c, "__Secure-3PAPISID").is_none() {
        return Err("that doesn't look like a signed-in youtube cookie (no SAPISID found)".into());
    }
    Ok(c)
}

/* ---------- ISO dates (tokens store expiry as an ISO string, like youtubei.js did) ---------- */
pub fn iso_ms(s: &str) -> u64 {
    // 2025-11-20T18:17:35.123Z
    let b = s.as_bytes();
    if b.len() < 19 {
        return 0;
    }
    let n = |a: usize, l: usize| s.get(a..a + l).and_then(|x| x.parse::<i64>().ok()).unwrap_or(0);
    let (y, m, d) = (n(0, 4), n(5, 2), n(8, 2));
    let (hh, mm, ss) = (n(11, 2), n(14, 2), n(17, 2));
    let ms = if b.get(19) == Some(&b'.') { n(20, 3) } else { 0 };
    let y2 = if m <= 2 { y - 1 } else { y };
    let era = y2.div_euclid(400);
    let yoe = y2 - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;
    ((days * 86400 + hh * 3600 + mm * 60 + ss) * 1000 + ms).max(0) as u64
}
pub fn ms_iso(ms: u64) -> String {
    let s = ms / 1000;
    let z = (s / 86400) as i64 + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    let r = s % 86400;
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{:03}Z", r / 3600, r % 3600 / 60, r % 60, ms % 1000)
}

/* ---------- OAuth (device code flow, same as youtubei.js / YouTube on TVs) ---------- */
async fn client_id(http: &reqwest::Client) -> Res<ClientId> {
    let cache = util::data("oauth-client.json");
    if let Ok(b) = std::fs::read(&cache) {
        if let Ok(c) = serde_json::from_slice::<ClientId>(&b) {
            if !c.client_id.is_empty() {
                return Ok(c);
            }
        }
    }
    let page = http
        .get("https://www.youtube.com/tv")
        .header("User-Agent", TV_UA)
        .header("Referer", "https://www.youtube.com/tv")
        .header("Accept-Language", "en-US")
        .send()
        .await
        .map_err(e)?
        .text()
        .await
        .map_err(e)?;
    let re = regex::Regex::new(r#"<script\s+id="base-js"\s+src="([^"]+)"[^>]*></script>"#).unwrap();
    let src = re.captures(&page).and_then(|c| c.get(1)).map(|m| m.as_str().to_string()).ok_or("could not find youtube's tv script")?;
    let url = if src.starts_with("http") { src } else { format!("https://www.youtube.com{src}") };
    let js = http.get(url).send().await.map_err(e)?.text().await.map_err(e)?;
    let re = regex::Regex::new(r#"clientId:"([^"]+)",[^"]*?:"([^"]+)""#).unwrap();
    let c = re.captures(&js).ok_or("could not obtain the sign-in client id")?;
    let id = ClientId { client_id: c[1].to_string(), client_secret: c[2].to_string() };
    let _ = util::write_atomic(&cache, serde_json::to_vec(&id).unwrap().as_slice());
    Ok(id)
}

async fn refresh(http: &reqwest::Client, mut t: Tokens) -> Res<Tokens> {
    let c = match &t.client {
        Some(c) if !c.client_id.is_empty() => c.clone(),
        _ => client_id(http).await?,
    };
    let r = http
        .post("https://www.youtube.com/o/oauth2/token")
        .json(&json!({ "client_id": c.client_id, "client_secret": c.client_secret, "refresh_token": t.refresh_token, "grant_type": "refresh_token" }))
        .send()
        .await
        .map_err(e)?;
    if !r.status().is_success() {
        return Err(format!("Failed to refresh access token: status code {}", r.status().as_u16()));
    }
    let j: Value = r.json().await.map_err(e)?;
    let at = j["access_token"].as_str().ok_or("Authorization server returned an error")?;
    t.access_token = at.to_string();
    t.expiry_date = ms_iso(util::now_ms() + j["expires_in"].as_u64().unwrap_or(3600) * 1000);
    t.client = Some(c);
    log("oauth token refreshed");
    Ok(t)
}

/// shows a code (on_code) and waits until the user approved it at google.com/device
pub async fn device_login(auth: &Auth, http: &reqwest::Client) -> Res<Tokens> {
    let ticket = auth.pending.fetch_add(1, Ordering::SeqCst) + 1;
    *auth.code.lock().unwrap() = None;
    let c = client_id(http).await?;
    let dev = uuid4();
    let r = http
        .post("https://www.youtube.com/o/oauth2/device/code")
        .json(&json!({ "client_id": c.client_id, "scope": "http://gdata.youtube.com https://www.googleapis.com/auth/youtube-paid-content", "device_id": dev, "device_model": "ytlr::" }))
        .send()
        .await
        .map_err(e)?;
    if !r.status().is_success() {
        return Err(format!("Failed to get device/user code: {}", r.status().as_u16()));
    }
    let d: Value = r.json().await.map_err(e)?;
    let device_code = d["device_code"].as_str().ok_or("no device code")?.to_string();
    let interval = d["interval"].as_u64().unwrap_or(5).max(1);
    *auth.code.lock().unwrap() = Some(json!({ "url": d["verification_url"], "code": d["user_code"], "expires": d["expires_in"] }));
    log("device code issued");
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(interval)).await;
        if auth.pending.load(Ordering::SeqCst) != ticket {
            return Err("cancelled".into());
        }
        let r = http
            .post("https://www.youtube.com/o/oauth2/token")
            .json(&json!({ "client_id": c.client_id, "client_secret": c.client_secret, "code": device_code, "grant_type": "http://oauth.net/grant_type/device/1.0" }))
            .send()
            .await;
        let Ok(r) = r else { continue };
        let j: Value = r.json().await.unwrap_or(Value::Null);
        if let Some(err) = j["error"].as_str() {
            match err {
                "authorization_pending" | "slow_down" => continue,
                "access_denied" => return Err("Access was denied.".into()),
                "expired_token" => return Err("The device code has expired.".into()),
                x => return Err(format!("sign-in failed ({x})")),
            }
        }
        if auth.pending.load(Ordering::SeqCst) != ticket {
            return Err("cancelled".into());
        }
        *auth.code.lock().unwrap() = None;
        return Ok(Tokens {
            access_token: j["access_token"].as_str().unwrap_or("").into(),
            refresh_token: j["refresh_token"].as_str().unwrap_or("").into(),
            expiry_date: ms_iso(util::now_ms() + j["expires_in"].as_u64().unwrap_or(3600) * 1000),
            client: Some(c),
        });
    }
}
fn uuid4() -> String {
    let mut b = [0u8; 16];
    let seed = util::now_ms() as u128 ^ ((std::process::id() as u128) << 64) ^ (&b as *const _ as u128);
    let mut x = seed;
    for v in b.iter_mut() {
        x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        *v = (x >> 64) as u8;
    }
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!("{}-{}-{}-{}-{}", &h[0..8], &h[8..12], &h[12..16], &h[16..20], &h[20..32])
}

/* ---------- storage ---------- */
/// our own file; the old Electron build's auth.bin is only ever read (so that app keeps working)
fn auth_file() -> std::path::PathBuf {
    util::data("auth-rs.bin")
}
fn old_auth_file() -> std::path::PathBuf {
    util::data("auth.bin")
}
pub fn save(s: &Saved) {
    let json = serde_json::to_vec(s).unwrap();
    let mut out = b"DPAPI1".to_vec();
    match dpapi(&json, true) {
        Some(enc) => out.extend(enc),
        None => out = json,
    }
    if let Err(x) = util::write_atomic(&auth_file(), &out) {
        log(format!("auth save failed: {x}"));
    }
    // Linux / macOS: no DPAPI, so the file is readable by you only
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(auth_file(), std::fs::Permissions::from_mode(0o600));
    }
}
pub fn remove() {
    let _ = std::fs::remove_file(auth_file());
    let _ = std::fs::remove_file(old_auth_file()); // signing out means signed out everywhere
}
pub fn load() -> Option<Saved> {
    let b = std::fs::read(auth_file()).or_else(|_| std::fs::read(old_auth_file())).ok()?;
    let new_format = b.starts_with(b"DPAPI1");
    let plain: Vec<u8> = if let Some(rest) = b.strip_prefix(b"DPAPI1") {
        dpapi(rest, false)?
    } else if b.starts_with(b"v10") || b.starts_with(b"v11") {
        // written by the Electron build (Chromium os_crypt)
        let p = chromium_decrypt(&b)?;
        log("moved sign-in over from the old app");
        p
    } else {
        b
    };
    let s = String::from_utf8(plain).ok()?;
    let t = s.trim();
    let saved = if t.starts_with('{') {
        let v: Value = serde_json::from_str(t).ok()?;
        match serde_json::from_value::<Saved>(v.clone()) {
            Ok(x) => x,
            Err(err) => {
                // shape only, never values
                let shape: Vec<String> = v.as_object().map(|o| o.iter().map(|(k, x)| format!("{k}:{}", if x.is_object() { x.as_object().unwrap().keys().cloned().collect::<Vec<_>>().join("/") } else { "v".into() })).collect()).unwrap_or_default();
                log(format!("saved sign-in not understood ({err}); fields {shape:?}"));
                return None;
            }
        }
    } else {
        Saved::Cookie { cookie: t.to_string() }
    };
    if !new_format {
        save(&saved); // re-store in the new format
    }
    Some(saved)
}

fn chromium_decrypt(b: &[u8]) -> Option<Vec<u8>> {
    use aes_gcm::aead::{Aead, KeyInit};
    use base64::Engine;
    let step = |m: &str| log(format!("old sign-in: {m}"));
    let Some(ls) = std::fs::read(util::data("Local State")).ok().and_then(|x| serde_json::from_slice::<Value>(&x).ok()) else {
        step("no Local State");
        return None;
    };
    let k64 = ls["os_crypt"]["encrypted_key"].as_str()?;
    let Ok(k) = base64::engine::general_purpose::STANDARD.decode(k64) else {
        step("key not base64");
        return None;
    };
    let Some(key) = k.strip_prefix(b"DPAPI").and_then(|x| dpapi(x, false)) else {
        step("key DPAPI decrypt failed");
        return None;
    };
    let Ok(cipher) = aes_gcm::Aes256Gcm::new_from_slice(&key) else {
        step(&format!("bad key length {}", key.len()));
        return None;
    };
    let (nonce, ct) = b[3..].split_at(12);
    match cipher.decrypt(nonce.try_into().ok()?, ct) {
        Ok(p) => Some(p),
        Err(_) => {
            step("AES-GCM decrypt failed");
            None
        }
    }
}

#[cfg(windows)]
fn dpapi(data: &[u8], encrypt: bool) -> Option<Vec<u8>> {
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{CRYPT_INTEGER_BLOB, CryptProtectData, CryptUnprotectData};
    unsafe {
        let input = CRYPT_INTEGER_BLOB { cbData: data.len() as u32, pbData: data.as_ptr() as *mut u8 };
        let mut out = CRYPT_INTEGER_BLOB { cbData: 0, pbData: std::ptr::null_mut() };
        let ok = if encrypt {
            CryptProtectData(&input, std::ptr::null(), std::ptr::null(), std::ptr::null(), std::ptr::null(), 0, &mut out)
        } else {
            CryptUnprotectData(&input, std::ptr::null_mut(), std::ptr::null(), std::ptr::null(), std::ptr::null(), 0, &mut out)
        };
        if ok == 0 || out.pbData.is_null() {
            return None;
        }
        let v = std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec();
        LocalFree(out.pbData as _);
        Some(v)
    }
}
#[cfg(not(windows))]
fn dpapi(data: &[u8], _encrypt: bool) -> Option<Vec<u8>> {
    Some(data.to_vec())
}
