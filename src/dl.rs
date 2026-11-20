// Download queue: songs, albums and playlists, 3 songs at a time, with live progress sent to the UI.
use crate::app::{App, UserEvent};
use crate::util::{self, log, Res};
use futures::stream::{self, StreamExt};
use serde_json::{json, Map, Value};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

struct Cur {
    id: String,
    title: String,
    pct: f32,
}
struct Job {
    id: u64,
    label: String,
    kind: String,
    total: usize,
    done: usize,
    skipped: usize,
    failed: Vec<String>,
    current: Vec<Cur>,
    cancelled: Arc<AtomicBool>,
    finished: Option<Instant>,
}

pub struct Downloads {
    jobs: Mutex<Vec<Job>>,
    added: Mutex<Vec<Value>>,
    next: AtomicU64,
    dirty: AtomicBool,
    index_lock: Mutex<()>,
}

pub fn dir() -> std::path::PathBuf {
    util::data("downloads")
}
pub fn index() -> Map<String, Value> {
    std::fs::read(dir().join("index.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}
fn save_index(m: &Map<String, Value>) {
    let _ = util::write_atomic(&dir().join("index.json"), &serde_json::to_vec_pretty(m).unwrap_or_default());
}

impl Downloads {
    pub fn new() -> Self {
        Downloads { jobs: Mutex::new(Vec::new()), added: Mutex::new(Vec::new()), next: AtomicU64::new(1), dirty: AtomicBool::new(false), index_lock: Mutex::new(()) }
    }

    pub fn state(&self) -> Value {
        let jobs = self.jobs.lock().unwrap();
        let list: Vec<Value> = jobs
            .iter()
            .map(|j| {
                json!({
                    "id": j.id, "label": j.label, "kind": j.kind, "total": j.total, "done": j.done, "skipped": j.skipped,
                    "failed": j.failed.len(), "failedTitles": j.failed.iter().take(5).collect::<Vec<_>>(),
                    "cancelled": j.cancelled.load(Ordering::SeqCst), "finished": j.finished.is_some(),
                    "current": j.current.iter().map(|c| json!({ "id": c.id, "title": c.title, "pct": (c.pct * 100.0).round() })).collect::<Vec<_>>(),
                })
            })
            .collect();
        json!({ "jobs": list })
    }
    fn touch(&self) {
        self.dirty.store(true, Ordering::SeqCst);
    }
    pub fn cancel(&self, id: u64) {
        for j in self.jobs.lock().unwrap().iter() {
            if j.id == id || id == 0 {
                j.cancelled.store(true, Ordering::SeqCst);
            }
        }
        self.touch();
    }

    /// queue songs; returns the job id right away
    pub fn queue(app: &Arc<App>, tracks: Vec<Value>, label: String, kind: String) -> Res<Value> {
        let tracks: Vec<Value> = tracks.into_iter().filter(|t| t["id"].as_str().is_some_and(|x| !x.is_empty())).collect();
        if tracks.is_empty() {
            return Err("nothing to download".into());
        }
        let d = &app.downloads;
        let id = d.next.fetch_add(1, Ordering::SeqCst);
        let cancelled = Arc::new(AtomicBool::new(false));
        d.jobs.lock().unwrap().push(Job { id, label, kind, total: tracks.len(), done: 0, skipped: 0, failed: vec![], current: vec![], cancelled: cancelled.clone(), finished: None });
        d.touch();
        let app2 = app.clone();
        tokio::spawn(async move {
            stream::iter(tracks)
                .for_each_concurrent(3, |t| {
                    let app = app2.clone();
                    let cancelled = cancelled.clone();
                    async move { one(&app, id, t, cancelled).await }
                })
                .await;
            let d = &app2.downloads;
            if let Some(j) = d.jobs.lock().unwrap().iter_mut().find(|j| j.id == id) {
                j.finished = Some(Instant::now());
                log(format!("download job '{}': {} saved, {} already had, {} failed", j.label, j.done, j.skipped, j.failed.len()));
            }
            d.touch();
        });
        Ok(json!({ "id": id }))
    }

    /// sends progress to the page (at most 4× a second) and forgets finished jobs after a minute
    pub async fn pump(app: Arc<App>) {
        loop {
            tokio::time::sleep(Duration::from_millis(250)).await;
            let d = &app.downloads;
            d.jobs.lock().unwrap().retain(|j| j.finished.is_none_or(|t| t.elapsed() < Duration::from_secs(60)));
            if !d.dirty.swap(false, Ordering::SeqCst) {
                continue;
            }
            let mut st = d.state();
            st["added"] = Value::Array(std::mem::take(&mut *d.added.lock().unwrap()));
            app.send(UserEvent::Eval(format!("window.__abyssDl&&window.__abyssDl({st})")));
        }
    }
}

async fn one(app: &Arc<App>, job: u64, t: Value, cancelled: Arc<AtomicBool>) {
    let d = &app.downloads;
    let id = t["id"].as_str().unwrap_or("").to_string();
    let title = t["title"].as_str().unwrap_or("song").to_string();
    let with_job = |f: &mut dyn FnMut(&mut Job)| {
        if let Some(j) = d.jobs.lock().unwrap().iter_mut().find(|j| j.id == job) {
            f(j);
        }
        d.touch();
    };
    if cancelled.load(Ordering::SeqCst) {
        return;
    }
    if index().contains_key(&id) {
        with_job(&mut |j| j.skipped += 1);
        return;
    }
    with_job(&mut |j| j.current.push(Cur { id: id.clone(), title: title.clone(), pct: 0.0 }));
    let _ = std::fs::create_dir_all(dir());
    let file = format!("{id}.m4a");
    let r = app
        .yt
        .download_audio(
            &id,
            &dir().join(&file),
            |p| {
                if let Some(j) = d.jobs.lock().unwrap().iter_mut().find(|j| j.id == job) {
                    if let Some(c) = j.current.iter_mut().find(|c| c.id == id) {
                        c.pct = p;
                    }
                }
                d.touch();
            },
            || cancelled.load(Ordering::SeqCst),
        )
        .await;
    match r {
        Ok((mime, size)) => {
            let mut entry = t.clone();
            if let Some(o) = entry.as_object_mut() {
                o.remove("mvt");
            }
            entry["file"] = json!(file);
            entry["mime"] = json!(mime);
            entry["size"] = json!(size);
            entry["at"] = json!(util::now_ms());
            {
                let _g = d.index_lock.lock().unwrap();
                let mut idx = index();
                idx.insert(id.clone(), entry.clone());
                save_index(&idx);
            }
            d.added.lock().unwrap().push(entry);
            with_job(&mut |j| {
                j.current.retain(|c| c.id != id);
                j.done += 1;
            });
        }
        Err(x) => {
            if x != "cancelled" {
                log(format!("download failed ({id}): {x}"));
            }
            with_job(&mut |j| {
                j.current.retain(|c| c.id != id);
                if x != "cancelled" {
                    j.failed.push(title.clone());
                }
            });
        }
    }
}
