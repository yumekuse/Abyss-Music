// Local web server for the UI (YouTube's embedded player needs a real http origin).
// Serves the UI from memory, the API, and a disk cache of the web fonts.
use crate::app::{App, UserEvent};
use crate::util::{self, log};
use axum::body::{Body, Bytes};
use axum::extract::{Path, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::extract::Request;
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{json, Map, Value};
use std::sync::{Arc, OnceLock};

const APP_JS: &str = include_str!("../ui/app.js");
const APP_CSS: &str = include_str!("../ui/app.css");
const EXTRA_CSS: &str = include_str!("../ui/extra.css");
const YTPLAYER_JS: &str = include_str!("../ui/ytplayer.js");
const SHIM_JS: &str = include_str!("../ui/shim.js");
const INDEX: &str = include_str!("../ui/index.html");
const FONTS_URL: &str = "https://fonts.googleapis.com/css2?family=Inter:wght@400;500;600;700;800&family=Pixelify+Sans:wght@400;500;600;700&family=Space+Mono:wght@400;700&family=Bungee&family=Anton&family=Orbitron:wght@600;800&family=Playfair+Display:ital,wght@0,700;0,800;1,700&family=VT323&family=Fredoka:wght@500;600;700&display=swap";

fn index() -> &'static str {
    static I: OnceLock<String> = OnceLock::new();
    I.get_or_init(|| {
        let mut h = INDEX.to_string();
        // fonts come from the local cache; api.js = settings + window.api, loaded before everything else
        if let (Some(a), Some(b)) = (h.find("<link rel=\"stylesheet\" href=\"https://fonts.googleapis.com"), h.find("<link rel=\"stylesheet\" href=\"app.css\">")) {
            h.replace_range(a..b, "<link rel=\"stylesheet\" href=\"fonts.css\">\n  ");
        }
        h = h.replace("<script src=\"ytplayer.js\"></script>", "<script src=\"api.js\"></script>\n  <script src=\"ytplayer.js\"></script>");
        h
    })
}

fn resp(ct: &'static str, body: impl Into<Body>, cache: &'static str) -> Response {
    let mut r = Response::new(body.into());
    r.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static(ct));
    r.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));
    r
}
const JS: &str = "text/javascript; charset=utf-8";
const CSS: &str = "text/css; charset=utf-8";

pub async fn serve(listener: std::net::TcpListener, app: Arc<App>) {
    listener.set_nonblocking(true).ok();
    let listener = tokio::net::TcpListener::from_std(listener).expect("listener");
    let r = Router::new()
        .route("/", get(|| async { resp("text/html; charset=utf-8", index(), "no-store") }))
        .route("/index.html", get(|| async { resp("text/html; charset=utf-8", index(), "no-store") }))
        .route("/app.js", get(|| async { resp(JS, APP_JS, "no-cache") }))
        .route("/ytplayer.js", get(|| async { resp(JS, YTPLAYER_JS, "no-cache") }))
        .route("/app.css", get(|| async { resp(CSS, APP_CSS, "no-cache") }))
        .route("/extra.css", get(|| async { resp(CSS, EXTRA_CSS, "no-cache") }))
        .route(
            "/api.js",
            get(|State(app): State<Arc<App>>| async move {
                let store = app.store_json().replace("</", "<\\/");
                resp(JS, format!("window.__ABYSS_STORE={store};\n{SHIM_JS}"), "no-store")
            }),
        )
        .route(
            "/api/{name}",
            post(|State(app): State<Arc<App>>, Path(name): Path<String>, body: Bytes| async move {
                let args: Vec<Value> = serde_json::from_slice(&body).unwrap_or_default();
                let r = match app.api(name, args).await {
                    Ok(v) => json!({ "ok": true, "v": v }),
                    Err(e) => json!({ "ok": false, "e": e }),
                };
                Json(r)
            }),
        )
        .route(
            "/store",
            post(|State(app): State<Arc<App>>, body: Bytes| async move {
                if let Ok(m) = serde_json::from_slice::<Map<String, Value>>(&body) {
                    app.store_set_many(m);
                }
                StatusCode::NO_CONTENT
            }),
        )
        .route("/auth-code", get(|State(app): State<Arc<App>>| async move { Json(app.yt.auth.code.lock().unwrap().clone().unwrap_or(Value::Null)) }))
        .route("/ping", get(|| async { "abyss" }))
        .route(
            "/focus",
            get(|State(app): State<Arc<App>>| async move {
                app.send(UserEvent::Focus);
                "abyss"
            }),
        )
        .route("/fonts.css", get(|State(app): State<Arc<App>>| async move { fonts_css(&app).await }))
        .route("/fonts/{*path}", get(|State(app): State<Arc<App>>, Path(p): Path<String>| async move { font_file(&app, &p).await }))
        .route(
            "/dev/eval",
            post(|State(app): State<Arc<App>>, body: String| async move {
                // only with ABYSS_DEV=1 (used to take the README screenshots)
                if std::env::var_os("ABYSS_DEV").is_none() {
                    return StatusCode::NOT_FOUND;
                }
                app.send(UserEvent::Eval(body));
                StatusCode::NO_CONTENT
            }),
        )
        .layer(middleware::from_fn(guard))
        .with_state(app);
    if let Err(e) = axum::serve(listener, r).await {
        log(format!("server stopped: {e}"));
    }
}

