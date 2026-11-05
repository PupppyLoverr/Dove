//! Focus groups (ARIA's composite widgets: toolbars, list boxes, tabs): the
//! group is one Tab stop, landing on the member last focused in it or the
//! selected one, and its arrow keys, Home and End move focus inside it. A
//! radio group's arrows choose as they move, wrapping at the ends.
use zgx::{
    compose::{ViewHandle, prelude::*},
    input::{InputEvent, Key, Modifiers},
    scene::NodeId,
    semantics::Role,
    widgets::Ui,
};

fn press(ui: &mut Ui, key: Key, shift: bool) {
    ui.dispatch(InputEvent::KeyDown {
        key,
        modifiers: Modifiers {
            shift,
            ..Default::default()
        },
        repeat: false,
    });
}
fn tab(ui: &mut Ui) {
    press(ui, Key::Tab, false);
}
fn focused(ui: &Ui, view: &ViewHandle) -> Option<String> {
    let id: NodeId = ui.input.focused()?;
    [
        "before", "a", "b", "c", "after", "field", "one", "two", "three",
    ]
    .into_iter()
    .find(|name| view.find(name) == Some(id))
    .map(str::to_owned)
}
fn button_named(id: &str) -> View {
    button().id(id).child(text(id))
}

#[test]
fn a_toolbar_is_one_tab_stop_its_arrows_move_inside() {
    let mut ui = Ui::new(500., 300.);
    let view = ui.mount(
        column()
            .child(button_named("before"))
            .child(
                row()
                    .role(Role::Toolbar)
                    .child(button_named("a"))
                    .child(button_named("b"))
                    .child(button_named("c")),
            )
            .child(button_named("after")),
    );
    ui.prepare_frame();
    let mut order = Vec::new();
    for _ in 0..3 {
        tab(&mut ui);
        order.push(focused(&ui, &view));
    }
    let names = |list: &[Option<String>]| list.iter().flatten().cloned().collect::<Vec<_>>();
    assert_eq!(names(&order), ["before", "a", "after"]);
    // Back into it, then along it; its ends stop the arrows.
    press(&mut ui, Key::Tab, true);
    assert_eq!(focused(&ui, &view).as_deref(), Some("a"));
    press(&mut ui, Key::ArrowLeft, false);
    assert_eq!(focused(&ui, &view).as_deref(), Some("a"));
    press(&mut ui, Key::ArrowRight, false);
    assert_eq!(focused(&ui, &view).as_deref(), Some("b"));
    press(&mut ui, Key::End, false);
    assert_eq!(focused(&ui, &view).as_deref(), Some("c"));
    // Down is not a toolbar's.
    press(&mut ui, Key::ArrowDown, false);
    assert_eq!(focused(&ui, &view).as_deref(), Some("c"));
    // Tab leaves it; coming back lands where it was left.
    tab(&mut ui);
    assert_eq!(focused(&ui, &view).as_deref(), Some("after"));
    press(&mut ui, Key::Tab, true);
    assert_eq!(focused(&ui, &view).as_deref(), Some("c"));
}

#[test]
fn a_list_box_is_entered_at_its_selected_option() {
    let mut ui = Ui::new(500., 300.);
    let chosen = ui.signal(1_usize);
    let option = |name: &'static str, i: usize| {
        let read = chosen.clone();
        let write = chosen.clone();
        row()
            .id(name)
            .role(Role::ListBoxOption)
            .selected_when(move || read.get() == i)
            .on_click(move || {
                write.set(i);
            })
            .child(text(name))
    };
    let view = ui.mount(
        column()
            .child(button_named("before"))
            .child(
                column()
                    .role(Role::ListBox)
                    .child(option("one", 0))
                    .child(option("two", 1))
                    .child(option("three", 2)),
            )
            .child(button_named("after")),
    );
    ui.prepare_frame();
    tab(&mut ui);
    tab(&mut ui);
    assert_eq!(focused(&ui, &view).as_deref(), Some("two"));
    press(&mut ui, Key::ArrowDown, false);
    assert_eq!(focused(&ui, &view).as_deref(), Some("three"));
    press(&mut ui, Key::Home, false);
    assert_eq!(focused(&ui, &view).as_deref(), Some("one"));
    tab(&mut ui);
    assert_eq!(focused(&ui, &view).as_deref(), Some("after"));
    // A selection made elsewhere is where Tab lands next.
    chosen.set(2);
    ui.prepare_frame();
    ui.input.focus(&ui.scene, None);
    tab(&mut ui);
    tab(&mut ui);
    assert_eq!(focused(&ui, &view).as_deref(), Some("three"));
}

