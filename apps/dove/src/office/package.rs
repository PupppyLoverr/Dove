//! An OOXML package (a ZIP of XML parts): reading parts and relationships,
//! and saving with replaced parts while every other entry is copied raw.

use std::collections::{HashMap, HashSet};
use std::io::{Cursor, Read, Write};
use std::path::Path;
use std::sync::Arc;

use super::xml::{self, Tok};

pub struct Package {
    bytes: Arc<Vec<u8>>,
    names: Vec<String>,
}

/// One relationship of a part.
pub struct Rel {
    pub target: String,
    pub kind: String,
}

impl Package {
    pub fn open(path: &Path) -> Result<Self, String> {
        let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
        Self::from_bytes(bytes)
    }

    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self, String> {
        let zip =
            zip::ZipArchive::new(Cursor::new(&bytes)).map_err(|_| "Not an Office document (bad ZIP)".to_string())?;
        let names = zip.file_names().map(str::to_string).collect();
        Ok(Self { bytes: Arc::new(bytes), names })
    }

    pub fn has(&self, name: &str) -> bool {
        self.names.iter().any(|n| n == name)
    }

    pub fn read(&self, name: &str) -> Option<Vec<u8>> {
        let mut zip = zip::ZipArchive::new(Cursor::new(self.bytes.as_slice())).ok()?;
        let mut f = zip.by_name(name).ok()?;
        let mut out = Vec::with_capacity(f.size() as usize);
        f.read_to_end(&mut out).ok()?;
        Some(out)
    }

    pub fn text(&self, name: &str) -> Option<String> {
        self.read(name).map(|b| String::from_utf8_lossy(&b).into_owned())
    }

    /// The relationships of `part`, by id, with targets resolved to part names.
    pub fn rels(&self, part: &str) -> HashMap<String, Rel> {
        let (dir, file) = part.rsplit_once('/').unwrap_or(("", part));
        let rels_name = if dir.is_empty() { format!("_rels/{file}.rels") } else { format!("{dir}/_rels/{file}.rels") };
        let mut out = HashMap::new();
        let Some(src) = self.text(&rels_name) else { return out };
        for t in xml::tokens(&src) {
            let Tok::Open { name, attrs, .. } = t else { continue };
            if xml::local(name) != "Relationship" || xml::attr(attrs, "TargetMode").as_deref() == Some("External") {
                continue;
            }
            let (Some(id), Some(target)) = (xml::attr(attrs, "Id"), xml::attr(attrs, "Target")) else { continue };
            let kind = xml::attr(attrs, "Type").unwrap_or_default();
            out.insert(id, Rel { target: resolve(dir, &target), kind });
        }
        out
    }

    /// Write the package to `dest`: `replace` parts get new contents,
    /// `remove` parts are dropped, the rest is copied byte for byte. The file
    /// is replaced atomically.
    pub fn save(
        &self,
        dest: &Path,
        replace: &HashMap<String, Vec<u8>>,
        remove: &HashSet<String>,
    ) -> Result<(), String> {
        let mut zip = zip::ZipArchive::new(Cursor::new(self.bytes.as_slice())).map_err(|e| e.to_string())?;
        let mut out = zip::ZipWriter::new(Cursor::new(Vec::with_capacity(self.bytes.len() + 4096)));
        let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        for i in 0..zip.len() {
            let file = zip.by_index_raw(i).map_err(|e| e.to_string())?;
            let name = file.name().to_string();
            if remove.contains(&name) {
                continue;
            }
            match replace.get(&name) {
                Some(data) => {
                    drop(file);
                    out.start_file(name, opts).map_err(|e| e.to_string())?;
                    out.write_all(data).map_err(|e| e.to_string())?;
                }
                None => out.raw_copy_file(file).map_err(|e| e.to_string())?,
            }
        }
        let bytes = out.finish().map_err(|e| e.to_string())?.into_inner();
        let tmp =
            dest.with_extension(format!("{}.dove-save", dest.extension().and_then(|e| e.to_str()).unwrap_or("tmp")));
        std::fs::write(&tmp, &bytes).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, dest).map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            e.to_string()
        })
    }
}

/// Resolve a relationship target against the source part's folder.
pub fn resolve(dir: &str, target: &str) -> String {
    if let Some(abs) = target.strip_prefix('/') {
        return abs.to_string();
    }
    let mut parts: Vec<&str> = dir.split('/').filter(|s| !s.is_empty()).collect();
    for seg in target.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    parts.join("/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_targets() {
        assert_eq!(resolve("ppt/slides", "../media/image1.png"), "ppt/media/image1.png");
        assert_eq!(resolve("xl", "worksheets/sheet1.xml"), "xl/worksheets/sheet1.xml");
        assert_eq!(resolve("xl", "/xl/worksheets/sheet2.xml"), "xl/worksheets/sheet2.xml");
    }
}
