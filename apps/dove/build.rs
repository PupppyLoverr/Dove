//! Generates the embedded icon table from `assets/icons/*.svg`: one
//! `pub const NAME: &str = "icons/name.svg"` per file plus a lookup that
//! returns the bytes. Only these files are embedded.

use std::fmt::Write as _;
use std::path::Path;

fn main() {
    let dir = Path::new("assets/icons");
    println!("cargo:rerun-if-changed=assets/icons");
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .expect("assets/icons exists")
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            name.strip_suffix(".svg").map(str::to_string)
        })
        .collect();
    names.sort();

    let mut out = String::new();
    for n in &names {
        let konst = n.to_uppercase().replace('-', "_");
        writeln!(out, "pub const {konst}: &str = \"icons/{n}.svg\";").unwrap();
    }
    out.push_str("\npub(crate) fn icon_bytes(path: &str) -> Option<&'static [u8]> {\n    match path {\n");
    for n in &names {
        let abs = std::fs::canonicalize(dir.join(format!("{n}.svg"))).unwrap();
        let abs = abs.to_string_lossy().replace('\\', "/");
        let abs = abs.strip_prefix("//?/").unwrap_or(&abs).to_string();
        writeln!(out, "        \"icons/{n}.svg\" => Some(include_bytes!(\"{abs}\")),").unwrap();
    }
    out.push_str("        _ => None,\n    }\n}\n\npub(crate) const ALL_ICONS: &[&str] = &[\n");
    for n in &names {
        writeln!(out, "    \"icons/{n}.svg\",").unwrap();
    }
    out.push_str("];\n");

    let dest = Path::new(&std::env::var("OUT_DIR").unwrap()).join("icons_gen.rs");
    std::fs::write(dest, out).unwrap();

    file_icons();
    linux_browser_helper();
    windows_icon();
}

/// Zeron's polychrome file-type icons (vscode-symbols) under
/// `assets/file-icons/{files,folders}`: a lookup from `files/x.svg` to the
/// SVG, deflated (1.4 MB of SVG embeds as about 0.4 MB).
fn file_icons() {
    use std::io::Write as _;
    let root = Path::new("assets/file-icons");
    println!("cargo:rerun-if-changed=assets/file-icons");
    let mut paths: Vec<String> = Vec::new();
    for sub in ["files", "folders"] {
        for e in std::fs::read_dir(root.join(sub)).expect("assets/file-icons exists").filter_map(|e| e.ok()) {
            let name = e.file_name().to_string_lossy().to_string();
            if name.ends_with(".svg") {
                paths.push(format!("{sub}/{name}"));
            }
        }
    }
    paths.sort();
    let mut out =
        String::from("pub(crate) fn file_icon_bytes(path: &str) -> Option<&'static [u8]> {\n    match path {\n");
    let packed = Path::new(&std::env::var("OUT_DIR").unwrap()).join("file-icons");
    for p in &paths {
        let svg = std::fs::read(root.join(p)).unwrap();
        let mut z = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::best());
        z.write_all(&svg).unwrap();
        let dest = packed.join(format!("{p}.deflate"));
        std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
        std::fs::write(&dest, z.finish().unwrap()).unwrap();
        let abs = dest.to_string_lossy().replace('\\', "/");
        let abs = abs.strip_prefix("//?/").unwrap_or(&abs).to_string();
        writeln!(out, "        \"{p}\" => Some(include_bytes!(\"{abs}\")),").unwrap();
    }
    out.push_str("        _ => None,\n    }\n}\n\npub(crate) const ALL_FILE_ICONS: &[&str] = &[\n");
    for p in &paths {
        writeln!(out, "    \"{p}\",").unwrap();
    }
    out.push_str("];\n");
    let dest = Path::new(&std::env::var("OUT_DIR").unwrap()).join("file_icons_gen.rs");
    std::fs::write(dest, out).unwrap();
}

/// The Linux browser renders in an offscreen WebKitGTK helper process (from
/// Zeron). It is compiled here and embedded in the binary.
fn linux_browser_helper() {
    use std::process::Command;
    println!("cargo:rerun-if-changed=src/browser/linux/helper.c");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("linux") {
        return;
    }
    let output = Path::new(&std::env::var("OUT_DIR").unwrap()).join("zeron-webkit");
    let flags = Command::new("pkg-config")
        .args(["--cflags", "--libs", "webkit2gtk-4.1", "json-glib-1.0"])
        .output()
        .expect("pkg-config is required to build the Linux browser helper");
    assert!(
        flags.status.success(),
        "The Linux browser needs WebKitGTK 4.1 and JSON-GLib development files (webkit2gtk-4.1, json-glib-1.0).\n{}",
        String::from_utf8_lossy(&flags.stderr)
    );
    let status = Command::new(std::env::var("CC").unwrap_or_else(|_| "cc".into()))
        .args(["-std=c11", "-O2", "-Wall", "-Wextra", "-Wno-unused-parameter", "src/browser/linux/helper.c", "-o"])
        .arg(&output)
        .args(String::from_utf8(flags.stdout).unwrap().split_whitespace())
        .status()
        .expect("a C compiler is required to build the Linux browser helper");
    assert!(status.success(), "Linux browser helper compilation failed");
}

/// Square PNG and multi-size ICO from the source photo, then the exe icon.
fn windows_icon() {
    println!("cargo:rerun-if-changed=assets/dove-icon.jpg");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let src = image::open("assets/dove-icon.jpg").expect("assets/dove-icon.jpg").into_rgba8();
    let (w, h) = src.dimensions();
    let side = w.min(h);
    let square = image::imageops::crop_imm(&src, (w - side) / 2, (h - side) / 2, side, side).to_image();
    square.save("assets/dove-icon.png").expect("write dove-icon.png");
    let sizes = [16u32, 24, 32, 48, 64, 128, 256];
    let mut raw = Vec::with_capacity(sizes.len());
    for size in sizes {
        let resized = image::imageops::resize(&square, size, size, image::imageops::FilterType::Lanczos3);
        raw.push((resized.into_raw(), size));
    }
    let frames = raw
        .iter()
        .map(|(buf, size)| {
            image::codecs::ico::IcoFrame::as_png(buf, *size, *size, image::ExtendedColorType::Rgba8).expect("ico frame")
        })
        .collect::<Vec<_>>();
    let file = std::fs::File::create("assets/dove.ico").expect("create dove.ico");
    image::codecs::ico::IcoEncoder::new(file).encode_images(&frames).expect("write dove.ico");
    let mut res = winresource::WindowsResource::new();
    res.set_icon("assets/dove.ico");
    res.compile().expect("embed the Dove icon in the exe");
}
