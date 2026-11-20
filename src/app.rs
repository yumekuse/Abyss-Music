// App state: settings store, the instant-start cache, and the API the UI calls.
use crate::discord::Discord;
use crate::util::{self, friendly, log, Res};
use crate::yt::Yt;
use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tao::event_loop::EventLoopProxy;
use tokio::sync::OnceCell;

#[derive(Debug)]
pub enum UserEvent {
    Eval(String),
    /// messages from the page (window.ipc.postMessage): drag / minimize / close / mode:{...}
    Ipc(String),
    Focus,
    Quit,
}

type Shared = Arc<OnceCell<Res<Value>>>;
type BoxFut = Pin<Box<dyn Future<Output = Res<Value>> + Send>>;

pub struct App {
    pub yt: Yt,
    pub discord: Discord,
    pub downloads: crate::dl::Downloads,
    pub updater: crate::update::Updater,
    store: Mutex<Map<String, Value>>,
    store_dirty: AtomicBool,
    memo: Mutex<HashMap<String, (Instant, Shared)>>,
    served: Mutex<HashSet<String>>,
    pub proxy: Mutex<EventLoopProxy<UserEvent>>,
}

/// calls whose last answer is shown instantly on start, then refreshed in the background
const SWR: [&str; 7] = ["home", "moods", "personal", "library", "following", "account", "playlist:LM"];

impl App {
    pub fn new(proxy: EventLoopProxy<UserEvent>) -> Arc<Self> {
        let store = std::fs::read(util::data("settings.json")).ok().and_then(|b| serde_json::from_slice::<Map<String, Value>>(&b).ok()).unwrap_or_default();
        Arc::new(App {
            yt: Yt::new(),
            discord: Discord::new(),
            downloads: crate::dl::Downloads::new(),
            updater: crate::update::Updater::new(),
            store: Mutex::new(store),
            store_dirty: AtomicBool::new(false),
            memo: Mutex::new(HashMap::new()),
            served: Mutex::new(HashSet::new()),
            proxy: Mutex::new(proxy),
        })
    }
    pub fn send(&self, e: UserEvent) {
        let _ = self.proxy.lock().unwrap().send_event(e);
    }

