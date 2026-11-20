// abyss – native shell: one small exe, a WebView2 window, and a Rust backend.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod auth;
mod discord;
mod dl;
mod lyrics;
mod romanize;
mod server;
mod update;
mod util;
mod yt;

use app::{App, UserEvent};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::{Duration, Instant};
use tao::dpi::{LogicalSize, PhysicalPosition};
use tao::event::{Event, StartCause, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tao::window::{Icon, WindowBuilder};
use wry::{NewWindowResponse, WebContext, WebViewBuilder};

const DEFAULT_PORT: u16 = 47811; // fixed port = same origin every launch (ABYSS_PORT overrides, for tests)
fn port() -> u16 {
    std::env::var("ABYSS_PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(DEFAULT_PORT)
}

/// another abyss already running? bring it to the front instead of starting twice
fn focus_running() -> bool {
    let Ok(mut s) = TcpStream::connect_timeout(&([127, 0, 0, 1], port()).into(), Duration::from_millis(300)) else { return false };
    let _ = s.set_read_timeout(Some(Duration::from_millis(800)));
    if s.write_all(b"GET /focus HTTP/1.0\r\nHost: localhost\r\n\r\n").is_err() {
        return false;
    }
    let mut buf = String::new();
    let _ = s.read_to_string(&mut buf);
    buf.ends_with("abyss")
}

/// ABYSS_SELFTEST=1: measures startup, plays a song, writes the results to settings key "__selftest", quits
fn selftest(app: std::sync::Arc<App>, start_ms: u64) {
    std::thread::spawn(move || {
        let report = |label: &str, expr: &str| {
            format!("(async()=>{{let v;try{{v=await ({expr})}}catch(e){{v='ERR '+e.message}}const p={{}};p['__selftest_{label}']=v;fetch('/store',{{method:'POST',headers:{{'content-type':'application/json','x-abyss':'1'}},body:JSON.stringify(p)}})}})()")
        };
        std::thread::sleep(Duration::from_secs(4));
        app.send(UserEvent::Eval(report("start", &format!("({{processStart:{start_ms},firstContentMs:Math.round(performance.timeOrigin+(window.__abyssFirst||NaN))-{start_ms},domReadyMs:Math.round(performance.timeOrigin+performance.getEntriesByType('navigation')[0].domContentLoadedEventEnd)-{start_ms},rows:document.querySelectorAll('.row,.card').length}})"))));
        // silent: player muted before anything plays
        app.send(UserEvent::Eval("(()=>{const a=window.__abyss.audioEl();a.volume=0;a.p.forEach(p=>{try{p.mute();p.setVolume(0)}catch(e){}})})()".into()));
        app.send(UserEvent::Eval(report("play", "(async()=>{const r=await window.__abyss.playFirst('daft punk one more time');const a=window.__abyss.audioEl();a.volume=0;a.p.forEach(p=>{try{p.mute()}catch(e){}});return r})()")));
        std::thread::sleep(Duration::from_secs(10));
        app.send(UserEvent::Eval(report("audio", "window.__abyss.audio()")));
        std::thread::sleep(Duration::from_secs(2));
        app.store_flush();
        app.send(UserEvent::Quit);
    });
}

fn theme_color(app: &App) -> (u8, u8, u8) {
    let hex = match app.store_get("theme").as_str().unwrap_or("cream") {
        "white" => "#ececec".to_string(),
        "dark" => "#0c0c0c".into(),
        "vice" => "#22093f".into(),
        "latte" => "#f5ecdd".into(),
        "sakura" => "#fff0f5".into(),
        "custom" => app.store_get("custom")["paper"].as_str().unwrap_or("#fff6ca").to_string(),
        _ => "#fff6ca".into(),
    };
    let h = hex.trim_start_matches('#');
    let p = |i: usize| u8::from_str_radix(h.get(i..i + 2).unwrap_or("ff"), 16).unwrap_or(255);
    (p(0), p(2), p(4))
}

fn main() {
    let t0 = Instant::now();
    #[cfg(target_os = "linux")]
    if std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none() {
        // avoids a blank window with some GPU drivers (NVIDIA, some Wayland setups); set it to 0 to opt out
        // SAFETY: nothing else is running yet (no threads started).
        unsafe { std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1") };
    }
    let listener = match TcpListener::bind(("127.0.0.1", port())) {
        Ok(l) => l,
        Err(_) => {
            if focus_running() {
                return;
            }
            TcpListener::bind(("127.0.0.1", 0)).expect("no free port")
        }
    };
    let port = listener.local_addr().map(|a| a.port()).unwrap_or(port());

    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    let app = App::new(event_loop.create_proxy());

    // backend: tokio on worker threads (server + YouTube), the UI thread stays free
    let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(4).enable_all().thread_name("abyss-rt").build().expect("runtime");
    rt.spawn(server::serve(listener, app.clone()));
    rt.spawn(app.clone().warm());
    rt.spawn(dl::Downloads::pump(app.clone()));
    rt.spawn(update::Updater::run(app.clone()));

    let (r, g, b) = theme_color(&app);
    let icon = Icon::from_rgba(include_bytes!(concat!(env!("OUT_DIR"), "/icon64.rgba")).to_vec(), 64, 64).ok();
    let window = WindowBuilder::new()
        .with_title("abyss")
        .with_inner_size(LogicalSize::new(1440.0, 900.0))
        .with_min_inner_size(LogicalSize::new(980.0, 640.0))
        .with_window_icon(icon)
        .with_visible(false)
        .build(&event_loop)
        .expect("window");
    // center on the current screen
    if let Some(m) = window.current_monitor() {
        let (ms, ws) = (m.size(), window.outer_size());
        let p = m.position();
        window.set_outer_position(PhysicalPosition::new(p.x + (ms.width as i32 - ws.width as i32).max(0) / 2, p.y + (ms.height as i32 - ws.height as i32).max(0) / 2));
    }

    let mut ctx = WebContext::new(Some(util::local_dir().join(if cfg!(windows) { "WebView2" } else { "webview" })));
    let base = format!("http://localhost:{port}/");
    let allowed = base.clone();
    let builder = WebViewBuilder::new_with_web_context(&mut ctx)
        .with_url(&base)
        .with_background_color((r, g, b, 255))
        .with_autoplay(true)
        .with_devtools(cfg!(debug_assertions))
        .with_hotkeys_zoom(false)
        .with_navigation_handler(move |url| {
            if url.starts_with(&allowed) || url.starts_with("about:") {
                return true;
            }
            if url.starts_with("https://") || url.starts_with("http://") {
                let _ = open::that_detached(url);
            }
            false
        })
        .with_new_window_req_handler(|url, _| {
            if url.starts_with("https://") || url.starts_with("http://") {
                let _ = open::that_detached(url);
            }
            NewWindowResponse::Deny
        });
    #[cfg(not(target_os = "linux"))]
    let webview = builder.build(&window).expect("webview");
    #[cfg(target_os = "linux")]
    let webview = {
        // GTK: put the web view in the window's box (works on X11 and Wayland)
        use tao::platform::unix::WindowExtUnix;
        use wry::WebViewBuilderExtUnix;
        builder.build_gtk(window.default_vbox().expect("gtk box")).expect("webview")
    };
    window.set_visible(true);
    util::log(format!("window up in {} ms", t0.elapsed().as_millis()));
    if std::env::var_os("ABYSS_SELFTEST").is_some() {
        selftest(app.clone(), util::now_ms() - t0.elapsed().as_millis() as u64);
    }

    let mut closing: Option<Instant> = None;
    event_loop.run(move |event, _, flow| {
        *flow = match closing {
            Some(t) => ControlFlow::WaitUntil(t + Duration::from_millis(1500)),
            None => ControlFlow::Wait,
        };
        let quit = |flow: &mut ControlFlow| {
            app.store_flush();
            app.discord.off();
            *flow = ControlFlow::Exit;
        };
        match event {
            Event::WindowEvent { event: WindowEvent::CloseRequested, .. } => {
                if closing.is_some() {
                    return quit(flow);
                }
                closing = Some(Instant::now());
                // let the page save the session (queue, position, ...) before quitting
                let a = app.clone();
                let ok = webview.evaluate_script_with_callback("window.__abyssClose ? window.__abyssClose() : true", move |_| a.send(UserEvent::Quit));
                if ok.is_err() {
                    quit(flow);
                }
            }
            Event::NewEvents(StartCause::ResumeTimeReached { .. }) if closing.is_some() => quit(flow),
            Event::UserEvent(UserEvent::Quit) => quit(flow),
            Event::UserEvent(UserEvent::Eval(js)) => {
                let _ = webview.evaluate_script(&js);
            }
            Event::UserEvent(UserEvent::Focus) => {
                window.set_minimized(false);
                window.set_visible(true);
                window.set_focus();
            }
            _ => {}
        }
    });
}
