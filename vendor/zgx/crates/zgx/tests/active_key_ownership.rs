use std::{cell::Cell, rc::Rc};
use zgx::{
    compose::prelude::*,
    input::{InputEvent, Key, Modifiers, PointerButton},
    scene::{Color, NodeId, NodeKind},
    widgets::Ui,
};
fn key(ui: &mut Ui, key: Key, down: bool, repeat: bool) {
    ui.dispatch(if down {
        InputEvent::KeyDown {
            key,
            modifiers: Modifiers::default(),
            repeat,
        }
    } else {
        InputEvent::KeyUp {
            key,
            modifiers: Modifiers::default(),
        }
    });
    ui.prepare_frame();
}
fn color(ui: &Ui, node: NodeId) -> Color {
    match ui.scene.borrow().kind(node) {
        NodeKind::Panel { quad, .. } => quad.fill,
        other => panic!("expected panel, got {other:?}"),
    }
}
#[test]
fn activation_style_tracks_latest_nonrepeat_key_and_preserves_pointer_press() {
    let mut ui = Ui::new(300., 200.);
    let clicks = Rc::new(Cell::new(0));
    let count = clicks.clone();
    let base = rgb(0x102030);
    let active = rgb(0x405060);
    let view = ui.mount(
        button()
            .size(100., 40.)
            .bg(base)
            .active(move |s| s.bg(active))
            .on_click(move || count.set(count.get() + 1)),
    );
    ui.prepare_frame();
    ui.input.focus(&ui.scene, Some(view.node()));
    key(&mut ui, Key::Space, true, false);
    assert_eq!(color(&ui, view.node()), active);
    key(&mut ui, Key::Enter, true, false);
    key(&mut ui, Key::Space, false, false);
    assert_eq!(
        color(&ui, view.node()),
        active,
        "unrelated Space release preserves newer Enter press"
    );
    // Enter activated as it went down, as a browser's button takes it.
    assert_eq!(clicks.get(), 1);
    key(&mut ui, Key::Character("a".into()), false, false);
    assert_eq!(color(&ui, view.node()), active);
    key(&mut ui, Key::Space, true, true);
    key(&mut ui, Key::Enter, false, false);
    assert_eq!(color(&ui, view.node()), base);
    assert_eq!(clicks.get(), 1);
    key(&mut ui, Key::Space, false, false);
    assert_eq!(clicks.get(), 1);
    ui.dispatch(InputEvent::PointerDown {
        x: 20.,
        y: 20.,
        button: PointerButton::Primary,
    });
    key(&mut ui, Key::Space, true, false);
    key(&mut ui, Key::Space, false, false);
    assert_eq!(
        color(&ui, view.node()),
        active,
        "keyboard release preserves primary press"
    );
    ui.dispatch(InputEvent::PointerUp {
        x: 20.,
        y: 20.,
        button: PointerButton::Primary,
    });
    ui.prepare_frame();
    assert_eq!(color(&ui, view.node()), base);
    key(&mut ui, Key::Enter, true, false);
    ui.dispatch(InputEvent::PointerMove { x: 150., y: 60. });
    ui.prepare_frame();
    assert_eq!(
        color(&ui, view.node()),
        active,
        "pointer leave cannot cancel keyboard press"
    );
    ui.input.focus(&ui.scene, None);
    ui.prepare_frame();
    assert_eq!(color(&ui, view.node()), base);
}

#[test]
fn a_button_takes_enter_as_it_goes_down_and_a_checkbox_space_alone() {
    let mut ui = Ui::new(300., 200.);
    let clicks = Rc::new(Cell::new(0));
    let count = clicks.clone();
    let checked = ui.signal(false);
    let view = ui.mount(
        column()
            .child(
                button()
                    .id("button")
                    .size(100., 40.)
                    .on_click(move || count.set(count.get() + 1)),
            )
            .child(checkbox("Check", checked.clone()).id("check")),
    );
    ui.prepare_frame();
    ui.input.focus(&ui.scene, view.find("button"));
    key(&mut ui, Key::Enter, true, false);
    assert_eq!(clicks.get(), 1, "Enter activates as it goes down");
    key(&mut ui, Key::Enter, true, true);
    key(&mut ui, Key::Enter, false, false);
    assert_eq!(clicks.get(), 1, "once: not on repeat, nor on release");
    key(&mut ui, Key::Space, true, false);
    assert_eq!(clicks.get(), 1, "Space waits for its release");
    key(&mut ui, Key::Space, false, false);
    assert_eq!(clicks.get(), 2);
    ui.input.focus(&ui.scene, view.find("check"));
    key(&mut ui, Key::Enter, true, false);
    key(&mut ui, Key::Enter, false, false);
    assert!(
        !checked.get(),
        "Enter goes on to the form, toggling nothing"
    );
    key(&mut ui, Key::Space, true, false);
    key(&mut ui, Key::Space, false, false);
    assert!(checked.get(), "Space toggles it");
}