    /* ---------------- settings.json ---------------- */
    pub fn store_json(&self) -> String {
        serde_json::to_string(&*self.store.lock().unwrap()).unwrap_or_else(|_| "{}".into())
    }
    pub fn store_get(&self, k: &str) -> Value {
        self.store.lock().unwrap().get(k).cloned().unwrap_or(Value::Null)
    }
    pub fn store_set_many(self: &Arc<Self>, m: Map<String, Value>) {
        {
            let mut s = self.store.lock().unwrap();
            for (k, v) in m {
                if v.is_null() {
                    s.remove(&k);
                } else {
                    s.insert(k, v);
                }
            }
        }
        if !self.store_dirty.swap(true, Ordering::SeqCst) {
            let app = self.clone();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(500)).await;
                app.store_flush();
            });
        }
    }
    pub fn store_set_now(&self, k: &str, v: Value) {
        self.store.lock().unwrap().insert(k.to_string(), v);
        self.store_dirty.store(true, Ordering::SeqCst);
    }
    pub fn store_flush(&self) {
        self.store_dirty.store(false, Ordering::SeqCst);
        let b = serde_json::to_vec(&*self.store.lock().unwrap()).unwrap_or_default();
        if let Err(x) = util::write_atomic(&util::data("settings.json"), &b) {
            log(format!("settings save failed: {x}"));
        }
    }

    /* ---------------- caches ---------------- */
    /// single-flight + short memory cache: concurrent and repeated calls share one request
    async fn memo(&self, key: &str, ttl: Duration, fut: BoxFut) -> Res<Value> {
        let cell = {
            let mut m = self.memo.lock().unwrap();
            m.retain(|_, (t, c)| t.elapsed() < ttl.max(Duration::from_secs(300)) || c.get().is_none());
            match m.get(key) {
                Some((t, c)) if t.elapsed() < ttl || c.get().is_none() => c.clone(),
                _ => {
                    let c: Shared = Arc::new(OnceCell::new());
                    m.insert(key.to_string(), (Instant::now(), c.clone()));
                    c
                }
            }
        };
        let r = cell.get_or_init(|| fut).await.clone();
        if r.is_err() {
            self.memo.lock().unwrap().remove(key);
        }
        r
    }
    fn disk_path(key: &str) -> PathBuf {
        let safe: String = key.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).take(120).collect();
        util::local_dir().join("cache").join(format!("{safe}.json"))
    }
    fn disk_get(key: &str) -> Option<Value> {
        serde_json::from_slice(&std::fs::read(Self::disk_path(key)).ok()?).ok()
    }
    fn disk_put(key: &str, v: &Value) {
        let _ = util::write_atomic(&Self::disk_path(key), &serde_json::to_vec(v).unwrap_or_default());
    }
    pub fn clear_caches(&self) {
        self.memo.lock().unwrap().clear();
        self.served.lock().unwrap().clear();
        let _ = std::fs::remove_dir_all(util::local_dir().join("cache"));
    }
    /// stale-while-revalidate: the first call of a session answers from disk at once
    async fn swr(self: &Arc<Self>, key: String, name: String, args: Vec<Value>) -> Res<Value> {
        let first = self.served.lock().unwrap().insert(key.clone());
        if first {
            if let Some(old) = Self::disk_get(&key) {
                let app = self.clone();
                let prev = old.clone();
                tokio::spawn(async move {
                    let f = Box::pin(call(app.clone(), name.clone(), args));
                    if let Ok(v) = app.memo(&key, Duration::from_secs(90), f).await {
                        if v != prev {
                            Self::disk_put(&key, &v);
                            app.send(UserEvent::Eval(format!("window.__abyssFresh&&window.__abyssFresh({})", json!(name))));
                        }
                    }
                });
                return Ok(old);
            }
        }
        let r = self.memo(&key, Duration::from_secs(90), Box::pin(call(self.clone(), name, args))).await;
        if let Ok(v) = &r {
            Self::disk_put(&key, v);
        }
        r
    }

    /// what the UI calls (window.api.*)
    pub async fn api(self: Arc<Self>, name: String, args: Vec<Value>) -> Result<Value, String> {
        let t0 = Instant::now();
        let r = match name.as_str() {
            "status" => {
                let signed = self.yt.auth.signed_in().await;
                let account = if signed { self.swr("account".into(), "account".into(), vec![]).await.unwrap_or(Value::Null) } else { Value::Null };
                Ok(json!({ "signedIn": signed, "account": account }))
            }
            "playlist" if args.first().and_then(Value::as_str).is_some_and(|x| x.trim_start_matches("VL") == "LM") => self.swr("playlist:LM".into(), "playlist:LM".into(), vec![]).await,
            n if SWR.contains(&n) => self.swr(n.to_string(), name.clone(), args).await,
            "genre" => {
                let key = format!("genre:{}:{}", args.first().and_then(Value::as_str).unwrap_or(""), args.get(1).and_then(Value::as_str).unwrap_or(""));
                self.swr(key, name.clone(), args).await
            }
            // pages: shared in memory for a few minutes so going back is instant
            "album" | "artist" | "playlist" | "related" => {
                let key = format!("{name}:{}", args.first().map(|x| x.to_string()).unwrap_or_default());
                self.memo(&key, Duration::from_secs(300), Box::pin(call(self.clone(), name.clone(), args))).await
            }
            _ => call(self.clone(), name.clone(), args).await,
        };
        let ms = t0.elapsed().as_millis();
        if let Err(x) = &r {
            log(format!("{name} failed: {x}"));
        } else if ms > 1500 {
            log(format!("{name} took {ms} ms"));
        }
        r.map_err(|x| friendly(&x))
    }

    /// load the sign-in and fetch what the first screen needs before the UI asks
    pub async fn warm(self: Arc<Self>) {
        if let Some(saved) = crate::auth::load() {
            *self.yt.auth.state.write().await = Some(saved);
        }
        log(format!("ready · signed in: {}", self.yt.auth.signed_in().await));
        let signed = self.yt.auth.signed_in().await;
        let mut jobs: Vec<(&str, &str)> = vec![("home", "home")];
        if signed {
            jobs.extend([("account", "account"), ("personal", "personal"), ("library", "library"), ("following", "following"), ("playlist:LM", "playlist:LM")]);
        }
        for (k, n) in jobs {
            // only warm what has no saved copy: saved copies are refreshed when the UI asks for them
            if Self::disk_get(k).is_none() {
                let app = self.clone();
                tokio::spawn(async move {
                    let _ = app.memo(k, Duration::from_secs(90), Box::pin(call(app.clone(), n.to_string(), vec![]))).await;
                });
            }
        }
    }
}

