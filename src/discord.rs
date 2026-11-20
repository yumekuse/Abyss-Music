// Discord Rich Presence: "Listening to abyss" with the current song.
// Runs on its own thread (Discord's IPC pipe is blocking); reconnects quietly every 15 s.
use crate::util::log;
use discord_rich_presence::activity::{Activity, ActivityType, Assets, Button, Timestamps};
use discord_rich_presence::{DiscordIpc, DiscordIpcClient};
use serde_json::Value;
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::Mutex;
use std::time::Duration;

enum Msg {
    Connect(String, std::sync::mpsc::Sender<Result<(), String>>),
    Set(Option<Value>),
    Off,
}
pub struct Discord {
    tx: Mutex<Sender<Msg>>,
}

fn clip(s: &str, n: usize) -> String {
    let mut s: String = s.trim().to_string();
    while s.chars().count() < 2 {
        s.push(' ');
    }
    if s.chars().count() > n { format!("{}…", s.chars().take(n - 1).collect::<String>()) } else { s }
}

impl Discord {
    pub fn new() -> Self {
        let (tx, rx) = channel();
        std::thread::Builder::new().name("discord".into()).spawn(move || run(rx)).unwrap();
        Discord { tx: Mutex::new(tx) }
    }
    pub async fn connect(&self, id: &str) -> Result<bool, String> {
        if !(17..=20).contains(&id.len()) || !id.chars().all(|c| c.is_ascii_digit()) {
            return Err("bad discord application id".into());
        }
        let (rtx, rrx) = channel();
        self.tx.lock().unwrap().send(Msg::Connect(id.to_string(), rtx)).map_err(|e| e.to_string())?;
        tokio::task::spawn_blocking(move || rrx.recv_timeout(Duration::from_secs(10)))
            .await
            .map_err(|e| e.to_string())?
            .map_err(|_| "couldn't reach discord — is the discord app open?".to_string())?
            .map(|_| true)
    }
    pub fn set(&self, a: Option<Value>) {
        let _ = self.tx.lock().unwrap().send(Msg::Set(a));
    }
    pub fn off(&self) {
        let _ = self.tx.lock().unwrap().send(Msg::Off);
    }
}

fn run(rx: Receiver<Msg>) {
    let mut client: Option<DiscordIpcClient> = None;
    let mut app: Option<String> = None;
    let mut last: Option<Value> = None;
    let try_connect = |id: &str| -> Result<DiscordIpcClient, String> {
        let mut c = DiscordIpcClient::new(id);
        c.connect().map_err(|e| format!("couldn't reach discord — is the discord app open? ({e})"))?;
        Ok(c)
    };
    loop {
        match rx.recv_timeout(Duration::from_secs(15)) {
            Ok(Msg::Connect(id, reply)) => {
                if client.is_some() && app.as_deref() == Some(&id) {
                    let _ = reply.send(Ok(()));
                    continue;
                }
                if let Some(mut c) = client.take() {
                    let _ = c.close();
                }
                app = Some(id.clone());
                match try_connect(&id) {
                    Ok(c) => {
                        log("discord: ready");
                        client = Some(c);
                        if let Some(a) = &last {
                            push(&mut client, Some(a));
                        }
                        let _ = reply.send(Ok(()));
                    }
                    Err(e) => {
                        let _ = reply.send(Err(e));
                    }
                }
            }
            Ok(Msg::Set(a)) => {
                last = a.clone();
                push(&mut client, a.as_ref());
            }
            Ok(Msg::Off) => {
                app = None;
                last = None;
                if let Some(mut c) = client.take() {
                    let _ = c.clear_activity();
                    let _ = c.close();
                }
            }
            Err(RecvTimeoutError::Timeout) => {
                // reconnect quietly if discord restarted
                if client.is_none() {
                    if let Some(id) = &app {
                        if let Ok(c) = try_connect(id) {
                            client = Some(c);
                            if let Some(a) = &last {
                                push(&mut client, Some(a));
                            }
                        }
                    }
                }
            }
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
}

fn push(client: &mut Option<DiscordIpcClient>, a: Option<&Value>) {
    let Some(c) = client.as_mut() else { return };
    let r = match a {
        None => c.clear_activity(),
        Some(a) => {
            let id = a["id"].as_str().unwrap_or("");
            let title = clip(a["title"].as_str().unwrap_or("unknown song"), 128);
            let playing = a["playing"].as_bool().unwrap_or(false);
            let artist = a["artist"].as_str().unwrap_or("");
            let state = clip(&format!("{}{}", if playing { "" } else { "paused · " }, if artist.is_empty() { "abyss".into() } else { format!("by {artist}") }), 128);
            // discord accepts external https images up to 256 chars; long covers fall back to the video thumbnail
            let thumb = a["thumb"].as_str().filter(|t| t.starts_with("https://") && t.len() <= 256).map(String::from).or_else(|| (!id.is_empty()).then(|| format!("https://i.ytimg.com/vi/{id}/hqdefault.jpg")));
            let large_text = clip(a["album"].as_str().filter(|x| !x.is_empty()).or(a["title"].as_str()).unwrap_or("abyss"), 128);
            let url = format!("https://music.youtube.com/watch?v={id}");
            let mut assets = Assets::new().large_text(&large_text);
            if let Some(t) = &thumb {
                assets = assets.large_image(t);
            }
            let mut act = Activity::new().activity_type(ActivityType::Listening).details(&title).state(&state).assets(assets);
            if !id.is_empty() {
                act = act.buttons(vec![Button::new("listen on youtube music", &url)]);
            }
            let dur = a["dur"].as_f64().unwrap_or(0.0);
            if playing && dur > 0.0 {
                let now = crate::util::now_ms() as f64;
                let start = (now - a["pos"].as_f64().unwrap_or(0.0) * 1000.0) as i64;
                act = act.timestamps(Timestamps::new().start(start / 1000).end((start + (dur * 1000.0) as i64) / 1000));
            }
            c.set_activity(act)
        }
    };
    if let Err(e) = r {
        log(format!("discord: setActivity failed: {e}"));
        let _ = c.close();
        *client = None; // reconnect on the next tick
    }
}
