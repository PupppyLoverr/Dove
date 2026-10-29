//! Artifacts: what threads with no folder make. Those threads run in
//! `<data folder>/chats`, so every doc, page, script, image or other file
//! an agent writes there is listed here to open again, newest first. A
//! card previews the file; opening one shows it whole (rendered Markdown,
//! the image, or the text) with ways out to the system and the folder.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement, ObjectFit, ParentElement as _, SharedString,
    StatefulInteractiveElement as _, Styled as _, StyledImage as _, Window, div, img, px,
};

use crate::assets::{icon, icons};
use crate::design::{radius, space, ty};
use crate::kit::controls::{self as ctl, Segment, Tone};
use crate::kit::page;
use crate::shell::Shell;
use crate::theme::Theme;
use crate::ui;

/// Where no-folder threads work, and so where artifacts live.
pub fn chats_dir() -> PathBuf {
    dove_core::store::default_root().join("chats")
}

/// What an artifact is, from its extension.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ArtKind {
    Doc,
    Page,
    Code,
    Image,
    File,
}

impl ArtKind {
    pub const TABS: [Option<ArtKind>; 6] =
        [None, Some(ArtKind::Doc), Some(ArtKind::Page), Some(ArtKind::Code), Some(ArtKind::Image), Some(ArtKind::File)];

    fn of(path: &Path) -> ArtKind {
        let ext = path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
        match ext.as_str() {
            "md" | "markdown" | "txt" | "rst" | "org" | "docx" | "pdf" => ArtKind::Doc,
            "html" | "htm" | "svg" => ArtKind::Page,
            "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" => ArtKind::Image,
            "rs" | "py" | "js" | "ts" | "tsx" | "jsx" | "go" | "c" | "cpp" | "h" | "cs" | "java" | "kt" | "swift"
            | "rb" | "php" | "sh" | "ps1" | "bat" | "css" | "scss" | "json" | "toml" | "yaml" | "yml" | "sql"
            | "lua" => ArtKind::Code,
            _ => ArtKind::File,
        }
    }

    fn label(kind: Option<ArtKind>) -> &'static str {
        match kind {
            None => "All",
            Some(ArtKind::Doc) => "Docs",
            Some(ArtKind::Page) => "Pages",
            Some(ArtKind::Code) => "Code",
            Some(ArtKind::Image) => "Images",
            Some(ArtKind::File) => "Files",
        }
    }

    fn glyph(self) -> &'static str {
        match self {
            ArtKind::Doc => icons::FILE_MARKDOWN,
            ArtKind::Page => icons::CODE,
            ArtKind::Code => icons::FILE_CODE,
            ArtKind::Image => icons::FILE_IMAGE,
            ArtKind::File => icons::DOCUMENT,
        }
    }

    /// Shown as text in the preview and the viewer.
    fn is_text(self, path: &Path) -> bool {
        let ext = path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
        matches!(self, ArtKind::Code | ArtKind::Page)
            || (self == ArtKind::Doc && !matches!(ext.as_str(), "docx" | "pdf"))
    }
}

#[derive(Clone, Debug)]
pub struct Artifact {
    pub path: PathBuf,
    pub kind: ArtKind,
    pub size: u64,
    pub modified: SystemTime,
}

/// Every file under `root` (skipping dot folders and dependency folders),
/// newest first, at most `cap`.
pub fn scan(root: &Path, cap: usize) -> Vec<Artifact> {
    fn walk(dir: &Path, depth: usize, out: &mut Vec<Artifact>) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for e in entries.flatten() {
            let path = e.path();
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with('.') {
                continue;
            }
            let Ok(meta) = e.metadata() else { continue };
            if meta.is_dir() {
                if depth < 5 && !matches!(name.as_str(), "node_modules" | "target" | "__pycache__" | "venv" | "dist") {
                    walk(&path, depth + 1, out);
                }
            } else if meta.is_file() {
                let kind = ArtKind::of(&path);
                out.push(Artifact {
                    path,
                    kind,
                    size: meta.len(),
                    modified: meta.modified().unwrap_or(SystemTime::UNIX_EPOCH),
                });
            }
        }
    }
    let mut out = Vec::new();
    walk(root, 0, &mut out);
    out.sort_by(|a, b| b.modified.cmp(&a.modified));
    out.truncate(cap);
    out
}

