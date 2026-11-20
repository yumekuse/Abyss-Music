// Lyrics from several sources at once: the first SYNCED result in priority order wins,
// otherwise the first plain one. Order: LRCLIB, BetterLyrics, Unison, lrc.red, KuGou, YouTube Music.
use crate::util::{e, Res};
use crate::yt::Yt;
use base64::Engine;
use regex::Regex;
use serde_json::{json, Value};
use std::sync::OnceLock;
use std::time::Duration;

const UA: &str = "abyss-music-player/0.2 (personal desktop app)";
type Lines = Vec<(Option<f64>, String)>;
struct Found {
    synced: bool,
    lines: Lines,
}

fn re(p: &'static str, cell: &'static OnceLock<Regex>) -> &'static Regex {
    cell.get_or_init(|| Regex::new(p).unwrap())
}
fn clean_title(s: &str) -> String {
    static A: OnceLock<Regex> = OnceLock::new();
    static B: OnceLock<Regex> = OnceLock::new();
    let s = re(r"(?i)\s*[\(\[][^\)\]]*?(feat|ft\.|with|remaster|official|video|audio|lyrics|mv|m/v)[^\)\]]*?[\)\]]", &A).replace_all(s, "");
    re(r"(?i)\s+-\s+.*(remaster|version|edit).*$", &B).replace(&s, "").trim().to_string()
}
fn main_artist(s: &str) -> String {
    static A: OnceLock<Regex> = OnceLock::new();
    re(r"(?i),| & | x | feat\.? ", &A).split(s).next().unwrap_or("").trim().to_string()
}
fn dur_ok(a: f64, b: f64) -> bool {
    a == 0.0 || b == 0.0 || (a - b).abs() <= 8.0
}
fn credit(l: &str) -> bool {
    static A: OnceLock<Regex> = OnceLock::new();
    re(r"(?i)^(作词|作曲|编曲|词|曲|制作人|监制|混音|和声|吉他|贝斯|鼓|录音|Lyricist|Composer|Arranger|Producer)\s*[:：]", &A).is_match(l)
}

