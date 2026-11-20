/* abyss – renderer. Two-tone UI on real YouTube Music data (via window.api from preload). */

/* ================= utils ================= */
// settings live in a file managed by the main process (userData/settings.json)
const PERSIST = (() => { try { return window.api.store.load() || {}; } catch { return {}; } })();
const store = {
  get(k, d) { return Object.prototype.hasOwnProperty.call(PERSIST, k) ? PERSIST[k] : d; },
  set(k, v) { PERSIST[k] = v; try { window.api.store.set(k, v); } catch {} },
};
// one-time move of anything saved by older versions in browser storage
if (!Object.keys(PERSIST).length) {
  try { for (let i = 0; i < localStorage.length; i++) { const k = localStorage.key(i); try { store.set(k, JSON.parse(localStorage.getItem(k))); } catch {} } } catch {}
}
const esc = s => String(s ?? "").replace(/[&<>"']/g, c => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]));
const fmt = s => (!s || !isFinite(s) ? "" : `${Math.floor(s / 60)}:${String(Math.floor(s % 60)).padStart(2, "0")}`);
const initials = t => String(t || "♪").replace(/[^\p{L}\p{N} ]/gu, "").split(" ").filter(Boolean).map(w => w[0]).join("").slice(0, 2).toUpperCase() || "♪";
const pl = (n, w) => `${n} ${w}${n === 1 ? "" : "s"}`;
const clamp = (v, a, b) => Math.max(a, Math.min(b, v));
const hash = s => [...String(s)].reduce((h, c) => (h * 31 + c.charCodeAt(0)) >>> 0, 7) % 360;

/* ================= constants ================= */
const THEMES = [
  ["white", "White", "#ececec", "#0c0c0c", 1], ["dark", "Dark", "#0c0c0c", "#ececec", 1], ["cream", "Cream & Blue", "#fff6ca", "#336699", 1],
  ["vice", "Vice Sunset", "#22093f", "#ff5fa8"], ["latte", "Latte", "#f5ecdd", "#8a3b1a"], ["sakura", "Sakura", "#fff0f5", "#c2185b"],
];
const EQ_FREQS = [32, 64, 125, 250, 500, 1000, 2000, 4000, 8000, 16000];
const EQ_PRESETS = {
  flat: [0, 0, 0, 0, 0, 0, 0, 0, 0, 0], "bass boost": [7, 6, 4, 2, 0, 0, 0, 0, 0, 0], vocal: [-2, -1, 0, 2, 4, 4, 3, 1, 0, -1],
  rock: [4, 3, 1, -1, -2, -1, 1, 3, 4, 4], electronic: [5, 4, 1, 0, -2, 1, 0, 2, 4, 5], "lo-fi": [3, 2, 1, 0, -1, -2, -4, -6, -8, -10],
};
const LANGS = [["en", "EN"], ["es", "ES"], ["fr", "FR"], ["de", "DE"], ["ja", "JA"], ["ko", "KO"], ["zh", "ZH"], ["pt", "PT"]];

const DISCORD_APP_ID = "1566051422296875109"; // abyss on discord (shows "listening to abyss")
/* ================= state ================= */
const S = {
  page: "home", params: {}, hist: [], fwd: [],
  queue: [], idx: -1, qName: "", qKey: "",
  playing: false, buffering: false, pos: 0, dur: 0,
  vol: store.get("vol", 0.7), prevVol: 0.7, tone: store.get("tone", 1),
  shuffle: false, repeat: false, autoplay: store.get("autoplay", true),
  theme: ["white", "dark", "cream", "vice", "latte", "sakura", "custom"].includes(store.get("theme", "cream")) ? store.get("theme", "cream") : "cream",
  font: store.get("font", ""), custom: store.get("custom", { paper: "#fff6ca", ink: "#336699" }), view: store.get("view", "cover"), gray: store.get("gray", true),
  tab: "lyrics", libTab: "library", artTab: "all", q: "", discord: store.get("discord", true), discordId: DISCORD_APP_ID, discordOk: false,
  lyrTr: false, roman: false, lyrSrc: {}, blur: true, lyrSize: store.get("lyrSize", 1), trLang: store.get("trLang", "en"),
  crossfade: store.get("crossfade", 0), eq: store.get("eq", [...EQ_PRESETS.flat]), eqOn: store.get("eqOn", true), eqPreset: store.get("eqPreset", "flat"), speed: 1,
  signedIn: false, account: null, syncing: false, lastSync: "never", modal: null, modalTrack: null,
  liked: new Map(store.get("liked", [])), plays: new Map(store.get("plays", [])), localPls: store.get("localPls", []), downloads: new Map(), subs: new Set(store.get("subs", [])),
};
const cur = () => S.queue[S.idx] || null;
const saveLiked = () => store.set("liked", [...S.liked]);
const saveLocal = () => store.set("localPls", S.localPls);

/* ================= async data cache ================= */
const cache = new Map(), pending = new Set();
function need(key, loader) {
  if (cache.has(key)) return cache.get(key);
  if (!pending.has(key)) {
    pending.add(key);
    loader().then(v => cache.set(key, { ok: true, v })).catch(e => cache.set(key, { ok: false, e: String(e.message || e) }))
      .finally(() => { pending.delete(key); queueRender(); });
  }
  return null;
}
let rq = false;
const queueRender = () => { if (rq) return; rq = true; requestAnimationFrame(() => { rq = false; render(); }); };
const loading = (t = "loading") => `<div class="loading">${t}…</div>`;
const errBox = (e, key) => `<div class="err"><span>${esc(e)}</span><button class="obtn" data-act="retry" data-k="${esc(key)}">try again</button></div>`;

/* ================= audio ================= */
const audio = new YTAudio(); // official YouTube player (see ytplayer.js)
// The official YouTube player plays the sound itself, so only volume and speed can be controlled here.
const AE = {
  ctx: { resume() {} },
  init() {},
  apply() { audio.volume = S.vol; },
  level() { return S.playing ? 1 : 0; },
};
AE.apply();
let errStreak = 0;
let lastLogged = null;
audio.addEventListener("playing", () => { S.playing = true; S.buffering = false; errStreak = 0; const t = cur(); if (t && lastLogged !== audio.src) { lastLogged = audio.src; logPlay(t); } render(); });
audio.addEventListener("pause", () => { S.playing = false; render(); pushDiscord(); saveSession(); });
audio.addEventListener("seeked", () => pushDiscord());
audio.addEventListener("waiting", () => { S.buffering = true; tick(); });
audio.addEventListener("loadedmetadata", () => { if (isFinite(audio.duration)) S.dur = audio.duration; tick(true); });
audio.addEventListener("timeupdate", () => { S.pos = audio.currentTime; tick(); maybeCrossfade(); });
/* smooth transition: when the song is within the crossfade time of its end, start the next one underneath */
let xfading = false;
function nextIndex() {
  if (S.repeat === "one" || !S.queue.length) return null;
  if (S.shuffle && S.queue.length > 1) { let j; do j = Math.floor(Math.random() * S.queue.length); while (j === S.idx); return j; }
  if (S.idx + 1 < S.queue.length) return S.idx + 1;
  return S.repeat ? 0 : null;
}
function maybeCrossfade() {
  const cf = S.crossfade;
  if (!cf || xfading || !S.playing || !S.dur || S.dur < cf * 3) return;
  if (S.dur - S.pos > cf) return;
  const ni = nextIndex();
  if (ni == null) return;
  xfading = true;
  S.idx = ni;
  const t = cur();
  S.pos = 0; S.dur = t.dur || 0;
  audio.crossTo(t.id, cf);
  mediaSession(t); setTimeout(pushDiscord, 1500); saveSession();
  if (S.autoplay && S.queue.length - S.idx <= 2) extendRadio();
  render();
  setTimeout(() => (xfading = false), cf * 1000 + 800);
}
audio.addEventListener("ended", () => next(true));
audio.addEventListener("error", () => {
  const t = cur();
  if (!t) return;
  toast(`can't play “${t.title}”${++errStreak < 4 ? " · skipping" : ""}`);
  if (errStreak < 4) setTimeout(() => next(true), 1200);
});

/* ---- session: remembered across restarts (queue, song, position, page, shuffle/repeat/speed) ---- */
const slim = t => ({ id: t.id, title: t.title, artist: t.artist, artistId: t.artistId || null, album: t.album || "", albumId: t.albumId || null, dur: t.dur || 0, thumb: t.thumb || null, video: t.video || undefined });
function saveSession() {
  const from = Math.max(0, S.idx - 50), q = S.queue.slice(from, S.idx + 150);
  store.set("session", {
    queue: q.map(slim), idx: S.idx - from, pos: Math.floor(audio.currentTime || S.pos || 0), qName: S.qName, qKey: S.qKey,
    page: S.page, params: S.params, tab: S.tab, shuffle: S.shuffle, repeat: S.repeat, speed: S.speed, libTab: S.libTab,
  });
}
function restoreSession() {
  const ses = store.get("session", null);
  if (!ses) return;
  if (Array.isArray(ses.queue) && ses.queue.length) {
    S.queue = ses.queue; S.idx = clamp(ses.idx | 0, 0, ses.queue.length - 1); S.qName = ses.qName || ""; S.qKey = ses.qKey || "";
    const t = cur(); S.pos = ses.pos || 0; S.dur = t.dur || 0;
    audio.cue(t.id, S.pos); mediaSession(t);
  }
  if (ses.page && P[ses.page] && !["genre", "eq"].includes(ses.page)) { S.page = ses.page; S.params = ses.params || {}; S.tab = ses.tab || "lyrics"; }
  if (ses.libTab) S.libTab = ses.libTab;
  S.shuffle = !!ses.shuffle; S.repeat = ses.repeat ?? false; S.speed = ses.speed || 1; audio.playbackRate = S.speed;
}
setInterval(() => { if (S.playing) saveSession(); }, 10000);
window.addEventListener("beforeunload", saveSession);

function load(autoplay = true) {
  const t = cur();
  if (!t) return;
  AE.init();
  AE.ctx.resume();
  audio.src = "yt:" + t.id;
  audio.playbackRate = S.speed;
  S.pos = 0; S.dur = t.dur || 0; S.buffering = true;
  if (autoplay) audio.play().catch(() => {});
  mediaSession(t);
  setTimeout(pushDiscord, 1500);
  saveSession();
  if (S.autoplay && S.queue.length - S.idx <= 2) extendRadio();
  render();
}
/* discord status: sent on play / pause / seek / song change (throttled) */
let dT = null;
function pushDiscord() {
  if (!S.discord || !S.discordOk) return;
  clearTimeout(dT);
  dT = setTimeout(() => {
    const t = cur();
    window.api.discordSet(t ? { id: t.id, title: t.title, artist: t.artist, album: t.album, thumb: t.thumb, playing: S.playing, pos: audio.currentTime || 0, dur: S.dur || t.dur || 0 } : null).catch(() => {});
  }, 600);
}
async function discordOn(quiet) {
  try { await window.api.discordConnect(S.discordId); S.discordOk = true; S.discord = true; store.set("discord", true); if (!quiet) toast("discord status on"); pushDiscord(); }
  catch (e) { S.discordOk = false; if (!quiet) toast(e.message.replace(/^Error invoking remote method [^:]+: (Error: )?/, "")); }
  render();
}
function mediaSession(t) {
  if (!("mediaSession" in navigator)) return;
  navigator.mediaSession.metadata = new MediaMetadata({ title: t.title, artist: t.artist, album: t.album || "", artwork: t.thumb ? [{ src: t.thumb, sizes: "544x544", type: "image/jpeg" }] : [] });
}
if ("mediaSession" in navigator) {
  navigator.mediaSession.setActionHandler("play", () => toggle(true));
  navigator.mediaSession.setActionHandler("pause", () => toggle(false));
  navigator.mediaSession.setActionHandler("nexttrack", () => next());
  navigator.mediaSession.setActionHandler("previoustrack", () => prev());
  navigator.mediaSession.setActionHandler("seekto", d => { audio.currentTime = d.seekTime; });
}
function toggle(force) {
  if (!cur()) return toast("pick a song first");
  const play = force ?? audio.paused;
  if (play) { AE.init(); AE.ctx.resume(); if (!audio.src) load(); else audio.play().catch(() => {}); } else audio.pause();
}
function playFrom(list, i, key, name) {
  const t = list[i];
  if (!t) return;
  if (key === "queue") { if (i === S.idx) return toggle(); S.idx = i; return load(); }
  if (cur() && cur().id === t.id && S.qKey === key) return toggle();
  S.queue = list.map(x => ({ ...x })); S.idx = i; S.qKey = key; S.qName = name || "";
  load();
}
function startRadio(t, name) {
  S.queue = [{ ...t }]; S.idx = 0; S.qKey = "radio:" + t.id; S.qName = name || `${t.title} radio`;
  load();
  const key = S.qKey;
  window.api.upNext(t.id).then(list => {
    if (S.qKey !== key) return;
    const seen = new Set([t.id]);
    S.queue.push(...list.filter(x => !seen.has(x.id) && seen.add(x.id)));
    render();
  }).catch(() => {});
}
let extending = false;
function extendRadio() {
  const last = S.queue[S.queue.length - 1];
  if (!last || extending) return Promise.resolve();
  extending = true;
  return window.api.upNext(last.id).then(list => {
    const seen = new Set(S.queue.map(x => x.id));
    const add = list.filter(x => !seen.has(x.id));
    S.queue.push(...add.slice(0, 25));
    if (add.length) render();
  }).catch(() => {}).finally(() => (extending = false));
}
async function next(auto) {
  if (!S.queue.length) return;
  if (auto && S.repeat === "one") { audio.currentTime = 0; return audio.play(); }
  let i = S.idx;
  if (S.shuffle && S.queue.length > 1) { let j; do j = Math.floor(Math.random() * S.queue.length); while (j === i); i = j; }
  else i += 1;
  if (i >= S.queue.length) {
    if (S.autoplay) { await extendRadio(); if (i >= S.queue.length) i = 0; }
    else if (S.repeat) i = 0;
    else { audio.pause(); return; }
  }
  S.idx = i; load();
}
function prev() {
  if (audio.currentTime > 3 || S.idx <= 0) { audio.currentTime = 0; return; }
  S.idx -= 1; load();
}

/* ================= icons ================= */
const sv = (d, fill) => `<svg ${fill ? 'class="fill"' : ""} viewBox="0 0 24 24">${d}</svg>`;
const I = {
  play: sv('<path d="M7.5 4.5v15L19.5 12z"/>', 1), pause: sv('<rect x="6" y="4" width="4" height="16"/><rect x="14" y="4" width="4" height="16"/>', 1),
  prev: sv('<path d="M19 20 9 12l10-8zM5 19V5"/>'), next: sv('<path d="m5 4 10 8-10 8zM19 5v14"/>'),
  shuffle: sv('<path d="M16 3h5v5M4 20 21 3M21 16v5h-5M15 15l6 6M4 4l5 5"/>'),
  repeat: sv('<path d="m17 2 4 4-4 4"/><path d="M3 11v-1a4 4 0 0 1 4-4h14M7 22l-4-4 4-4"/><path d="M21 13v1a4 4 0 0 1-4 4H3"/>'),
  heart: sv('<path d="M19 14c1.5-1.5 3-3.2 3-5.5A5.5 5.5 0 0 0 16.5 3c-1.8 0-3 .5-4.5 2-1.5-1.5-2.7-2-4.5-2A5.5 5.5 0 0 0 2 8.5c0 2.3 1.5 4 3 5.5l7 7z"/>'),
  heartF: sv('<path d="M19 14c1.5-1.5 3-3.2 3-5.5A5.5 5.5 0 0 0 16.5 3c-1.8 0-3 .5-4.5 2-1.5-1.5-2.7-2-4.5-2A5.5 5.5 0 0 0 2 8.5c0 2.3 1.5 4 3 5.5l7 7z"/>', 1),
  search: sv('<circle cx="11" cy="11" r="7"/><path d="m21 21-4.3-4.3"/>'), x: sv('<path d="M18 6 6 18M6 6l12 12"/>'),
  left: sv('<path d="m15 18-6-6 6-6"/>'), right: sv('<path d="m9 18 6-6-6-6"/>'), up: sv('<path d="m18 15-6-6-6 6"/>'),
  gear: sv('<circle cx="12" cy="12" r="3"/><path d="M12 2v3M12 19v3M4.2 4.2l2.1 2.1M17.7 17.7l2.1 2.1M2 12h3M19 12h3M4.2 19.8l2.1-2.1M17.7 6.3l2.1-2.1"/>'),
  sliders: sv('<path d="M4 21v-7M4 10V3M12 21v-9M12 8V3M20 21v-5M20 12V3M1 14h6M9 8h6M17 16h6"/>'),
  sync: sv('<path d="M21 12a9 9 0 0 1-15.5 6.2L3 16M3 12a9 9 0 0 1 15.5-6.2L21 8M21 3v5h-5M3 21v-5h5"/>'),
  dl: sv('<path d="M12 3v12M7 10l5 5 5-5M5 21h14"/>'), plus: sv('<path d="M12 5v14M5 12h14"/>'), check: sv('<path d="M20 6 9 17l-5-5"/>'),
  grip: sv('<path d="M4 9h16M4 15h16"/>'), mic: sv('<rect x="9" y="2" width="6" height="12"/><path d="M5 10a7 7 0 0 0 14 0M12 17v5"/>'),
  list: sv('<path d="M3 6h12M3 12h12M3 18h8M18 15v6M15 18h6"/>'), folder: sv('<path d="M3 6h7l2 2h9v11H3z"/>'),
  vol: sv('<path d="M11 5 6 9H2v6h4l5 4zM15.5 8.5a5 5 0 0 1 0 7M19 5a10 10 0 0 1 0 14"/>'), volLow: sv('<path d="M11 5 6 9H2v6h4l5 4zM15.5 8.5a5 5 0 0 1 0 7"/>'),
  volX: sv('<path d="M11 5 6 9H2v6h4l5 4zM22 9l-6 6M16 9l6 6"/>'), radio: sv('<circle cx="12" cy="12" r="2"/><path d="M16.2 7.8a6 6 0 0 1 0 8.5M7.8 16.2a6 6 0 0 1 0-8.5M19 5a10 10 0 0 1 0 14M5 19A10 10 0 0 1 5 5"/>'),
  trend: sv('<path d="M3 17l6-6 4 4 8-8M15 7h6v6"/>'), user: sv('<circle cx="12" cy="8" r="4"/><path d="M4 21a8 8 0 0 1 16 0"/>'),
  trash: sv('<path d="M4 7h16M9 7V4h6v3M6 7l1 13h10l1-13"/>'),
};

/* ================= components ================= */
const artBg = h => `background:linear-gradient(135deg,hsl(${h} 70% 55%),hsl(${(h + 50) % 360} 65% 30%))`;
const art = (src, label = "♪", cls = "") => src
  ? `<div class="art ${cls}"><img src="${esc(src)}" alt="" loading="lazy" referrerpolicy="no-referrer"></div>`
  : `<div class="art ${cls}" style="${artBg(hash(label))}"><span class="glyph">${esc(initials(label))}</span></div>`;
const mosaic = list => {
  const th = [...new Set(list.map(t => t.thumb).filter(Boolean))];
  if (th.length < 4) return art(th[0], list[0]?.title || "♪");
  return `<div class="mosaic">${th.slice(0, 4).map(u => art(u)).join("")}</div>`;
};
const playIcon = () => (S.playing ? I.pause : I.play);
const heartBtn = t => `<button class="ibtn ${S.liked.has(t.id) ? "on" : ""}" data-act="like" data-id="${esc(t.id)}" title="like">${S.liked.has(t.id) ? I.heartF : I.heart}</button>`;
const eqI = () => '<span class="eq"><i></i><i></i><i></i></span>';
const pct = () => (S.dur ? (S.pos / S.dur) * 100 : 0);
const wave = (n, seed) => Array.from({ length: n }, (_, i) => `<i style="height:${16 + Math.round(84 * Math.abs(Math.sin((i + seed * 7) * 1.7) * Math.cos((i + seed) * 0.37)))}%"></i>`).join("");
const volIcon = () => (S.vol === 0 ? I.volX : S.vol < 0.45 ? I.volLow : I.vol);
const sw = (on, act, k = "", dis) => `<button class="sw ${on ? "on" : ""}" data-act="${act}" data-k="${k}" ${dis ? "disabled style=opacity:.4" : ""}></button>`;
const artistLink = t => (t.artistId ? `<a data-act="artist" data-id="${esc(t.artistId)}">${esc(t.artist)}</a>` : t.artist ? `<a data-act="artistname" data-n="${esc(t.artist.split(",")[0].trim())}">${esc(t.artist)}</a>` : "");
const albumLink = t => (t.albumId ? `<a data-act="album" data-id="${esc(t.albumId)}">${esc(t.album)}</a>` : esc(t.album));
const lc = s => esc(String(s || "").toLowerCase());
const ctxs = {};
const reg = (key, list) => ((ctxs[key] = list), key);

function rows(list, key, o = {}) {
  reg(key, list);
  if (!list.length) return `<div class="empty">nothing here yet.</div>`;
  const cid = cur()?.id;
  return `<div class="rows">${list.map((t, i) => {
    const c = key === "queue" ? i === S.idx : t.id === cid;
    return `<div class="row ${c ? "cur" : ""}" data-act="play" data-k="${esc(key)}" data-i="${i}" ${o.drag ? `draggable="true" data-qi="${i}"` : ""}>
      ${o.num ? `<span class="num">${c && S.playing ? eqI() : i + 1}</span>` : ""}
      ${art(t.thumb, t.title)}
      <div class="meta"><div class="ti">${esc(t.title)}${t.video ? '<span class="caps">video</span>' : ""}${S.downloads.has(t.id) ? '<span class="caps">↓ saved</span>' : ""}</div>
        <div class="su">${artistLink(t)}${o.album && t.album ? ` · ${albumLink(t)}` : ""}</div></div>
      ${o.drag ? "" : heartBtn(t)}<span class="du">${fmt(t.dur)}</span>
      ${o.drag
        ? `<button class="ibtn" data-act="rmq" data-i="${i}" title="remove">${I.x}</button><span class="grip">${I.grip}</span>`
        : `<button class="ibtn" data-act="addq" data-k="${esc(key)}" data-i="${i}" title="play next">${I.plus}</button><button class="ibtn" data-act="addpl" data-k="${esc(key)}" data-i="${i}" title="add to playlist">${I.list}</button>`}
    </div>`;
  }).join("")}</div>`;
}
function card(it, key, i) {
  if (it.type === "song") return `<div class="card" data-act="radio1" data-k="${esc(key)}" data-i="${i}">${art(it.thumb, it.title)}<div class="n ell">${esc(it.title)}</div><div class="s muted ell">${esc(it.artist)}</div></div>`;
  const act = it.type === "album" ? "album" : it.type === "artist" ? "artist" : "pl";
  return `<div class="card ${it.type === "artist" ? "round" : ""}" data-act="${act}" data-id="${esc(it.id)}">${art(it.thumb, it.title)}<div class="n ell">${esc(it.title)}</div><div class="s muted ell">${esc(it.sub || it.artist || it.type)}</div></div>`;
}
function section(sec, key) {
  reg(key, sec.items);
  const songs = sec.items.filter(x => x.type === "song");
  if (songs.length === sec.items.length) return `<div class="sec"><div class="phead"><div class="h2">${lc(sec.title)}</div></div>${rows(songs.slice(0, 10), key, { album: true })}</div>`;
  return `<div class="sec"><div class="phead"><div class="h2">${lc(sec.title)}</div></div><div class="cards">${sec.items.slice(0, 10).map((it, i) => card(it, key, i)).join("")}</div></div>`;
}

/* ================= turntable (deck view) ================= */
const mix = p => `color-mix(in srgb, var(--ink) ${p}%, var(--paper))`;
const knobY = v => 39 + (1 - v) * 28;
function turntable(t) {
  const txt = (x, y, s, v, extra = "") => `<text x="${x}" y="${y}" font-size="${s}" text-anchor="middle" font-family="Space Mono" font-weight="700" style="fill:var(--ink)" ${extra}>${v}</text>`;
  return `<svg class="turntable" viewBox="0 0 130 100" xmlns="http://www.w3.org/2000/svg">
  <defs><clipPath id="lcn"><circle cx="47" cy="50" r="15.5"/></clipPath></defs>
  <rect x="1.5" y="1.5" width="127" height="97" style="fill:${mix(10)};stroke:var(--ink)" stroke-width="1.4"/>
  <circle cx="47" cy="50" r="44.5" style="fill:var(--ink)"/>
  <circle cx="47" cy="50" r="43" fill="none" style="stroke:var(--paper)" stroke-width="1.4" stroke-dasharray=".8 1.1"/>
  <g class="spin">
    <circle cx="47" cy="50" r="40" fill="#0d0d0d"/>
    ${[37.5, 35, 32.5, 30, 27.5, 25, 22.5, 20, 17.8].map(r => `<circle cx="47" cy="50" r="${r}" fill="none" stroke="#2a2a2a" stroke-width=".45"/>`).join("")}
    <g class="cover" clip-path="url(#lcn)">${t?.thumb ? `<image href="${esc(t.thumb)}" x="31" y="34" width="32" height="32" preserveAspectRatio="xMidYMid slice"/>` : `<rect x="31" y="34" width="32" height="32" style="fill:${mix(40)}"/>`}</g>
    <circle cx="47" cy="50" r="15.5" fill="none" stroke="#000" stroke-width=".7"/><circle cx="47" cy="50" r="1.4" style="fill:var(--paper)" stroke="#000" stroke-width=".4"/>
  </g>
  <g data-act="toggle"><rect x="6" y="83" width="10" height="10" style="fill:${S.playing ? "var(--ink)" : "var(--paper)"};stroke:var(--ink)" stroke-width="1"/></g>
  ${txt(11, 81, 2, S.playing ? "STOP" : "START")}
  ${txt(113.75, 37, 2.5, "VOL")}
  ${Array.from({ length: 9 }, (_, i) => `<rect x="108.3" y="${41 + i * 3.5}" width="1.7" height=".4" style="fill:var(--ink)"/>`).join("")}
  <rect x="111" y="39" width="5.5" height="32" style="fill:var(--ink)" data-vol-track/>
  <rect data-bind="volknob" x="109.5" y="${knobY(S.vol)}" width="8.5" height="4" style="fill:var(--paper);stroke:var(--ink)" stroke-width=".6"/>
  ${txt(113.75, 75.4, 2.6, Math.round(S.vol * 100), 'data-bind="volnum"')}
  <rect x="104" y="34" width="19" height="43" fill="transparent" data-drag="vol" style="cursor:ns-resize"/>
  <g data-drag="tone" style="cursor:ns-resize"><circle cx="113.5" cy="83" r="4.6" style="fill:var(--paper);stroke:var(--ink)" stroke-width=".8"/>
    <g data-bind="toneknob" transform="rotate(${(S.tone - 0.5) * 270} 113.5 83)"><rect x="113" y="78.8" width="1" height="3.2" style="fill:var(--ink)"/></g></g>
  ${txt(113.5, 92.4, 2.2, "TONE")}
  <circle cx="104" cy="20" r="10" style="fill:var(--paper);stroke:var(--ink)" stroke-width="1"/>
  <g class="arm"><rect x="99.5" y="4" width="9" height="9" style="fill:var(--ink)"/>
    <path d="M104 20 L104 55 Q104 65 97 69 L77 77" fill="none" style="stroke:var(--ink)" stroke-width="2.6"/>
    <g transform="translate(75 78) rotate(68)"><rect x="-3.5" y="-6" width="7.5" height="11.5" style="fill:var(--paper);stroke:var(--ink)" stroke-width=".8"/></g>
    <circle cx="104" cy="20" r="3.4" style="fill:var(--ink)"/></g>
</svg>`;
}

/* ================= shell ================= */
const NAV = [["home", "home"], ["search", "search"], ["library", "library"], ["playlists", "playlists"], ["downloads", "downloads"]];
const topbar = () => `<header class="top">
  <div class="logo" data-act="go" data-p="home">abyss</div>
  <div class="arrows"><button class="ibtn" data-act="back" title="back">${I.left}</button><button class="ibtn" data-act="fwd" title="forward">${I.right}</button></div>
  <nav class="nav">${NAV.map(([p, n]) => `<a class="${S.page === p || (p === "playlists" && S.page === "playlist") ? "on" : ""}" data-act="go" data-p="${p}">${n}</a>`).join("")}</nav>
  <label class="srch">${I.search}<input id="tbq" placeholder="search anything" value="${esc(S.q)}" autocomplete="off" spellcheck="false">${S.q ? `<button class="x" data-act="clears" title="clear">${I.x}</button>` : ""}</label>
  <div class="top-r">
    <button class="obtn ${S.discord && S.discordOk ? "" : "off"}" data-act="discord" title="Discord status"><span class="sq"></span><span class="t">${S.discord && S.discordOk ? "discord" : "discord off"}</span></button>
    <button class="ibtn" data-act="mini-on" title="mini player (ctrl+m)">${I.mini}</button>
    <button class="ibtn ${S.page === "settings" ? "on" : ""}" data-act="go" data-p="settings" title="settings">${I.gear}</button>
    <span class="avatar ${S.signedIn ? "in" : ""}" data-act="${S.signedIn ? "go" : "signin"}" data-p="settings" title="${S.signedIn ? esc(S.account?.name || "account") : "sign in with google"}">${S.account?.photo ? `<img src="${esc(S.account.photo)}" alt="" style="width:100%;height:100%;object-fit:cover" referrerpolicy="no-referrer">` : S.signedIn ? esc((S.account?.name || "y")[0].toLowerCase()) : "?"}</span>
  </div></header>`;

function nowPanel() {
  const t = cur();
  if (!t) return `<aside class="np"><div class="np-top"><div class="np-lbl"><span class="caps">now playing</span></div>
      <div class="np-title">nothing yet.</div><div class="muted" style="font-weight:700">search for a song or pick something from home.</div></div>
    <div class="np-bot inv"><div class="np-artist">abyss</div><div class="np-from caps">no ads · ever</div>${volRowHtml()}</div></aside>`;
  const tl = t.title.length, tcls = tl > 46 ? "s" : tl > 26 ? "m" : "";
  return `<aside class="np">
    <div class="np-top">
      <div class="np-lbl"><span class="caps">now playing${S.buffering && S.playing ? " · buffering" : ""}</span>${S.qName ? `<span class="caps ell" style="max-width:55%">${esc(S.qName)}</span>` : ""}</div>
      <div class="np-title ${tcls}" ${t.albumId ? `data-act="album" data-id="${esc(t.albumId)}" title="open album"` : ""}>${lc(t.title)}.</div>
      <div class="stage-art">
        <div class="vinyl"><div class="spinner"><div class="lbl">${art(t.thumb, t.title)}</div></div></div>
        <div class="sleeve" ${t.albumId ? `data-act="album" data-id="${esc(t.albumId)}"` : ""}>${art(t.thumb, t.title)}</div></div>
    </div>
    <div class="np-bot inv">
      <div class="np-artist">${t.artistId ? `<a data-act="artist" data-id="${esc(t.artistId)}">${lc(t.artist)}</a>` : t.artist ? `<a data-act="artistname" data-n="${esc(t.artist.split(",")[0].trim())}">${lc(t.artist)}</a>` : ""}</div>
      ${t.album ? `<div class="np-from caps">from “${esc(t.album)}”</div>` : ""}
      <div class="np-btns">
        <button class="obtn ${S.liked.has(t.id) ? "on" : ""}" data-act="like" data-id="${esc(t.id)}">${S.liked.has(t.id) ? I.heartF : I.heart}${S.liked.has(t.id) ? "liked" : "like"}</button>
        <button class="obtn ${S.page === "np" && S.tab === "lyrics" ? "on" : ""}" data-act="np" data-t="lyrics">${I.mic}lyrics</button>
        <button class="obtn ${S.downloads.has(t.id) ? "on" : ""}" data-act="dl" data-id="${esc(t.id)}">${S.downloads.has(t.id) ? I.check + "saved" : dlActive(t.id) ? I.dl + "saving…" : I.dl + "download"}</button>
      </div>
      <div class="wave-row"><span data-bind="pos">${fmt(S.pos) || "0:00"}</span><div class="wave" data-wave data-act="seek">${wave(46, hash(t.id))}</div><span data-bind="dur">${fmt(S.dur)}</span></div>
      <div class="ctrls">
        <button class="ibtn ${S.shuffle ? "on" : ""}" data-act="shuffle" title="shuffle">${I.shuffle}</button>
        <button class="ibtn" data-act="prev" title="previous">${I.prev}</button>
        <button class="play" data-act="toggle" title="play / pause (space)">${playIcon()}</button>
        <button class="ibtn" data-act="next" title="next">${I.next}</button>
        <button class="ibtn ${S.repeat ? "on" : ""}" data-act="repeat" title="repeat">${I.repeat}</button>
      </div>
      ${volRowHtml()}
    </div>
  </aside>`;
}
const volRowHtml = () => `<div class="volrow"><button class="ibtn" data-act="mute" data-bind="volicon" title="mute (m)">${volIcon()}</button>
  <div class="volseg" data-drag="volh" title="volume (↑ / ↓)">${Array.from({ length: 20 }, (_, i) => `<i style="height:${35 + (65 * i) / 19}%"></i>`).join("")}</div>
  <span class="volnum" data-bind="volnum">${Math.round(S.vol * 100)}</span></div>`;

/* ================= pages ================= */
const P = {};
const signBanner = () => S.signedIn ? "" : `<div class="gban"><div><div class="h2">sign in for your own music.</div><div class="muted" style="font-weight:700;margin-top:6px">your youtube music playlists, likes, mixes and history — synced both ways.</div></div><button class="gbtn" data-act="signin"><span class="g"></span>sign in with google</button></div>`;

const today = () => new Date().toISOString().slice(0, 10);
const shead = (title, sub, link) => `<div class="phead"><div>${sub ? `<div class="hsub">${esc(sub)}</div>` : ""}<div class="h2">${esc(title)}</div></div>${link || ""}</div>`;
const cardsHtml = (items, key, n = 10) => (reg(key, items), `<div class="cards">${items.slice(0, n).map((it, i) => card(it, key, i)).join("")}</div>`);
function mixSeeds(fallback) {
  const byArtist = new Map();
  for (const t of [...recent(), ...S.liked.values(), ...fallback]) {
    if (!t?.id || !t.artist) continue;
    const a = t.artist.split(",")[0].trim();
    if (!byArtist.has(a)) byArtist.set(a, t);
  }
  return [...byArtist.entries()].slice(0, 8).map(([a, t]) => ({ ...t, mixName: `${a} mix` }));
}
P.home = () => {
  let up = "";
  if (cur()) {
    const nx = S.queue.slice(S.idx + 1, S.idx + 6);
    up = `<div class="phead"><div class="h1">up next</div><button class="obtn" data-act="np" data-t="queue">see full queue · ${S.queue.length - S.idx - 1}</button></div>
      ${nx.length ? rows(nx, "upnext") : `<div class="empty">${S.autoplay ? "finding more songs…" : "queue ends after this song."}</div>`}<div style="height:30px"></div>`;
  }
  // signed in: build home from YOUR youtube (picks, history, playlists) — no country charts
  const me = S.signedIn ? need("me", () => window.api.personal()) : null;
  const usePersonal = !!(S.signedIn && me?.ok && me.v && (me.v.picks.length || me.v.history.length));
  const h = usePersonal ? { ok: true, v: [] } : S.signedIn && !me ? null : need("home", () => window.api.home());
  if (!h) return `${signBanner()}${up}${loading(S.signedIn ? "loading your youtube" : "loading your home")}`;
  if (!h.ok) return `${signBanner()}${up}${errBox(h.e, "home")}`;
  const used = new Set();
  const take = re => h.v.filter(s => !used.has(s) && re.test(s.title) && used.add(s));
  const quick = take(/quick picks|song mixes|listen again|speed dial|start radio/i);
  const ytMixes = take(/mix/i);
  const community = take(/community|featured|playlists/i);
  const trending = take(/trend|chart|top|video/i);
  const fresh = take(/new (release|album|single)/i);
  const rest = h.v.filter(s => !used.has(s));
  const qsongs = usePersonal ? (me.v.picks.length ? me.v.picks : me.v.history) : (quick.flatMap(s => s.items).filter(i => i.type === "song").length ? quick : h.v).flatMap(s => s.items).filter(i => i.type === "song");
  let html = signBanner() + up;
  if (!cur()) html += `<div class="h1" style="margin-bottom:10px">home.</div>`;
  // quick picks
  if (qsongs.length) { reg("qp", qsongs.slice(0, 12)); html += `<div class="sec" style="margin-top:10px">${shead("quick picks", usePersonal ? "picked for you from your youtube" : "start radio from a song")}<div class="qp">${rows(qsongs.slice(0, 12), "qp")}</div></div>`; }
  if (usePersonal) {
    const m = me.v, lib = need("library", () => window.api.library());
    if (m.history.length) html += `<div class="sec">${shead("listen again", "from your youtube history")}${cardsHtml(m.history.slice(0, 12), "hist", 12)}</div>`;
    if (lib?.ok && lib.v.length) html += `<div class="sec">${shead("your playlists", "from your youtube account", `<a class="caps" data-act="go" data-p="playlists">view all</a>`)}${cardsHtml(lib.v, "mypl", 10)}</div>`;
    if (m.mixes.length) html += `<div class="sec">${shead("mixed for you", "playlists youtube picked for you")}${cardsHtml(m.mixes, "ytpicks", 10)}</div>`;
  }
  // for you: rebuilt whenever your top recent songs change
  const fy = forYouSeeds();
  if (fy.length) {
    const key = "fy:" + fy.map(t => t.id).join(",");
    const r = need(key, () => Promise.all(fy.map(t => window.api.upNext(t.id).catch(() => []))));
    const known = new Set([...S.liked.keys(), ...S.plays.keys()]);
    const list = [];
    if (r?.ok) { const ls = r.v.map(l => l.filter(t => !known.has(t.id))); for (let i = 0; list.length < 15 && i < 60; i++) for (const l of ls) { const t = l[i]; if (t && list.length < 15 && !list.some(x => x.id === t.id)) list.push(t); } }
    reg("foryou", list);
    const names = [...new Set(fy.map(t => t.artist.split(",")[0].trim()))].slice(0, 3).join(", ");
    html += `<div class="sec">${shead("for you", "changes as you listen · based on " + names, list.length ? `<button class="obtn fill" data-act="playk" data-k="foryou">${I.play}play all</button>` : "")}${!r ? loading("picking songs for you") : !r.ok ? errBox(r.e, key) : rows(list, "foryou", { album: true })}</div>`;
  }
  // because you listened to … (similar artists of your latest artists)
  const recentArtists = [...new Map(recent().filter(t => t.artistId).map(t => [t.artistId, t])).values()].slice(0, 2);
  for (const t of recentArtists) {
    const ar = need("ar:" + t.artistId, () => pool.run(() => window.api.artist(t.artistId)));
    const fans = ar?.ok ? ar.v.sections.find(x => /fans|similar|also like/i.test(x.title)) : null;
    if (fans?.items.length) html += `<div class="sec">${shead("because you listened to " + (ar.v.name || t.artist).toLowerCase(), "artists you might like")}${cardsHtml(fans.items, "bc:" + t.artistId, 8)}</div>`;
  }
  // mixed for you
  const seeds = mixSeeds(qsongs);
  if (seeds.length) {
    reg("mixes", seeds);
    html += `<div class="sec">${shead(usePersonal ? "your artist mixes" : "mixed for you", "endless radio built around your taste")}<div class="cards">${seeds.map((t, i) => `<div class="card mixcard" data-act="radio1" data-k="mixes" data-i="${i}">${art(t.thumb, t.title).replace("</div>", `<span class="mixlbl ell">${esc(t.mixName)}</span></div>`)}<div class="n ell">${esc(t.mixName)}</div><div class="s muted ell">${esc(t.artist)} and more</div></div>`).join("")}</div></div>`;
  }
  html += ytMixes.map((s, i) => `<div class="sec">${shead(s.title.toLowerCase(), s.strap)}${cardsHtml(s.items, "ytmix" + i)}</div>`).join("");
  // your daily discover
  const pool2 = [...S.liked.values(), ...recent(), ...qsongs].filter(t => t?.id);
  if (pool2.length) {
    const seed = recent()[0] || pool2[hash(today()) % pool2.length];
    const d = need("discover:" + today() + ":" + seed.id, () => window.api.upNext(seed.id));
    const known = new Set([...S.liked.keys(), ...S.plays.keys(), seed.id]);
    const list = d?.ok ? d.v.filter(t => !known.has(t.id)).slice(0, 10) : [];
    html += `<div class="sec">${shead("your daily discover", `new for you today · because you like ${seed.artist.split(",")[0]}`, list.length ? `<button class="obtn" data-act="playk" data-k="discover">${I.play}play all</button>` : "")}${!d ? loading() : !d.ok ? errBox(d.e, "discover:" + today() + ":" + seed.id) : rows(list, "discover", { album: true })}</div>`;
  }
  // from your library
  const rec = recent().slice(0, 10);
  if (rec.length || S.liked.size) {
    const libItems = [{ type: "playlist", id: "liked", title: "liked music", sub: `${pl(likedCount(), "song")}`, thumb: null }, ...rec];
    html += `<div class="sec">${shead("from your library", "recently played & liked", `<a class="caps" data-act="go" data-p="library">open library</a>`)}${cardsHtml(libItems, "fromlib")}</div>`;
  }
  html += community.map((s, i) => `<div class="sec">${shead("from the community", s.title.toLowerCase())}${cardsHtml(s.items, "comm" + i)}</div>`).join("");
  html += trending.map((s, i) => `<div class="sec">${shead("trending for you", s.title.toLowerCase())}${s.items.every(x => x.type === "song") ? rows(s.items.slice(0, 10), "trend" + i, { album: true }) : cardsHtml(s.items, "trend" + i)}</div>`).join("");
  html += fresh.map((s, i) => `<div class="sec">${shead("new releases", s.title.toLowerCase())}${cardsHtml(s.items, "new" + i, 12)}</div>`).join("");
  html += rest.map((s, i) => section(s, "home" + i)).join("");
  return html;
};

P.search = () => {
  const q = S.params.q || "";
  if (!q) return moodsGrid();
  const r = need("s:" + q, () => window.api.search(q));
  let body = loading("searching");
  if (r && !r.ok) body = errBox(r.e, "s:" + q);
  if (r && r.ok) {
    if (!r.v.length) body = `<div class="empty">no results for “${esc(q)}”.</div>`;
    else body = r.v.map((sec, si) => {
      const key = `s${si}:${q}`;
      reg(key, sec.items);
      if (sec.kind === "top") {
        const it = sec.items[0];
        const act = it.type === "song" ? `data-act="radio1" data-k="${esc(key)}" data-i="0"` : `data-act="${it.type === "album" ? "album" : it.type === "artist" ? "artist" : "pl"}" data-id="${esc(it.id)}"`;
        return `<div class="topcard ${it.type === "artist" ? "round" : ""}" ${act}>${art(it.thumb, it.title)}<div><span class="caps">top result · ${esc(it.type)}</span><div class="h2">${lc(it.title)}</div><div class="muted" style="font-weight:700">${esc(it.sub || it.artist || "")}</div></div></div>`;
      }
      if (sec.kind === "song" || sec.kind === "video") return `<div class="sec"><div class="phead"><div class="h2">${sec.title}</div></div>${rows(sec.items.slice(0, sec.kind === "song" ? 8 : 5), key, { album: true })}</div>`;
      return `<div class="sec"><div class="phead"><div class="h2">${sec.title}</div></div><div class="cards">${sec.items.slice(0, 10).map((it, i) => card(it, key, i)).join("")}</div></div>`;
    }).join("");
  }
  return `<div class="phead"><div class="h1">“${lc(q)}”</div></div>${body}`;
};

P.np = () => {
  const t = cur(), tab = S.tab;
  let body = "";
  if (!t) body = `<div class="empty">nothing playing yet.</div>`;
  else if (tab === "lyrics") {
    const pref = S.lyrSrc[t.id] || "";
    const lkey = "lyr:" + t.id + ":" + pref;
    const r = need(lkey, () => window.api.lyrics(t, pref || undefined));
    if (!r) body = loading("searching 6 lyrics sources");
    else if (!r.ok) body = errBox(r.e, lkey);
    else if (!r.v.lines.length) body = `<div class="empty">no lyrics found for this song on LRCLIB, BetterLyrics, Unison, lrc.red, KuGou or YouTube Music.</div>`;
    else {
      const L = r.v, texts = L.lines.map(l => l[1]), sk = `${t.id}:${L.source}`;
      const tr = S.lyrTr ? need(`tr:${sk}:${S.trLang}`, () => window.api.translate(texts, S.trLang)) : null;
      const rm = S.roman ? need(`rm:${sk}`, () => window.api.romanize(texts)) : null;
      const noRoman = rm?.ok && !rm.v.some(Boolean);
      const sub = i => {
        const b = S.roman && rm?.ok && rm.v[i] ? `<small class="rm">${esc(rm.v[i])}</small>` : "";
        const a = S.lyrTr && tr?.ok && tr.v.lines[i] && tr.v.lines[i] !== texts[i] ? `<small>${esc(tr.v.lines[i])}</small>` : "";
        return b + a;
      };
      const busy = (S.lyrTr && !tr) || (S.roman && !rm);
      body = `<div class="ptools">
          <div class="l"><button class="obtn ${S.roman ? "on" : ""}" data-act="roman">romanize</button><button class="obtn ${S.lyrTr ? "on" : ""}" data-act="lytr">translate · ${S.trLang.toUpperCase()}</button>
            <button class="obtn" data-act="lysz" data-d="-1">a−</button><button class="obtn" data-act="lysz" data-d="1">a+</button>${L.synced ? `<button class="obtn ${S.blur ? "on" : ""}" data-act="blur">focus</button>` : ""}</div>
          <span class="caps">${L.synced ? "synced" : "not synced"} · ${esc(L.source)}${busy ? " · working…" : ""}${noRoman ? " · already latin letters" : ""}</span></div>
        ${(L.sources || []).length > 1 ? `<div class="chips srcs">${L.sources.map(s => `<button class="obtn ${s.name === L.source ? "on" : ""}" data-act="lysrc" data-s="${esc(s.name)}">${esc(s.name.toLowerCase())}${s.synced ? "" : " · plain"}</button>`).join("")}</div>` : ""}
        <div class="lyr ${L.synced ? "" : "plain"} ${S.blur ? "" : "noblur"}" ${L.synced ? "data-lyrics" : ""} style="--lsz:${S.lyrSize}"><div class="pad"></div>
          ${L.lines.map(([s, l], i) => `<p style="--i:${Math.min(i, 24)}" ${s != null ? `data-act="seekto" data-t="${s}"` : ""}>${esc(l || "♪")}${sub(i)}</p>`).join("")}<div class="pad"></div></div>`;
    }
  } else if (tab === "queue") {
    body = `<div class="qhead"><span class="caps">${S.queue.length - S.idx - 1} up next${S.qName ? " · from " + esc(S.qName) : ""}</span>
      <div style="display:flex;gap:8px"><button class="obtn ${S.autoplay ? "on" : ""}" data-act="auto">${I.radio}autoplay radio</button><button class="obtn" data-act="clearq">clear</button></div></div>
      <div class="qscroll">${rows(S.queue, "queue", { num: true, drag: true })}<div class="qhint" style="margin-top:12px">drag songs to reorder · autoplay radio keeps adding similar songs</div></div>`;
  } else {
    const r = need("rel:" + t.id, () => window.api.related(t.id));
    body = !r ? loading() : !r.ok ? errBox(r.e, "rel:" + t.id) : `<div class="qhead"><span class="caps">more like “${esc(t.title)}”</span><button class="obtn" data-act="radio1" data-k="queue" data-i="${S.idx}">${I.radio}start radio</button></div>
      <div class="qscroll">${r.v.map((s, i) => section(s, "rel" + i)).join("") || `<div class="empty">nothing related found.</div>`}</div>`;
  }
  return `<div class="panel inv"><div class="tabs">${["lyrics", "queue", "related"].map(x => `<button class="${tab === x ? "on" : ""}" data-act="np" data-t="${x}">${x}</button>`).join("")}</div>${body}</div>`;
};

function likedList() {
  if (S.signedIn) { const r = need("pl:LM", () => window.api.playlist("LM")); if (r?.ok) return { list: r.v.tracks, r }; return { list: null, r }; }
  return { list: [...S.liked.values()], r: { ok: true } };
}
P.library = () => {
  const t = S.libTab;
  let body = "";
  if (t === "playlists") {
    const yt = S.signedIn ? need("library", () => window.api.library()) : null;
    const ytCards = !S.signedIn ? "" : !yt ? loading() : !yt.ok ? errBox(yt.e, "library") : yt.v.map((p, i) => card({ ...p, type: "playlist" }, "lib", i)).join("");
    body = `<div class="cards"><div class="card" data-act="pl" data-id="liked">${art(null, "♥")}<div class="n">liked music</div><div class="s muted">${S.signedIn ? "synced with youtube" : S.liked.size + " songs"}</div></div>
      ${S.localPls.map(p => `<div class="card" data-act="pl" data-id="${esc(p.id)}">${mosaic(p.tracks)}<div class="n ell">${esc(p.name)}</div><div class="s muted">on this pc · ${pl(p.tracks.length, "song")}</div></div>`).join("")}
      ${typeof ytCards === "string" && ytCards.startsWith("<div class=\"loading") || ytCards.startsWith?.("<div class=\"err") ? "" : ytCards}
      <div class="card new" data-act="newpl">${'<div class="art">' + I.plus + "</div>"}<div class="n">new playlist</div><div class="s muted">${S.signedIn ? "also on youtube music" : "saved on this pc"}</div></div></div>
      ${S.signedIn && yt && !yt.ok ? errBox(yt.e, "library") : S.signedIn && !yt ? loading() : ""}`;
  }
  if (t === "library") {
    const rec = recent().slice(0, 10), mixes = mixSeeds([]);
    reg("mixes", mixes);
    body = `<div class="lgrid">
        <div class="ltile big" data-act="libtab" data-t="songs">${I.heartF}<div><div class="num">${likedCount()}</div><small>liked songs</small></div></div>
        <div class="ltile" data-act="go" data-p="downloads"><span class="ic">${I.dl}</span><div><b>offline</b><small>${S.downloads.size} downloaded</small></div></div>
        <div class="ltile" data-act="libtab" data-t="recent"><span class="ic">${I.sync}</span><div><b>recently played</b><small>${pl(S.plays.size || (S.signedIn && cache.get("me")?.ok ? cache.get("me").v.history.length : 0), "song")}</small></div></div>
        <div class="ltile" data-act="libtab" data-t="top"><span class="ic">${I.trend}</span><div><b>my top 50</b><small>all time</small></div></div>
        <div class="ltile" data-act="libtab" data-t="artists"><span class="ic">${I.user}</span><div><b>artists</b><small>${myArtists().length} · ${S.subs.size} followed</small></div></div>
      </div>
      ${rec.length ? `<div class="sec">${shead("recently played")}${cardsHtml(rec, "librec")}</div>` : ""}
      ${mixes.length ? `<div class="sec">${shead("top mixes for you")}<div class="cards">${mixes.map((m, i) => `<div class="card mixcard" data-act="radio1" data-k="mixes" data-i="${i}">${art(m.thumb, m.title).replace("</div>", `<span class="mixlbl ell">${esc(m.mixName)}</span></div>`)}<div class="n ell">${esc(m.mixName)}</div><div class="s muted">mix</div></div>`).join("")}</div></div>` : ""}
      ${!rec.length && !S.liked.size ? `<div class="empty">play and like songs — your library fills up here.</div>` : ""}`;
  }
  if (t === "songs") {
    const { list, r } = likedList();
    body = !list ? (!r ? loading() : errBox(r.e, "pl:LM")) : `<div class="phead"><div class="h2">liked songs · ${list.length}</div><div style="display:flex;gap:8px"><button class="obtn fill" data-act="playpl" data-id="liked">${I.play}play</button><button class="obtn" data-act="playpl" data-id="liked" data-shuf="1">${I.shuffle}shuffle</button></div></div>${rows(list, "pl:liked", { num: true, album: true })}`;
  }
  const ytHist = S.signedIn && cache.get("me")?.ok ? cache.get("me").v.history : [];
  if (t === "recent") { const l = S.plays.size ? recent().slice(0, 100) : ytHist; body = `<div class="phead"><div class="h2">recently played · ${l.length}</div>${S.plays.size ? "" : "<span class=caps>from your youtube history</span>"}</div>${rows(l, "recent", { album: true })}`; }
  if (t === "top") { const l = S.plays.size >= 10 ? top50() : [...new Map([...top50(), ...ytHist].map(x => [x.id, x])).values()].slice(0, 50); body = `<div class="phead"><div class="h2">my top 50 · all time</div>${l.length ? `<button class="obtn fill" data-act="playk" data-k="top">${I.play}play</button>` : ""}</div>${S.plays.size < 10 && ytHist.length ? "<div class=caps style=margin-bottom:12px>filled from your youtube music history until abyss learns what you play</div>" : ""}${rows(l, "top", { num: true, album: true })}`; }
  if (t === "artists") {
    const fol = S.signedIn ? need("following", () => window.api.following()) : null;
    const subsMap = new Map([...S.subs].map(s => JSON.parse(s)).map(a => [a.id, a]));
    if (fol?.ok) fol.v.forEach(a => subsMap.set(a.id, { id: a.id, title: a.title, thumb: a.thumb }));
    const all = myArtists(), subs = [...subsMap.values()], list = S.artTab === "subscribed" ? subs : all;
    const hero = all[0] || subs[0];
    const songsN = new Set([...S.liked.keys(), ...S.plays.keys()]).size;
    body = `<div class="yart">${artistArt(hero)}<div><div class="hsub">library</div><div class="h1" style="font-size:52px">your artists.</div>
        <div class="counts"><div><b>${all.length}</b><span class="caps">artists</span></div><div><b>${subs.length}</b><span class="caps">followed</span></div><div><b>${songsN}</b><span class="caps">songs</span></div></div></div>
        <button class="obtn fill rand" data-act="randart" ${all.length || subs.length ? "" : "disabled"}>${I.shuffle}random artist</button></div>
      ${S.signedIn && !fol ? loading("loading the artists you follow on youtube") : ""}
      <div class="tabs light" style="margin-bottom:22px">${[["subscribed", `followed · ${subs.length}`], ["all", `all artists · ${all.length}`]].map(([k, n]) => `<button class="${(S.artTab || "all") === k ? "on" : ""}" data-act="arttab" data-t="${k}">${n}</button>`).join("")}</div>
      ${list.length ? `<div class="cards">${list.map(a => `<div class="card round" data-act="artist" data-id="${esc(a.id)}">${artistArt(a)}<div class="n ell">${esc(a.title)}</div><div class="s muted">${a.songs ? a.songs + " songs" : "followed"}</div></div>`).join("")}</div>`
        : `<div class="empty">${S.artTab === "subscribed" ? "press subscribe on an artist page to follow them." : "play some music and your artists show up here."}</div>`}`;
  }
  if (t === "albums") {
    const al = myAlbums();
    body = al.length ? `<div class="phead"><div class="h2">albums · ${al.length}</div></div><div class="cards">${al.map(a => `<div class="card" data-act="album" data-id="${esc(a.id)}">${art(a.thumb, a.title)}<div class="n ell">${esc(a.title)}</div><div class="s muted ell">${esc(a.artist)} · ${pl(a.songs, "song")}</div></div>`).join("")}</div>` : `<div class="empty">albums from songs you play and like show up here.</div>`;
  }
  return `${signBanner()}<div class="phead"><div class="h1">library.</div>${syncBtn()}</div>
    <div class="lchips">${[["library", "library"], ["playlists", "playlists"], ["songs", "songs"], ["artists", "artists"], ["albums", "albums"]].map(([k, n]) => `<button class="obtn ${t === k || (k === "library" && ["recent", "top"].includes(t)) ? "on" : ""}" data-act="libtab" data-t="${k}">${n}</button>`).join("")}</div>${body}`;
};

/* ---- history, derived artists/albums ---- */
function logPlay(t) {
  const p = S.plays.get(t.id) || { t, n: 0, last: 0 };
  p.t = t; p.n++; p.last = Date.now();
  S.plays.set(t.id, p);
  if (S.plays.size > 800) { const old = [...S.plays.entries()].sort((a, b) => a[1].last - b[1].last)[0]; S.plays.delete(old[0]); }
  store.set("plays", [...S.plays]);
}
function forYouSeeds() {
  const now = Date.now();
  return [...S.plays.values()].map(p => ({ t: p.t, s: p.n * Math.exp(-(now - p.last) / (3 * 864e5)) })).sort((a, b) => b.s - a.s).slice(0, 3).map(x => x.t).filter(t => t?.id);
}
const recent = () => [...S.plays.values()].sort((a, b) => b.last - a.last).map(p => p.t);
const top50 = () => [...S.plays.values()].sort((a, b) => b.n - a.n).slice(0, 50).map(p => p.t);
const likedCount = () => (S.signedIn && cache.get("pl:LM")?.ok ? cache.get("pl:LM").v.tracks.length : S.liked.size);
function myArtists() {
  const m = new Map();
  for (const t of [...S.liked.values(), ...recent()]) {
    if (!t?.artistId) continue;
    const e = m.get(t.artistId) || { id: t.artistId, title: t.artist.split(",")[0].trim(), songs: 0 };
    e.songs++; m.set(t.artistId, e);
  }
  return [...m.values()].sort((a, b) => b.songs - a.songs);
}
function myAlbums() {
  const m = new Map();
  for (const t of [...S.liked.values(), ...recent()]) {
    if (!t?.albumId) continue;
    const e = m.get(t.albumId) || { id: t.albumId, title: t.album, artist: t.artist.split(",")[0], thumb: t.thumb, songs: 0 };
    e.songs++; m.set(t.albumId, e);
  }
  return [...m.values()].sort((a, b) => b.songs - a.songs);
}
/* artist photos load lazily (4 at a time) from the artist page */
function artistArt(a) {
  if (!a) return art(null, "♪");
  if (a.thumb) return art(a.thumb, a.title);
  const r = need("ar:" + a.id, () => pool.run(() => window.api.artist(a.id)));
  return art(r?.ok ? r.v.thumb : null, a.title);
}
const pool = {
  q: [], active: 0,
  run(fn) { return new Promise((res, rej) => { this.q.push([fn, res, rej]); this.next(); }); },
  next() { while (this.active < 4 && this.q.length) { const [fn, res, rej] = this.q.shift(); this.active++; fn().then(res, rej).finally(() => { this.active--; this.next(); }); } },
};

/* ---- mood & genres grid (search page) ---- */
const PAL = ["#f08a4b", "#3aa7d6", "#e8603c", "#ef8fa6", "#f2cf4a", "#79d98f", "#9b6cff", "#c9cdd1", "#e2483a", "#ff7ae0", "#45c9a0", "#5aa0ff"];
const SIZES = ["tall", "short", "mid", "short", "mid", "tall", "short", "mid"];
const SHAPE_NAMES = ["squircle", "blob", "scallop", "heart", "diamond", "pill", "star", "wave"];
function moodsGrid() {
  const r = need("moods", () => window.api.moods());
  const body = !r ? loading("loading moods & genres") : !r.ok ? errBox(r.e, "moods") : `<div class="moods">${r.v.map((m, i) => {
    const g = need("g:" + m.browseId + (m.params || ""), () => pool.run(() => window.api.genre(m.browseId, m.params)));
    const thumb = g?.ok ? g.v.thumb : null;
    return `<div class="mood ${SIZES[i % SIZES.length]}" style="--mc:${m.color || PAL[i % PAL.length]};--clip:url(#cp-${SHAPE_NAMES[i % SHAPE_NAMES.length]})" data-act="genre" data-i="${i}">
      <div class="mt">${esc(m.title)}</div>
      <div class="shape"><div class="ly"><i></i></div><div class="ly"><i></i></div><div class="ly"><i></i></div><div class="ly img"><i></i>${thumb ? `<img src="${esc(thumb)}" alt="" referrerpolicy="no-referrer">` : ""}</div></div></div>`;
  }).join("")}</div>`;
  return `<div class="phead"><div class="h1">mood & genres.</div><span class="caps">type above to search songs, artists, albums</span></div>${body}`;
}
P.genre = () => {
  const m = cache.get("moods")?.v?.[S.params.i];
  if (!m) return loading();
  const key = "g:" + m.browseId + (m.params || ""), g = need(key, () => window.api.genre(m.browseId, m.params));
  const color = m.color || PAL[S.params.i % PAL.length];
  return `<div class="ghead" style="--mc:${color}"><span class="sw3"></span><div class="h1">${lc(m.title)}.</div></div>
    ${!g ? loading() : !g.ok ? errBox(g.e, key) : g.v.sections.map((s, i) => section(s, `gen${i}`)).join("") || `<div class="empty">nothing here right now.</div>`}`;
};

/* clip-path shapes (objectBoundingBox units) for the mood cards */
(function shapes() {
  const sc = (n, a, b) => { let d = ""; for (let i = 0; i <= n * 2; i++) { const t = (i / (n * 2)) * Math.PI * 2, r = i % 2 ? b : a, x = (0.5 + Math.cos(t) * r).toFixed(3), y = (0.5 + Math.sin(t) * r).toFixed(3); d += i === 0 ? `M${x} ${y}` : i % 2 ? ` Q${x} ${y}` : ` ${x} ${y}`; } return d + "Z"; };
  const star = (n, a, b) => Array.from({ length: n * 2 }, (_, i) => { const t = (i / (n * 2)) * Math.PI * 2 - Math.PI / 2, r = i % 2 ? b : a; return `${i ? "L" : "M"}${(0.5 + Math.cos(t) * r).toFixed(3)} ${(0.5 + Math.sin(t) * r).toFixed(3)}`; }).join("") + "Z";
  const D = {
    squircle: "M.5 0C.92 0 1 .08 1 .5S.92 1 .5 1 0 .92 0 .5.08 0 .5 0Z",
    blob: "M.55 .02C.8 .03 .99 .22 .98 .5C.97 .78 .76 .99 .48 .98C.2 .97 .02 .76 .03 .47C.04 .2 .3 0 .55 .02Z",
    scallop: sc(12, 0.44, 0.52),
    heart: "M.5 .96C.18 .72 0 .52 0 .3A.25 .25 0 0 1 .5 .2A.25 .25 0 0 1 1 .3C1 .52 .82 .72 .5 .96Z",
    diamond: "M.5 .02Q.56 .02 .62 .1L.94 .44Q.98 .5 .94 .56L.62 .9Q.56 .98 .5 .98Q.44 .98 .38 .9L.06 .56Q.02 .5 .06 .44L.38 .1Q.44 .02 .5 .02Z",
    pill: "M.5 .12C.86 .1 1 .3 1 .5S.86 .9 .5 .88 0 .7 0 .5.14 .14 .5 .12Z",
    star: star(8, 0.5, 0.38),
    wave: sc(8, 0.46, 0.5),
  };
  document.body.insertAdjacentHTML("beforeend", `<svg width="0" height="0" style="position:absolute" aria-hidden="true"><defs>${Object.entries(D).map(([k, d]) => `<clipPath id="cp-${k}" clipPathUnits="objectBoundingBox"><path d="${d}"/></clipPath>`).join("")}</defs></svg>`);
})();
const syncBtn = () => `<div style="display:flex;gap:8px;align-items:center"><span class="caps">${S.signedIn ? `synced · ${S.lastSync}` : "not signed in"}</span>
  <button class="obtn ${S.syncing ? "on" : ""}" data-act="sync">${I.sync}${S.syncing ? "syncing…" : "sync with youtube"}</button></div>`;

P.playlists = () => {
  const yt = S.signedIn ? need("library", () => window.api.library()) : null;
  const row = (id, thumbHtml, name, sub) => `<div class="row" data-act="pl" data-id="${esc(id)}">${thumbHtml}<div class="meta"><div class="ti">${esc(name)}</div><div class="su">${esc(sub)}</div></div><button class="ibtn box" data-act="playpl" data-id="${esc(id)}" title="play">${S.qKey === "pl:" + id && S.playing ? I.pause : I.play}</button></div>`;
  return `${signBanner()}<div class="phead"><div class="h1">playlists.</div><div style="display:flex;gap:8px;flex-wrap:wrap"><button class="obtn fill" data-act="newpl">${I.plus}new playlist</button>${syncBtn()}</div></div>
    <div class="rows">${row("liked", art(null, "♥"), "Liked Music", S.signedIn ? "auto playlist · synced with youtube" : `auto playlist · ${pl(S.liked.size, "song")}`)}
      ${S.localPls.map(p => row(p.id, mosaic(p.tracks), p.name, `on this pc · ${pl(p.tracks.length, "song")}`)).join("")}
      ${!S.signedIn ? "" : !yt ? "" : yt.ok ? yt.v.map(p => row(p.id, art(p.thumb, p.title), p.title, (p.sub || "playlist") + " · youtube")).join("") : ""}</div>
    ${S.signedIn && !yt ? loading("loading your youtube playlists") : S.signedIn && yt && !yt.ok ? errBox(yt.e, "library") : ""}`;
};

P.downloads = () => {
  const list = [...S.downloads.values()].sort((a, b) => b.at - a.at);
  const mb = (list.reduce((s, d) => s + (d.size || 0), 0) / 1048576).toFixed(1);
  return `<div class="phead"><div class="h1">downloads.</div><div style="display:flex;gap:8px"><button class="obtn fill" data-act="playdl">${I.play}play all</button><button class="obtn" data-act="opendl">${I.folder}open folder</button></div></div>
    <div class="stat"><div><b>${list.length}</b><span class="caps">songs</span></div><div><b>${mb} MB</b><span class="caps">on this pc</span></div><div><b>offline</b><span class="caps">no internet needed</span></div></div>
    ${list.length ? `<div class="rows">${reg("downloads", list) && list.map((t, i) => `<div class="row ${cur()?.id === t.id ? "cur" : ""}" data-act="play" data-k="downloads" data-i="${i}">${art(t.thumb, t.title)}<div class="meta"><div class="ti">${esc(t.title)}</div><div class="su">${esc(t.artist)}${t.album ? " · " + esc(t.album) : ""}</div></div><span class="du">${fmt(t.dur)}</span><button class="ibtn" data-act="rmdl" data-id="${esc(t.id)}" title="remove download">${I.trash}</button></div>`).join("")}</div>`
      : `<div class="empty">press “download” on the now playing panel to save a song.</div>`}`;
};

function getLocal(id) { return S.localPls.find(p => p.id === id); }
P.playlist = () => {
  const id = S.params.id;
  let head = null, list = null, state = null;
  if (id === "liked") { const r = likedList(); list = r.list; state = r.r; head = { title: "liked music", sub: S.signedIn ? "synced with youtube music" : "saved on this pc", kind: "auto playlist" }; }
  else if (id.startsWith("local:")) { const p = getLocal(id); if (!p) return `<div class="empty">playlist not found.</div>`; list = p.tracks; state = { ok: true }; head = { title: p.name, sub: `on this pc · ${pl(p.tracks.length, "song")}`, kind: "playlist", local: true }; }
  else { state = need("pl:" + id, () => window.api.playlist(id)); if (state?.ok) { list = state.v.tracks; head = { title: state.v.title, sub: state.v.sub, desc: state.v.desc, thumb: state.v.thumb, kind: "playlist" }; } }
  if (!state) return loading("loading playlist");
  if (!state.ok) return errBox(state.e, id === "liked" ? "pl:LM" : "pl:" + id);
  const key = "pl:" + id, mins = Math.round(list.reduce((s, t) => s + (t.dur || 0), 0) / 60);
  return `<div class="dhead">${head.thumb ? art(head.thumb, head.title) : mosaic(list.length ? list : [{ title: head.title }])}<div><span class="caps">${head.kind}</span>
      <div class="h1">${lc(head.title)}.</div><div class="meta"><span>${esc(head.sub || "")}</span><span>· ${pl(list.length, "song")}${mins ? ` · ${mins} min` : ""}</span></div>
      <div class="acts"><button class="obtn fill" data-act="playpl" data-id="${esc(id)}">${S.qKey === key && S.playing ? I.pause + "pause" : I.play + "play"}</button>
        <button class="obtn" data-act="playpl" data-id="${esc(id)}" data-shuf="1">${I.shuffle}shuffle</button>
        <button class="obtn" data-act="dlall" data-id="${esc(id)}">${I.dl}download all</button>
        ${head.local ? `<button class="obtn" data-act="delpl" data-id="${esc(id)}">${I.trash}delete</button>` : `<button class="obtn" data-act="retry" data-k="${esc(id === "liked" ? "pl:LM" : "pl:" + id)}">${I.sync}refresh</button>`}</div></div></div>
    ${rows(list, key, { num: true, album: true })}`;
};
P.album = () => {
  const r = need("al:" + S.params.id, () => window.api.album(S.params.id));
  if (!r) return loading("loading album");
  if (!r.ok) return errBox(r.e, "al:" + S.params.id);
  const a = r.v, key = "al:" + a.id;
  return `<div class="dhead">${art(a.thumb, a.title)}<div><span class="caps">${esc(a.sub || "album")}</span><div class="h1">${lc(a.title)}.</div>
    <div class="meta">${a.tracks[0]?.artistId ? `<a data-act="artist" data-id="${esc(a.tracks[0].artistId)}">${esc(a.artist)}</a>` : esc(a.artist)}<span>· ${pl(a.tracks.length, "song")}</span></div>
    <div class="acts"><button class="obtn fill" data-act="playal">${S.qKey === key && S.playing ? I.pause + "pause" : I.play + "play"}</button><button class="obtn" data-act="playal" data-shuf="1">${I.shuffle}shuffle</button>
      <button class="obtn" data-act="dlall" data-id="${esc(key)}">${I.dl}download all</button></div></div></div>
    ${rows(a.tracks, key, { num: true })}`;
};
P.artist = () => {
  const r = need("ar:" + S.params.id, () => window.api.artist(S.params.id));
  if (!r) return loading("loading artist");
  if (!r.ok) return errBox(r.e, "ar:" + S.params.id);
  const a = r.v, subKey = JSON.stringify({ id: a.id, title: a.name, thumb: a.thumb }), sub = S.subs.has(subKey);
  const key = "arsongs:" + a.id, showAll = S.artAll === a.id, list = showAll ? a.songs : a.songs.slice(0, 10);
  reg(key, a.songs);
  const st = a.stats || {}, inLib = a.songs.filter(s => S.liked.has(s.id)).length, plays = a.songs.reduce((n, s) => n + (S.plays.get(s.id)?.n || 0), 0);
  const isVid = s => /video|live|performance/i.test(s.title) && s.items.every(x => x.type === "song");
  const secHtml = (s, i) => {
    const k = `ar${i}:${a.id}`;
    if (isVid(s)) return `<div class="sec">${shead(s.title.toLowerCase())}${cardsHtml(s.items, k, 10).replace(/class="card"/g, 'class="card wide"')}</div>`;
    return section(s, k);
  };
  return `<div class="ahero">
      <div class="aban">${a.banner ? `<img src="${esc(a.banner)}" alt="" referrerpolicy="no-referrer">` : ""}<div class="ashade"></div>
        <div class="aname"><span class="caps">artist${a.sub ? " · " + esc(a.sub) : ""}</span><div class="h1">${lc(a.name)}.</div></div></div>
      <div class="arow">
        <div class="aacts">
          ${a.songs.length ? `<button class="obtn fill" data-act="playart" data-id="${esc(a.id)}">${I.play}play</button><button class="obtn" data-act="playart" data-id="${esc(a.id)}" data-shuf="1">${I.shuffle}shuffle</button><button class="obtn" data-act="radio1" data-k="${esc(key)}" data-i="0">${I.radio}radio</button>` : ""}
          <button class="obtn ${sub ? "on" : ""}" data-act="sub" data-v="${esc(subKey)}">${sub ? I.check + "subscribed" : I.plus + "subscribe"}</button>
        </div>
        <div class="astats">
          <div><b>${st.songs || a.songs.length}</b><span class="caps">songs</span></div>
          <div><b>${st.albums || 0}</b><span class="caps">albums</span></div>
          <div><b>${st.singles || 0}</b><span class="caps">singles</span></div>
          <div><b>${inLib}</b><span class="caps">you liked</span></div>
          <div><b>${plays}</b><span class="caps">your plays</span></div>
        </div>
      </div>
    </div>
    <div class="agrid">
      <div class="sec" style="margin-top:0">${shead("popular", `${a.songs.length} songs`, a.songs.length > 10 ? `<button class="obtn" data-act="artall" data-id="${esc(a.id)}">${showAll ? "show top 10" : `show all ${a.songs.length}`}</button>` : "")}
        ${rows(list, key, { num: true, album: true })}</div>
      ${a.desc ? `<div class="about ${S.artBio === a.id ? "open" : ""}"><div class="hsub">about</div><div class="h2">${lc(a.name)}</div><p>${esc(a.desc)}</p>
        <button class="obtn" data-act="artbio" data-id="${esc(a.id)}">${S.artBio === a.id ? "show less" : "read more"}</button></div>` : ""}
    </div>
    ${a.sections.map(secHtml).join("")}`;
};
P.eq_removed = () => `<div class="err" style="margin:0 0 20px"><span>songs now play through youtube's official player, so only volume and speed change the sound — the equalizer bands can't reach inside it.</span></div><div class="phead"><div class="h1">equalizer.</div><div style="display:flex;gap:10px;align-items:center"><span class="caps">${S.eqOn ? "on" : "off"}</span>${sw(S.eqOn, "eqon")}</div></div>
  <div class="box"><div class="chips">${Object.keys(EQ_PRESETS).concat(S.eqPreset === "custom" ? ["custom"] : []).map(n => `<button class="obtn ${S.eqPreset === n ? "on" : ""}" data-act="eqp" data-n="${n}">${n}</button>`).join("")}</div>
    <div class="bands">${EQ_FREQS.map((f, i) => `<div class="band"><span class="db" data-db="${i}">${S.eq[i] > 0 ? "+" : ""}${S.eq[i]}</span><input type="range" min="-12" max="12" step="1" value="${S.eq[i]}" data-band="${i}"><span>${f >= 1000 ? f / 1000 + "k" : f}</span></div>`).join("")}</div></div>
  <div class="boxes">
    <div class="box"><div class="t">volume<span data-bind="volnum">${Math.round(S.vol * 100)}</span></div><div class="volrow" style="margin:0"><div class="volseg" data-drag="volh">${Array.from({ length: 20 }, (_, i) => `<i style="height:${35 + (65 * i) / 19}%"></i>`).join("")}</div></div><small>same as the panel volume and the deck VOL fader.</small></div>
    <div class="box"><div class="t">tone<span data-bind="tonev">${Math.round(S.tone * 100)}</span></div><input type="range" min="0" max="100" value="${Math.round(S.tone * 100)}" data-fx="tone"><small>100 = full sound. lower = warmer, duller.</small></div>
    <div class="box"><div class="t">speed<span data-fxv="speed">${S.speed.toFixed(2)}×</span></div><input type="range" min="50" max="200" step="5" value="${S.speed * 100}" data-fx="speed"><small>keeps the pitch the same.</small></div>
  </div>`;
const FONTS = [["", "theme font"], ["Pixelify Sans", "pixel"], ["Bungee", "bungee"], ["Playfair Display", "playfair"], ["Fredoka", "fredoka"], ["Orbitron", "orbitron"], ["VT323", "terminal"], ["Space Mono", "mono"], ["Anton", "anton"], ["Inter", "clean"]];
const customCard = () => `<button class="tcard ${S.theme === "custom" ? "on" : ""}" data-act="theme" data-t="custom"><div class="sw2"><span style="background:${esc(S.custom.paper)};color:${esc(S.custom.ink)}">a</span><span style="background:${esc(S.custom.ink)};color:${esc(S.custom.paper)}">a</span></div><div class="tn">your colors${S.theme === "custom" ? I.check : ""}</div></button>`;
const TFONT = { vice: "Bungee", deep: "Orbitron", latte: "Playfair Display", matrix: "VT323", sakura: "Fredoka" };
const tcard = ([id, n, p, k]) => `<button class="tcard ${S.theme === id ? "on" : ""}" data-act="theme" data-t="${id}" style="--tf:${TFONT[id] ? `&quot;${TFONT[id]}&quot;` : `&quot;Pixelify Sans&quot;`}"><div class="sw2"><span style="background:${p};color:${k}">a</span><span style="background:${k};color:${p}">a</span></div><div class="tn">${n.toLowerCase()}${S.theme === id ? I.check : ""}</div></button>`;
P.settings = () => `<div class="phead"><div class="h1">settings.</div></div>
  <div class="sbox"><div class="h2">account</div>
    ${S.signedIn
      ? `<div class="srow"><div><b>signed in · ${esc(S.account?.name || "google account")}</b><small>youtube music library connected · last sync ${S.lastSync}</small></div><button class="obtn" data-act="signout">sign out</button></div>`
      : `<div class="srow"><div><b>not signed in</b><small>sign in to sync your youtube music playlists, likes and mixes.</small></div><button class="gbtn" data-act="signin"><span class="g"></span>sign in with google</button></div>`}
    <div class="srow"><div><b>likes sync with youtube</b><small>liking a song here likes it on youtube music too.</small></div><span class="caps">${S.signedIn ? "on" : "sign in first"}</span></div>
    <div class="srow"><div><b>playlists sync with youtube</b><small>new playlists and added songs show up on youtube music.</small></div><span class="caps">${S.signedIn ? "on" : "sign in first"}</span></div></div>
  <div class="sbox"><div class="h2">theme</div>
    <div class="themes">${THEMES.filter(t => t[4]).map(tcard).join("")}</div>
    <div class="srow" style="border:0;padding-bottom:6px"><div><b>more themes</b></div></div>
    <div class="themes more">${THEMES.filter(t => !t[4]).map(tcard).join("")}${customCard()}</div>
    <div class="srow"><div><b>your own colors</b><small>pick a background and a text color — abyss builds a two-tone theme from them.</small></div><div class="ccpick"><label>background<input type="color" data-cc="paper" value="${esc(S.custom.paper)}"></label><label>text<input type="color" data-cc="ink" value="${esc(S.custom.ink)}"></label></div></div>
    <div class="srow" style="flex-wrap:wrap"><div><b>font</b><small>use any font with any theme. “theme font” keeps each theme's own.</small></div><div class="fontpick">${FONTS.map(([f, n]) => `<button class="obtn ${S.font === f ? "on" : ""}" data-act="font" data-f="${esc(f)}" style="font-family:${f ? `&quot;${f}&quot;` : "inherit"}">${n}</button>`).join("")}</div></div>
    <div class="srow"><div><b>gray covers</b><small>show album covers in black & white (fits the white and dark themes).</small></div>${sw(S.gray, "gray")}</div></div>
  <div class="sbox"><div class="h2">lyrics</div>
    <div class="srow"><div><b>sources</b><small>LRCLIB (synced) first, then YouTube Music.</small></div><span class="caps">auto</span></div>
    <div class="srow"><div><b>translate to</b></div><div class="selrow">${LANGS.map(([c, n]) => `<button class="obtn ${S.trLang === c ? "on" : ""}" data-act="trlang" data-l="${c}">${n}</button>`).join("")}</div></div></div>
  <div class="sbox"><div class="h2">playback</div>
    <div class="srow"><div><b>no ads</b><small>always. abyss never plays ads.</small></div><span class="caps">always on</span></div>
    <div class="srow"><div><b>smooth transitions</b><small>fade one song into the next so there's no silence between them.</small></div><div class="seg">${[[0, "off"], [2, "2s"], [5, "5s"], [7, "7s"], [12, "12s"]].map(([v, n]) => `<button class="${S.crossfade === v ? "on" : ""}" data-act="xfade" data-v="${v}">${n}</button>`).join("")}</div></div>
    <div class="srow"><div><b>speed</b><small>play songs slower or faster (pitch stays the same).</small></div><div class="seg">${[0.75, 1, 1.25, 1.5].map(v => `<button class="${S.speed === v ? "on" : ""}" data-act="speed" data-v="${v}">${v}×</button>`).join("")}</div></div>
    <div class="srow"><div><b>mini player</b><small>a small old-school player (winamp style) with a skin for every theme. ctrl+m switches.</small></div><button class="obtn" data-act="mini-on">${I.mini}open mini player</button></div>
    <div class="srow"><div><b>mini player stays on top</b><small>keep the mini player above other windows.</small></div>${sw(S.miniTop, "minitop")}</div>
    <div class="srow"><div><b>autoplay radio</b><small>keep playing similar songs when the queue ends.</small></div>${sw(S.autoplay, "auto")}</div>
    <div class="srow"><div><b>stream quality</b><small>${esc(streamQ || "plays at the best audio youtube gives this app (AAC ~128 kbps).")}</small></div><span class="caps">auto</span></div></div>
  <div class="sbox"><div class="h2">integrations</div>
    <div class="srow"><div><b>discord status</b><small>shows “listening to …” with the song, cover and time on your discord profile. the discord app on this pc must be open.</small></div>${sw(S.discord && S.discordOk, "discord")}</div>
    <div class="srow"><div><b>last.fm scrobbling</b><small>log every song to last.fm.</small></div><span class="soon">coming soon</span></div></div>
  <div class="sbox"><div class="h2">storage</div>
    <div class="srow"><div><b>downloads · ${pl(S.downloads.size, "song")}</b><small>saved inside the app's data folder.</small></div><button class="obtn" data-act="opendl">${I.folder}open folder</button></div></div>
  <div class="sbox"><div class="h2">about</div>
    <div class="srow"><div><b>abyss ${esc(S.upd?.current || "")}</b><small>${esc(updLine())}</small></div><div style="display:flex;gap:8px">${S.upd?.latest?.newer ? `<button class="obtn fill" data-act="upd-show">${I.dl}see update</button>` : ""}<button class="obtn" data-act="upd-check">${I.sync}check for updates</button></div></div>
    <div class="srow"><div><b>announcements</b><small>${S.news.length ? `${pl(S.news.length, "announcement")} from the abyss team.` : "news from the abyss team shows up at the top of the app."}</small></div>${S.seenNews.size ? `<button class="obtn" data-act="news-reset">show dismissed again</button>` : ""}</div>
    <div class="srow"><div><b>open source</b><small>keys: space play · ←/→ seek · ↑/↓ volume · m mute · l lyrics · t theme · ctrl+f search</small></div><button class="obtn" data-act="openurl" data-u="https://github.com/yumekuse/Abyss-Music">github</button></div></div>`;
let streamQ = "";

function modal() {
  if (S.modal === "signin") return `<div class="modal-bg" data-act="modal-x"><div class="modal" data-act="noop">
    <div class="h2">sign in with google</div>
    <div>you sign in on google's own site in your normal browser — abyss never sees your password. your login is saved encrypted on this pc.</div>
    <button class="pick" data-act="signin-code"><b>1 · sign in with a code</b><small>recommended. go to google.com/device and type a short code. same way smart tvs sign in.</small></button>
    <button class="pick" data-act="signin-paste"><b>2 · paste your login from the browser</b><small>backup. copy your music.youtube.com cookie and paste it here.</small></button>
    <div class="r"><button class="obtn" data-act="modal-x">cancel</button></div></div></div>`;
  if (S.modal === "signin-code") return `<div class="modal-bg"><div class="modal" data-act="noop">
    <div class="h2">sign in with a code</div>
    ${S.authCode ? `<div>1. open <b>google.com/device</b> in your browser<br>2. type this code and pick your youtube account<br>3. press allow — abyss finishes by itself</div>
      <div class="devcode" data-act="copycode" title="click to copy">${esc(S.authCode.code)}</div>
      <div class="r" style="justify-content:space-between"><button class="obtn" data-act="signin-cancel">cancel</button>
        <div style="display:flex;gap:8px"><button class="obtn" data-act="copycode">copy code</button><button class="obtn fill" data-act="opendevice">open google.com/device</button></div></div>
      <div class="loading" style="padding:4px 0 0">waiting for you to approve…</div>`
      : `<div class="loading">getting a code from google…</div><div class="r"><button class="obtn" data-act="signin-cancel">cancel</button></div>`}</div></div>`;
  if (S.modal === "signin-paste") return `<div class="modal-bg"><div class="modal wide" data-act="noop">
    <div class="h2">paste your login</div>
    <div class="steps">1. open <b>music.youtube.com</b> in chrome / edge / firefox and make sure you're signed in<br>
      2. press <b>F12</b> → <b>Network</b> tab → reload the page → click any request named <b>browse</b> or <b>next</b><br>
      3. under <b>Request Headers</b>, find <b>cookie</b>, right-click its value → copy (or copy all request headers)<br>
      4. paste it below and press save</div>
    <textarea class="txt" id="cookiebox" rows="6" placeholder="paste the cookie value or the request headers here" spellcheck="false"></textarea>
    <div class="muted" style="font-weight:700;font-size:12px">this is your login — never paste it anywhere else or share it. abyss stores it encrypted on this pc only.</div>
    <div class="r"><button class="obtn" data-act="signin">back</button><button class="obtn fill" data-act="paste-ok">save & sign in</button></div></div></div>`;
  if (S.modal === "upd" && S.upd?.latest) {
    const L = S.upd.latest, ph = S.upd.phase;
    return `<div class="modal-bg" data-act="modal-x"><div class="modal wide" data-act="noop">
      <div class="h2">what's new in abyss ${esc(L.version)}</div>
      <div class="muted" style="font-weight:700">you have ${esc(S.upd.current)}${L.published ? ` · released ${esc(String(L.published).slice(0, 10))}` : ""}</div>
      <div class="notes">${md(L.notes || "no notes for this release.")}</div>
      ${ph === "downloading" || ph === "installing" ? updProgress() : ""}
      ${S.upd.error ? `<div class="dln">${esc(S.upd.error)}</div>` : ""}
      <div class="r"><button class="obtn" data-act="modal-x">later</button>${updButton()}</div></div></div>`;
  }
  if (S.modal === "newpl") return `<div class="modal-bg" data-act="modal-x"><div class="modal" data-act="noop">
    <div class="h2">new playlist</div><input class="txt" id="plname" placeholder="playlist name" maxlength="80">
    <div class="muted" style="font-weight:700">${S.signedIn ? "it will also be created on youtube music." : "saved on this pc. sign in to put it on youtube music."}</div>
    <div class="r"><button class="obtn" data-act="modal-x">cancel</button><button class="obtn fill" data-act="newpl-ok">create</button></div></div></div>`;
  if (S.modal === "addpl") {
    const t = S.modalTrack, yt = S.signedIn ? need("library", () => window.api.library()) : null;
    return `<div class="modal-bg" data-act="modal-x"><div class="modal" data-act="noop">
      <div class="h2">add to playlist</div><div class="muted" style="font-weight:700">${esc(t.title)} · ${esc(t.artist)}</div>
      <div class="list-pick">
        ${S.localPls.map(p => `<button data-act="addpl-ok" data-id="${esc(p.id)}">${mosaic(p.tracks)}${esc(p.name)}<span class="caps" style="margin-left:auto">pc</span></button>`).join("")}
        ${yt?.ok ? yt.v.filter(p => !/^VL(LM|SE)$/.test(p.id)).map(p => `<button data-act="addpl-ok" data-id="${esc(p.id)}">${art(p.thumb, p.title)}${esc(p.title)}<span class="caps" style="margin-left:auto">youtube</span></button>`).join("") : S.signedIn ? loading() : ""}
        <button data-act="newpl">${art(null, "+")}new playlist…</button>
      </div>
      <div class="r"><button class="obtn" data-act="modal-x">close</button></div></div></div>`;
  }
  return "";
}

/* ================= mini player (winamp / aimp style) ================= */
// window sizes (logical px); the app window is resized to exactly this
const MINI_W = 520, MINI_H = 244, MINI_H_PL = 500;
Object.assign(I, {
  mini: sv('<rect x="3" y="5" width="18" height="14" rx="1"/><rect x="12" y="11" width="7" height="6" class="f"/>'),
  stop: sv('<rect x="6" y="6" width="12" height="12"/>', 1),
  winMin: sv('<path d="M6 17h12"/>'), winFull: sv('<rect x="5" y="5" width="14" height="14"/>'), pin: sv('<path d="M12 17v5M8 3h8l-1 6 3 4H6l3-4z"/>'),
});
S.mini = store.get("mini", false); S.miniPanel = store.get("miniPanel", store.get("miniPL", false) ? "pl" : null);
S.mq = ""; S.mres = { state: "idle", items: [] }; S.miniTop = store.get("miniTop", true); S.miniRemain = false;
// skin per theme: cream = aimp modern, dark = classic winamp, vice = red winamp, sakura = pink, latte = bronze, white = silver lcd
const SKINS = { cream: ["aimp", "abyss modern"], dark: ["winamp", "abyss classic"], vice: ["vice", "abyss inferno"], sakura: ["sakura", "abyss sakura"], latte: ["latte", "abyss bronze"], white: ["silver", "abyss silver"], custom: ["custom", "abyss custom"] };
const winMsg = m => { try { window.ipc && window.ipc.postMessage(m); } catch {} };
const miniSize = () => ({ mini: S.mini, w: MINI_W, h: S.miniPanel ? MINI_H_PL : MINI_H, onTop: S.miniTop });
function miniPanel(p) {
  S.miniPanel = S.miniPanel === p ? null : p;
  store.set("miniPanel", S.miniPanel); store.set("miniPL", !!S.miniPanel); // miniPL: the window opens tall next time
  winMsg("mode:" + JSON.stringify(miniSize()));
  render();
  if (S.miniPanel === "pl") setTimeout(scrollPL, 30);
  if (S.miniPanel === "sr") setTimeout(() => document.getElementById("mq")?.focus(), 30);
}
async function miniSearch() {
  const q = S.mq.trim();
  if (!q) return;
  S.mres = { state: "loading", q, items: [] }; render();
  try {
    const res = await window.api.search(q);
    const songs = res.find(x => x.kind === "song")?.items || [], vids = res.find(x => x.kind === "video")?.items || [];
    S.mres = { state: "ok", q, items: [...songs, ...vids.slice(0, 8)].filter(t => t.type === "song") };
  } catch (err) { S.mres = { state: "err", q, items: [], e: err.message }; }
  render();
}
// put songs from mini search into the queue: "now" | "next" | "end"
function miniQueue(t, where) {
  if (!t) return;
  const item = { ...t };
  if (!cur()) { S.queue = [item]; S.idx = 0; S.qKey = "queue"; S.qName = "your queue"; load(); return toast(`playing “${t.title}”`); }
  if (where === "end") { S.queue.push(item); toast(`added “${t.title}”`); }
  else { S.queue.splice(S.idx + 1, 0, item); if (where === "now") { S.idx++; load(); } else toast(`“${t.title}” plays next`); }
  saveSession(); render();
}
function setMini(on) {
  S.mini = !!on; store.set("mini", S.mini);
  winMsg("mode:" + JSON.stringify(miniSize()));
  render(true);
  if (S.mini) setTimeout(scrollPL, 30);
}
const scrollPL = () => { const l = document.querySelector(".mpl .mlist"), c = l && l.querySelector(".cur"); if (c) l.scrollTop = c.offsetTop - l.clientHeight / 2 + c.offsetHeight / 2; };
const totalTime = list => { const s = list.reduce((a, t) => a + (t.dur || 0), 0), h = Math.floor(s / 3600), m = Math.floor((s % 3600) / 60), x = Math.floor(s % 60); return (h ? h + ":" + String(m).padStart(2, "0") : m) + ":" + String(x).padStart(2, "0"); };

function miniView() {
  const t = cur(), [skin, skinName] = SKINS[S.theme] || SKINS.cream;
  const text = t ? `${t.artist ? t.artist + " - " : ""}${t.title}${t.album ? " (" + t.album + ")" : ""}  ✦  ${fmt(t.dur || S.dur) || "--:--"}  ✦  ` : "abyss  ✦  pick a song in the full player  ✦  no ads ever  ✦  ";
  const speed = Math.max(8, text.length * 0.22);
  const spec = Array.from({ length: 22 }, () => "<i></i>").join("");
  const pos = S.queue.length ? `${S.idx + 1}/${S.queue.length}` : "0/0";
  return `<div class="mini skin-${skin} ${S.playing ? "playing" : ""}" style="${t?.thumb ? `--cover:url('${esc(t.thumb)}')` : ""}">
    <div class="mtitle" title="drag to move · double-click for the full player">
      <span class="mgrip"></span><span class="mname">${esc(skinName)}</span><span class="mgrip"></span>
      <div class="mwin"><button data-act="mpin" class="${S.miniTop ? "on" : ""}" title="${S.miniTop ? "stays on top (click to let go)" : "keep on top"}">${I.pin}</button><button data-act="mmin" title="minimize">${I.winMin}</button><button data-act="mini-off" title="full player (ctrl+m)">${I.winFull}</button><button data-act="mclose" title="close abyss">${I.x}</button></div>
    </div>
    <div class="mbody">
      <div class="mart" ${t ? `data-act="mini-off" title="full player"` : ""}>${t ? art(t.thumb, t.title) : `<div class="mlogo">abyss</div>`}</div>
      <div class="mright">
        <div class="lcd">
          <div class="lrow">
            <span class="lstate">${S.playing ? "▶" : t ? "❚❚" : "■"}</span>
            <span class="ltime" data-act="mrem" title="elapsed / remaining">${S.miniRemain ? "-" : ""}<b data-bind="${S.miniRemain ? "rem" : "pos"}">${fmt(S.miniRemain ? Math.max(0, S.dur - S.pos) : S.pos) || "0:00"}</b></span>
            <div class="ltags"><span>src <b>yt</b></span><span>vol <b data-bind="volnum">${Math.round(S.vol * 100)}</b></span><span>${pos}</span>
              <span class="${S.shuffle ? "lit" : ""}">shuf</span><span class="${S.repeat ? "lit" : ""}">${S.repeat === "one" ? "rep1" : "rep"}</span>
              ${S.upd?.latest?.newer ? `<span class="lit blink" data-act="mini-upd" title="abyss ${esc(S.upd.latest.version)} is out — click to see it">upd</span>` : ""}
              ${S.news.some(x => !S.seenNews.has(x.id)) ? `<span class="lit blink" data-act="mini-off" title="there's news — open the full player">news</span>` : ""}</div>
            <div class="spec" data-spec>${spec}</div>
          </div>
          <div class="marq" title="${t ? esc(t.title) : ""}"><div style="animation-duration:${speed}s"><span>${esc(text)}</span><span>${esc(text)}</span></div></div>
        </div>
        <div class="mseek">${t ? `<div class="wave" data-wave data-act="seek">${wave(52, hash(t.id))}</div>` : `<div class="wave"></div>`}</div>
        <div class="mctl">
          <button data-act="prev" title="previous">${I.prev}</button>
          <button class="big" data-act="toggle" title="play / pause (space)">${S.playing ? I.pause : I.play}</button>
          <button data-act="stop" title="stop">${I.stop}</button>
          <button data-act="next" title="next">${I.next}</button>
          <div class="mvol volseg" data-drag="volh" title="volume (↑ / ↓)">${Array.from({ length: 16 }, (_, i) => `<i style="height:${30 + (70 * i) / 15}%"></i>`).join("")}</div>
        </div>
        <div class="mbtns">
          <button class="${S.shuffle ? "on" : ""}" data-act="shuffle" title="shuffle">${I.shuffle}</button>
          <button class="${S.repeat ? "on" : ""}" data-act="repeat" title="repeat">${I.repeat}</button>
          ${t ? `<button class="${S.liked.has(t.id) ? "on" : ""}" data-act="like" data-id="${esc(t.id)}" title="like">${S.liked.has(t.id) ? I.heartF : I.heart}</button>` : ""}
          <span class="mfill"></span>
          <button class="txt ${S.miniPanel === "sr" ? "on" : ""}" data-act="mpanel" data-p="sr" title="search & add songs">sr</button>
          <button class="txt ${S.miniPanel === "pl" ? "on" : ""}" data-act="mpanel" data-p="pl" title="playlist">pl</button>
          <button class="txt" data-act="theme-next" title="next skin (t)">sk</button>
        </div>
      </div>
    </div>
    ${S.miniPanel === "pl" ? miniPL() : S.miniPanel === "sr" ? miniSR() : ""}
  </div>`;
}

function miniPL() {
  const rows = S.queue.map((x, i) => `<div class="mrow ${i === S.idx ? "cur" : ""}" draggable="true" data-qi="${i}" data-act="play" data-k="queue" data-i="${i}" title="click to play · drag to move">
      <span class="n">${i + 1}.</span><span class="tt">${esc((x.artist ? x.artist + " - " : "") + x.title)}</span><span class="d">${fmt(x.dur) || ""}</span>
      <span class="ma"><button data-act="mup" data-i="${i}" title="move up">▲</button><button data-act="mdown" data-i="${i}" title="move down">▼</button><button data-act="rmq" data-i="${i}" title="remove">✕</button></span></div>`).join("");
  return `<div class="mpl"><div class="mplh"><span class="mgrip"></span><span>playlist editor</span><span class="mgrip"></span></div>
    <div class="mlist">${reg("queue", S.queue) && rows || `<div class="mrow"><span class="tt">the queue is empty — press SR to find songs.</span></div>`}</div>
    <div class="mplf"><span>time ${totalTime(S.queue)}</span><span>${pl(S.queue.length, "song")}</span><button data-act="clearq" title="keep only the song that's playing">clear</button><button data-act="mpanel" data-p="sr">+ add</button></div></div>`;
}
function miniSR() {
  const r = S.mres, list = r.items || [];
  const body = r.state === "loading" ? `<div class="mrow"><span class="tt">searching “${esc(r.q)}”…</span></div>`
    : r.state === "err" ? `<div class="mrow"><span class="tt">${esc(r.e || "search failed")}</span></div>`
    : r.state === "ok" && !list.length ? `<div class="mrow"><span class="tt">nothing found for “${esc(r.q)}”.</span></div>`
    : r.state === "idle" ? `<div class="mrow"><span class="tt">type a song or artist and press enter.</span></div>`
    : reg("msr", list) && list.map((x, i) => `<div class="mrow ${cur()?.id === x.id ? "cur" : ""}" data-act="mplay" data-i="${i}" title="double-click to play">
        <span class="n">${x.video ? "▶" : "♪"}</span><span class="tt">${esc((x.artist ? x.artist + " - " : "") + x.title)}</span><span class="d">${fmt(x.dur) || ""}</span>
        <span class="ma show"><button data-act="mplay" data-i="${i}" title="play now">▶</button><button data-act="mnext" data-i="${i}" title="play next">⤵</button><button data-act="madd" data-i="${i}" title="add to the end of the queue">+</button></span></div>`).join("");
  return `<div class="mpl msr"><div class="mplh"><span class="mgrip"></span><span>search</span><span class="mgrip"></span></div>
    <div class="msbox"><input id="mq" value="${esc(S.mq)}" placeholder="search songs, artists…" spellcheck="false" autocomplete="off"><button data-act="mq-go">go</button></div>
    <div class="mlist">${body}</div>
    <div class="mplf"><span>${r.state === "ok" ? pl(list.length, "result") : "▶ play · ⤵ next · + queue"}</span><span>queue ${S.queue.length}</span><button data-act="mpanel" data-p="pl">playlist</button></div></div>`;
}

/* spectrum: youtube's player doesn't share its audio, so this is a lively decoration that follows play / pause */
let specVals = new Array(22).fill(0), specT = null;
function specLoop() {
  const bars = document.querySelectorAll("[data-spec] i");
  if (!S.mini || !bars.length) { clearInterval(specT); specT = null; return; }
  const n = bars.length, now = Date.now() / 1000;
  for (let i = 0; i < n; i++) {
    const target = S.playing && !S.buffering ? Math.max(0.08, Math.min(1, 0.55 + 0.35 * Math.sin(now * (2.1 + i * 0.37) + i) * Math.cos(now * 1.3 + i * 0.6) + (Math.random() - 0.5) * 0.45 - i / (n * 2.2))) : 0;
    specVals[i] = target > specVals[i] ? target : specVals[i] * 0.82;
    bars[i].style.height = Math.round(specVals[i] * 100) + "%";
  }
}
const startSpec = () => { if (!specT) specT = setInterval(specLoop, 70); };

// title bar: drag the window, double-click for the full player
document.addEventListener("mousedown", e => {
  if (!S.mini || e.button !== 0) return;
  const bar = e.target.closest(".mtitle, .mplh");
  if (bar && !e.target.closest("button")) { e.preventDefault(); winMsg("drag"); }
});
document.addEventListener("dblclick", e => { if (S.mini && e.target.closest(".mtitle") && !e.target.closest("button")) setMini(false); });
// remaining time ticks with the clock
setInterval(() => { if (S.mini && S.miniRemain) document.querySelectorAll('[data-bind="rem"]').forEach(x => (x.textContent = fmt(Math.max(0, S.dur - S.pos)) || "0:00")); }, 500);
// started in mini mode: tell the window (it already is mini, this syncs "on top")
if (S.mini) setTimeout(() => winMsg("mode:" + JSON.stringify(miniSize())), 50);

/* ================= render ================= */
const root = document.getElementById("root");
function applyLook() {
  const b = document.body.style;
  if (S.theme === "custom") { b.setProperty("--paper", S.custom.paper); b.setProperty("--ink", S.custom.ink); } else { b.removeProperty("--paper"); b.removeProperty("--ink"); }
  if (S.font) b.setProperty("--px", `"${S.font}", "Pixelify Sans", sans-serif`); else b.removeProperty("--px");
}
let lastView = "", lastLyrKey = "";
function render(reset) {
  const m = document.querySelector(".main"), st = !reset && m ? m.scrollTop : 0;
  const q = document.querySelector(".qscroll"), qst = q ? q.scrollTop : 0;
  const ae = document.activeElement, fid = ae && ae.id, sel = ae && ae.selectionStart;
  document.body.dataset.theme = S.theme;
  document.body.classList.toggle("gray", !!S.gray);
  applyLook();
  document.body.classList.toggle("mini-on", !!S.mini);
  if (S.mini) {
    const ml = document.querySelector(".mpl .mlist"), mst = ml ? ml.scrollTop : null;
    root.innerHTML = miniView();
    if (mst !== null) { const nl = document.querySelector(".mpl .mlist"); if (nl) nl.scrollTop = mst; }
    updateVol(); tick(true); placeYT(); startSpec();
    return;
  }
  root.innerHTML = `<div class="ab ${S.playing ? "playing" : "paused"}">${topbar()}
    <div class="body">${nowPanel()}<main class="main ${S.page === "np" ? "flush" : ""}">${S.page === "np" ? "" : newsBar()}${(P[S.page] || P.home)()}</main></div>${modal()}</div>`;
  const nm = document.querySelector(".main"); if (nm) nm.scrollTop = st;
  const view = S.page + (S.page === "np" ? ":" + S.tab : "") + JSON.stringify(S.params);
  if (nm && view !== lastView) { nm.classList.add("enter"); lastView = view; }
  const ly = document.querySelector(".lyr");
  if (ly) { const k = S.page + ":" + (cur()?.id || "") + ":" + (S.lyrSrc[cur()?.id] || ""); if (k !== lastLyrKey) { ly.classList.add("lyr-in"); lastLyrKey = k; } } else lastLyrKey = "";
  const nq = document.querySelector(".qscroll"); if (nq && !reset) nq.scrollTop = qst;
  if (fid) { const el = document.getElementById(fid); if (el) { el.focus(); try { el.setSelectionRange(sel, sel); } catch {} } }
  updateVol(); tick(true); placeYT();
}
/* keep the (persistent) youtube player sitting exactly on the cover sleeve in the now playing panel */
function placeYT() {
  const w = document.getElementById("ytwrap"), slot = document.querySelector(S.mini ? ".mini .mart" : ".np .sleeve");
  if (!w) return;
  if (!slot || !cur()) { w.style.display = "none"; return; }
  const r = slot.getBoundingClientRect();
  Object.assign(w.style, { display: "block", left: r.left + "px", top: r.top + "px", width: r.width + "px", height: r.height + "px" });
}
window.addEventListener("resize", () => placeYT());
// follow the cover when anything scrolls or the layout shifts (narrow / short windows)
let ytRaf = 0;
const placeSoon = () => { if (!ytRaf) ytRaf = requestAnimationFrame(() => { ytRaf = 0; placeYT(); }); };
document.addEventListener("scroll", placeSoon, true);
new ResizeObserver(placeSoon).observe(document.body);
let lastLi = -1;
function tick(force) {
  document.querySelectorAll('[data-bind="pos"]').forEach(e => (e.textContent = fmt(S.pos) || "0:00"));
  document.querySelectorAll('[data-bind="dur"]').forEach(e => (e.textContent = fmt(S.dur)));
  const p = pct();
  document.querySelectorAll("[data-wave]").forEach(w => { const b = w.children, n = b.length; for (let i = 0; i < n; i++) b[i].classList.toggle("on", i / n < p / 100); });
  const box = document.querySelector("[data-lyrics]");
  if (box) {
    const ps = box.querySelectorAll("p[data-t]");
    let li = -1;
    ps.forEach((el, i) => { if (S.pos + 0.25 >= +el.dataset.t) li = i; });
    if (force || li !== lastLi) {
      ps.forEach((el, i) => el.classList.toggle("now", i === li));
      if (ps[li]) box.scrollTop = ps[li].offsetTop - box.offsetTop - box.clientHeight / 2 + ps[li].clientHeight / 2;
      lastLi = li;
    }
  }
}
function updateVol() {
  const v = S.vol;
  document.querySelectorAll(".volseg").forEach(s => { const b = s.children, n = b.length, on = Math.round(v * n); for (let i = 0; i < n; i++) b[i].classList.toggle("on", i < on); });
  document.querySelectorAll('[data-bind="volnum"]').forEach(e => (e.textContent = Math.round(v * 100)));
  document.querySelectorAll('[data-bind="volknob"]').forEach(e => e.setAttribute("y", knobY(v)));
  document.querySelectorAll('[data-bind="volicon"]').forEach(e => (e.innerHTML = volIcon()));
  document.querySelectorAll('[data-bind="toneknob"]').forEach(e => e.setAttribute("transform", `rotate(${(S.tone - 0.5) * 270} 113.5 83)`));
  document.querySelectorAll('[data-bind="tonev"]').forEach(e => (e.textContent = Math.round(S.tone * 100)));
}
const setVol = v => { S.vol = clamp(Math.round(v * 100) / 100, 0, 1); store.set("vol", S.vol); AE.apply(); updateVol(); };
const setTone = v => { S.tone = clamp(v, 0, 1); store.set("tone", S.tone); AE.apply(); updateVol(); };
let toastT;
function toast(m) { const t = document.getElementById("toast"); t.textContent = m; t.classList.add("show"); clearTimeout(toastT); toastT = setTimeout(() => t.classList.remove("show"), 2200); }

function go(page, params = {}) {
  if (page === S.page && JSON.stringify(params) === JSON.stringify(S.params)) return render();
  S.hist.push({ page: S.page, params: S.params, tab: S.tab }); S.fwd = [];
  S.page = page; S.params = params; render(true); saveSession();
}

/* ================= account + sync ================= */
/* pull all of your youtube data in the background so every tab is ready before you open it */
function prefetchAll() {
  if (!S.signedIn) return;
  need("me", () => window.api.personal());
  need("library", () => window.api.library());
  need("following", () => window.api.following());
  need("pl:LM", () => window.api.playlist("LM").then(p => { p.tracks.forEach(t => S.liked.set(t.id, t)); saveLiked(); return p; }));
}
async function refreshStatus() {
  try {
    const st = await window.api.status();
    S.signedIn = st.signedIn; S.account = st.account;
    if (S.signedIn) { S.lastSync = "just now"; prefetchAll(); }
  } catch {}
  render();
}
async function signedIn(r) {
  S.modal = null; S.authCode = null; S.signedIn = true; S.account = r.account; cache.clear();
  render();
  prefetchAll(); S.lastSync = "just now";
  toast(`signed in${r.account?.name ? " as " + r.account.name : ""} · library synced`);
  render();
}
window.api.onAuthCode(code => { S.authCode = code; if (S.modal === "signin-code") render(); });
async function loadLiked() {
  try {
    const p = await window.api.playlist("LM");
    cache.set("pl:LM", { ok: true, v: p });
    p.tracks.forEach(t => S.liked.set(t.id, t));
    saveLiked();
  } catch {}
}
async function doSync() {
  if (!S.signedIn) { S.modal = "signin"; return render(); }
  if (S.syncing) return;
  S.syncing = true; render();
  ["library", "pl:LM", "home", "me", "following"].forEach(k => cache.delete(k));
  for (const k of [...cache.keys()]) if (k.startsWith("pl:")) cache.delete(k);
  prefetchAll();
  await new Promise(r => { const t = setInterval(() => { if (!pending.size) { clearInterval(t); r(); } }, 300); setTimeout(() => { clearInterval(t); r(); }, 30000); });
  S.syncing = false; S.lastSync = "just now"; render(); toast("synced with youtube music");
}
/* ---- updates + announcements (pushed by the app: window.__abyssUpdate / window.__abyssNews) ---- */
S.upd = null; S.news = []; S.seenNews = new Set(store.get("seenNews", [])); S.updLater = false;
const md = t => esc(t).split(/\r?\n/).map(l => {
  l = l.replace(/\*\*(.+?)\*\*/g, "<b>$1</b>").replace(/(^|[^*])\*([^*\s][^*]*?)\*(?!\*)/g, "$1<i>$2</i>").replace(/`(.+?)`/g, "<code>$1</code>");
  if (/^#{1,6}\s/.test(l)) return `<div class="nh">${l.replace(/^#+\s*/, "")}</div>`;
  if (/^\s*[-*]\s/.test(l)) return `<div class="nl">• ${l.replace(/^\s*[-*]\s/, "")}</div>`;
  return l.trim() ? `<div>${l}</div>` : `<div class="gap"></div>`;
}).join("");
const updProgress = () => {
  const u = S.upd, pct = u.progress || 0;
  return `<div class="updp"><div class="dlbar"><i style="width:${pct}%"></i></div><div class="dlm"><span>${u.phase === "installing" ? (u.platform === "windows" ? "installing — abyss will close and reopen by itself" : "restarting into the new version…") : "downloading update"}</span><span>${pct}%</span></div></div>`;
};
const updButton = () => {
  const u = S.upd, L = u?.latest;
  if (!L?.newer) return "";
  if (u.phase === "downloading" || u.phase === "installing") return `<button class="obtn fill" disabled>${u.phase}…</button>`;
  return u.canInstall ? `<button class="obtn fill" data-act="upd-go">${I.dl}update now</button>`
    : `<button class="obtn fill" data-act="openurl" data-u="${esc(L.url || "https://github.com/yumekuse/Abyss-Music/releases/latest")}">${I.dl}get it on github</button>`;
};
const noteSummary = t => String(t || "").split(/\r?\n/).map(x => x.trim()).filter(x => x && !/^#/.test(x))
  .map(x => x.replace(/^[-*]\s+/, "").replace(/[*`_>]/g, "")).slice(0, 2).join(" · ").slice(0, 160);
function updLine() {
  const u = S.upd;
  if (!u) return "checking for updates…";
  if (u.phase === "downloading") return `downloading update… ${u.progress || 0}%`;
  if (u.phase === "installing") return "installing the update…";
  if (u.latest?.newer) return `abyss ${u.latest.version} is available.${u.canInstall ? "" : u.platform === "linux" ? " update it with your package manager or grab it from github." : ""}`;
  if (u.error) return u.error;
  if (u.checkedAt) return "you're on the latest version.";
  return "checks for updates by itself.";
}
function newsBar() {
  const out = [], u = S.upd, L = u?.latest;
  if (L?.newer && (!S.updLater || u.phase !== "idle")) {
    const busy = u.phase === "downloading" || u.phase === "installing";
    out.push(`<div class="news update"><div class="nb"><span class="caps">update</span><div class="h2">abyss ${esc(L.version)} is here.</div>
      ${busy ? updProgress() : `<div class="muted">${esc(noteSummary(L.notes))}</div>`}
      ${u.error ? `<div class="dln">${esc(u.error)}</div>` : ""}</div>
      <div class="na">${updButton()}<button class="obtn" data-act="upd-show">what's new</button>${busy ? "" : `<button class="ibtn" data-act="upd-later" title="later">${I.x}</button>`}</div></div>`);
  }
  const n = S.news.find(x => !S.seenNews.has(x.id));
  if (n) out.push(`<div class="news ${esc(n.level)}"><div class="nb"><span class="caps">${esc(n.level === "info" ? "news" : n.level)}${n.date ? " · " + esc(n.date) : ""}</span>${n.title ? `<div class="h2">${esc(n.title)}</div>` : ""}${n.body ? `<div class="muted">${md(n.body)}</div>` : ""}</div>
    <div class="na">${n.link ? `<button class="obtn fill" data-act="openurl" data-u="${esc(n.link.url)}">${esc(n.link.label)}</button>` : ""}${n.dismissible !== false ? `<button class="ibtn" data-act="news-x" data-id="${esc(n.id)}" title="dismiss">${I.x}</button>` : ""}</div></div>`);
  return out.join("");
}
window.__abyssUpdate = st => { S.upd = st; queueRender(); };
window.__abyssNews = list => { S.news = Array.isArray(list) ? list : []; queueRender(); };
window.api.updateState().then(st => window.__abyssUpdate(st)).catch(() => {});
window.api.news().then(l => window.__abyssNews(l)).catch(() => {});

/* ---- downloads: queued in the app (3 at a time); progress arrives through window.__abyssDl ---- */
let DL = { jobs: [] };
const dlActive = id => DL.jobs.some(j => !j.finished && j.current.some(c => c.id === id));
async function queueDownload(list, label, kind) {
  const songs = list.filter(t => t && t.id && (t.type || "song") === "song");
  if (!songs.length) return toast("nothing to download");
  const todo = songs.filter(t => !S.downloads.has(t.id));
  if (!todo.length) return toast(kind === "song" ? "already downloaded" : "all of these are already downloaded");
  try { await window.api.dlQueue(todo, label, kind); dlWatch(); toast(kind === "song" ? `downloading “${label}”` : `downloading ${pl(todo.length, "song")} from “${label}”`); }
  catch (e) { toast("download failed: " + e.message); }
}
function dlBox() {
  let box = document.getElementById("dlbox");
  if (!box) { box = document.createElement("div"); box.id = "dlbox"; document.body.appendChild(box); }
  const jobs = DL.jobs.filter(j => !j.dismissed);
  box.classList.toggle("show", jobs.length > 0);
  box.innerHTML = jobs.map(j => {
    const handled = j.done + j.skipped + j.failed, part = j.current.reduce((a, c) => a + c.pct / 100, 0);
    const pct = j.total ? Math.min(100, Math.round(((handled + part) / j.total) * 100)) : 0;
    const what = j.kind === "song" ? "song" : j.kind === "album" ? "album" : "playlist";
    const head = j.finished
      ? (j.cancelled ? "stopped" : j.failed ? "finished with problems" : "downloaded")
      : `downloading ${what}`;
    const count = j.kind === "song" ? "" : `<span>${j.done + j.skipped} / ${j.total} songs</span>`;
    const notes = [j.skipped ? `${j.skipped} already saved` : "", j.failed ? `${j.failed} failed` : ""].filter(Boolean).join(" · ");
    return `<div class="dlj ${j.finished ? "fin" : ""}">
      <div class="dlh"><span class="caps">${head}</span>${j.finished
        ? `<button class="ibtn" data-act="dlclose" data-id="${j.id}" title="close">${I.x}</button>`
        : `<button class="obtn" data-act="dlcancel" data-id="${j.id}">${I.x}cancel</button>`}</div>
      <div class="dlt ell">${esc(j.label)}</div>
      <div class="dlbar"><i style="width:${j.finished && !j.cancelled ? 100 : pct}%"></i></div>
      <div class="dlm">${count}<span>${j.finished ? "" : pct + "%"}</span></div>
      ${!j.finished && j.current.length ? j.current.map(c => `<div class="dlc ell"><b>${c.pct}%</b> ${esc(c.title)}</div>`).join("") : ""}
      ${notes ? `<div class="dln">${esc(notes)}${j.failedTitles?.length ? ` <span title="${esc(j.failedTitles.join("\n"))}">(why?)</span>` : ""}</div>` : ""}
      ${j.finished && j.done ? `<button class="obtn" data-act="opendl">${I.folder}open folder</button>` : ""}
    </div>`;
  }).join("");
}
const dlSeen = new Set();
window.__abyssDl = st => {
  const old = new Map(DL.jobs.map(j => [j.id, j]));
  DL = { jobs: st.jobs.map(j => ({ ...j, dismissed: old.get(j.id)?.dismissed || false })) };
  let changed = false;
  for (const e of st.added || []) { S.downloads.set(e.id, e); changed = true; }
  for (const j of DL.jobs) if (j.finished && !dlSeen.has(j.id)) {
    dlSeen.add(j.id); changed = true;
    window.api.downloads().then(list => { list.forEach(e => S.downloads.set(e.id, e)); render(); }).catch(() => {});
    setTimeout(() => { const x = DL.jobs.find(y => y.id === j.id); if (x) { x.dismissed = true; dlBox(); } }, 12000);
  }
  dlBox();
  if (changed || S.page === "np") queueRender();
};
// progress is also pushed by the app; polling while something downloads keeps the panel right either way
let dlPoll = null;
function dlWatch() {
  if (dlPoll) return;
  dlPoll = setInterval(async () => {
    try { const st = await window.api.dlState(); window.__abyssDl(st); if (!st.jobs.some(j => !j.finished)) { clearInterval(dlPoll); dlPoll = null; } }
    catch { clearInterval(dlPoll); dlPoll = null; }
  }, 500);
}
window.api.dlState().then(st => { window.__abyssDl(st); if (st.jobs.some(j => !j.finished)) dlWatch(); }).catch(() => {});
function listFor(key) {
  if (key === "pl:liked") return likedList().list || [];
  return ctxs[key] || [];
}

/* ================= events ================= */
document.addEventListener("click", async e => {
  const a = e.target.closest("[data-act]");
  if (!a || a.disabled) return;
  const d = a.dataset;
  switch (d.act) {
    case "noop": return;
    case "go": return go(d.p);
    case "np": S.tab = d.t; return go("np");
    case "back": if (S.hist.length) { S.fwd.push({ page: S.page, params: S.params, tab: S.tab }); Object.assign(S, S.hist.pop()); render(true); } return;
    case "fwd": if (S.fwd.length) { S.hist.push({ page: S.page, params: S.params, tab: S.tab }); Object.assign(S, S.fwd.pop()); render(true); } return;
    case "retry": cache.delete(d.k); return render();
    case "play": { const list = listFor(d.k); return playFrom(list, +d.i, d.k, nameFor(d.k)); }
    case "radio1": { const t = (d.k === "queue" ? S.queue : listFor(d.k))[+d.i]; if (t) startRadio(t); return; }
    case "playk": { const list = listFor(d.k).filter(x => x.type === "song"); if (list.length) playFrom(list, 0, d.k, nameFor(d.k)); return; }
    case "toggle": return toggle();
    case "next": return next();
    case "prev": return prev();
    case "shuffle": S.shuffle = !S.shuffle; toast(S.shuffle ? "shuffle on" : "shuffle off"); return render();
    case "repeat": S.repeat = S.repeat === false ? true : S.repeat === true ? "one" : false; toast(S.repeat === "one" ? "repeat one" : S.repeat ? "repeat all" : "repeat off"); return render();
    case "like": {
      const id = d.id, t = S.liked.get(id) || S.queue.find(x => x.id === id) || Object.values(ctxs).flat().find(x => x?.id === id);
      const on = !S.liked.has(id);
      on ? S.liked.set(id, t) : S.liked.delete(id); saveLiked(); render();
      if (S.signedIn) { try { await window.api.like(id, on); cache.delete("pl:LM"); toast(on ? "liked · synced with youtube" : "removed from liked · synced"); } catch (err) { toast("couldn't sync like: " + err.message); } }
      else toast(on ? "added to liked" : "removed from liked");
      return;
    }
    case "dl": { const t = S.queue.find(x => x.id === d.id) || cur(); if (t) queueDownload([t], t.title, "song"); return; }
    case "dlcancel": await window.api.dlCancel(+d.id); return;
    case "mini-on": return setMini(true);
    case "mini-off": return setMini(false);
    case "mini-upd": setMini(false); S.modal = "upd"; return render();
    case "mmin": return winMsg("minimize");
    case "mclose": return winMsg("close");
    case "mpl": return miniPanel("pl");
    case "mpanel": return miniPanel(d.p);
    case "mq-go": S.mq = document.getElementById("mq")?.value || S.mq; return miniSearch();
    case "mplay": if (e.detail < 2 && a.classList.contains("mrow")) return; return miniQueue(listFor("msr")[+d.i], "now");
    case "mnext": return miniQueue(listFor("msr")[+d.i], "next");
    case "madd": return miniQueue(listFor("msr")[+d.i], "end");
    case "mup": case "mdown": {
      const i = +d.i, j = d.act === "mup" ? i - 1 : i + 1;
      if (j < 0 || j >= S.queue.length) return;
      const ct = cur(); [S.queue[i], S.queue[j]] = [S.queue[j], S.queue[i]]; S.idx = S.queue.indexOf(ct);
      saveSession(); return render();
    }
    case "minitop":
    case "mpin": S.miniTop = !S.miniTop; store.set("miniTop", S.miniTop); winMsg("mode:" + JSON.stringify(miniSize())); return render();
    case "mrem": S.miniRemain = !S.miniRemain; return render();
    case "stop": audio.pause(); audio.currentTime = 0; S.pos = 0; return render();
    case "theme-next": { const i = THEMES.findIndex(x => x[0] === S.theme), n = THEMES[(i + 1) % THEMES.length]; S.theme = n[0]; store.set("theme", S.theme); render(); return toast((SKINS[S.theme] || SKINS.cream)[1]); }
    case "upd-show": S.modal = "upd"; return render();
    case "upd-later": S.updLater = true; return render();
    case "upd-check": { toast("checking for updates…"); try { const st = await window.api.updateCheck(); S.upd = st; toast(st.latest?.newer ? `abyss ${st.latest.version} is available` : "you're on the latest version"); } catch (err) { toast(err.message); } window.api.newsRefresh().then(l => window.__abyssNews(l)).catch(() => {}); return render(); }
    case "upd-go": { try { await window.api.updateInstall(); } catch (err) { toast(err.message); } return render(); }
    case "news-x": S.seenNews.add(d.id); store.set("seenNews", [...S.seenNews]); return render();
    case "news-reset": S.seenNews.clear(); store.set("seenNews", []); toast("announcements will show again"); return render();
    case "openurl": if (/^https:\/\//.test(d.u || "")) window.api.openUrl(d.u); return;
    case "dlclose": { const j = DL.jobs.find(x => x.id === +d.id); if (j) j.dismissed = true; return dlBox(); }
    case "openyt": { const t = cur(); if (t) window.api.openYT(t.id); return; }
    case "dlall": {
      const album = d.id.startsWith("al:");
      const list = album ? (cache.get(d.id)?.v?.tracks || listFor(d.id)) : listFor("pl:" + d.id);
      const label = album ? (cache.get(d.id)?.v?.title || "album") : nameFor("pl:" + d.id);
      return queueDownload(list, label, album ? "album" : "playlist");
    }
    case "rmdl": await window.api.removeDownload(d.id); S.downloads.delete(d.id); toast("download removed"); return render();
    case "playdl": { const list = [...S.downloads.values()]; if (list.length) playFrom(list, 0, "downloads", "downloads"); return; }
    case "opendl": return window.api.openDownloads();
    case "artist": return go("artist", { id: d.id });
    case "playart": { const r = cache.get("ar:" + d.id); if (!r?.ok || !r.v.songs.length) return; const l = [...r.v.songs]; if (d.shuf) l.sort(() => Math.random() - 0.5); return playFrom(l, 0, "arsongs:" + d.id + (d.shuf ? ":s" : ""), r.v.name); }
    case "artall": S.artAll = S.artAll === d.id ? null : d.id; return render();
    case "artbio": S.artBio = S.artBio === d.id ? null : d.id; return render();
    case "artistname": {
      const name = d.n;
      toast(`opening ${name}…`);
      try {
        const res = await window.api.search(name);
        const all = res.flatMap(x => x.items);
        const hit = all.find(x => x.type === "artist" && x.title.toLowerCase() === name.toLowerCase()) || all.find(x => x.type === "artist") || all.find(x => x.type === "song" && x.artistId);
        const id = hit?.type === "artist" ? hit.id : hit?.artistId;
        if (id) { const t = cur(); if (t && !t.artistId && t.artist.startsWith(name)) t.artistId = id; return go("artist", { id }); }
        toast(`couldn't find ${name}`);
      } catch (err) { toast("couldn't open artist: " + err.message); }
      return;
    }
    case "album": return go("album", { id: d.id });
    case "pl": return go("playlist", { id: d.id });
    case "playpl": {
      const id = d.id, key = "pl:" + id;
      let list = id === "liked" ? likedList().list : id.startsWith("local:") ? getLocal(id)?.tracks : null;
      if (!list) { try { toast("loading playlist…"); const p = await window.api.playlist(id); cache.set(key, { ok: true, v: p }); list = p.tracks; } catch (err) { return toast(err.message); } }
      if (!list?.length) return toast("this playlist is empty");
      if (S.qKey === key && !d.shuf) return toggle();
      const l = [...list]; if (d.shuf) l.sort(() => Math.random() - 0.5);
      return playFrom(l, 0, key, id === "liked" ? "liked music" : getLocal(id)?.name || cache.get(key)?.v?.title || "playlist");
    }
    case "playal": { const r = cache.get("al:" + S.params.id); if (!r?.ok) return; const key = "al:" + r.v.id; if (S.qKey === key && !d.shuf) return toggle(); const l = [...r.v.tracks]; if (d.shuf) l.sort(() => Math.random() - 0.5); return playFrom(l, 0, key, r.v.title); }
    case "seek": { if (!S.dur) return; const r = a.getBoundingClientRect(); audio.currentTime = clamp(((e.clientX - r.left) / r.width) * S.dur, 0, S.dur - 1); return; }
    case "seekto": audio.currentTime = +d.t; if (audio.paused) toggle(true); return;
    case "genre": return go("genre", { i: +d.i });
    case "arttab": S.artTab = d.t; return render();
    case "randart": { const all = [...myArtists(), ...[...S.subs].map(x => JSON.parse(x))]; if (all.length) go("artist", { id: all[Math.floor(Math.random() * all.length)].id }); return; }
    case "libtab": S.libTab = d.t; return S.page !== "library" ? go("library") : render();
    case "sync": return doSync();
    case "newpl": S.modal = "newpl"; render(); setTimeout(() => document.getElementById("plname")?.focus(), 30); return;
    case "newpl-ok": {
      const name = (document.getElementById("plname")?.value || "").trim() || `playlist ${S.localPls.length + 1}`;
      const seed = S.modalTrack ? [S.modalTrack] : [];
      S.modal = null; S.modalTrack = null;
      if (S.signedIn) {
        try { const r = await window.api.createPlaylist(name, seed.map(t => t.id)); cache.delete("library"); toast(`created “${name}” on youtube music`); render(); if (r.id) go("playlist", { id: "VL" + r.id }); }
        catch (err) { toast("couldn't create on youtube: " + err.message); }
      } else { const p = { id: "local:" + Date.now(), name, tracks: seed }; S.localPls.push(p); saveLocal(); toast(`created “${name}”`); go("playlist", { id: p.id }); }
      return;
    }
    case "addpl": { const t = listFor(d.k)[+d.i]; if (!t) return; S.modalTrack = t; S.modal = "addpl"; return render(); }
    case "addpl-ok": {
      const t = S.modalTrack, id = d.id; S.modal = null; S.modalTrack = null;
      if (id.startsWith("local:")) { const p = getLocal(id); if (!p.tracks.some(x => x.id === t.id)) p.tracks.push(t); saveLocal(); toast(`added to “${p.name}”`); }
      else { try { await window.api.addToPlaylist(id, [t.id]); cache.delete("pl:" + id); toast("added · synced with youtube"); } catch (err) { toast("couldn't add: " + err.message); } }
      return render();
    }
    case "delpl": S.localPls = S.localPls.filter(p => p.id !== d.id); saveLocal(); toast("playlist deleted"); return go("playlists");
    case "addq": { const t = listFor(d.k)[+d.i]; if (!t) return; if (!cur()) return startRadio(t); S.queue.splice(S.idx + 1, 0, { ...t }); toast(`“${t.title}” plays next`); return render(); }
    case "rmq": { const i = +d.i; if (i === S.idx) return toast("can't remove the song that's playing"); S.queue.splice(i, 1); if (i < S.idx) S.idx--; return render(); }
    case "clearq": if (cur()) { S.queue = [cur()]; S.idx = 0; } toast("queue cleared"); return render();
    case "clears": S.q = ""; return S.page === "search" ? go("home") : render();
    case "auto": S.autoplay = !S.autoplay; store.set("autoplay", S.autoplay); toast(S.autoplay ? "autoplay radio on" : "autoplay radio off"); if (S.autoplay) extendRadio(); return render();
    case "lysrc": { const t = cur(); if (t) S.lyrSrc[t.id] = d.s; lastLi = -1; return render(); }
    case "lytr": S.lyrTr = !S.lyrTr; return render();
    case "roman": S.roman = !S.roman; return render();
    case "blur": S.blur = !S.blur; return render();
    case "lysz": S.lyrSize = clamp(S.lyrSize + +d.d * 0.12, 0.64, 1.6); store.set("lyrSize", S.lyrSize); return render();
    case "trlang": S.trLang = d.l; store.set("trLang", S.trLang); return render();
    case "view": S.view = d.v; store.set("view", S.view); return render();
    case "gray": S.gray = !S.gray; store.set("gray", S.gray); return render();
    case "discord":
      if (S.discord && S.discordOk) { S.discord = false; S.discordOk = false; store.set("discord", false); await window.api.discordOff(); toast("discord status off"); return render(); }
      return discordOn(false);
    case "eqon": S.eqOn = !S.eqOn; store.set("eqOn", S.eqOn); AE.apply(); return render();
    case "eqp": if (EQ_PRESETS[d.n]) { S.eq = [...EQ_PRESETS[d.n]]; S.eqPreset = d.n; store.set("eq", S.eq); store.set("eqPreset", d.n); AE.apply(); } return render();
    case "theme": S.theme = d.t; store.set("theme", S.theme); return render();
    case "xfade": S.crossfade = +d.v; store.set("crossfade", S.crossfade); toast(S.crossfade ? `smooth transitions · ${S.crossfade}s` : "smooth transitions off"); return render();
    case "speed": S.speed = +d.v; audio.playbackRate = S.speed; saveSession(); return render();
    case "font": S.font = d.f; store.set("font", S.font); return render();
    case "sub": { S.subs.has(d.v) ? S.subs.delete(d.v) : S.subs.add(d.v); store.set("subs", [...S.subs]); return render(); }
    case "mute": if (S.vol > 0) { S.prevVol = S.vol; setVol(0); } else setVol(S.prevVol || 0.7); return;
    case "signin": S.modal = "signin"; return render();
    case "signin-code": {
      S.modal = "signin-code"; S.authCode = null; render();
      try {
        const r = await window.api.loginCode();
        if (S.modal !== "signin-code") return;
        if (r.ok && r.signedIn) await signedIn(r);
        else { S.modal = null; toast(r.error === "cancelled" ? "sign-in cancelled" : "sign-in failed: " + (r.error || "google didn't confirm")); render(); }
      } catch (err) { S.modal = null; toast("sign-in failed: " + err.message); render(); }
      return;
    }
    case "signin-cancel": window.api.loginCancel(); S.modal = null; S.authCode = null; toast("sign-in cancelled"); return render();
    case "opendevice": return window.api.openDevice(S.authCode?.url || "https://www.google.com/device");
    case "copycode": if (S.authCode) { navigator.clipboard.writeText(S.authCode.code).then(() => toast("code copied"), () => toast("couldn't copy — type it in")); } return;
    case "signin-paste": S.modal = "signin-paste"; render(); setTimeout(() => document.getElementById("cookiebox")?.focus(), 30); return;
    case "paste-ok": {
      const text = document.getElementById("cookiebox")?.value || "";
      if (!text.trim()) return toast("paste your cookie first");
      toast("checking your login…");
      try { const r = await window.api.loginCookie(text); await signedIn(r); }
      catch (err) { toast(err.message.replace(/^Error invoking remote method '[^']+': (Error: )?/, "")); }
      return;
    }
    case "signout": await window.api.logout(); S.signedIn = false; S.account = null; S.lastSync = "never"; cache.clear(); toast("signed out"); return render();
    case "modal-x": if (e.target === a || a.tagName === "BUTTON") { S.modal = null; S.modalTrack = null; render(); } return;
  }
});
const nameFor = k => k === "queue" ? S.qName : k.startsWith("pl:") ? (k === "pl:liked" ? "liked music" : getLocal(k.slice(3))?.name || cache.get(k)?.v?.title || "playlist")
  : k.startsWith("al:") ? cache.get(k)?.v?.title || "album" : k.startsWith("s") ? "search" : k.startsWith("home") ? "home" : k === "downloads" ? "downloads" : "";

/* drag: volume (deck fader + pixel bars) and tone knob */
let drag = null;
function dragVal(e) {
  if (!drag) return;
  if (drag.kind === "vol") { const r = drag.track.getBoundingClientRect(); setVol(1 - (e.clientY - r.top) / r.height); }
  else if (drag.kind === "volh") { const r = drag.el.getBoundingClientRect(); setVol((e.clientX - r.left) / r.width); }
  else if (drag.kind === "tone") setTone(drag.start + (drag.y - e.clientY) / 120);
}
document.addEventListener("pointerdown", e => {
  const el = e.target.closest("[data-drag]");
  if (!el) return;
  e.preventDefault();
  drag = { kind: el.dataset.drag, el, track: el.closest("svg")?.querySelector("[data-vol-track]"), y: e.clientY, start: S.tone };
  dragVal(e);
});
document.addEventListener("pointermove", e => drag && dragVal(e));
document.addEventListener("pointerup", () => (drag = null));
document.addEventListener("wheel", e => {
  const el = e.target.closest("[data-drag]");
  if (!el) return;
  e.preventDefault();
  if (el.dataset.drag === "tone") setTone(S.tone - Math.sign(e.deltaY) * 0.05); else setVol(S.vol - Math.sign(e.deltaY) * 0.05);
}, { passive: false });

/* search box: debounced live search */
let qT;
document.addEventListener("input", e => {
  const el = e.target;
  if (el.id === "tbq") {
    S.q = el.value;
    clearTimeout(qT);
    qT = setTimeout(() => { const q = S.q.trim(); if (!q) return; if (S.page === "search") { S.params = { q }; render(); } else go("search", { q }); }, 380);
    return;
  }
  if (el.dataset.cc) { S.custom = { ...S.custom, [el.dataset.cc]: el.value }; S.theme = "custom"; store.set("custom", S.custom); store.set("theme", "custom"); document.body.dataset.theme = "custom"; return applyLook(); }
  if (el.dataset.band) { const i = +el.dataset.band; S.eq[i] = +el.value; S.eqPreset = "custom"; store.set("eq", S.eq); store.set("eqPreset", "custom"); AE.apply(); document.querySelector(`[data-db="${i}"]`).textContent = (S.eq[i] > 0 ? "+" : "") + S.eq[i]; return; }
  if (el.dataset.fx === "tone") return setTone(el.value / 100);
  if (el.dataset.fx === "speed") { S.speed = el.value / 100; audio.playbackRate = S.speed; document.querySelector('[data-fxv="speed"]').textContent = S.speed.toFixed(2) + "×"; }
});
document.addEventListener("change", e => { if (e.target.dataset.band || e.target.dataset.cc) render(); });
document.addEventListener("keydown", e => {
  if (e.target.id === "tbq" && e.key === "Enter") { clearTimeout(qT); const q = S.q.trim(); if (q) go("search", { q }); return; }
  if (e.target.id === "mq" && e.key === "Enter") { S.mq = e.target.value; miniSearch(); return; }
  if (e.target.id === "plname" && e.key === "Enter") return document.querySelector('[data-act="newpl-ok"]')?.click();
  if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "f") { e.preventDefault(); const i = document.getElementById("tbq"); i?.focus(); i?.select(); return; }
  if (["INPUT", "TEXTAREA"].includes(e.target.tagName)) { if (e.key === "Escape") e.target.blur(); return; }
  if (e.code === "Space") { e.preventDefault(); toggle(); }
  else if (e.key === "ArrowRight") audio.currentTime = Math.min((S.dur || 0) - 1, audio.currentTime + 5);
  else if (e.key === "ArrowLeft") audio.currentTime = Math.max(0, audio.currentTime - 5);
  else if (e.key === "ArrowUp") { e.preventDefault(); setVol(S.vol + 0.05); toast(`volume ${Math.round(S.vol * 100)}`); }
  else if (e.key === "ArrowDown") { e.preventDefault(); setVol(S.vol - 0.05); toast(`volume ${Math.round(S.vol * 100)}`); }
  else if (e.key.toLowerCase() === "m" && e.ctrlKey) { e.preventDefault(); setMini(!S.mini); }
  else if (e.key === "m") { if (S.vol > 0) { S.prevVol = S.vol; setVol(0); toast("muted"); } else { setVol(S.prevVol || 0.7); toast("unmuted"); } }
  else if (e.key === "l") { if (S.page === "np" && S.tab === "lyrics") go("home"); else { S.tab = "lyrics"; go("np"); } }
  else if (e.key === "t") { const i = THEMES.findIndex(t => t[0] === S.theme), n = THEMES[(i + 1) % THEMES.length]; S.theme = n[0]; store.set("theme", S.theme); render(); toast(n[1].toLowerCase()); }
  else if (e.key === "Escape" && S.modal) { S.modal = null; render(); }
});

/* queue drag & drop */
let dragQi = null;
document.addEventListener("dragstart", e => { const r = e.target.closest("[data-qi]"); if (r) { dragQi = +r.dataset.qi; e.dataTransfer.effectAllowed = "move"; } });
document.addEventListener("dragover", e => { const r = e.target.closest("[data-qi]"); if (r && dragQi !== null) { e.preventDefault(); document.querySelectorAll(".over").forEach(x => x.classList.remove("over")); r.classList.add("over"); } });
document.addEventListener("drop", e => {
  const r = e.target.closest("[data-qi]");
  if (!r || dragQi === null) return;
  e.preventDefault();
  const to = +r.dataset.qi, curT = cur();
  const [m] = S.queue.splice(dragQi, 1); S.queue.splice(to, 0, m);
  S.idx = S.queue.indexOf(curT); dragQi = null; render();
});
document.addEventListener("dragend", () => { dragQi = null; document.querySelectorAll(".over").forEach(x => x.classList.remove("over")); });
document.addEventListener("input", e => { if (e.target.id === "mq") S.mq = e.target.value; });

/* ================= boot ================= */
restoreSession();
render(true);
requestAnimationFrame(() => requestAnimationFrame(() => document.body.classList.add("ready")));
refreshStatus();
if (S.discord && S.discordId) discordOn(true);
window.api.downloads().then(list => { list.forEach(d => S.downloads.set(d.id, d)); render(); }).catch(() => {});

/* self-test hook (used only when the app is started with ABYSS_SELFTEST) */
window.__abyss = {
  S, go, render, cache, level: () => AE.level(), audioEl: () => audio, store,
  async playFirst(q) {
    const res = await window.api.search(q);
    const songs = res.find(s => s.kind === "song")?.items || [];
    if (!songs.length) throw new Error("no songs");
    startRadio(songs[0]);
    return songs[0].title;
  },
  audio: () => ({ t: audio.currentTime, paused: audio.paused, ready: audio.readyState, err: audio.error?.message || null, src: audio.src.slice(0, 40) }),
};