/// The first `lines` lines of a text file, read without loading big files.
fn head(path: &Path, lines: usize, max_bytes: usize) -> String {
    use std::io::Read as _;
    let Ok(f) = std::fs::File::open(path) else { return String::new() };
    let mut buf = Vec::new();
    let _ = f.take(max_bytes as u64).read_to_end(&mut buf);
    String::from_utf8_lossy(&buf).lines().take(lines).collect::<Vec<_>>().join("\n")
}

fn age(t: SystemTime) -> String {
    let ms = t.duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0);
    ui::relative_time(ms).to_string()
}

fn size_label(bytes: u64) -> String {
    match bytes {
        b if b < 1024 => format!("{b} B"),
        b if b < 1024 * 1024 => format!("{:.0} KB", b as f64 / 1024.0),
        b => format!("{:.1} MB", b as f64 / (1024.0 * 1024.0)),
    }
}

impl Shell {
    pub fn render_artifacts_page(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let t = Theme::of(cx).clone();
        let root = chats_dir();
        if let Some(open) = self.pages.artifact_open.clone() {
            return self.render_artifact_view(&root, &open, &t, window, cx);
        }
        let all = scan(&root, 400);
        let filter = self.pages.artifact_kind;
        let shown: Vec<&Artifact> = all.iter().filter(|a| filter.is_none_or(|k| a.kind == k)).collect();

        let tabs = ctl::segmented(
            &t,
            "art-tabs",
            ArtKind::TABS.iter().map(|k| Segment::new(ArtKind::label(*k))).collect(),
            ArtKind::TABS.iter().position(|k| *k == filter).unwrap_or(0),
            {
                let weak = cx.weak_entity();
                move |i, _, cx| {
                    let _ = weak.update(cx, |this, cx| {
                        this.pages.artifact_kind = ArtKind::TABS[i];
                        cx.notify();
                    });
                }
            },
        );
        let folder = root.clone();
        let reveal = ctl::button_sm(&t, "art-reveal", "Open folder", Tone::Quiet).on_click(move |_, _, cx| {
            let _ = std::fs::create_dir_all(&folder);
            cx.open_with_system(&folder);
        });
        let new_chat = ctl::button_sm(&t, "art-new", "New chat", Tone::Filled)
            .on_click(cx.listener(|this, _, window, cx| this.new_chat_without_folder(window, cx)));
        let trailing = div().flex().items_center().gap(px(8.0)).child(reveal).child(new_chat).into_any_element();

        let body: AnyElement = if shown.is_empty() {
            page::card(&t)
                .mt(px(space::MD))
                .child(page::empty_state(
                    &t,
                    icons::ARTIFACTS,
                    if all.is_empty() { "Things your chats make show up here" } else { "Nothing of this kind yet" },
                    "Start a chat with No folder and ask for a doc, a page or a script. Every file it writes stays here.",
                    None,
                ))
                .into_any_element()
        } else {
            let mut grid = div().mt(px(space::MD)).flex().flex_wrap().gap(px(12.0));
            for (i, a) in shown.iter().enumerate() {
                grid = grid.child(self.artifact_card(i, a, &root, &t, cx));
            }
            div().id("art-scroll").flex_1().min_h_0().overflow_y_scroll().pb(px(24.0)).child(grid).into_any_element()
        };

        let (page_root, column) = page::page_fill("artifacts-page", page::PAGE_WIDE);
        page_root
            .child(crate::motion::fade_in(
                "artifacts-in",
                column
                    .child(page::page_head(
                        &t,
                        "Artifacts",
                        "Docs, pages, code and files made in chats with no folder.",
                        Some(trailing),
                    ))
                    .child(div().mt(px(20.0)).flex_none().flex().child(tabs))
                    .child(body),
            ))
            .into_any_element()
    }

