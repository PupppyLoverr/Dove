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

    linux_browser_helper();
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
