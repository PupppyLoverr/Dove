//! Tab in the command palette: it opens the highlighted row's action panel,
//! as `→` and the pane's own affordance do (macOS 27's Tab). Mounted headless
//! on the shell's fixture, with the window's size fed as a window feeds it
//! (without it the palette draws its chrome alone, having no room for rows).

use zgx_script::{ScriptHost, Value, testing::TestApp};

fn mount() -> TestApp {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("ui/shell_fixture.luau");
    let app = TestApp::new(|runtime| ScriptHost::new(runtime, path));
    app.ui.scene.borrow_mut().resize(1240., 820.);
    let viewport = app.ui.runtime.signal((1240f32, 820f32));
    app.host.window_size(viewport);
    app.frames(30);
    assert_eq!(app.error(), None, "the shell failed");
    app
}

/// The palette open, its field focused, nothing typed.
fn open(app: &mut TestApp) {
    assert!(app.host.run_command("shell.palette"), "the shell's command");
    app.wait_until(|app| app.find("#palette-search").is_some());
    app.frames(30);
    assert!(app.find("#palette-search").is_some(), "the palette never opened");
}

/// Whether the action panel is up (the palette says so as `palette.panel`).
fn panel_up(app: &TestApp) -> bool {
    app.host.states().into_iter().any(|(name, value)| name == "palette.panel" && matches!(value, Value::Bool(true)))
}

#[test]
fn tab_opens_the_action_panel() {
    let mut app = mount();
    open(&mut app);
    assert!(!panel_up(&app), "the panel is up before Tab");

    app.key(zgx::input::Key::Tab);
    app.frames(30);
    assert!(panel_up(&app), "Tab opened nothing:\n{}", app.tree());

    // The panel holds the keys: Escape closes it, the palette staying open
    // on the row it was opened for.
    app.key(zgx::input::Key::Escape);
    app.frames(30);
    assert!(!panel_up(&app), "Escape left the panel up");
    assert!(app.find("#palette-search").is_some(), "Escape closed the palette, not the panel");

    // And again, from the keys the field still has.
    app.key(zgx::input::Key::Tab);
    app.frames(30);
    assert!(panel_up(&app), "Tab opened nothing the second time");
}

#[test]
fn shift_tab_moves_the_scope() {
    let mut app = mount();
    open(&mut app);
    let before = app.text("#palette-list");
    app.key_with(zgx::input::Key::Tab, zgx::input::Modifiers { shift: true, ..Default::default() });
    app.frames(30);
    assert!(!panel_up(&app), "Shift-Tab opened the panel");
    assert_ne!(app.text("#palette-list"), before, "Shift-Tab left the list as it was");
}
