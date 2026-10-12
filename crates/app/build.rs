//! Generates the app icon from `assets/icon.png`: a small RGBA buffer for the window icon
//! and, on Windows, a multi-size `.ico` embedded in the executable.

use std::path::PathBuf;

const SOURCE: &str = "../../assets/icon.png";
const WINDOW_ICON_SIZE: u32 = 64;
/// Windows application manifest: per-monitor-v2 DPI, long paths, UTF-8 code page, Windows 10+.
const MANIFEST: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <compatibility xmlns="urn:schemas-microsoft-com:compatibility.v1">
    <application>
      <supportedOS Id="{8e0f7a12-bfb3-4fe8-b9a5-48fd50a15a9a}"/>
    </application>
  </compatibility>
  <application xmlns="urn:schemas-microsoft-com:asm.v3">
    <windowsSettings>
      <dpiAware xmlns="http://schemas.microsoft.com/SMI/2005/WindowsSettings">true/pm</dpiAware>
      <dpiAwareness xmlns="http://schemas.microsoft.com/SMI/2016/WindowsSettings">PerMonitorV2</dpiAwareness>
      <longPathAware xmlns="http://schemas.microsoft.com/SMI/2016/WindowsSettings">true</longPathAware>
      <activeCodePage xmlns="http://schemas.microsoft.com/SMI/2019/WindowsSettings">UTF-8</activeCodePage>
    </windowsSettings>
  </application>
</assembly>
"#;

fn main() {
    println!("cargo:rerun-if-changed={SOURCE}");
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR is set by cargo"));
    let icon = image::open(SOURCE).expect("assets/icon.png must be a readable PNG");
    let resize = |s: u32| {
        icon.resize_exact(s, s, image::imageops::FilterType::Lanczos3)
            .into_rgba8()
    };

    std::fs::write(
        out.join("window_icon.rgba"),
        resize(WINDOW_ICON_SIZE).into_raw(),
    )
    .expect("write window icon");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let ico_path = out.join("icon.ico");
        let file = std::fs::File::create(&ico_path).expect("create icon.ico");
        let frames: Vec<image::codecs::ico::IcoFrame> = [16, 24, 32, 48, 64, 128, 256]
            .into_iter()
            .map(|s| {
                let img = resize(s);
                image::codecs::ico::IcoFrame::as_png(&img, s, s, image::ExtendedColorType::Rgba8)
                    .expect("encode icon frame")
            })
            .collect();
        image::codecs::ico::IcoEncoder::new(file)
            .encode_images(&frames)
            .expect("write icon.ico");
        let mut res = winresource::WindowsResource::new();
        res.set_icon(ico_path.to_str().expect("utf-8 path"));
        res.set("ProductName", "KuArt");
        res.set("FileDescription", "KuArt design editor");
        res.set_manifest(MANIFEST);
        res.compile().expect("embed Windows resources");
    }
}
