// YouTube Music over InnerTube (the same private API music.youtube.com uses).
// Public content goes out anonymously; library / likes / playlists use your sign-in.
use crate::auth::{self, Auth, Saved};
use crate::util::{self, all, at, best_thumb, e, first, is_dur, log, runs, s, secs, text, thumb_of, Res};
use futures::stream::{self, StreamExt};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex as StdMutex, OnceLock, RwLock};
use std::time::{Duration, Instant};
use tokio::sync::OnceCell;

const MUSIC_V: &str = "1.20250219.01.00";
const WEB_V: &str = "2.20260623.01.00";
const TV_V: &str = "7.20260311.12.00";
const DESKTOP_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";

/// content region (charts, home picks). ABYSS_REGION=JP etc. overrides the default US.
fn region() -> &'static str {
    static R: OnceLock<String> = OnceLock::new();
    R.get_or_init(|| std::env::var("ABYSS_REGION").ok().filter(|r| r.len() == 2).map(|r| r.to_uppercase()).unwrap_or_else(|| "US".into()))
}

#[derive(Clone, Copy, PartialEq)]
pub enum C {
    Music,
    Web,
    Tv,
}

#[derive(Clone)]
pub struct Stream {
    pub url: String,
    pub mime: String,
    pub size: u64,
}
const VISION_UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 15_7_3) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/26.0 Safari/605.1.15";

type Shared = Arc<OnceCell<Res<Value>>>;

pub struct Yt {
    pub http: reqwest::Client,
    visitor: RwLock<String>,
    pub auth: Auth,
    dl_vis: StdMutex<Option<(String, Instant)>>,
    music: StdMutex<HashMap<String, Value>>,
    music_dirty: std::sync::atomic::AtomicBool,
    next_cache: StdMutex<HashMap<String, (Instant, Shared)>>,
}

impl Yt {
    pub fn new() -> Self {
        let http = reqwest::Client::builder()
            .user_agent(DESKTOP_UA)
            .gzip(true)
            .brotli(true)
            .pool_idle_timeout(Duration::from_secs(90))
            .tcp_nodelay(true)
            .connect_timeout(Duration::from_secs(8))
            .timeout(Duration::from_secs(25))
            .build()
            .expect("http client");
        let visitor = std::fs::read_to_string(util::data("visitor.txt")).unwrap_or_default().trim().to_string();
        let music = std::fs::read(util::data("music-check.json"))
            .ok()
            .and_then(|b| serde_json::from_slice::<HashMap<String, Value>>(&b).ok())
            .unwrap_or_default();
        Yt {
            http,
            visitor: RwLock::new(visitor),
            auth: Auth::new(),
            dl_vis: StdMutex::new(None),
            music: StdMutex::new(music),
            music_dirty: false.into(),
            next_cache: StdMutex::new(HashMap::new()),
        }
    }

    /* ---------------- transport ---------------- */
    pub async fn call(&self, ep: &str, c: C, mut body: Value, signed: bool) -> Res<Value> {
        let vis = self.visitor.read().unwrap().clone();
        let (name, ver, id) = match c {
            C::Music => ("WEB_REMIX", MUSIC_V, "67"),
            C::Web => ("WEB", WEB_V, "1"),
            C::Tv => ("TVHTML5", TV_V, "7"),
        };
        let mut client = json!({ "clientName": name, "clientVersion": ver, "hl": "en", "gl": region() });
        if !vis.is_empty() {
            client["visitorData"] = json!(vis);
        }
        match c {
            C::Tv => client["userAgent"] = json!(auth::TV_UA),
            _ => {}
        }
        body["context"] = json!({ "client": client, "user": { "lockedSafetyMode": false } });
        if c == C::Music {
            body["isAudioOnly"] = json!(true);
        }
        let url = format!("https://www.youtube.com/youtubei/v1/{ep}?prettyPrint=false&alt=json");
        let mut rb = self
            .http
            .post(url)
            .header("Accept", "*/*")
            .header("Accept-Language", "*")
            .header("X-Goog-Visitor-Id", &vis)
            .header("X-Youtube-Client-Name", id)
            .header("X-Youtube-Client-Version", ver)
            .header("Origin", "https://www.youtube.com")
            .json(&body);
        if signed {
            for (k, v) in self.auth.headers(&self.http).await? {
                rb = rb.header(k, v);
            }
        }
        let r = rb.send().await.map_err(e)?;
        let st = r.status();
        if !st.is_success() {
            let t = r.text().await.unwrap_or_default();
            log(format!("{ep} ({name}) HTTP {} {}", st.as_u16(), t.chars().take(160).collect::<String>().replace('\n', " ")));
            return Err(format!("Request to {ep} failed with status code {}", st.as_u16()));
        }
        let j: Value = r.json().await.map_err(e)?;
        if vis.is_empty() {
            if let Some(v) = j["responseContext"]["visitorData"].as_str() {
                *self.visitor.write().unwrap() = v.to_string();
                let _ = std::fs::write(util::data("visitor.txt"), v);
            }
        }
        Ok(j)
    }
    async fn music(&self, ep: &str, body: Value) -> Res<Value> {
        self.call(ep, C::Music, body, false).await
    }
    async fn browse(&self, id: &str) -> Res<Value> {
        self.music("browse", json!({ "browseId": id })).await
    }

    /* ---------------- account / status ---------------- */
    pub async fn account(&self) -> Value {
        if !self.auth.signed_in().await {
            return Value::Null;
        }
        match self.call("account/accounts_list", C::Tv, json!({}), true).await {
            Ok(j) => {
                let item = first(&j, "accountItem").or_else(|| first(&j, "accountItemRenderer")).cloned().unwrap_or(Value::Null);
                let name = text(&item["accountName"]);
                json!({ "name": if name.is_empty() { "you".into() } else { name }, "photo": best_thumb(at(&item, &["accountPhoto", "thumbnails"])), "handle": text(&item["channelHandle"]) })
            }
            Err(_) => json!({ "name": "you", "photo": null, "handle": "" }),
        }
    }