#[test]
fn an_editor_in_a_toolbar_keeps_its_arrows() {
    let mut ui = Ui::new(500., 300.);
    let value = ui.signal("text".to_owned());
    let view = ui.mount(
        row()
            .role(Role::Toolbar)
            .child(button_named("a"))
            .child(text_input("Field", value).id("field").w(120.))
            .child(button_named("b")),
    );
    ui.prepare_frame();
    let field = view.find("field").unwrap();
    ui.input.focus(&ui.scene, Some(field));
    press(&mut ui, Key::ArrowRight, false);
    press(&mut ui, Key::ArrowLeft, false);
    assert_eq!(ui.input.focused(), Some(field));
}

#[test]
fn a_virtual_lists_rows_are_one_tab_stop() {
    let mut ui = Ui::new(500., 300.);
    let offset = ui.signal(0.);
    let view = ui.mount(
        column()
            .child(button_named("before"))
            .child(
                virtual_list(
                    offset,
                    20.,
                    1,
                    || 100,
                    |n| n,
                    |_, n, _| button().child(text(format!("Row {n}"))),
                )
                .size(200., 100.)
                .keyboard_navigation(true),
            )
            .child(button_named("after")),
    );
    ui.prepare_frame();
    tab(&mut ui);
    tab(&mut ui);
    let row = ui.input.focused();
    assert!(row.is_some() && focused(&ui, &view).is_none(), "a row");
    press(&mut ui, Key::ArrowDown, false);
    ui.prepare_frame();
    let next = ui.input.focused();
    assert_ne!(next, row, "Down moves to the next row");
    tab(&mut ui);
    assert_eq!(focused(&ui, &view).as_deref(), Some("after"));
}

#[test]
fn right_to_left_the_arrows_read_the_other_way() {
    let mut ui = Ui::new(500., 300.);
    let view = ui.mount(
        row()
            .role(Role::Toolbar)
            .rtl()
            .child(button_named("a"))
            .child(button_named("b"))
            .child(button_named("c")),
    );
    ui.prepare_frame();
    tab(&mut ui);
    assert_eq!(focused(&ui, &view).as_deref(), Some("a"));
    // "a" is at the right: Left moves on to "b".
    press(&mut ui, Key::ArrowLeft, false);
    assert_eq!(focused(&ui, &view).as_deref(), Some("b"));
    press(&mut ui, Key::ArrowRight, false);
    assert_eq!(focused(&ui, &view).as_deref(), Some("a"));
}