pub fn parse_lrc(lrc: &str) -> Lines {
    static T: OnceLock<Regex> = OnceLock::new();
    static ALL: OnceLock<Regex> = OnceLock::new();
    let tag = re(r"\[(\d+):(\d+(?:\.\d+)?)\]", &T);
    let any = re(r"\[[^\]]*\]", &ALL);
    let mut out: Lines = Vec::new();
    for line in lrc.lines() {
        let caps: Vec<_> = tag.captures_iter(line).collect();
        if caps.is_empty() {
            continue;
        }
        let t = any.replace_all(line, "").trim().to_string();
        let t = if t.is_empty() { "♪".to_string() } else { t };
        for c in caps {
            let s = c[1].parse::<f64>().unwrap_or(0.0) * 60.0 + c[2].parse::<f64>().unwrap_or(0.0);
            out.push((Some(s), t.clone()));
        }
    }
    out.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    out
}
fn plain(s: &str) -> Lines {
    s.lines().map(|l| (None, l.to_string())).collect()
}
fn ttml_time(s: &str) -> Option<f64> {
    let s = s.trim();
    if let Some(x) = s.strip_suffix('s') {
        return x.parse().ok();
    }
    let p: Vec<f64> = s.split(':').map(|x| x.parse().unwrap_or(0.0)).collect();
    Some(match p.len() {
        3 => p[0] * 3600.0 + p[1] * 60.0 + p[2],
        2 => p[0] * 60.0 + p[1],
        _ => *p.first()?,
    })
}
fn decode(s: &str) -> String {
    s.replace("&amp;", "&").replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&#39;", "'").replace("&apos;", "'")
}
fn parse_ttml(x: &str) -> Lines {
    static P: OnceLock<Regex> = OnceLock::new();
    static B: OnceLock<Regex> = OnceLock::new();
    static SP: OnceLock<Regex> = OnceLock::new();
    static TG: OnceLock<Regex> = OnceLock::new();
    static WS: OnceLock<Regex> = OnceLock::new();
    let p = re(r"(?s)<p\b([^>]*)>(.*?)</p>", &P);
    let begin = re(r#"\bbegin="([^"]+)""#, &B);
    let side = re(r#"(?s)<span[^>]*ttm:role="x-(bg|translation|roman)[^"]*"[^>]*>.*?</span>"#, &SP);
    let tags = re(r"<[^>]+>", &TG);
    let ws = re(r"\s+", &WS);
    let mut out = Vec::new();
    for m in p.captures_iter(x) {
        let b = begin.captures(&m[1]).and_then(|c| ttml_time(&c[1]));
        let body = side.replace_all(&m[2], "");
        let t = ws.replace_all(&decode(&tags.replace_all(&body, "")), " ").trim().to_string();
        if !t.is_empty() {
            out.push((b, t));
        }
    }
    out
}
/// find lyrics inside any JSON/text response: TTML, LRC, or plain text
fn extract(v: &Value) -> Option<Found> {
    match v {
        Value::Null => None,
        Value::String(s) => {
            if s.contains("<tt ") || s.contains("<tt>") {
                let l = parse_ttml(s);
                if !l.is_empty() {
                    return Some(Found { synced: l.iter().all(|x| x.0.is_some()), lines: l });
                }
            }
            static C: OnceLock<Regex> = OnceLock::new();
            if re(r"\[\d+:\d+", &C).find_iter(s).count() > 3 {
                return Some(Found { synced: true, lines: parse_lrc(s).into_iter().filter(|l| !credit(&l.1)).collect() });
            }
            if s.lines().count() > 4 && s.len() > 80 {
                return Some(Found { synced: false, lines: plain(s) });
            }
            None
        }
        Value::Array(a) => {
            if a.len() > 3 && a.iter().all(|o| o.is_object() && (o.get("text").is_some() || o.get("words").is_some() || o.get("line").is_some())) {
                let lines: Lines = a
                    .iter()
                    .map(|o| {
                        let t = if let Some(st) = o["startTime"].as_f64().or_else(|| o["startTime"].as_str().and_then(|x| x.parse().ok())) {
                            Some(if st > 10000.0 { st / 1000.0 } else { st })
                        } else {
                            o["time"].as_f64().or_else(|| o["start"].as_f64())
                        };
                        let tx = o.get("text").or(o.get("words")).or(o.get("line")).map(|x| x.as_str().map(String::from).unwrap_or_else(|| x.to_string())).unwrap_or_default();
                        (t, tx.trim().to_string())
                    })
                    .collect();
                return Some(Found { synced: lines.iter().all(|l| l.0.is_some_and(f64::is_finite)), lines });
            }
            a.iter().find_map(extract)
        }
        Value::Object(m) => {
            for k in ["syncedLyrics", "ttml", "lrc", "lyrics", "lines", "data", "plainLyrics", "content", "result"] {
                if let Some(x) = m.get(k) {
                    if let Some(r) = extract(x) {
                        return Some(r);
                    }
                }
            }
            m.values().find_map(extract)
        }
        _ => None,
    }
}

async fn get_j(http: &reqwest::Client, url: &str) -> Res<Value> {
    let r = http.get(url).header("User-Agent", UA).header("Accept", "application/json, text/plain, */*").timeout(Duration::from_secs(7)).send().await.map_err(e)?;
    if !r.status().is_success() {
        return Err(format!("HTTP {}", r.status().as_u16()));
    }
    r.json().await.map_err(e)
}
async fn get_t(http: &reqwest::Client, url: &str) -> Res<String> {
    let r = http.get(url).header("User-Agent", UA).timeout(Duration::from_secs(7)).send().await.map_err(e)?;
    if !r.status().is_success() {
        return Err(format!("HTTP {}", r.status().as_u16()));
    }
    r.text().await.map_err(e)
}
fn qs(p: &[(&str, String)]) -> String {
    p.iter().map(|(k, v)| format!("{k}={}", urlencoding::encode(v))).collect::<Vec<_>>().join("&")
}

struct T {
    id: String,
    title: String,
    artist: String,
    album: String,
    dur: f64,
}

async fn lrclib(h: &reqwest::Client, t: &T) -> Res<Option<Found>> {
    let mut p = vec![("track_name", clean_title(&t.title)), ("artist_name", main_artist(&t.artist))];
    if !t.album.is_empty() {
        p.push(("album_name", t.album.clone()));
    }
    if t.dur > 0.0 {
        p.push(("duration", format!("{}", t.dur.round())));
    }
    let mut j = get_j(h, &format!("https://lrclib.net/api/get?{}", qs(&p))).await.unwrap_or(Value::Null);
    if !(j["syncedLyrics"].is_string() || j["plainLyrics"].is_string()) {
        let list = get_j(h, &format!("https://lrclib.net/api/search?{}", qs(&p[..2]))).await?;
        let a = list.as_array().cloned().unwrap_or_default();
        let d = |x: &Value| dur_ok(x["duration"].as_f64().unwrap_or(0.0), t.dur);
        j = a.iter().find(|x| x["syncedLyrics"].is_string() && d(x)).or_else(|| a.iter().find(|x| x["plainLyrics"].is_string() && d(x))).cloned().unwrap_or(Value::Null);
    }
    if let Some(s) = j["syncedLyrics"].as_str() {
        return Ok(Some(Found { synced: true, lines: parse_lrc(s) }));
    }
    Ok(j["plainLyrics"].as_str().map(|s| Found { synced: false, lines: plain(s) }))
}
async fn better(h: &reqwest::Client, t: &T) -> Res<Option<Found>> {
    let mut p = vec![("s", clean_title(&t.title)), ("a", main_artist(&t.artist))];
    if t.dur > 0.0 {
        p.push(("d", format!("{}", t.dur.round())));
    }
    if !t.album.is_empty() {
        p.push(("al", t.album.clone()));
    }
    Ok(extract(&get_j(h, &format!("https://lyrics-api.boidu.dev/getLyrics?{}", qs(&p))).await?))
}
async fn unison(h: &reqwest::Client, t: &T) -> Res<Option<Found>> {
    let mut p = vec![("v", t.id.clone()), ("song", clean_title(&t.title)), ("artist", main_artist(&t.artist))];
    if !t.album.is_empty() {
        p.push(("album", t.album.clone()));
    }
    if t.dur > 0.0 {
        p.push(("duration", format!("{}", t.dur.round())));
    }
    Ok(extract(&get_j(h, &format!("https://unison.boidu.dev/lyrics?{}", qs(&p))).await?))
}
async fn lrcred(h: &reqwest::Client, t: &T) -> Res<Option<Found>> {
    let r = get_j(h, &format!("https://lrc.red/api/v1?{}", qs(&[("q", format!("{} {}", main_artist(&t.artist), clean_title(&t.title)))]))).await?;
    let want: String = clean_title(&t.title).to_lowercase().chars().take(12).collect();
    let hit = r["results"].as_array().into_iter().flatten().find(|x| {
        let d = x["duration"].as_f64().unwrap_or(0.0);
        dur_ok(if d > 10000.0 { d / 1000.0 } else { d }, t.dur) && x["track_name"].as_str().unwrap_or("").to_lowercase().contains(&want)
    });
    let Some(u) = hit.and_then(|x| x["lyricsUrl"].as_str()) else { return Ok(None) };
    let body = get_t(h, &format!("https://lrc.red{u}")).await?;
    let j = serde_json::from_str::<Value>(&body).unwrap_or(Value::String(body));
    Ok(extract(&j))
}
async fn kugou(h: &reqwest::Client, t: &T) -> Res<Option<Found>> {
    let s = get_j(
        h,
        &format!(
            "https://lyrics.kugou.com/search?{}",
            qs(&[("ver", "1".into()), ("man", "yes".into()), ("client", "pc".into()), ("keyword", format!("{} - {}", main_artist(&t.artist), clean_title(&t.title))), ("duration", format!("{}", (t.dur * 1000.0).round()))])
        ),
    )
    .await?;
    let mut c: Vec<&Value> = s["candidates"].as_array().into_iter().flatten().filter(|x| dur_ok(x["duration"].as_f64().unwrap_or(0.0) / 1000.0, t.dur)).collect();
    c.sort_by(|a, b| {
        let d = |x: &Value| (x["duration"].as_f64().unwrap_or(0.0) / 1000.0 - t.dur).abs();
        d(a).partial_cmp(&d(b)).unwrap_or(std::cmp::Ordering::Equal)
    });
    let Some(c) = c.first() else { return Ok(None) };
    let id = c["id"].as_str().map(String::from).unwrap_or_else(|| c["id"].to_string());
    let d = get_j(
        h,
        &format!(
            "https://lyrics.kugou.com/download?{}",
            qs(&[("ver", "1".into()), ("client", "pc".into()), ("id", id), ("accesskey", c["accesskey"].as_str().unwrap_or("").into()), ("fmt", "lrc".into()), ("charset", "utf8".into())])
        ),
    )
    .await?;
    let Some(b64) = d["content"].as_str() else { return Ok(None) };
    let raw = base64::engine::general_purpose::STANDARD.decode(b64).map_err(e)?;
    let lines: Lines = parse_lrc(&String::from_utf8_lossy(&raw)).into_iter().filter(|l| !credit(&l.1)).collect();
    Ok((lines.len() > 3).then_some(Found { synced: true, lines }))
}
async fn ytmusic(yt: &Yt, t: &T) -> Res<Option<Found>> {
    let body = yt.yt_lyrics(&t.id).await?;
    Ok((!body.is_empty()).then(|| Found { synced: false, lines: plain(&body) }))
}

pub async fn lyrics(yt: &Yt, t: &Value, prefer: Option<&str>) -> Res<Value> {
    let tr = T {
        id: t["id"].as_str().unwrap_or("").into(),
        title: t["title"].as_str().unwrap_or("").into(),
        artist: t["artist"].as_str().unwrap_or("").into(),
        album: t["album"].as_str().unwrap_or("").into(),
        dur: t["dur"].as_f64().unwrap_or(0.0),
    };
    let h = &yt.http;
    let (a, b, c, d, k, y) = tokio::join!(lrclib(h, &tr), better(h, &tr), unison(h, &tr), lrcred(h, &tr), kugou(h, &tr), ytmusic(yt, &tr));
    let mut res: Vec<(&str, Option<Found>)> =
        vec![("LRCLIB", a.ok().flatten()), ("BetterLyrics", b.ok().flatten()), ("Unison", c.ok().flatten()), ("lrc.red", d.ok().flatten()), ("KuGou", k.ok().flatten()), ("YouTube Music", y.ok().flatten())];
    if let Some(p) = prefer {
        if let Some(i) = res.iter().position(|r| r.0 == p) {
            let x = res.remove(i);
            res.insert(0, x);
        }
    }
    let found: Vec<(&str, Found)> = res.into_iter().filter_map(|(n, f)| f.filter(|f| !f.lines.is_empty()).map(|f| (n, f))).collect();
    let Some(best) = found.iter().position(|f| f.1.synced).or(if found.is_empty() { None } else { Some(0) }) else {
        return Ok(json!({ "synced": false, "source": null, "lines": [], "sources": [] }));
    };
    let sources: Vec<Value> = found.iter().map(|f| json!({ "name": f.0, "synced": f.1.synced })).collect();
    let (name, f) = &found[best];
    let lines: Vec<Value> = f.lines.iter().map(|l| json!([l.0, l.1])).collect();
    Ok(json!({ "source": name, "synced": f.synced, "lines": lines, "sources": sources }))
}

/* ---- translate, and Google's romanization (reads kanji correctly) ---- */
/// romanized lines from Google; lines joined with " ⏎ " so one request keeps line breaks
pub async fn google_roman(http: &reqwest::Client, lines: &[String], sl: &str) -> Option<Vec<String>> {
    let mut out = Vec::with_capacity(lines.len());
    for chunk in lines.chunks(40) {
        let q = chunk.iter().map(|l| l.replace('⏎', " ")).collect::<Vec<_>>().join(" ⏎ ");
        let url = format!("https://translate.googleapis.com/translate_a/single?client=gtx&sl={sl}&tl=en&dt=rm&q={}", urlencoding::encode(&q));
        let j: Value = http.get(url).timeout(Duration::from_secs(8)).send().await.ok()?.json().await.ok()?;
        let rm: String = j[0].as_array()?.iter().filter(|x| x[0].is_null()).filter_map(|x| x[3].as_str()).collect();
        let parts: Vec<String> = rm.split('⏎').map(|x| x.trim().to_lowercase()).collect();
        if parts.len() != chunk.len() {
            return None;
        }
        out.extend(parts);
    }
    Some(out)
}

pub async fn translate(http: &reqwest::Client, lines: Vec<String>, target: &str) -> Res<Value> {
    let tr = async {
        let mut out: Vec<String> = Vec::new();
        for chunk in lines.chunks(40) {
            let q = chunk.iter().map(|l| if l.is_empty() { " " } else { l.as_str() }).collect::<Vec<_>>().join("\n");
            let url = format!("https://translate.googleapis.com/translate_a/single?client=gtx&sl=auto&tl={}&dt=t&q={}", urlencoding::encode(target), urlencoding::encode(&q));
            let r = http.get(url).timeout(Duration::from_secs(10)).send().await.map_err(e)?;
            if !r.status().is_success() {
                return Err("translation failed".to_string());
            }
            let j: Value = r.json().await.map_err(e)?;
            let t: String = j[0].as_array().into_iter().flatten().filter_map(|x| x[0].as_str()).collect();
            let mut parts: Vec<String> = t.split('\n').map(|s| s.trim().to_string()).collect();
            parts.resize(chunk.len(), String::new());
            out.extend(parts);
        }
        Ok(out)
    };
    let needs_rom = lines.iter().any(|l| l.chars().any(|c| c.is_alphabetic() && !c.is_ascii()));
    let rom = async { if needs_rom { google_roman(http, &lines, "auto").await } else { None } };
    let (out, rom) = tokio::join!(tr, rom);
    let out = out?;
    let rom = rom.unwrap_or_else(|| vec![String::new(); lines.len()]);
    Ok(json!({ "lines": out, "roman": rom }))
}
