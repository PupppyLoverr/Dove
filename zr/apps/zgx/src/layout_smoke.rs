//! Stage 6 of the theme check: **mount, and look at what mounted.**
//!
//! The clamps in `src/authored.rs` stop the obvious ways a theme can hurt a
//! row (16 points tall, where the hit target's floor is 24). They cannot stop
//! the ones only a layout pass sees: a row of 24 points carrying two lines at
//! a body size of 18 clips its second line, and a `metric.radius` of 16 turns
//! a 28-point control into a circle inside an 8-point card. This runs the real
//! shell (`ui/shell_fixture.luau`, the same fixture the fit tests use) with
//! the theme under test at three widths and reads the scene: every label's box
//! against its own font size, and every label's box against the row it is in.
//!
//! It costs a few hundred milliseconds and is a rejection: a theme that
//! clips a label is not worn, and the error names the surface.

use std::path::{Path, PathBuf};

use serde_json::Value;
use zgx_script::{ScriptHost, testing::TestApp};

use crate::plugins::json_value;

/// The widths a theme is checked at: the narrowest window the shell is
/// designed for, the baseline, and a wide one.
pub const WIDTHS: [f32; 3] = [1024.0, 1280.0, 1600.0];
const HEIGHT: f32 = 800.0;

#[derive(Debug, Clone)]
pub struct Failure {
    pub width: f32,
    pub surface: String,
    pub what: String,
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "at {} wide, {}: {}", self.width, self.surface, self.what)
    }
}

/// The six surfaces, as the fixture in `ui/layout_smoke_fixture.luau` builds
/// them: the design system and the theme under test, nothing else (this runs
/// before a theme is installed, so it cannot need a host, an engine or a
/// session).
fn fixture() -> PathBuf {
    std::env::var_os("ZERON_ZGX_TEST_UI")
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("ui"))
        .join("layout_smoke_fixture.luau")
}

/// The shell as the theme makes it, at each width: every label's box against
/// its own font size (a line whose box is shorter than its type is clipped),
/// and against the box it sits in (a label that escapes its row is truncated
/// or painted outside it).
pub fn check(authored: &Value) -> Vec<Failure> {
    // (The checked themes are in place before the fixture loads, so `ds.setup`
    // wears them as it builds: a token read before it is registered is a
    // token the nodes built then never get.)
    let app = TestApp::new(|runtime| {
        let host = ScriptHost::new(runtime, fixture());
        host.shared("themes.authored", zgx_script::Value::Nil).set(json_value(authored));
        host
    });
    app.ui.scene.borrow_mut().resize(WIDTHS[1], HEIGHT);
    app.frames(40);

    let mut failures = Vec::new();
    for width in WIDTHS {
        app.ui.scene.borrow_mut().resize(width, HEIGHT);
        app.frames(30);
        walk(&app, width, &mut failures);
    }
    failures
}

/// One width: the window's tree, every text node measured.
fn walk(app: &TestApp, width: f32, failures: &mut Vec<Failure>) {
    let scene = app.ui.scene.borrow();
    // The rows the fixture names: what each label must stay inside. (Not
    // "the nearest ancestor", which is a column that grows with its text and
    // so never overflows: the row is the thing whose height is fixed.)
    let tagged: Vec<(zgx::scene::NodeId, &str)> = SURFACES.iter().filter_map(|(id, name)| app.handle.find(id).map(|node| (node, *name))).collect();
    let mut stack = vec![(scene.root(), scene.root())];
    while let Some((node, parent)) = stack.pop() {
        let bounds = scene.bounds(node);
        // The row this label sits in, by the fixture's own ids; a label that
        // is in none of them is in the page, and the page is the window.
        let mut at = parent;
        let mut outer = None;
        let mut named = None;
        loop {
            if let Some((_, name)) = tagged.iter().find(|(node, _)| *node == at) {
                outer = Some(scene.bounds(at));
                named = Some((*name).to_owned());
                break;
            }
            match scene.parent(at) {
                Some(next) if next != at => at = next,
                _ => break,
            }
        }
        let outer = outer.unwrap_or_else(|| scene.bounds(scene.root()));
        let named = named.unwrap_or_else(|| "the page".to_owned());
        let label = match scene.kind(node) {
            zgx::scene::NodeKind::Text { text, font_size, .. } => Some((text.to_string(), *font_size)),
            zgx::scene::NodeKind::RichText { text } => Some((text.text().to_owned(), 0.0)),
            _ => None,
        };
        if let Some((text, font_size)) = label {
            {
                // A line needs a little more box than its type: a box at or
                // under the type's own size has had its line cut.
                if bounds.height + 0.5 < font_size {
                    failures.push(Failure {
                        width,
                        surface: named.clone(),
                        what: format!("\"{}\" is {:.0} tall at {font_size:.0} type", clip(&text), bounds.height),
                    });
                }
                if bounds.width + 0.5 < 2.0 && !text.trim().is_empty() {
                    failures.push(Failure {
                        width,
                        surface: named.clone(),
                        what: format!("\"{}\" has no width to be read in", clip(&text)),
                    });
                }
                if node != parent && !inside(bounds, outer, 1.0) {
                    failures.push(Failure {
                        width,
                        surface: named.clone(),
                        what: format!(
                            "\"{}\" escapes its row: {:.1}x{:.1} at {:.1},{:.1} in {:.1}x{:.1} at {:.1},{:.1}",
                            clip(&text),
                            bounds.width,
                            bounds.height,
                            bounds.x,
                            bounds.y,
                            outer.width,
                            outer.height,
                            outer.x,
                            outer.y
                        ),
                    });
                }
            }
        }
        for child in scene.children(node) {
            stack.push((*child, node));
        }
    }
}

