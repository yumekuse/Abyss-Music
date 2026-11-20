/* Official YouTube IFrame player wrapped to look like an <audio> element
   (currentTime, paused, play(), pause(), src, playbackRate, volume + media events),
   so the rest of the app keeps working. Two players are stacked so songs can crossfade.
   The active player is always the visible one on top (YouTube's rules: player stays visible). */
class YTAudio extends EventTarget {
  constructor() {
    super();
    this._t = 0; this._dur = 0; this._paused = true; this._src = ""; this._rate = 1; this._vol = 0.7;
    this.error = null; this.readyState = 0;
    this.p = [null, null]; this.ready = [false, false]; this.a = 0; this.fading = false; this._fadeT = null;
    this._queued = null; this._queuedCue = null;
    window.onYouTubeIframeAPIReady = () => { this._create(0, "ytplayer"); this._create(1, "ytplayer2"); };
    const s = document.createElement("script");
    s.src = "https://www.youtube.com/iframe_api";
    s.onerror = () => { this.error = { code: -1, message: "couldn't load the youtube player (no internet?)" }; this._emit("error"); };
    document.head.appendChild(s);
    setInterval(() => this._poll(), 250);
  }
  get player() { return this.p[this.a]; }
  get _ready() { return this.ready[this.a]; }
  _create(i, el) {
    this.p[i] = new YT.Player(el, {
      width: "100%", height: "100%",
      playerVars: { autoplay: 1, controls: 0, disablekb: 1, fs: 0, rel: 0, playsinline: 1, iv_load_policy: 3, origin: location.origin },
      events: {
        onReady: () => {
          this.ready[i] = true;
          this.p[i].setVolume(i === this.a ? Math.round(this._vol * 100) : 0);
          this._layer();
          if (i !== this.a) return;
          if (this._queued) { const q = this._queued; this._queued = null; this._load(q); }
          else if (this._queuedCue) { const c = this._queuedCue; this._queuedCue = null; this.player.cueVideoById({ videoId: c.id, startSeconds: c.t }); }
        },
        onStateChange: e => { if (i === this.a) this._state(e.data); },
        onError: e => { if (i === this.a) this._err(e.data); },
      },
    });
  }
  _layer() {
    this.p.forEach((pl, i) => { const f = pl?.getIframe?.(); if (f) { f.style.position = "absolute"; f.style.inset = "0"; f.style.zIndex = i === this.a ? 2 : 1; } });
  }
  _emit(n) { this.dispatchEvent(new Event(n)); }
  _stopFade() {
    if (!this._fadeT) return;
    clearInterval(this._fadeT); this._fadeT = null; this.fading = false;
    const old = this.p[1 - this.a];
    try { old?.pauseVideo(); old?.setVolume(0); } catch {}
    try { this.player?.setVolume(Math.round(this._vol * 100)); } catch {}
  }
  _load(id) { this._stopFade(); this.player.setVolume(Math.round(this._vol * 100)); this.player.loadVideoById({ videoId: id, startSeconds: 0 }); this.player.setPlaybackRate(this._rate); }
  get src() { return this._src; }
  set src(v) {
    this._src = v || "";
    this.error = null; this.readyState = 0; this._t = 0; this._dur = 0;
    const id = this._src.replace(/^yt:/, "");
    if (!id) return;
    if (this._ready) this._load(id); else this._queued = id;
  }
  // load a song paused at a position (used to resume the last session)
  cue(id, t = 0) {
    this._src = "yt:" + id; this.error = null; this._t = t; this._dur = 0; this._paused = true;
    if (this._ready) this.player.cueVideoById({ videoId: id, startSeconds: t }); else this._queuedCue = { id, t };
  }
  // smooth transition: start `id` on the other player and slide the volumes across `secs` seconds
  crossTo(id, secs) {
    const n = 1 - this.a;
    if (!this.ready[n] || !this._ready || secs <= 0) { this.src = "yt:" + id; return; }
    this._stopFade();
    const old = this.p[this.a], nw = this.p[n], full = this._vol * 100;
    nw.setVolume(0); nw.loadVideoById({ videoId: id, startSeconds: 0 }); nw.setPlaybackRate(this._rate);
    this.a = n; this._layer();
    this._src = "yt:" + id; this._t = 0; this._dur = 0; this.error = null; this.readyState = 0;
    this.fading = true;
    const steps = Math.max(1, Math.round(secs * 10)); let k = 0;
    this._fadeT = setInterval(() => {
      k++;
      const f = Math.min(1, k / steps);
      try { nw.setVolume(Math.round(full * Math.sin((f * Math.PI) / 2))); old.setVolume(Math.round(full * Math.cos((f * Math.PI) / 2))); } catch {}
      if (f >= 1) { clearInterval(this._fadeT); this._fadeT = null; this.fading = false; try { old.pauseVideo(); old.setVolume(0); } catch {} }
    }, 100);
  }
  play() { if (this._ready) this.player.playVideo(); return Promise.resolve(); }
  pause() { this._stopFade(); if (this._ready) this.player.pauseVideo(); }
  get paused() { return this._paused; }
  get currentTime() { return this._t; }
  set currentTime(v) { this._t = Math.max(0, +v || 0); if (this._ready) { this.player.seekTo(this._t, true); this._emit("timeupdate"); this._emit("seeked"); } }
  get duration() { return this._dur; }
  get playbackRate() { return this._rate; }
  set playbackRate(r) { this._rate = r; this.p.forEach((pl, i) => { if (this.ready[i]) pl.setPlaybackRate(r); }); }
  get volume() { return this._vol; }
  set volume(v) { this._vol = v; if (this._ready && !this.fading) this.player.setVolume(Math.round(v * 100)); }
  // YT states: -1 unstarted, 0 ended, 1 playing, 2 paused, 3 buffering, 5 cued
  _state(s) {
    if (s === 1) {
      this._paused = false; this.readyState = 4;
      const d = this.player.getDuration();
      if (d && Math.abs(d - this._dur) > 0.5) { this._dur = d; this._emit("loadedmetadata"); }
      this._emit("playing");
    } else if (s === 2) { this._paused = true; this._emit("pause"); }
    else if (s === 3) this._emit("waiting");
    else if (s === 0) { this._paused = true; this._emit("ended"); }
  }
  _err(code) {
    const msg = code === 101 || code === 150 ? "the owner doesn't allow this song outside youtube" : code === 100 ? "video not found or private" : code === 5 ? "player can't play this video" : "player error " + code;
    this.error = { code, message: msg };
    this._emit("error");
  }
  _poll() {
    if (!this._ready || this._paused) return;
    const t = this.player.getCurrentTime?.();
    if (typeof t === "number" && Math.abs(t - this._t) > 0.05) { this._t = t; this._emit("timeupdate"); }
    const d = this.player.getDuration?.();
    if (d && Math.abs(d - this._dur) > 0.5) { this._dur = d; this._emit("loadedmetadata"); }
  }
}
window.YTAudio = YTAudio;