    /* ---------------- home / explore / moods ---------------- */
    pub async fn home(&self) -> Res<Value> {
        let feed = async {
            let mut out = Vec::new();
            let j = self.browse("FEmusic_home").await?;
            let sl = first(&j, "sectionListRenderer").cloned().unwrap_or(Value::Null);
            out.extend(sections(&sl["contents"]));
            let mut tok = next_cont(&sl).map(String::from);
            for _ in 0..6 {
                let Some(t) = tok.take() else { break };
                match self.music("browse", json!({ "continuation": t })).await {
                    Ok(c) => {
                        let sc = &c["continuationContents"]["sectionListContinuation"];
                        out.extend(sections(&sc["contents"]));
                        tok = next_cont(sc).map(String::from);
                    }
                    Err(x) => {
                        log(format!("home continuation: {x}"));
                        break;
                    }
                }
            }
            Ok::<_, String>(out)
        };
        let explore = async {
            match self.browse("FEmusic_explore").await {
                Ok(j) => {
                    let re = regex::Regex::new("(?i)mood|genre").unwrap();
                    let sl = first(&j, "sectionListRenderer").cloned().unwrap_or(Value::Null);
                    sections(&sl["contents"]).into_iter().filter(|x| !re.is_match(x["title"].as_str().unwrap_or(""))).collect()
                }
                Err(x) => {
                    log(format!("explore: {x}"));
                    vec![]
                }
            }
        };
        let charts = async {
            match self.browse("FEmusic_charts").await {
                Ok(j) => {
                    let re = regex::Regex::new("(?i)trend|top").unwrap();
                    all(&j, "musicCarouselShelfRenderer")
                        .into_iter()
                        .map(|x| {
                            let mut sec = shelf(x);
                            let t = sec["title"].as_str().unwrap_or("").to_string();
                            if !re.is_match(&t) {
                                sec["title"] = json!(format!("charts · {t}"));
                            }
                            sec
                        })
                        .collect()
                }
                Err(x) => {
                    log(format!("charts: {x}"));
                    vec![]
                }
            }
        };
        let (feed, ex, ch) = tokio::join!(feed, explore, charts);
        let mut out = feed?;
        out.extend(ex);
        out.extend(ch);
        let mut seen = HashSet::new();
        Ok(Value::Array(
            out.into_iter()
                .filter(|x| x["items"].as_array().is_some_and(|a| !a.is_empty()) && seen.insert(x["title"].as_str().unwrap_or("").to_string()))
                .collect(),
        ))
    }

    pub async fn moods(&self) -> Res<Value> {
        let j = self.browse("FEmusic_moods_and_genres").await?;
        let list: Vec<Value> = all(&j, "musicNavigationButtonRenderer")
            .into_iter()
            .filter_map(|b| {
                let title = text(&b["buttonText"]);
                let ep = &b["clickCommand"]["browseEndpoint"];
                let id = ep["browseId"].as_str()?;
                if title.is_empty() {
                    return None;
                }
                let color = b["solid"]["leftStripeColor"].as_u64().map(|n| format!("#{:06x}", n & 0xffffff));
                Some(json!({ "title": title, "color": color, "browseId": id, "params": ep["params"] }))
            })
            .collect();
        Ok(Value::Array(list))
    }

    pub async fn genre(&self, id: &str, params: Option<&str>) -> Res<Value> {
        let mut body = json!({ "browseId": id });
        if let Some(p) = params {
            body["params"] = json!(p);
        }
        let j = self.music("browse", body).await?;
        let mut secs: Vec<Value> = all(&j, "musicCarouselShelfRenderer").into_iter().map(shelf).collect();
        secs.extend(all(&j, "gridRenderer").into_iter().map(|g| {
            let t = text(&g["header"]["gridHeaderRenderer"]["title"]);
            json!({ "title": if t.is_empty() { "playlists".into() } else { t }, "items": map_list(&g["items"]) })
        }));
        let secs: Vec<Value> = secs.into_iter().filter(|x| x["items"].as_array().is_some_and(|a| !a.is_empty())).collect();
        let thumb = secs.first().and_then(|x| x["items"].as_array()?.iter().find_map(|i| i["thumb"].as_str().map(String::from)));
        let title = first(&j, "musicHeaderRenderer").map(|h| text(&h["title"])).unwrap_or_default();
        Ok(json!({ "title": title, "sections": secs, "thumb": thumb }))
    }

    /* ---------------- search ---------------- */
    pub async fn search(&self, q: &str) -> Res<Value> {
        const KINDS: [(&str, &str, &str); 5] = [
            ("songs", "song", "EgWKAQIIAQ=="),
            ("artists", "artist", "EgWKAQIgAQ=="),
            ("albums", "album", "EgWKAQIYAQ=="),
            ("playlists", "playlist", "EgWKAQIoAQ=="),
            ("videos", "video", "EgWKAQIQAQ=="),
        ];
        let top = self.music("search", json!({ "query": q }));
        let typed = futures::future::join_all(KINDS.into_iter().map(|k| self.music("search", json!({ "query": q, "params": k.2 }))));
        let (top, typed) = tokio::join!(top, typed);
        let mut out = Vec::new();
        if let Some(card) = top.ok().as_ref().and_then(|j| first(j, "musicCardShelfRenderer")).and_then(top_card) {
            out.push(json!({ "title": "top result", "kind": "top", "items": [card] }));
        }
        for (i, r) in typed.into_iter().enumerate() {
            let Ok(j) = r else { continue };
            let mut items: Vec<Value> = all(&j, "musicShelfRenderer").into_iter().flat_map(|sh| map_list(&sh["contents"])).collect();
            if KINDS[i].1 == "video" {
                for x in items.iter_mut() {
                    x["type"] = json!("song");
                    x["video"] = json!(true);
                }
            }
            items.truncate(20);
            if !items.is_empty() {
                out.push(json!({ "title": KINDS[i].0, "kind": KINDS[i].1, "items": items }));
            }
        }
        Ok(Value::Array(out))
    }
    pub async fn suggest(&self, q: &str) -> Res<Value> {
        let Ok(j) = self.music("music/get_search_suggestions", json!({ "input": q })).await else { return Ok(json!([])) };
        let mut out: Vec<String> = Vec::new();
        for sec in all(&j, "searchSuggestionsSectionRenderer") {
            for c in sec["contents"].as_array().into_iter().flatten() {
                let t = if let Some(x) = c.get("searchSuggestionRenderer") {
                    text(&x["suggestion"])
                } else if let Some(x) = c.get("musicResponsiveListItemRenderer") {
                    text(at(x, &["flexColumns", "0", "musicResponsiveListItemFlexColumnRenderer", "text"]))
                } else {
                    String::new()
                };
                if !t.is_empty() && out.len() < 8 {
                    out.push(t);
                }
            }
        }
        Ok(json!(out))
    }