fn inside(inner: zgx::scene::Rect, outer: zgx::scene::Rect, slack: f32) -> bool {
    inner.x >= outer.x - slack
        && inner.y >= outer.y - slack
        && inner.x + inner.width <= outer.x + outer.width + slack
        && inner.y + inner.height <= outer.y + outer.height + slack
}

fn clip(text: &str) -> String {
    let one = text.replace('\n', " ");
    if one.chars().count() > 40 {
        one.chars().take(40).collect::<String>() + "…"
    } else {
        one
    }
}

/// The six surfaces the fixture names, and what to call each in a failure.
const SURFACES: &[(&str, &str)] = &[
    ("smoke-row-1", "a list row"),
    ("smoke-row-2", "a list row"),
    ("smoke-row-3", "a list row"),
    ("smoke-band", "the band"),
    ("smoke-setting", "a settings row"),
    ("smoke-panel-row", "the panel"),
];

#[cfg(test)]
mod tests {
    use super::*;

    /// The themes this build ships, as `themes.rs` hands them to the script.
    fn shipped() -> Value {
        let ui = Path::new(env!("CARGO_MANIFEST_DIR")).join("ui");
        crate::authored::prepare(&ui)
    }

    #[test]
    fn the_shipped_theme_holds_at_three_widths() {
        let failures = check(&shipped());
        assert!(failures.is_empty(), "{} failures:\n{}", failures.len(), failures.iter().map(|f| f.to_string()).collect::<Vec<_>>().join("\n"));
    }

    /// The whole scale from the one authored size: at 11 the steps are 9, 9,
    /// 10, 11, 13, 14, 18, 24; at 18 they are 15, 16, 17, 18, 20, 21, 25, 39.
    /// Measured off the render, not read off the arithmetic.
    #[test]
    fn the_whole_scale_moves_with_the_one_size() {
        let sizes = |size: f32| {
            let mut shipped = shipped();
            for scheme in ["dark", "light"] {
                shipped[scheme]["tokens"]["font.size"] = serde_json::json!(size);
                shipped[scheme]["tokens"]["font.line"] = serde_json::json!((size * 1.38).ceil());
            }
            let app = TestApp::new(|runtime| {
                let host = ScriptHost::new(runtime, fixture());
                host.shared("themes.authored", zgx_script::Value::Nil).set(json_value(&shipped));
                host
            });
            app.ui.scene.borrow_mut().resize(WIDTHS[1], HEIGHT);
            app.frames(20);
            let scene = app.ui.scene.borrow();
            let mut out = std::collections::BTreeSet::new();
            let mut stack = vec![scene.root()];
            while let Some(node) = stack.pop() {
                if let zgx::scene::NodeKind::Text { font_size, .. } = scene.kind(node) {
                    out.insert((font_size * 10.0).round() as i32);
                }
                stack.extend(scene.children(node).iter().copied());
            }
            out
        };
        let small = sizes(11.0);
        let large = sizes(18.0);
        assert_ne!(small, large, "the scale did not move");
        assert!(small.iter().any(|&s| s <= 90), "no step at or under 9 points at 11: {small:?}");
        assert!(large.iter().any(|&s| s >= 200), "no step at or above 20 points at 18: {large:?}");
        eprintln!("small {small:?} large {large:?}");
    }

    /// A theme the clamps let through and a layout pass must not: a body size
    /// at its most (18) on controls at their least (20). Every check in
    /// `src/authored.rs` passes it — a 20-point control is a legal control and
    /// an 18-point body is a legal body size — and the label is cut, because a
    /// 21.6-point line cannot fit a 20-point control.
    #[test]
    fn a_theme_whose_controls_clip_the_label_they_carry_is_caught() {
        let mut shipped = shipped();
        for scheme in ["dark", "light"] {
            let report = &mut shipped[scheme];
            report["tokens"]["font.size"] = serde_json::json!(18.0);
            report["tokens"]["font.line"] = serde_json::json!(21.6);
            report["tokens"]["metric.control"] = serde_json::json!(20.0);
        }
        let failures = check(&shipped);
        assert!(!failures.is_empty(), "the layout pass found nothing to reject");
        assert!(
            failures.iter().any(|f| f.what.contains("tall") || f.what.contains("escapes")),
            "the failures do not name a clipped label: {:?}",
            failures.iter().map(|f| f.to_string()).collect::<Vec<_>>()
        );
    }
}
