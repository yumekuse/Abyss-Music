// Small helpers for walking YouTube's JSON and for logging.
use serde_json::Value;
use std::io::Write;
use std::path::PathBuf;
use std::sync::OnceLock;

pub type Res<T> = Result<T, String>;

pub fn e<E: std::fmt::Display>(x: E) -> String {
    x.to_string()
}

/* ---------- data folders ---------- */
static DATA: OnceLock<PathBuf> = OnceLock::new();
/// home-relative XDG folder on Linux / macOS (e.g. ~/.config/abyss)
#[cfg(not(windows))]
fn xdg(var: &str, fallback: &str) -> PathBuf {
    std::env::var_os(var)
        .filter(|x| !x.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_else(|| ".".into())).join(fallback))
        .join("abyss")
}
/// settings, sign-in, downloads: %APPDATA%byss on Windows, ~/.config/abyss on Linux
pub fn data_dir() -> &'static PathBuf {
    DATA.get_or_init(|| {
        let p = std::env::var_os("ABYSS_USERDATA").map(PathBuf::from).unwrap_or_else(|| {
            #[cfg(windows)]
            return PathBuf::from(std::env::var_os("APPDATA").unwrap_or_else(|| ".".into())).join("abyss");
            #[cfg(not(windows))]
            return xdg("XDG_CONFIG_HOME", ".config");
        });
        let _ = std::fs::create_dir_all(&p);
        p
    })
}
pub fn data(name: &str) -> PathBuf {
    data_dir().join(name)
}
/// caches (pages, fonts, web view): %LOCALAPPDATA%byss on Windows, ~/.cache/abyss on Linux
pub fn local_dir() -> PathBuf {
    let base = std::env::var_os("ABYSS_USERDATA").map(|p| PathBuf::from(p).join("local")).unwrap_or_else(|| {
        #[cfg(windows)]
        return PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap_or_else(|| ".".into())).join("abyss");
        #[cfg(not(windows))]
        return xdg("XDG_CACHE_HOME", ".cache");
    });
    let _ = std::fs::create_dir_all(&base);
    base
}

/// atomic write: tmp file + rename
pub fn write_atomic(path: &PathBuf, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(d) = path.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)
}

/* ---------- log (abyss.log, rotated at 2 MB) ---------- */
pub fn log(msg: impl AsRef<str>) {
    let msg = msg.as_ref();
    #[cfg(debug_assertions)]
    eprintln!("[abyss] {msg}");
    let f = data("abyss.log");
    if std::fs::metadata(&f).map(|m| m.len() > 2_000_000).unwrap_or(false) {
        let _ = std::fs::rename(&f, data("abyss.log.old"));
    }
    if let Ok(mut h) = std::fs::OpenOptions::new().create(true).append(true).open(&f) {
        let _ = writeln!(h, "{} {}", now_iso(), msg);
    }
}
pub fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}
fn now_iso() -> String {
    // seconds since epoch -> UTC timestamp (no chrono needed)
    let s = now_ms() / 1000;
    let (days, rem) = (s / 86400, s % 86400);
    let (y, m, d) = civil(days as i64);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", rem / 3600, rem % 3600 / 60, rem % 60)
}
fn civil(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/* ---------- JSON walking ---------- */
/// follow a path of object keys / array indexes ("0", "1", ...)
pub fn at<'a>(v: &'a Value, path: &[&str]) -> &'a Value {
    let mut cur = v;
    for k in path {
        cur = match cur {
            Value::Object(m) => m.get(*k).unwrap_or(&Value::Null),
            Value::Array(a) => k.parse::<usize>().ok().and_then(|i| a.get(i)).unwrap_or(&Value::Null),
            _ => &Value::Null,
        };
    }
    cur
}
pub fn s<'a>(v: &'a Value, path: &[&str]) -> &'a str {
    at(v, path).as_str().unwrap_or("")
}
/// every value stored under `key`, anywhere inside `v` (like youtubei's contents_memo)
pub fn find_all<'a>(v: &'a Value, key: &str, out: &mut Vec<&'a Value>) {
    match v {
        Value::Object(m) => {
            for (k, x) in m {
                if k == key {
                    out.push(x);
                }
                find_all(x, key, out);
            }
        }
        Value::Array(a) => a.iter().for_each(|x| find_all(x, key, out)),
        _ => {}
    }
}
pub fn all<'a>(v: &'a Value, key: &str) -> Vec<&'a Value> {
    let mut o = Vec::new();
    find_all(v, key, &mut o);
    o
}
pub fn first<'a>(v: &'a Value, key: &str) -> Option<&'a Value> {
    match v {
        Value::Object(m) => {
            if let Some(x) = m.get(key) {
                return Some(x);
            }
            m.values().find_map(|x| first(x, key))
        }
        Value::Array(a) => a.iter().find_map(|x| first(x, key)),
        _ => None,
    }
}
/// YouTube text object -> String ({simpleText}, {runs:[{text}]}, {content}, or plain string)
pub fn text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Object(m) => {
            if let Some(Value::String(s)) = m.get("simpleText") {
                return s.clone();
            }
            if let Some(Value::Array(r)) = m.get("runs") {
                return r.iter().map(|x| x.get("text").and_then(|t| t.as_str()).unwrap_or("")).collect();
            }
            if let Some(Value::String(s)) = m.get("content") {
                return s.clone();
            }
            String::new()
        }
        _ => String::new(),
    }
}
pub fn runs(v: &Value) -> &[Value] {
    v.get("runs").and_then(|r| r.as_array()).map(|a| a.as_slice()).unwrap_or(&[])
}