fn arg_s(a: &[Value], i: usize) -> String {
    a.get(i).and_then(Value::as_str).unwrap_or("").to_string()
}
fn arg_list(a: &[Value], i: usize) -> Vec<String> {
    a.get(i).and_then(Value::as_array).map(|x| x.iter().filter_map(|s| s.as_str().map(String::from)).collect()).unwrap_or_default()
}
fn call(app: Arc<App>, name: String, a: Vec<Value>) -> impl Future<Output = Res<Value>> + Send + 'static {
    async move {
        let yt = &app.yt;
        match name.as_str() {
            "account" => Ok(yt.account().await),
            "home" => yt.home().await,
            "moods" => yt.moods().await,
            "genre" => yt.genre(&arg_s(&a, 0), a.get(1).and_then(Value::as_str)).await,
            "search" => yt.search(&arg_s(&a, 0)).await,
            "suggest" => yt.suggest(&arg_s(&a, 0)).await,
            "upNext" => yt.up_next(&arg_s(&a, 0)).await,
            "related" => yt.related(&arg_s(&a, 0)).await,
            "album" => yt.album(&arg_s(&a, 0)).await,
            "artist" => yt.artist(&arg_s(&a, 0)).await,
            "playlist" => yt.playlist(&arg_s(&a, 0)).await,
            "playlist:LM" => yt.playlist("LM").await,
            "library" => yt.library().await,
            "personal" => yt.personal().await,
            "following" => yt.following().await,
            "lyrics" => crate::lyrics::lyrics(yt, a.first().unwrap_or(&Value::Null), a.get(1).and_then(Value::as_str)).await,
            "romanize" => Ok(json!(crate::romanize::romanize_lines(&yt.http, arg_list(&a, 0)).await)),
            "translate" => {
                let to = arg_s(&a, 1);
                crate::lyrics::translate(&yt.http, arg_list(&a, 0), if to.is_empty() { "en" } else { &to }).await
            }
            "discordConnect" => app.discord.connect(&arg_s(&a, 0)).await.map(Value::Bool),
            "discordOff" => {
                app.discord.off();
                Ok(json!(true))
            }
            "discordSet" => {
                app.discord.set(a.first().cloned().filter(|v| !v.is_null()));
                Ok(json!(true))
            }
            "streamInfo" => Ok(Value::Null),
            "like" => yt.like(&arg_s(&a, 0), a.get(1).and_then(Value::as_bool).unwrap_or(false)).await,
            "createPlaylist" => yt.create_playlist(&arg_s(&a, 0), arg_list(&a, 1)).await,
            "addToPlaylist" => yt.edit_playlist(&arg_s(&a, 0), arg_list(&a, 1), true).await,
            "removeFromPlaylist" => yt.edit_playlist(&arg_s(&a, 0), arg_list(&a, 1), false).await,
            "loginCode" => {
                let r = yt.login_code().await?;
                app.clear_caches();
                Ok(r)
            }
            "loginCancel" => {
                yt.auth.pending.fetch_add(1, Ordering::SeqCst);
                *yt.auth.code.lock().unwrap() = None;
                Ok(json!(true))
            }
            "loginCookie" => {
                let r = yt.login_cookie(&arg_s(&a, 0)).await?;
                app.clear_caches();
                Ok(r)
            }
            "logout" => {
                let r = yt.logout().await;
                app.clear_caches();
                Ok(r)
            }
            "openYT" => {
                let id = arg_s(&a, 0);
                if id.len() == 11 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
                    let _ = open::that_detached(format!("https://music.youtube.com/watch?v={id}"));
                }
                Ok(json!(true))
            }
            "openDevice" => {
                let u = arg_s(&a, 0);
                if u.starts_with("https://www.google.com/device") || u.starts_with("https://google.com/device") {
                    let _ = open::that_detached(u);
                }
                Ok(json!(true))
            }
            "downloads" => {
                let mut v: Vec<Value> = crate::dl::index().into_iter().map(|x| x.1).collect();
                v.sort_by_key(|x| std::cmp::Reverse(x["at"].as_u64().unwrap_or(0)));
                Ok(Value::Array(v))
            }
            // dlQueue(tracks, label, kind): kind is "song" | "album" | "playlist"
            "dlQueue" => {
                let tracks = a.first().and_then(Value::as_array).cloned().unwrap_or_default();
                crate::dl::Downloads::queue(&app, tracks, arg_s(&a, 1), arg_s(&a, 2))
            }
            "download" => crate::dl::Downloads::queue(&app, a.first().cloned().into_iter().collect(), a.first().map(|t| t["title"].as_str().unwrap_or("song").to_string()).unwrap_or_default(), "song".into()),
            "dlCancel" => {
                app.downloads.cancel(a.first().and_then(Value::as_u64).unwrap_or(0));
                Ok(json!(true))
            }
            "dlState" => Ok(app.downloads.state()),
            "updateState" => Ok(app.updater.state()),
            "updateCheck" => crate::update::Updater::check(&app).await,
            "updateInstall" => crate::update::Updater::install(&app).await,
            "news" => Ok(app.updater.news()),
            "newsRefresh" => crate::update::Updater::fetch_news(&app).await,
            "openUrl" => {
                let u = arg_s(&a, 0);
                if u.starts_with("https://") {
                    let _ = open::that_detached(u);
                }
                Ok(json!(true))
            }
            "removeDownload" => {
                let id = arg_s(&a, 0);
                let mut idx = crate::dl::index();
                if let Some(f) = idx.remove(&id) {
                    let _ = std::fs::remove_file(crate::dl::dir().join(f["file"].as_str().unwrap_or("")));
                    let _ = util::write_atomic(&crate::dl::dir().join("index.json"), &serde_json::to_vec_pretty(&idx).unwrap_or_default());
                }
                Ok(json!(true))
            }
            "openDownloads" => {
                let _ = std::fs::create_dir_all(crate::dl::dir());
                let _ = open::that_detached(crate::dl::dir());
                Ok(json!(""))
            }
            x => Err(format!("unknown call: {x}")),
        }
    }
}
