/* window.api for the Rust build: same calls as the old Electron preload, over the local server. */
(() => {
  const names = ["status", "home", "moods", "genre", "search", "suggest", "upNext", "related", "album", "artist", "playlist", "library", "personal", "following", "lyrics", "romanize", "translate", "discordConnect", "discordOff", "discordSet",
    "streamInfo", "like", "createPlaylist", "addToPlaylist", "removeFromPlaylist", "loginCode", "loginCancel", "loginCookie", "openDevice", "openYT", "logout",
    "downloads", "download", "removeDownload", "openDownloads", "dlQueue", "dlCancel", "dlState",
    "updateState", "updateCheck", "updateInstall", "news", "newsRefresh", "openUrl"];
  const call = (n, a) => fetch("/api/" + n, { method: "POST", headers: { "content-type": "application/json", "x-abyss": "1" }, body: JSON.stringify(a) })
    .then(r => r.json()).then(r => { if (!r.ok) throw new Error(r.e); return r.v; });
  const api = Object.fromEntries(names.map(n => [n, (...a) => call(n, a)]));

  // device sign-in: the code is picked up while the login call is running
  let onCode = null;
  api.onAuthCode = cb => { onCode = cb; };
  const login = api.loginCode;
  api.loginCode = (...a) => {
    let last = "";
    const t = setInterval(() => fetch("/auth-code", { headers: { "x-abyss": "1" } }).then(r => r.json()).then(c => { const s = JSON.stringify(c); if (c && s !== last) { last = s; onCode && onCode(c); } }).catch(() => {}), 600);
    return login(...a).finally(() => clearInterval(t));
  };

  // settings: batched writes; flushed synchronously when the window closes
  let pending = {}, timer = null;
  const body = () => { const p = pending; pending = {}; return JSON.stringify(p); };
  const flush = () => { clearTimeout(timer); timer = null; if (Object.keys(pending).length) fetch("/store", { method: "POST", headers: { "content-type": "application/json", "x-abyss": "1" }, body: body(), keepalive: true }).catch(() => {}); };
  const flushSync = () => {
    clearTimeout(timer); timer = null;
    if (!Object.keys(pending).length) return;
    try { const x = new XMLHttpRequest(); x.open("POST", "/store", false); x.setRequestHeader("content-type", "application/json"); x.setRequestHeader("x-abyss", "1"); x.send(body()); } catch {}
  };
  api.store = {
    load: () => window.__ABYSS_STORE || {},
    set: (k, v) => { pending[k] = v === undefined ? null : v; if (!timer) timer = setTimeout(flush, 300); },
  };
  window.addEventListener("pagehide", flushSync);
  window.__abyssClose = () => { try { window.dispatchEvent(new Event("beforeunload")); } catch {} flushSync(); return true; };

  // the app answered from its saved copy at start; fresher data arrived in the background
  const KEYS = { home: "home", personal: "me", library: "library", following: "following", moods: "moods" };
  window.__abyssFresh = async n => {
    const A = window.__abyss;
    if (!A) return;
    try {
      if (n === "account") { const st = await api.status(); A.S.account = st.account; return A.render(); }
      if (n === "playlist:LM") {
        const p = await api.playlist("LM");
        A.cache.set("pl:LM", { ok: true, v: p });
        p.tracks.forEach(t => A.S.liked.set(t.id, t));
        A.store.set("liked", [...A.S.liked]);
        return A.render();
      }
      const k = KEYS[n] || (n.startsWith("genre") ? null : n);
      if (k && A.cache.has(k)) { A.cache.delete(k); A.render(); }
    } catch {}
  };
  window.api = api;

  // startup timing: when the first song row / card is on screen
  const mo = new MutationObserver(() => { if (document.querySelector(".row, .card")) { window.__abyssFirst = performance.now(); mo.disconnect(); } });
  document.addEventListener("DOMContentLoaded", () => mo.observe(document.body, { childList: true, subtree: true }));
})();