    /* ---------------- watch next: up next, related, lyrics tab (one shared request) ---------------- */
    async fn next(&self, id: &str) -> Res<Value> {
        let cell = {
            let mut m = self.next_cache.lock().unwrap();
            m.retain(|_, (t, _)| t.elapsed() < Duration::from_secs(600));
            m.entry(id.to_string()).or_insert_with(|| (Instant::now(), Arc::new(OnceCell::new()))).1.clone()
        };
        let r = cell
            .get_or_init(|| self.music("next", json!({ "videoId": id, "playlistId": format!("RDAMVM{id}"), "params": "wAEB" })))
            .await
            .clone();
        if r.is_err() {
            self.next_cache.lock().unwrap().remove(id);
        }
        r
    }
    pub async fn up_next(&self, id: &str) -> Res<Value> {
        let j = self.next(id).await?;
        let panel = first(&j, "playlistPanelRenderer").ok_or("Music queue was empty, the given id is probably invalid.")?;
        Ok(Value::Array(map_list(&panel["contents"])))
    }
    async fn tab_page(&self, id: &str, page: &str) -> Res<Value> {
        let j = self.next(id).await?;
        let tab = all(&j, "tabRenderer")
            .into_iter()
            .find(|t| s(t, &["endpoint", "browseEndpoint", "browseEndpointContextSupportedConfigs", "browseEndpointContextMusicConfig", "pageType"]) == page)
            .ok_or("Could not find target tab.")?;
        self.browse(s(tab, &["endpoint", "browseEndpoint", "browseId"])).await
    }
    pub async fn related(&self, id: &str) -> Res<Value> {
        let Ok(j) = self.tab_page(id, "MUSIC_PAGE_TYPE_TRACK_RELATED").await else { return Ok(json!([])) };
        let sl = first(&j, "sectionListRenderer").cloned().unwrap_or(Value::Null);
        Ok(Value::Array(sections(&sl["contents"]).into_iter().filter(|x| x["items"].as_array().is_some_and(|a| !a.is_empty())).collect()))
    }
    pub async fn yt_lyrics(&self, id: &str) -> Res<String> {
        let j = self.tab_page(id, "MUSIC_PAGE_TYPE_TRACK_LYRICS").await?;
        Ok(first(&j, "musicDescriptionShelfRenderer").map(|d| text(&d["description"])).unwrap_or_default())
    }

    /* ---------------- album / artist / playlist ---------------- */
    pub async fn album(&self, id: &str) -> Res<Value> {
        let j = self.browse(id).await?;
        let h = first(&j, "musicResponsiveHeaderRenderer").or_else(|| first(&j, "musicDetailHeaderRenderer")).cloned().unwrap_or(Value::Null);
        let thumb = thumb_of(&h).or_else(|| first(&j, "background").and_then(thumb_of));
        let title = text(&h["title"]);
        let sub = text(&h["subtitle"]);
        let mut artist = text(&h["straplineTextOne"]);
        if artist.is_empty() {
            artist = sub.split(" • ").nth(1).unwrap_or("").to_string();
        }
        let year = regex::Regex::new(r"\b(19|20)\d{2}\b").unwrap().find(&sub).map(|m| m.as_str().to_string()).unwrap_or_default();
        let tracks: Vec<Value> = all(&j, "musicShelfRenderer")
            .into_iter()
            .flat_map(|sh| map_list(&sh["contents"]))
            .map(|mut t| {
                fill(&mut t, "album", json!(title));
                fill(&mut t, "albumId", json!(id));
                fill(&mut t, "thumb", json!(thumb));
                fill(&mut t, "artist", json!(artist));
                t
            })
            .collect();
        Ok(json!({ "id": id, "title": title, "artist": artist, "sub": sub, "year": year, "thumb": thumb, "tracks": tracks }))
    }

    fn artist_head(j: &Value) -> Value {
        first(j, "musicImmersiveHeaderRenderer").or_else(|| first(j, "musicVisualHeaderRenderer")).cloned().unwrap_or(Value::Null)
    }
    pub async fn artist(&self, id: &str) -> Res<Value> {
        let j = self.browse(id).await?;
        let h = Self::artist_head(&j);
        let list = first(&j, "sectionListRenderer").map(|x| x["contents"].clone()).unwrap_or(Value::Null);
        let secs: Vec<Value> = sections(&list).into_iter().filter(|x| x["items"].as_array().is_some_and(|a| !a.is_empty())).collect();
        let mut thumbs: Vec<&Value> = first(&h["thumbnail"], "thumbnails").and_then(|t| t.as_array()).map(|a| a.iter().collect()).unwrap_or_default();
        thumbs.sort_by_key(|t| std::cmp::Reverse(t["width"].as_u64().unwrap_or(0)));
        let sb = first(&h, "subscribeButtonRenderer").cloned().unwrap_or(Value::Null);
        let sbt: String = ["subscriberCountText", "longSubscriberCountText", "shortSubscriberCountText", "unsubscribedButtonText", "subscribedButtonText", "subscriberCountWithSubscribeText"]
            .iter()
            .map(|k| text(&sb[*k]))
            .collect::<Vec<_>>()
            .join(" ");
        let subs = regex::Regex::new(r"(?i)\d[\d.,]*\s?[KMB]\b").unwrap().find(&sbt).map(|m| m.as_str().to_string()).unwrap_or_default();
        let name = text(&h["title"]);

        // all songs: the "Top songs" shelf links to a full playlist
        let mut songs: Vec<Value> = Vec::new();
        if let Some(sh) = list.as_array().and_then(|a| a.iter().find_map(|x| x.get("musicShelfRenderer"))) {
            let bid = s(sh, &["title", "runs", "0", "navigationEndpoint", "browseEndpoint", "browseId"]);
            let bid = if bid.is_empty() { s(sh, &["bottomEndpoint", "browseEndpoint", "browseId"]) } else { bid };
            if !bid.is_empty() {
                if let Ok(p) = self.browse(bid).await {
                    songs = all(&p, "musicPlaylistShelfRenderer").into_iter().flat_map(|x| map_list(&x["contents"])).take(100).collect();
                }
            }
        }
        if songs.is_empty() {
            songs = secs
                .iter()
                .find(|x| x["items"].as_array().is_some_and(|a| a.iter().all(|i| i["type"] == "song")))
                .and_then(|x| x["items"].as_array().cloned())
                .unwrap_or_default();
        }
        for t in songs.iter_mut() {
            fill(t, "artist", json!(name));
            fill(t, "artistId", json!(id));
        }
        let count = |re: &str| {
            let re = regex::Regex::new(re).unwrap();
            secs.iter().find(|x| re.is_match(x["title"].as_str().unwrap_or(""))).and_then(|x| x["items"].as_array().map(|a| a.len())).unwrap_or(0)
        };
        let stats = json!({ "songs": songs.len(), "albums": count("(?i)^albums"), "singles": count("(?i)singles"), "videos": count("(?i)^videos") });
        let thumb = if thumbs.is_empty() { thumb_of(&h) } else { thumbs[1.min(thumbs.len() - 1)]["url"].as_str().map(util::big) };
        Ok(json!({
            "id": id, "name": name, "sub": if subs.is_empty() { String::new() } else { format!("{subs} subscribers") }, "desc": text(&h["description"]),
            "thumb": thumb, "banner": thumbs.first().and_then(|t| t["url"].as_str()),
            "songs": songs, "sections": secs.into_iter().filter(|x| !x["title"].as_str().unwrap_or("").eq_ignore_ascii_case("top songs")).collect::<Vec<_>>(),
            "stats": stats,
        }))
    }