    fn artifact_card(&self, i: usize, a: &Artifact, root: &Path, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let name = a.path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        let folder = a
            .path
            .parent()
            .and_then(|p| p.strip_prefix(root).ok())
            .map(|p| p.to_string_lossy().to_string())
            .filter(|p| !p.is_empty());
        let preview: AnyElement = match a.kind {
            ArtKind::Image => img(a.path.clone()).size_full().object_fit(ObjectFit::Cover).into_any_element(),
            k if k.is_text(&a.path) => div()
                .size_full()
                .p(px(10.0))
                .overflow_hidden()
                .text_size(ty::MICRO)
                .line_height(px(14.0))
                .text_color(t.text_muted)
                .when(k != ArtKind::Doc, |d| d.font_family(t.font_mono.clone()))
                .child(head(&a.path, 9, 4096))
                .into_any_element(),
            k => div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .child(icon(k.glyph()).size(px(28.0)).text_color(t.text_faint))
                .into_any_element(),
        };
        let path = a.path.clone();
        div()
            .id(SharedString::from(format!("art-card-{i}")))
            .w(px(236.0))
            .flex()
            .flex_col()
            .rounded(px(radius::CARD))
            .bg(t.surface_card)
            .border_1()
            .border_color(t.border.opacity(0.55))
            .overflow_hidden()
            .cursor_pointer()
            .hover(|s| s.border_color(t.border_strong))
            .child(div().h(px(132.0)).w_full().bg(t.wash(0.03)).overflow_hidden().child(preview))
            .child(
                div()
                    .p(px(10.0))
                    .flex()
                    .flex_col()
                    .gap(px(2.0))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.0))
                            .child(icon(a.kind.glyph()).size(px(13.0)).flex_none().text_color(t.text_faint))
                            .child(div().min_w_0().truncate().text_size(ty::BODY).text_color(t.text).child(name)),
                    )
                    .child(div().truncate().text_size(ty::CAPTION).text_color(t.text_faint).child(match folder {
                        Some(f) => format!("{f} · {}", age(a.modified)),
                        None => format!("{} · {}", size_label(a.size), age(a.modified)),
                    })),
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                this.pages.artifact_open = Some(path.clone());
                cx.notify();
            }))
            .into_any_element()
    }

    fn render_artifact_view(
        &mut self,
        root: &Path,
        path: &Path,
        t: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let kind = ArtKind::of(path);
        let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        let rel = path.strip_prefix(root).map(|p| p.to_string_lossy().to_string()).unwrap_or_else(|_| name.clone());
        let back = div()
            .id("art-back")
            .flex()
            .items_center()
            .gap(px(6.0))
            .cursor_pointer()
            .text_size(ty::BODY)
            .text_color(t.text_muted)
            .hover(|s| s.text_color(t.text))
            .child(icon(icons::ALT_ARROW_LEFT).size(px(14.0)))
            .child("Artifacts")
            .on_click(cx.listener(|this, _, _, cx| {
                this.pages.artifact_open = None;
                cx.notify();
            }));
        let open_path = path.to_path_buf();
        let reveal_dir = path.parent().map(Path::to_path_buf).unwrap_or_else(|| root.to_path_buf());
        let copy = path.to_string_lossy().to_string();
        let actions = div()
            .flex()
            .items_center()
            .gap(px(8.0))
            .child(ctl::button_sm(t, "art-copy", "Copy path", Tone::Quiet).on_click(move |_, _, cx| {
                cx.write_to_clipboard(gpui::ClipboardItem::new_string(copy.clone()));
            }))
            .child(
                ctl::button_sm(t, "art-folder", "Show in folder", Tone::Quiet)
                    .on_click(move |_, _, cx| cx.open_with_system(&reveal_dir)),
            )
            .child(
                ctl::button_sm(t, "art-system", "Open", Tone::Filled)
                    .on_click(move |_, _, cx| cx.open_with_system(&open_path)),
            );
        let body: AnyElement = match kind {
            ArtKind::Image => div()
                .w_full()
                .h(px(560.0))
                .rounded(px(radius::CARD))
                .overflow_hidden()
                .bg(t.wash(0.03))
                .child(img(path.to_path_buf()).size_full().object_fit(ObjectFit::Contain))
                .into_any_element(),
            ArtKind::Doc if kind.is_text(path) && !rel.ends_with(".txt") => {
                let src = head(path, 4000, 512 * 1024);
                let blocks = crate::markdown::parse(&src);
                page::card(t)
                    .p(px(24.0))
                    .child(crate::markdown::render(SharedString::from(format!("art-md-{rel}")), &blocks, t, window))
                    .into_any_element()
            }
            k if k.is_text(path) => page::card(t)
                .p(px(16.0))
                .font_family(t.font_mono.clone())
                .text_size(ty::META)
                .line_height(px(18.0))
                .text_color(t.text)
                .child(head(path, 4000, 512 * 1024))
                .into_any_element(),
            k => page::card(t)
                .child(page::empty_state(
                    t,
                    k.glyph(),
                    "No preview for this kind of file",
                    "Open it with its own app, or show it in its folder.",
                    None,
                ))
                .into_any_element(),
        };
        let (page_root, column) = page::page_fill("artifact-view", page::PAGE_WIDE);
        page_root
            .child(
                column
                    .child(div().flex_none().child(back))
                    .child(div().mt(px(12.0)).flex_none().child(page::page_head(
                        t,
                        name,
                        format!("{rel} · {}", kind_label(kind)),
                        Some(actions.into_any_element()),
                    )))
                    .child(
                        div()
                            .id("art-view-scroll")
                            .mt(px(space::MD))
                            .flex_1()
                            .min_h_0()
                            .overflow_y_scroll()
                            .pb(px(24.0))
                            .child(body),
                    ),
            )
            .into_any_element()
    }
}

