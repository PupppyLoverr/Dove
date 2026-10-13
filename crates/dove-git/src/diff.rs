//! Unified-diff parsing into a shape the diff view renders directly.

use std::path::Path;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileStatus {
    Added,
    Deleted,
    Modified,
    Renamed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    Context,
    Added,
    Removed,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DiffLine {
    pub kind: LineKind,
    pub old_no: Option<u32>,
    pub new_no: Option<u32>,
    pub text: Arc<str>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Hunk {
    pub header: Arc<str>,
    pub lines: Vec<DiffLine>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FileDiff {
    pub path: String,
    pub old_path: Option<String>,
    pub status: FileStatus,
    pub binary: bool,
    pub additions: u32,
    pub deletions: u32,
    pub hunks: Vec<Hunk>,
}

impl FileDiff {
    fn new(path: String) -> Self {
        Self {
            path,
            old_path: None,
            status: FileStatus::Modified,
            binary: false,
            additions: 0,
            deletions: 0,
            hunks: Vec::new(),
        }
    }
}

/// Parse `git diff` output. Unknown header lines are ignored, so newer git
/// versions adding metadata do not break it.
pub fn parse(patch: &str) -> Vec<FileDiff> {
    let mut files: Vec<FileDiff> = Vec::new();
    let (mut old_no, mut new_no) = (0u32, 0u32);
    for line in patch.lines() {
        if let Some(rest) = line.strip_prefix("diff --git ") {
            let path = rest.rsplit_once(" b/").map(|(_, b)| b).unwrap_or(rest);
            files.push(FileDiff::new(unquote(path)));
            continue;
        }
        let Some(f) = files.last_mut() else { continue };
        if f.hunks.is_empty() {
            if line.starts_with("new file mode") {
                f.status = FileStatus::Added;
            } else if line.starts_with("deleted file mode") {
                f.status = FileStatus::Deleted;
            } else if let Some(from) = line.strip_prefix("rename from ") {
                f.status = FileStatus::Renamed;
                f.old_path = Some(unquote(from));
            } else if let Some(to) = line.strip_prefix("rename to ") {
                f.path = unquote(to);
            } else if line.starts_with("Binary files ") {
                f.binary = true;
            }
        }
        if line.starts_with("@@") {
            let (o, n) = hunk_starts(line);
            old_no = o;
            new_no = n;
            f.hunks.push(Hunk { header: line.into(), lines: Vec::new() });
            continue;
        }
        let Some(h) = f.hunks.last_mut() else { continue };
        let (kind, text) = match line.as_bytes().first() {
            Some(b'+') => (LineKind::Added, &line[1..]),
            Some(b'-') => (LineKind::Removed, &line[1..]),
            Some(b' ') => (LineKind::Context, &line[1..]),
            Some(b'\\') => continue, // "\ No newline at end of file"
            None => (LineKind::Context, ""),
            _ => continue,
        };
        let (o, n) = match kind {
            LineKind::Added => {
                f.additions += 1;
                new_no += 1;
                (None, Some(new_no - 1))
            }
            LineKind::Removed => {
                f.deletions += 1;
                old_no += 1;
                (Some(old_no - 1), None)
            }
            LineKind::Context => {
                old_no += 1;
                new_no += 1;
                (Some(old_no - 1), Some(new_no - 1))
            }
        };
        h.lines.push(DiffLine { kind, old_no: o, new_no: n, text: text.into() });
    }
    files
}

/// `@@ -12,7 +12,8 @@ fn x` → (12, 12)
fn hunk_starts(header: &str) -> (u32, u32) {
    let mut parts = header.split_whitespace().skip(1);
    let num =
        |s: Option<&str>| -> u32 { s.and_then(|s| s[1..].split(',').next()).and_then(|n| n.parse().ok()).unwrap_or(1) };
    let o = num(parts.next());
    let n = num(parts.next());
    (o, n)
}

fn unquote(p: &str) -> String {
    p.trim_matches('"').to_string()
}

/// An untracked file rendered as an all-added diff. Large or binary files get
/// no hunks, only the flag, so the view never loads megabytes.
pub fn untracked_file(wt: &Path, path: &str) -> FileDiff {
    const MAX: u64 = 512 * 1024;
    let mut f = FileDiff::new(path.to_string());
    f.status = FileStatus::Added;
    let full = wt.join(path);
    let size = std::fs::metadata(&full).map(|m| m.len()).unwrap_or(0);
    let bytes = if size <= MAX { std::fs::read(&full).unwrap_or_default() } else { Vec::new() };
    if size > MAX || bytes.contains(&0) {
        f.binary = true;
        return f;
    }
    let text = String::from_utf8_lossy(&bytes);
    let lines: Vec<DiffLine> = text
        .lines()
        .enumerate()
        .map(|(i, l)| DiffLine { kind: LineKind::Added, old_no: None, new_no: Some(i as u32 + 1), text: l.into() })
        .collect();
    f.additions = lines.len() as u32;
    if !lines.is_empty() {
        f.hunks.push(Hunk { header: format!("@@ -0,0 +1,{} @@", lines.len()).into(), lines });
    }
    f
}

#[cfg(test)]
mod tests {
    use super::*;

    const PATCH: &str = "diff --git a/src/a.rs b/src/a.rs
index 1111111..2222222 100644
--- a/src/a.rs
+++ b/src/a.rs
@@ -10,4 +10,5 @@ fn main() {
 keep
-old
+new
+added
 tail
\\ No newline at end of file
diff --git a/gone.txt b/gone.txt
deleted file mode 100644
index 3333333..0000000
--- a/gone.txt
+++ /dev/null
@@ -1 +0,0 @@
-bye
diff --git a/old name.md b/new name.md
similarity index 90%
rename from old name.md
rename to new name.md
diff --git a/img.png b/img.png
new file mode 100644
Binary files /dev/null and b/img.png differ
";

    #[test]
    fn parses_files_and_line_numbers() {
        let f = parse(PATCH);
        assert_eq!(f.len(), 4);
        let a = &f[0];
        assert_eq!((a.path.as_str(), a.status, a.additions, a.deletions), ("src/a.rs", FileStatus::Modified, 2, 1));
        let l = &a.hunks[0].lines;
        assert_eq!(l.len(), 5);
        assert_eq!((l[0].old_no, l[0].new_no), (Some(10), Some(10)));
        assert_eq!((l[1].kind, l[1].old_no, l[1].new_no), (LineKind::Removed, Some(11), None));
        assert_eq!((l[2].kind, l[2].new_no), (LineKind::Added, Some(11)));
        assert_eq!((l[4].old_no, l[4].new_no), (Some(12), Some(13)));

        assert_eq!(f[1].status, FileStatus::Deleted);
        assert_eq!(f[1].deletions, 1);
        assert_eq!(
            (f[2].status, f[2].path.as_str(), f[2].old_path.as_deref()),
            (FileStatus::Renamed, "new name.md", Some("old name.md"))
        );
        assert!(f[3].binary && f[3].status == FileStatus::Added);
    }
}