    async fn music_playlist(&self, id: &str, signed: bool) -> Res<Value> {
        let bare = id.trim_start_matches("VL");
        let j = self.call("browse", C::Music, json!({ "browseId": format!("VL{bare}") }), signed).await?;
        let h = first(&j, "musicResponsiveHeaderRenderer").or_else(|| first(&j, "musicDetailHeaderRenderer")).cloned().unwrap_or(Value::Null);
        let shelf = first(&j, "musicPlaylistShelfRenderer").cloned().unwrap_or(Value::Null);
        let mut items = map_list(&shelf["contents"]);
        let mut tok = cont_token(&shelf["contents"]).or_else(|| next_cont(&shelf).map(String::from));
        let mut guard = 0;
        while let Some(t) = tok.take() {
            guard += 1;
            if guard > 20 || items.len() >= 1000 {
                break;
            }
            let Ok(c) = self.call("browse", C::Music, json!({ "continuation": t }), signed).await else { break };
            let acts = all(&c, "appendContinuationItemsAction");
            if let Some(a) = acts.first() {
                items.extend(map_list(&a["continuationItems"]));
                tok = cont_token(&a["continuationItems"]);
            } else {
                let pc = &c["continuationContents"]["musicPlaylistShelfContinuation"];
                items.extend(map_list(&pc["contents"]));
                tok = next_cont(pc).map(String::from);
            }
        }
        let mut desc = text(&h["description"]);
        if desc.is_empty() {
            desc = first(&h["description"], "description").map(text).unwrap_or_default();
        }
        let title = text(&h["title"]);
        let mut sub = text(&h["subtitle"]);
        if sub.is_empty() {
            sub = text(&h["secondSubtitle"]);
        }
        Ok(json!({ "id": id, "title": if title.is_empty() { "playlist".into() } else { title }, "sub": sub, "desc": desc, "thumb": thumb_of(&h), "tracks": items }))
    }

    /// a playlist with only music in it (videos that aren't music are hidden and counted)
    pub async fn playlist(&self, id: &str) -> Res<Value> {
        let mut p = self.playlist_raw(id).await?;
        let tracks = p["tracks"].as_array().cloned().unwrap_or_default();
        let (kept, hidden) = self.music_filter(tracks).await;
        p["tracks"] = Value::Array(kept);
        if hidden > 0 {
            p["hidden"] = json!(hidden);
            let sub = p["sub"].as_str().unwrap_or("").to_string();
            let note = format!("{hidden} non-music video{} hidden", if hidden == 1 { "" } else { "s" });
            p["sub"] = json!(if sub.is_empty() { note } else { format!("{sub} · {note}") });
        }
        Ok(p)
    }
    async fn playlist_raw(&self, id: &str) -> Res<Value> {
        let bare = id.trim_start_matches("VL").to_string();
        if bare == "LM" || bare == "LL" {
            return self.liked().await;
        }
        let signed = self.auth.signed_in().await;
        match self.music_playlist(id, false).await {
            Ok(p) if !signed || p["tracks"].as_array().is_some_and(|a| !a.is_empty()) => Ok(p),
            Ok(_) | Err(_) if signed => {
                if self.auth.kind().await == Some("cookie") {
                    if let Ok(p) = self.music_playlist(id, true).await {
                        if p["tracks"].as_array().is_some_and(|a| !a.is_empty()) {
                            return Ok(p);
                        }
                    }
                }
                let own = self.library().await.ok().and_then(|l| l.as_array()?.iter().find(|p| p["id"] == bare.as_str()).cloned());
                let mut p = self.tv_playlist(&bare).await?;
                if let Some(o) = own {
                    p["title"] = o["title"].clone();
                    p["sub"] = o["sub"].clone();
                }
                Ok(p)
            }
            Ok(p) => Ok(p),
            Err(x) => Err(x),
        }
    }
    async fn liked(&self) -> Res<Value> {
        if !self.auth.signed_in().await {
            return Err("sign in to see your liked songs".into());
        }
        if self.auth.kind().await == Some("cookie") {
            if let Ok(mut p) = self.music_playlist("LM", true).await {
                if p["tracks"].as_array().is_some_and(|a| !a.is_empty()) {
                    p["title"] = json!("Liked Music");
                    return Ok(p);
                }
            }
        }
        let mut p = self.tv_playlist("LL").await?;
        let tracks = self.only_music(p["tracks"].as_array().cloned().unwrap_or_default()).await;
        p["tracks"] = Value::Array(tracks);
        p["id"] = json!("LM");
        p["title"] = json!("Liked Music");
        p["sub"] = json!("your music likes on youtube");
        Ok(p)
    }
    pub async fn library(&self) -> Res<Value> {
        if !self.auth.signed_in().await {
            return Err("not signed in".into());
        }
        if self.auth.kind().await == Some("cookie") {
            if let Ok(j) = self.call("browse", C::Music, json!({ "browseId": "FEmusic_liked_playlists" }), true).await {
                let mut list: Vec<Value> = all(&j, "musicTwoRowItemRenderer").into_iter().filter_map(two_row).collect();
                list.extend(all(&j, "musicResponsiveListItemRenderer").into_iter().filter_map(list_item));
                let re = regex::Regex::new("^(VL|PL)").unwrap();
                let list: Vec<Value> = list.into_iter().filter(|x| x["type"] == "playlist" || re.is_match(x["id"].as_str().unwrap_or(""))).collect();
                if !list.is_empty() {
                    return Ok(Value::Array(list));
                }
            }
        }
        let tiles = self.tv_all("FEplaylist_aggregation", 15).await?;
        Ok(Value::Array(
            tiles.iter().filter(|t| tile_type(t) == "PLAYLIST" && !["LL", "WL", "LM"].contains(&t["contentId"].as_str().unwrap_or(""))).map(tile_playlist).collect(),
        ))
    }