/// The UI's own requests carry `x-abyss: 1` and go to localhost. Web pages from other sites
/// can't add that header without a CORS preflight (which this server never approves), and a
/// DNS-rebinding page would send a different Host — so neither can drive the app.
async fn guard(req: Request, next: Next) -> Response {
    let p = req.uri().path();
    let sensitive = p.starts_with("/api/") || p == "/store" || p == "/auth-code" || p.starts_with("/dev/") || p == "/api.js";
    let host_ok = req
        .headers()
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .map(|h| h.split(':').next().unwrap_or(""))
        .is_some_and(|h| h == "localhost" || h == "127.0.0.1");
    if !host_ok {
        return StatusCode::FORBIDDEN.into_response();
    }
    // browsers say where a request comes from; only the app's own page may read these
    if let Some(site) = req.headers().get("sec-fetch-site").and_then(|v| v.to_str().ok()) {
        if sensitive && site != "same-origin" && site != "none" {
            return StatusCode::FORBIDDEN.into_response();
        }
    }
    if sensitive && p != "/api.js" && req.headers().get("x-abyss").and_then(|v| v.to_str().ok()) != Some("1") {
        return StatusCode::FORBIDDEN.into_response();
    }
    if let Some(o) = req.headers().get(header::ORIGIN).and_then(|v| v.to_str().ok()) {
        if sensitive && !(o.starts_with("http://localhost:") || o.starts_with("http://127.0.0.1:")) {
            return StatusCode::FORBIDDEN.into_response();
        }
    }
    next.run(req).await
}

/* ---- web fonts: fetched once, then served from disk (no network needed to draw the UI) ---- */
fn font_dir() -> std::path::PathBuf {
    util::local_dir().join("fonts")
}
async fn fonts_css(app: &App) -> Response {
    let f = font_dir().join("fonts.css");
    if let Ok(b) = tokio::fs::read(&f).await {
        return resp(CSS, b, "no-cache");
    }
    let got = async {
        let r = app.yt.http.get(FONTS_URL).header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36").send().await.ok()?;
        if !r.status().is_success() {
            return None;
        }
        Some(r.text().await.ok()?.replace("https://fonts.gstatic.com/s/", "/fonts/"))
    }
    .await;
    match got {
        Some(css) => {
            let _ = util::write_atomic(&f, css.as_bytes());
            resp(CSS, css, "no-cache")
        }
        None => resp(CSS, "", "no-store"), // offline on the very first start: system fonts for now
    }
}
async fn font_file(app: &App, p: &str) -> Response {
    if p.contains("..") || p.contains('\\') {
        return StatusCode::NOT_FOUND.into_response();
    }
    let f = font_dir().join(p.replace('/', "_"));
    let ct = if p.ends_with(".woff2") { "font/woff2" } else { "application/octet-stream" };
    if let Ok(b) = tokio::fs::read(&f).await {
        return resp(ct, b, "public, max-age=31536000, immutable");
    }
    let Ok(r) = app.yt.http.get(format!("https://fonts.gstatic.com/s/{p}")).send().await else { return StatusCode::BAD_GATEWAY.into_response() };
    if !r.status().is_success() {
        return StatusCode::BAD_GATEWAY.into_response();
    }
    let Ok(b) = r.bytes().await else { return StatusCode::BAD_GATEWAY.into_response() };
    let _ = util::write_atomic(&f, &b);
    resp(ct, b, "public, max-age=31536000, immutable")
}