fn kind_label(k: ArtKind) -> &'static str {
    match k {
        ArtKind::Doc => "Document",
        ArtKind::Page => "Page",
        ArtKind::Code => "Code",
        ArtKind::Image => "Image",
        ArtKind::File => "File",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_come_from_extensions() {
        assert_eq!(ArtKind::of(Path::new("notes.md")), ArtKind::Doc);
        assert_eq!(ArtKind::of(Path::new("site/index.html")), ArtKind::Page);
        assert_eq!(ArtKind::of(Path::new("main.rs")), ArtKind::Code);
        assert_eq!(ArtKind::of(Path::new("shot.PNG")), ArtKind::Image);
        assert_eq!(ArtKind::of(Path::new("data.bin")), ArtKind::File);
    }

    #[test]
    fn scan_skips_hidden_and_dependency_folders_newest_first() {
        let dir = std::env::temp_dir().join(format!("dove-art-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".git")).unwrap();
        std::fs::create_dir_all(dir.join("node_modules")).unwrap();
        std::fs::create_dir_all(dir.join("site")).unwrap();
        std::fs::write(dir.join(".git/HEAD"), "x").unwrap();
        std::fs::write(dir.join("node_modules/a.js"), "x").unwrap();
        std::fs::write(dir.join("old.md"), "# old").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(dir.join("site/index.html"), "<p>").unwrap();
        let found = scan(&dir, 10);
        let names: Vec<String> =
            found.iter().map(|a| a.path.file_name().unwrap().to_string_lossy().to_string()).collect();
        assert_eq!(names, vec!["index.html", "old.md"]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