    /* ---------------- signed-in TV pages (the device-code login only works there) ---------------- */
    async fn tv_all(&self, browse_id: &str, max_pages: usize) -> Res<Vec<Value>> {
        let mut seen = HashSet::new();
        let mut tiles = Vec::new();
        let mut add = |list: Vec<&Value>, tiles: &mut Vec<Value>| {
            let mut n = 0;
            for t in list {
                let Some(id) = tile_id(t) else { continue };
                if seen.insert(id) {
                    tiles.push(t.clone());
                    n += 1;
                }
            }
            n
        };
        let r = self.call("browse", C::Tv, json!({ "browseId": browse_id }), true).await?;
        add(all(&r, "tileRenderer"), &mut tiles);
        let mut tok = raw_next_token(&r);
        let mut i = 1;
        while let Some(t) = tok.take() {
            if i >= max_pages {
                break;
            }
            i += 1;
            let Ok(r) = self.call("browse", C::Tv, json!({ "continuation": t }), true).await else { break };
            if add(all(&r, "tileRenderer"), &mut tiles) == 0 {
                break;
            }
            tok = raw_next_token(&r);
        }
        Ok(tiles)
    }
    async fn tv_playlist(&self, bare: &str) -> Res<Value> {
        let tiles = self.tv_all(&format!("VL{bare}"), 40).await?;
        let tracks: Vec<Value> = tiles.iter().filter_map(tile_track).collect();
        let thumb = tracks.first().map(|t| t["thumb"].clone()).unwrap_or(Value::Null);
        Ok(json!({ "id": bare, "title": "playlist", "sub": "from your youtube account", "desc": "", "thumb": thumb, "tracks": tracks }))
    }
    pub async fn following(&self) -> Res<Value> {
        if !self.auth.signed_in().await {
            return Ok(json!([]));
        }
        let re = regex::Regex::new(r"^UC[\w-]{22}$").unwrap();
        let mut ids: Vec<String> = Vec::new();
        let collect = |v: &Value, ids: &mut Vec<String>| {
            for b in all(v, "browseEndpoint") {
                if let Some(id) = b["browseId"].as_str() {
                    if re.is_match(id) && !ids.iter().any(|x| x == id) {
                        ids.push(id.to_string());
                    }
                }
            }
        };
        if let Ok(mut r) = self.call("browse", C::Tv, json!({ "browseId": "FEsubscriptions" }), true).await {
            collect(&r, &mut ids);
            for _ in 1..8 {
                let Some(t) = raw_next_token(&r) else { break };
                let Ok(n) = self.call("browse", C::Tv, json!({ "continuation": t }), true).await else { break };
                r = n;
                collect(&r, &mut ids);
            }
        }
        ids.truncate(150);
        // keep real music artists only (they have songs / albums / singles on youtube music)
        let has_music = regex::Regex::new("(?i)songs|albums|singles").unwrap();
        let found: Vec<Option<Value>> = stream::iter(ids.clone())
            .map(|id| {
                let has_music = &has_music;
                async move {
                    let j = self.browse(&id).await.ok()?;
                    let h = Self::artist_head(&j);
                    let name = text(&h["title"]);
                    let list = first(&j, "sectionListRenderer").map(|x| x["contents"].clone()).unwrap_or(Value::Null);
                    let ok = sections(&list).iter().any(|x| has_music.is_match(x["title"].as_str().unwrap_or("")));
                    (!name.is_empty() && ok).then(|| json!({ "type": "artist", "id": id, "title": name, "thumb": thumb_of(&h), "sub": "" }))
                }
            })
            .buffered(8)
            .collect()
            .await;
        Ok(Value::Array(found.into_iter().flatten().collect()))
    }
    pub async fn personal(&self) -> Res<Value> {
        if !self.auth.signed_in().await {
            return Ok(Value::Null);
        }
        let (m, h) = tokio::join!(self.tv_all("FEtopics_music", 3), self.tv_all("FEhistory", 4));
        let mut out = json!({ "picks": [], "mixes": [], "history": [], "feed": [] });
        if let Ok(m) = m {
            out["picks"] = Value::Array(self.only_music(m.iter().filter_map(tile_track).collect()).await);
            out["mixes"] = Value::Array(m.iter().filter(|t| tile_type(t) == "PLAYLIST").map(tile_playlist).collect());
        }
        if let Ok(h) = h {
            out["history"] = Value::Array(self.only_music(h.iter().filter_map(tile_track).collect()).await);
        }
        Ok(out)
    }

    /* ---- music-only: YouTube's own category ("Music") decides; results remembered on disk ---- */
    /// is it music? YouTube's own tag (song / official music video) decides; anything else
    /// (uploads, podcasts) is looked up by its YouTube category, remembered on disk
    async fn music_filter(&self, tracks: Vec<Value>) -> (Vec<Value>, usize) {
        let sure = |t: &Value| matches!(t["mvt"].as_str(), Some("MUSIC_VIDEO_TYPE_ATV" | "MUSIC_VIDEO_TYPE_OMV"));
        let podcast = |t: &Value| t["mvt"].as_str().is_some_and(|m| m.contains("PODCAST"));
        let unsure: Vec<String> = tracks.iter().filter(|t| !sure(t) && !podcast(t)).filter_map(|t| t["id"].as_str().map(String::from)).collect();
        self.check_music(unsure).await;
        let m = self.music.lock().unwrap();
        let total = tracks.len();
        let kept: Vec<Value> = tracks
            .into_iter()
            .filter(|t| sure(t) || (!podcast(t) && m.get(t["id"].as_str().unwrap_or("")).map(|x| x["music"] != false).unwrap_or(true)))
            .collect();
        let hidden = total - kept.len();
        (kept, hidden)
    }
    async fn check_music(&self, ids: Vec<String>) {
        let todo: Vec<String> = {
            let m = self.music.lock().unwrap();
            let mut seen = HashSet::new();
            ids.into_iter().filter(|id| !m.contains_key(id) && seen.insert(id.clone())).collect()
        };
        if !todo.is_empty() {
            stream::iter(todo.clone())
                .for_each_concurrent(16, |id| async move {
                    if let Ok(j) = self.call("player", C::Web, json!({ "videoId": &id }), false).await {
                        let vd = &j["videoDetails"];
                        let cat = s(&j, &["microformat", "playerMicroformatRenderer", "category"]);
                        if vd.is_object() || !cat.is_empty() {
                            let v = json!({ "music": cat == "Music", "ch": vd["channelId"], "author": vd["author"].as_str().unwrap_or(""), "dur": vd["lengthSeconds"].as_str().and_then(|x| x.parse::<u64>().ok()).unwrap_or(0) });
                            self.music.lock().unwrap().insert(id, v);
                        }
                    }
                })
                .await;
            self.save_music();
        }
    }
    async fn only_music(&self, tracks: Vec<Value>) -> Vec<Value> {
        self.check_music(tracks.iter().filter_map(|t| t["id"].as_str().map(String::from)).collect()).await;
        let m = self.music.lock().unwrap();
        let clean = |s: &str| {
            static RE: OnceLock<regex::Regex> = OnceLock::new();
            RE.get_or_init(|| regex::Regex::new(r"(?i)\s*-\s*Topic$|VEVO$|\s+Official$").unwrap()).replace_all(s, "").trim().to_string()
        };
        tracks
            .into_iter()
            .filter(|t| m.get(t["id"].as_str().unwrap_or("")).map(|x| x["music"] != false).unwrap_or(true))
            .map(|mut t| {
                let info = m.get(t["id"].as_str().unwrap_or("")).cloned().unwrap_or(Value::Null);
                let a = clean(info["author"].as_str().unwrap_or(""));
                let a = if a.is_empty() { clean(t["artist"].as_str().unwrap_or("")) } else { a };
                t["artist"] = json!(a);
                if t["artistId"].is_null() {
                    t["artistId"] = info["ch"].clone();
                }
                if t["dur"].as_u64().unwrap_or(0) == 0 {
                    t["dur"] = json!(info["dur"].as_u64().unwrap_or(0));
                }
                t
            })
            .collect()
    }
    fn save_music(&self) {
        if self.music_dirty.swap(true, std::sync::atomic::Ordering::SeqCst) {
            return;
        }
        let snap = serde_json::to_vec(&*self.music.lock().unwrap()).unwrap_or_default();
        self.music_dirty.store(false, std::sync::atomic::Ordering::SeqCst);
        let _ = util::write_atomic(&util::data("music-check.json"), &snap);
    }