#[test]
fn a_radio_group_is_one_tab_stop_and_its_arrows_choose_wrapping() {
    let mut ui = Ui::new(500., 300.);
    let size = ui.signal(String::from("m"));
    let view = ui.mount(
        column()
            .child(button_named("before"))
            .child(
                radio_group("Size")
                    .child(radio("Small", size.clone(), "s".to_string()).id("one"))
                    .child(radio("Medium", size.clone(), "m".to_string()).id("two"))
                    .child(radio("Large", size.clone(), "l".to_string()).id("three")),
            )
            .child(button_named("after")),
    );
    ui.prepare_frame();
    let semantics = |ui: &Ui, name: &str| {
        let id = view.find(name).unwrap();
        ui.semantics.borrow().get(id).map(|n| (n.role, n.checked))
    };
    assert_eq!(semantics(&ui, "two"), Some((Role::RadioButton, Some(true))));
    tab(&mut ui);
    tab(&mut ui);
    assert_eq!(
        focused(&ui, &view).as_deref(),
        Some("two"),
        "Tab lands on the checked radio"
    );
    press(&mut ui, Key::ArrowDown, false);
    ui.prepare_frame();
    assert_eq!(focused(&ui, &view).as_deref(), Some("three"));
    assert_eq!(size.get(), "l", "the arrow chose it");
    press(&mut ui, Key::ArrowRight, false);
    ui.prepare_frame();
    assert_eq!(
        focused(&ui, &view).as_deref(),
        Some("one"),
        "past the end, it wraps"
    );
    assert_eq!(size.get(), "s");
    press(&mut ui, Key::ArrowUp, false);
    ui.prepare_frame();
    assert_eq!(size.get(), "l", "and back");
    assert_eq!(
        semantics(&ui, "three"),
        Some((Role::RadioButton, Some(true)))
    );
    assert_eq!(
        semantics(&ui, "one"),
        Some((Role::RadioButton, Some(false)))
    );
    tab(&mut ui);
    assert_eq!(
        focused(&ui, &view).as_deref(),
        Some("after"),
        "one Tab stop"
    );
    press(&mut ui, Key::Tab, true);
    assert_eq!(
        focused(&ui, &view).as_deref(),
        Some("three"),
        "back at the checked one"
    );
    // Space chooses the focused radio; a click chooses too.
    let one = view.find("one").unwrap();
    let r = ui.scene.borrow().bounds(one);
    let (x, y) = (r.x + 8., r.y + r.height / 2.);
    ui.dispatch(InputEvent::PointerDown {
        x,
        y,
        button: zgx::input::PointerButton::Primary,
    });
    ui.dispatch(InputEvent::PointerUp {
        x,
        y,
        button: zgx::input::PointerButton::Primary,
    });
    assert_eq!(size.get(), "s");
}

#[test]
fn typing_in_a_list_box_jumps_by_name_and_page_keys_move_a_page() {
    let mut ui = Ui::new(500., 300.);
    let names = [
        "Apple",
        "Banana",
        "Blueberry",
        "Cherry",
        "Date",
        "Fig",
        "Grape",
        "Kiwi",
    ];
    let mut list = column().role(Role::ListBox);
    for name in names {
        list = list.child(
            row()
                .id(name)
                .h(20.)
                .shrink_0()
                .role(Role::ListBoxOption)
                .on_click(|| {})
                .child(text(name)),
        );
    }
    let offset = ui.signal(0_f32);
    let view = ui.mount(column().child(scroll(offset).h(60.).child(list)));
    ui.prepare_frame();
    let focused = |ui: &Ui| {
        let id = ui.input.focused()?;
        names.into_iter().find(|name| view.find(name) == Some(id))
    };
    ui.input.focus(&ui.scene, view.find("Apple"));
    let typed = |ui: &mut Ui, text: &str| {
        ui.dispatch(InputEvent::KeyDown {
            key: Key::Character(text.into()),
            modifiers: Modifiers::default(),
            repeat: false,
        })
    };
    typed(&mut ui, "b");
    assert_eq!(focused(&ui), Some("Banana"));
    typed(&mut ui, "l");
    assert_eq!(focused(&ui), Some("Blueberry"), "bl");
    // A page: as much as its 60-point scroll view shows (three options),
    // as a browser's list box goes to the last one showing, then on.
    ui.input.focus(&ui.scene, view.find("Apple"));
    press(&mut ui, Key::PageDown, false);
    ui.prepare_frame();
    assert_eq!(focused(&ui), Some("Blueberry"));
    press(&mut ui, Key::PageDown, false);
    ui.prepare_frame();
    assert_eq!(focused(&ui), Some("Date"));
    press(&mut ui, Key::PageUp, false);
    ui.prepare_frame();
    assert_eq!(focused(&ui), Some("Blueberry"));
}