/// biggest thumbnail inside a node, upscaled to 544px like the old app did
pub fn big(url: &str) -> String {
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    let re = RE.get_or_init(|| regex::Regex::new(r"(?i)=w\d+-h\d+(-[a-z0-9-]+)?$").unwrap());
    let u = re.replace(url, "=w544-h544-l90-rj").to_string();
    if let Some(rest) = u.strip_prefix("//") { format!("https://{rest}") } else { u }
}
pub fn best_thumb(list: &Value) -> Option<String> {
    let a = list.as_array()?;
    a.iter()
        .max_by_key(|t| t.get("width").and_then(|w| w.as_u64()).unwrap_or(0))
        .and_then(|t| t.get("url").and_then(|u| u.as_str()))
        .map(big)
}
pub fn thumb_of(node: &Value) -> Option<String> {
    for k in ["thumbnail", "thumbnailRenderer", "thumbnails", "foregroundThumbnail", "accountPhoto"] {
        if let Some(x) = node.get(k) {
            if x.is_array() {
                if let Some(t) = best_thumb(x) {
                    return Some(t);
                }
            }
            if let Some(list) = first(x, "thumbnails") {
                if let Some(t) = best_thumb(list) {
                    return Some(t);
                }
            }
        }
    }
    None
}

pub fn secs(t: &str) -> u64 {
    t.split(':').fold(0u64, |a, p| a * 60 + p.trim().parse::<u64>().unwrap_or(0))
}
pub fn is_dur(t: &str) -> bool {
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    RE.get_or_init(|| regex::Regex::new(r"^\d{1,2}(:\d{2}){1,2}$").unwrap()).is_match(t.trim())
}

/// turn internal errors into something a person can read (full error goes to abyss.log)
pub fn friendly(m: &str) -> String {
    let l = m.to_lowercase();
    let has = |xs: &[&str]| xs.iter().any(|x| l.contains(x));
    if has(&["status code 401", "status code 403", "unauthori", "forbidden"]) {
        return "youtube refused this — try signing out and in again".into();
    }
    if l.contains("status code 429") {
        return "youtube is busy, try again in a minute".into();
    }
    if has(&["error sending request", "dns", "connect", "timed out", "timeout", "network"]) {
        return "no connection to youtube — check your internet".into();
    }
    if l.contains("status code 5") {
        return "youtube is having trouble right now, try again soon".into();
    }
    if has(&["missing field", "invalid type", "expected value", "eof while parsing"]) {
        return "youtube sent something unexpected — please try again".into();
    }
    if m.chars().count() > 140 { format!("{}…", m.chars().take(140).collect::<String>()) } else { m.to_string() }
}