    /* ---------------- actions (signed in) ---------------- */
    async fn act(&self, ep: &str, body: Value) -> Res<Value> {
        if !self.auth.signed_in().await {
            return Err("sign in with google first".into());
        }
        let (a, b) = if self.auth.kind().await == Some("cookie") { (C::Music, C::Web) } else { (C::Web, C::Tv) };
        match self.call(ep, a, body.clone(), true).await {
            Ok(v) => Ok(v),
            Err(_) => self.call(ep, b, body, true).await,
        }
    }
    pub async fn like(&self, id: &str, on: bool) -> Res<Value> {
        self.act(if on { "like/like" } else { "like/removelike" }, json!({ "target": { "videoId": id } })).await.map(|_| json!(true))
    }
    pub async fn create_playlist(&self, title: &str, ids: Vec<String>) -> Res<Value> {
        let r = self.act("playlist/create", json!({ "title": title, "videoIds": ids })).await?;
        Ok(json!({ "id": r["playlistId"] }))
    }
    pub async fn edit_playlist(&self, pid: &str, ids: Vec<String>, add: bool) -> Res<Value> {
        let id = pid.trim_start_matches("VL");
        let actions: Vec<Value> = ids
            .iter()
            .map(|v| if add { json!({ "action": "ACTION_ADD_VIDEO", "addedVideoId": v }) } else { json!({ "action": "ACTION_REMOVE_VIDEO_BY_VIDEO_ID", "removedVideoId": v }) })
            .collect();
        self.act("browse/edit_playlist", json!({ "playlistId": id, "actions": actions })).await.map(|_| json!(true))
    }

