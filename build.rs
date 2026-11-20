// Pre-decodes the app icon at build time (raw RGBA for the window, .ico for the exe),
// so nothing has to be decoded when abyss starts.
use std::{env, fs, path::PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=assets/icon.png");
    println!("cargo:rerun-if-changed=ui");
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    let img = image::open("assets/icon.png").expect("assets/icon.png").to_rgba8();

    let small = image::imageops::resize(&img, 64, 64, image::imageops::FilterType::Lanczos3);
    fs::write(out.join("icon64.rgba"), small.as_raw()).unwrap();

    let ico_path = out.join("icon.ico");
    let mut frames = Vec::new();
    for s in [16u32, 32, 48, 64, 128, 256] {
        let r = image::imageops::resize(&img, s, s, image::imageops::FilterType::Lanczos3);
        frames.push(image::codecs::ico::IcoFrame::as_png(r.as_raw(), s, s, image::ExtendedColorType::Rgba8).unwrap());
    }
    let f = fs::File::create(&ico_path).unwrap();
    image::codecs::ico::IcoEncoder::new(f).encode_images(&frames).unwrap();

    if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon(ico_path.to_str().unwrap());
        res.set("ProductName", "abyss");
        res.set("FileDescription", "abyss - music player");
        res.compile().unwrap();
    }
}