    /* ---------------- downloads ----------------
       The visionOS client with a visitor id from a normal watch page gets full-length audio
       URLs that need no extra token (iOS URLs stop after the first MB). */
    async fn dl_visitor(&self, id: &str, fresh: bool) -> Res<String> {
        if !fresh {
            if let Some((v, t)) = self.dl_vis.lock().unwrap().clone() {
                if t.elapsed() < Duration::from_secs(2 * 3600) {
                    return Ok(v);
                }
            }
        }
        let page = self
            .http
            .get(format!("https://www.youtube.com/watch?v={id}"))
            .header("Cookie", "PREF=hl=en&tz=UTC; SOCS=CAI")
            .header("Accept-Language", "en-us,en;q=0.5")
            .send()
            .await
            .map_err(e)?
            .text()
            .await
            .map_err(e)?;
        let v = regex::Regex::new(r#""VISITOR_DATA":"([^"]+)""#).unwrap().captures(&page).map(|c| c[1].to_string()).ok_or("youtube didn't give a download session")?;
        *self.dl_vis.lock().unwrap() = Some((v.clone(), Instant::now()));
        Ok(v)
    }
    async fn dl_format(&self, id: &str, fresh: bool) -> Res<Stream> {
        let vis = self.dl_visitor(id, fresh).await?;
        let body = json!({
            "context": { "client": { "clientName": "VISIONOS", "clientVersion": "1.02", "deviceMake": "Apple", "deviceModel": "RealityDevice17,1", "userAgent": VISION_UA,
                "osName": "visionOS", "osVersion": "26.5.23O471", "hl": "en", "timeZone": "UTC", "utcOffsetMinutes": 0, "visitorData": vis } },
            "videoId": id, "contentCheckOk": true, "racyCheckOk": true,
        });
        let j: Value = self
            .http
            .post("https://www.youtube.com/youtubei/v1/player?prettyPrint=false")
            .header("User-Agent", VISION_UA)
            .header("X-Youtube-Client-Name", "101")
            .header("X-Youtube-Client-Version", "1.02")
            .header("X-Goog-Visitor-Id", &vis)
            .header("Origin", "https://www.youtube.com")
            .json(&body)
            .send()
            .await
            .map_err(e)?
            .json()
            .await
            .map_err(e)?;
        let st = s(&j, &["playabilityStatus", "status"]);
        if st != "OK" {
            return Err(j["playabilityStatus"]["reason"].as_str().unwrap_or(if st.is_empty() { "not playable" } else { st }).to_string());
        }
        let f = j["streamingData"]["adaptiveFormats"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|f| f["mimeType"].as_str().is_some_and(|m| m.starts_with("audio/")) && f["url"].is_string())
            .max_by_key(|f| (f["mimeType"].as_str().unwrap_or("").starts_with("audio/mp4") as u8, f["bitrate"].as_u64().unwrap_or(0)))
            .ok_or("no downloadable audio for this song")?;
        Ok(Stream {
            url: f["url"].as_str().unwrap_or("").to_string(),
            mime: f["mimeType"].as_str().unwrap_or("audio/mp4").split(';').next().unwrap_or("audio/mp4").to_string(),
            size: f["contentLength"].as_str().and_then(|x| x.parse().ok()).unwrap_or(0),
        })
    }
    /// saves the song's audio to `dest`; `progress` gets 0..1. Returns (mime, bytes).
    pub async fn download_audio(&self, id: &str, dest: &std::path::Path, progress: impl Fn(f32), cancelled: impl Fn() -> bool) -> Res<(String, u64)> {
        use tokio::io::AsyncWriteExt;
        let mut f = match self.dl_format(id, false).await {
            Ok(f) => f,
            Err(_) => self.dl_format(id, true).await?,
        };
        if f.size == 0 {
            return Err("unknown file size".into());
        }
        let part = dest.with_extension("part");
        let mut out = tokio::fs::File::create(&part).await.map_err(e)?;
        let (mut pos, mut retried) = (0u64, false);
        while pos < f.size {
            if cancelled() {
                drop(out);
                let _ = tokio::fs::remove_file(&part).await;
                return Err("cancelled".into());
            }
            let end = (pos + (1 << 20) - 1).min(f.size - 1);
            let r = self.http.get(&f.url).header("Range", format!("bytes={pos}-{end}")).header("User-Agent", VISION_UA).send().await.map_err(e)?;
            if !r.status().is_success() {
                let code = r.status().as_u16();
                if !retried && (code == 403 || code == 410) {
                    retried = true;
                    f = self.dl_format(id, true).await?;
                    continue;
                }
                drop(out);
                let _ = tokio::fs::remove_file(&part).await;
                return Err(format!("download failed (HTTP {code})"));
            }
            let b = r.bytes().await.map_err(e)?;
            out.write_all(&b).await.map_err(e)?;
            pos += b.len() as u64;
            progress(pos as f32 / f.size as f32);
        }
        out.flush().await.map_err(e)?;
        drop(out);
        tokio::fs::rename(&part, dest).await.map_err(e)?;
        Ok((f.mime, f.size))
    }

    /* ---------------- sign in / out ---------------- */
    pub async fn login_code(&self) -> Res<Value> {
        let t = auth::device_login(&self.auth, &self.http).await?;
        self.auth.set(Some(Saved::Oauth { tokens: t })).await;
        log("device sign-in done");
        Ok(json!({ "ok": true, "signedIn": true, "account": self.account().await }))
    }
    pub async fn login_cookie(&self, text: &str) -> Res<Value> {
        let cookie = auth::parse_cookie(text)?;
        self.auth.set(Some(Saved::Cookie { cookie })).await;
        Ok(json!({ "ok": true, "signedIn": true, "account": self.account().await }))
    }
    pub async fn logout(&self) -> Value {
        self.auth.pending.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        auth::remove();
        *self.auth.state.write().await = None;
        json!({ "signedIn": false })
    }
}

/* ================= renderer mapping (raw JSON -> the app's item shape) ================= */
fn fill(t: &mut Value, k: &str, v: Value) {
    let empty = match &t[k] {
        Value::Null => true,
        Value::String(x) => x.is_empty(),
        _ => false,
    };
    if empty {
        t[k] = v;
    }
}
fn page_type(ep: &Value) -> &str {
    s(ep, &["browseEndpoint", "browseEndpointContextSupportedConfigs", "browseEndpointContextMusicConfig", "pageType"])
}
fn run_browse(r: &Value) -> &str {
    s(r, &["navigationEndpoint", "browseEndpoint", "browseId"])
}
fn artist_runs(rs: &[Value]) -> Vec<(String, String)> {
    rs.iter().filter(|r| run_browse(r).starts_with("UC")).map(|r| (r["text"].as_str().unwrap_or("").to_string(), run_browse(r).to_string())).collect()
}
fn album_run(rs: &[Value]) -> Option<(String, String)> {
    rs.iter().find(|r| run_browse(r).starts_with("MPR")).map(|r| (r["text"].as_str().unwrap_or("").to_string(), run_browse(r).to_string()))
}
/// first plain run that looks like a name (skips "Song", "•", durations, view counts)
fn name_run(rs: &[Value]) -> String {
    const SKIP: [&str; 10] = ["", "•", "Song", "Video", "Episode", "Single", "EP", "Album", "Playlist", "Artist"];
    rs.iter()
        .filter_map(|r| r["text"].as_str())
        .map(str::trim)
        .find(|t| !SKIP.contains(t) && !is_dur(t) && !t.ends_with(" views") && !t.ends_with(" plays") && !t.ends_with(" listeners"))
        .unwrap_or("")
        .to_string()
}
fn song(id: &str, title: String, arts: &[(String, String)], fallback: String, album: Option<(String, String)>, dur: u64, thumb: Option<String>) -> Value {
    let artist = if arts.is_empty() { fallback } else { arts.iter().map(|a| a.0.as_str()).collect::<Vec<_>>().join(", ") };
    json!({
        "type": "song", "id": id, "title": title, "artist": artist, "artistId": arts.first().map(|a| a.1.clone()),
        "album": album.as_ref().map(|a| a.0.clone()).unwrap_or_default(), "albumId": album.map(|a| a.1), "dur": dur,
        "thumb": thumb,
    })
}
fn kind_of(pt: &str, id: &str) -> Option<&'static str> {
    Some(match pt {
        "MUSIC_PAGE_TYPE_ALBUM" | "MUSIC_PAGE_TYPE_AUDIOBOOK" => "album",
        "MUSIC_PAGE_TYPE_PLAYLIST" => "playlist",
        "MUSIC_PAGE_TYPE_ARTIST" | "MUSIC_PAGE_TYPE_USER_CHANNEL" | "MUSIC_PAGE_TYPE_LIBRARY_ARTIST" => "artist",
        _ if id.starts_with("MPRE") => "album",
        _ if id.starts_with("VL") || id.starts_with("PL") || id.starts_with("RD") => "playlist",
        _ if id.starts_with("UC") => "artist",
        _ => return None,
    })
}
fn two_row(r: &Value) -> Option<Value> {
    let title = text(&r["title"]);
    let ep = &r["navigationEndpoint"];
    let sub_runs = runs(&r["subtitle"]);
    let arts = artist_runs(sub_runs);
    let thumb = thumb_of(r);
    let mut vid = s(ep, &["watchEndpoint", "videoId"]);
    if vid.is_empty() {
        vid = s(r, &["title", "runs", "0", "navigationEndpoint", "watchEndpoint", "videoId"]);
    }
    if !vid.is_empty() {
        let mut t = song(vid, title, &arts, name_run(sub_runs), album_run(sub_runs), 0, thumb);
        if let Some(m) = first(ep, "musicVideoType").and_then(Value::as_str) {
            t["mvt"] = json!(m);
        }
        return Some(t);
    }
    let id = s(ep, &["browseEndpoint", "browseId"]);
    let ty = kind_of(page_type(ep), id)?;
    let artist = if arts.is_empty() { name_run(sub_runs) } else { arts.iter().map(|a| a.0.as_str()).collect::<Vec<_>>().join(", ") };
    Some(json!({ "type": ty, "id": id, "title": title, "artist": artist, "artistId": arts.first().map(|a| a.1.clone()), "thumb": thumb, "sub": text(&r["subtitle"]) }))
}
fn list_item(r: &Value) -> Option<Value> {
    let cols: Vec<&Value> = r["flexColumns"].as_array().map(|a| a.iter().map(|c| &c["musicResponsiveListItemFlexColumnRenderer"]["text"]).collect()).unwrap_or_default();
    let c0 = cols.first().copied().unwrap_or(&Value::Null);
    let c1 = cols.get(1).copied().unwrap_or(&Value::Null);
    let c1r = runs(c1);
    let title = text(c0);
    let thumb = thumb_of(r);
    let ep = &r["navigationEndpoint"];
    let pt = page_type(ep);
    let bid = s(ep, &["browseEndpoint", "browseId"]);
    if !bid.is_empty() {
        if let Some(ty) = kind_of(pt, bid) {
            let arts = artist_runs(c1r);
            let artist = if arts.is_empty() { if ty == "artist" { String::new() } else { name_run(c1r) } } else { arts.iter().map(|a| a.0.as_str()).collect::<Vec<_>>().join(", ") };
            return Some(json!({ "type": ty, "id": bid, "title": title, "artist": artist, "artistId": arts.first().map(|a| a.1.clone()), "thumb": thumb, "sub": text(c1) }));
        }
    }
    let mut vid = s(r, &["playlistItemData", "videoId"]);
    if vid.is_empty() {
        vid = s(c0, &["runs", "0", "navigationEndpoint", "watchEndpoint", "videoId"]);
    }
    if vid.is_empty() {
        vid = first(&r["overlay"], "watchEndpoint").map(|w| w["videoId"].as_str().unwrap_or("")).unwrap_or("");
    }
    if vid.is_empty() {
        return None;
    }
    let arts = artist_runs(c1r);
    let album = album_run(c1r).or_else(|| cols.get(2).and_then(|c| album_run(runs(c))));
    let dur_txt = c1r
        .iter()
        .filter_map(|x| x["text"].as_str())
        .find(|t| is_dur(t))
        .map(String::from)
        .unwrap_or_else(|| text(at(r, &["fixedColumns", "0", "musicResponsiveListItemFixedColumnRenderer", "text"])));
    let mut t = song(vid, title, &arts, name_run(c1r), album, if is_dur(&dur_txt) { secs(&dur_txt) } else { 0 }, thumb);
    let mut mvt = s(c0, &["runs", "0", "navigationEndpoint", "watchEndpoint", "watchEndpointMusicSupportedConfigs", "watchEndpointMusicConfig", "musicVideoType"]);
    if mvt.is_empty() {
        mvt = first(&r["overlay"], "musicVideoType").and_then(Value::as_str).unwrap_or("");
    }
    if !mvt.is_empty() {
        t["mvt"] = json!(mvt);
    }
    Some(t)
}
fn panel(r: &Value) -> Option<Value> {
    let id = r["videoId"].as_str()?;
    let lb = runs(&r["longBylineText"]);
    let arts = artist_runs(lb);
    let mut fb = text(&r["shortBylineText"]);
    if fb.is_empty() {
        fb = name_run(lb);
    }
    Some(song(id, text(&r["title"]), &arts, fb, album_run(lb), secs(&text(&r["lengthText"])), best_thumb(&r["thumbnail"]["thumbnails"])))
}
pub fn map_item(n: &Value) -> Option<Value> {
    if let Some(r) = n.get("musicTwoRowItemRenderer") {
        return two_row(r);
    }
    if let Some(r) = n.get("musicResponsiveListItemRenderer") {
        return list_item(r);
    }
    if let Some(r) = n.get("playlistPanelVideoRenderer") {
        return panel(r);
    }
    if let Some(r) = n.get("playlistPanelVideoWrapperRenderer") {
        return map_item(&r["primaryRenderer"]);
    }
    None
}
pub fn map_list(arr: &Value) -> Vec<Value> {
    arr.as_array().map(|a| a.iter().filter_map(map_item).collect()).unwrap_or_default()
}
fn shelf(x: &Value) -> Value {
    let h = &x["header"]["musicCarouselShelfBasicHeaderRenderer"];
    json!({ "title": text(&h["title"]), "strap": text(&h["strapline"]), "items": map_list(&x["contents"]) })
}
/// shelves of a sectionList, in order
fn sections(list: &Value) -> Vec<Value> {
    let mut out = Vec::new();
    for n in list.as_array().into_iter().flatten() {
        if let Some(x) = n.get("musicCarouselShelfRenderer") {
            out.push(shelf(x));
        } else if let Some(x) = n.get("musicShelfRenderer") {
            out.push(json!({ "title": text(&x["title"]), "strap": "", "items": map_list(&x["contents"]) }));
        } else if let Some(x) = n.get("gridRenderer") {
            out.push(json!({ "title": text(&x["header"]["gridHeaderRenderer"]["title"]), "strap": "", "items": map_list(&x["items"]) }));
        }
    }
    out
}
fn next_cont(v: &Value) -> Option<&str> {
    let t = s(v, &["continuations", "0", "nextContinuationData", "continuation"]);
    (!t.is_empty()).then_some(t)
}
fn cont_token(items: &Value) -> Option<String> {
    items.as_array()?.iter().rev().find_map(|x| first(x.get("continuationItemRenderer")?, "token").and_then(|t| t.as_str()).map(String::from))
}
fn top_card(c: &Value) -> Option<Value> {
    let mut ep = &c["onTap"];
    if ep.is_null() {
        ep = at(c, &["title", "runs", "0", "navigationEndpoint"]);
    }
    let sub = text(&c["subtitle"]);
    let parts: Vec<&str> = sub.split(" • ").collect();
    let kind = parts.first().unwrap_or(&"").to_lowercase();
    let second = parts.get(1).copied().unwrap_or("");
    let title = text(&c["title"]);
    let thumb = thumb_of(c);
    if let Some(v) = ep["watchEndpoint"]["videoId"].as_str() {
        return Some(json!({ "type": "song", "id": v, "title": title, "artist": second, "album": "", "dur": 0, "thumb": thumb, "sub": sub }));
    }
    let id = ep["browseEndpoint"]["browseId"].as_str()?;
    let ty = if kind.starts_with("artist") {
        "artist"
    } else if kind.starts_with("album") || kind.starts_with("single") || kind.starts_with("ep") {
        "album"
    } else {
        "playlist"
    };
    Some(json!({ "type": ty, "id": id, "title": title, "artist": second, "thumb": thumb, "sub": sub }))
}

/* ---- TV tiles (raw tileRenderer objects) ---- */
fn tile_id(t: &Value) -> Option<String> {
    if let Some(id) = t["contentId"].as_str() {
        return Some(id.to_string());
    }
    first(&t["onSelectCommand"], "videoId").and_then(|v| v.as_str()).map(String::from)
}
fn raw_next_token(v: &Value) -> Option<String> {
    if let Some(n) = first(v, "nextContinuationData") {
        if let Some(c) = n["continuation"].as_str() {
            return Some(c.to_string());
        }
    }
    first(v, "continuationCommand").and_then(|c| c["token"].as_str()).map(String::from)
}
fn tile_type(t: &Value) -> &str {
    t["contentType"].as_str().unwrap_or("").trim_start_matches("TILE_CONTENT_TYPE_")
}
fn tile_lines(t: &Value) -> Vec<String> {
    let mut out = Vec::new();
    for l in at(t, &["metadata", "tileMetadataRenderer", "lines"]).as_array().into_iter().flatten() {
        for i in l["lineRenderer"]["items"].as_array().into_iter().flatten() {
            let x = text(&i["lineItemRenderer"]["text"]);
            if !x.is_empty() {
                out.push(x);
            }
        }
    }
    out
}
fn tile_thumb(t: &Value) -> Option<String> {
    best_thumb(at(t, &["header", "tileHeaderRenderer", "thumbnail", "thumbnails"]))
}
fn tile_track(t: &Value) -> Option<Value> {
    let ty = tile_type(t);
    if !ty.is_empty() && ty != "VIDEO" {
        return None;
    }
    let id = tile_id(t)?;
    let lines = tile_lines(t);
    let artist = lines.first().map(|l| l.trim_end_matches(" - Topic").split(" • ").next().unwrap_or("").to_string()).unwrap_or_default();
    let dur = all(&t["header"], "simpleText").into_iter().filter_map(|x| x.as_str()).find(|x| is_dur(x)).map(secs).unwrap_or(0);
    let thumb = tile_thumb(t).unwrap_or_else(|| format!("https://i.ytimg.com/vi/{id}/hqdefault.jpg"));
    Some(json!({ "type": "song", "id": id, "title": text(at(t, &["metadata", "tileMetadataRenderer", "title"])), "artist": artist, "artistId": null, "album": "", "albumId": null, "dur": dur, "thumb": thumb }))
}
fn tile_playlist(t: &Value) -> Value {
    json!({ "type": "playlist", "id": t["contentId"], "title": text(at(t, &["metadata", "tileMetadataRenderer", "title"])), "sub": tile_lines(t).join(" · "), "thumb": tile_thumb(t) })
}
