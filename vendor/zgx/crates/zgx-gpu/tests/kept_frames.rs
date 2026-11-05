//! A window whose leaves alone change (bars resizing, values changing) is
//! drawn by patching those items into its last whole frame (`render_kept`):
//! each such frame is pixel for pixel what drawing the scene afresh draws.
use zgx::{compose::prelude::*, reactive::Signal, scene::Color, widgets::Ui};
use zgx_gpu::{GpuRenderer, text::FontData};

const GEIST: &[u8] = include_bytes!("fonts/GeistVariable-subset.ttf");
const SIZE: (f32, f32) = (480., 260.);

fn view(live: Signal<u32>, mounted: Signal<u32>) -> View {
    let rows = live.clone();
    let card = move |index: u32| {
        let (value, bars, fill) = (live.clone(), live.clone(), live.clone());
        div()
            .absolute()
            .ml(8. + 116. * index as f32)
            .mt(8.)
            .size(108., 120.)
            .rounded(6.)
            .border(1.)
            .border_color(Color(40, 50, 64, 255))
            .bg(Color(22, 28, 38, 255))
            .child(
                text_signal(move || format!("{:.1} ms", 3.7 * (value.get() + index) as f32))
                    .text_size(15.)
                    .absolute()
                    .ml(8.)
                    .mt(6.)
                    .size(92., 20.),
            )
            .child(
                div()
                    .absolute()
                    .ml(8.)
                    .mt(32.)
                    .h(4.)
                    .rounded(2.)
                    .bg(Color(94, 177, 255, 255))
                    .reactive_style(move || {
                        Styles::new().w(8. + ((fill.get() * 7 + index) % 13) as f32 * 6.3)
                    }),
            )
            .children((0..12).map(move |bar| {
                let bars = bars.clone();
                div()
                    .absolute()
                    .ml(8. + 7.5 * bar as f32)
                    .w(5.)
                    .bg(Color(94, 177, 255, 255))
                    .reactive_style(move || {
                        let height = 2. + ((bars.get() + bar + index) % 9) as f32 * 7.25;
                        Styles::new().mt(110. - height).h(height)
                    })
            }))
    };
    let row = move |row: u32| {
        let (status, label, latency) = (rows.clone(), rows.clone(), rows.clone());
        div()
            .absolute()
            .ml(8.)
            .mt(140. + 22. * row as f32)
            .size(464., 22.)
            .bg(if row.is_multiple_of(2) {
                Color(22, 28, 38, 255)
            } else {
                Color(25, 32, 43, 255)
            })
            .child(
                text(format!("worker-{row:03}"))
                    .text_size(12.)
                    .absolute()
                    .ml(6.)
                    .mt(3.)
                    .size(120., 16.),
            )
            .child(
                div()
                    .absolute()
                    .ml(140.)
                    .mt(3.)
                    .size(60., 16.)
                    .rounded(8.)
                    .reactive_style(move || {
                        let down = (status.get() / 4 + row).is_multiple_of(3);
                        Styles::new().bg(if down {
                            Color(138, 34, 48, 255)
                        } else {
                            Color(29, 107, 74, 255)
                        })
                    })
                    .child(
                        text_signal(move || {
                            ["down", "healthy", "healthy"][((label.get() / 4 + row) % 3) as usize]
                                .into()
                        })
                        .text_size(10.)
                        .absolute()
                        .ml(8.)
                        .mt(1.)
                        .size(48., 14.),
                    ),
            )
            .child(
                text_signal(move || format!("{} ms", (latency.get() * 13 + row * 7) % 997))
                    .text_size(12.)
                    .absolute()
                    .ml(220.)
                    .mt(3.)
                    .size(80., 16.),
            )
    };
    div()
        .size(SIZE.0, SIZE.1)
        .bg(Color(14, 18, 24, 255))
        .text_color(Color(229, 237, 247, 255))
        .font_family("Geist")
        .children((0..4).map(card))
        .children((0..5).map(row))
        // Mounted again whenever `mounted` changes: a change no patch makes.
        .child(switch(
            move || mounted.get(),
            |which, _| {
                div()
                    .absolute()
                    .ml(8. + 40. * which as f32)
                    .mt(252.)
                    .size(100., 6.)
                    .bg(Color(255, 196, 61, 255))
            },
        ))
}

struct Window {
    ui: Ui,
    gpu: GpuRenderer,
    live: Signal<u32>,
    mounted: Signal<u32>,
}
impl Window {
    fn new(live: u32) -> Self {
        Self::mounted(live, 0)
    }
    fn mounted(live: u32, mounted: u32) -> Self {
        let gpu = GpuRenderer::new((SIZE.0 * 2.) as u32, (SIZE.1 * 2.) as u32).unwrap();
        let mut gpu = gpu;
        gpu.set_scale_factor(2.);
        FontData::new(GEIST.to_vec())
            .unwrap()
            .install(&mut gpu.text_system().borrow_mut());
        let mut ui = Ui::new(SIZE.0, SIZE.1);
        gpu.install_text(&mut ui.scene.borrow_mut());
        let state = ui.signal(live);
        let which = ui.signal(mounted);
        ui.mount(view(state.clone(), which.clone()));
        Window {
            ui,
            gpu,
            live: state,
            mounted: which,
        }
    }
    fn draw(&mut self) -> zgx_gpu::GpuStats {
        self.ui.prepare_frame();
        let report = self.ui.scene.borrow_mut().flush();
        self.gpu
            .render(&self.ui.scene.borrow(), &report.damage)
            .unwrap()
    }
}

#[test]
fn a_frame_patched_into_the_kept_window_is_the_frame_drawn_afresh() {
    let mut window = Window::new(1);
    window.draw();
    let mut patched = 0;
    for live in 2..14 {
        window.live.set(live);
        let stats = window.draw();
        patched += usize::from(stats.patched_items > 0);
        let mut fresh = Window::new(live);
        fresh.draw();
        assert!(
            window.gpu.readback().unwrap() == fresh.gpu.readback().unwrap(),
            "frame {live} differs from the scene drawn afresh (patched {})",
            stats.patched_items
        );
    }
    // Every frame but those where a pill (a box holding a text) changes
    // colour patches; drawn whole then, the next patches again.
    assert!(patched >= 8, "{patched} frames patched");
}

/// A window that lost its kept frame to a change no patch makes (a node
/// mounted) before patching from it keeps one again once a few frames in a
/// row repaint leaves alone: patching goes on, each frame as drawn afresh.
#[test]
fn a_window_keeps_a_frame_again_after_a_change_no_patch_makes() {
    let mut window = Window::new(1);
    window.draw();
    window.mounted.set(1);
    window.live.set(2);
    window.draw();
    let mut patched = 0;
    for live in 3..14 {
        window.live.set(live);
        let stats = window.draw();
        patched += usize::from(stats.patched_items > 0);
        let mut fresh = Window::mounted(live, 1);
        fresh.draw();
        assert!(
            window.gpu.readback().unwrap() == fresh.gpu.readback().unwrap(),
            "frame {live} differs from the scene drawn afresh (patched {})",
            stats.patched_items
        );
    }
    assert!(patched >= 5, "{patched} frames patched");
}

/// Boxes a frame patches alone (`patch_box`): a flat colour, translucent,
/// edge-faded, bordered and rounded, one sliding across a clip's edge
/// (inside it, cut by it, then outside) and one swinging between inside
/// it and cut by it, each frame as drawn afresh.
/// `wide`: over a bar across the window under the clip, changing colour
/// each frame, so that frames draw from the kept frame (damage over
/// `KEPT_SHARE` of the window) where the clip cuts the sliding box.
fn boxes(live: Signal<u32>, wide: bool) -> View {
    let (bar, swing) = (live.clone(), live.clone());
    let shade = live.clone();
    let tint = live.clone();
    let slide = live.clone();
    let border = live.clone();
    div()
        .size(SIZE.0, SIZE.1)
        .bg(Color(14, 18, 24, 255))
        .child(
            div()
                .absolute()
                .mt(95.)
                .size(SIZE.0, if wide { 70. } else { 0. })
                .reactive_style(move || {
                    Styles::new().bg(Color(40, (bar.get() * 29 % 200) as u8, 70, 255))
                }),
        )
        .child(
            div()
                .absolute()
                .ml(10.)
                .mt(10.)
                .size(60., 40.)
                .reactive_style(move || {
                    Styles::new().bg(Color((shade.get() * 37 % 256) as u8, 90, 140, 255))
                }),
        )
        .child(
            div()
                .absolute()
                .ml(80.)
                .mt(10.)
                .size(60., 40.)
                .rounded(9.)
                .opacity(0.6)
                .reactive_style(move || {
                    Styles::new().bg(Color(200, (tint.get() * 53 % 256) as u8, 60, 255))
                }),
        )
        .child(
            div()
                .absolute()
                .ml(150.)
                .mt(10.)
                .size(70., 44.)
                .edge_fade(6.)
                .bg(Color(90, 200, 120, 255))
                .reactive_style(move || Styles::new().w(40. + (border.get() % 5) as f32 * 7.5)),
        )
        .child(
            div()
                .absolute()
                .ml(230.)
                .mt(10.)
                .size(90., 60.)
                .rounded(4.)
                .border(2.)
                .border_color(Color(250, 250, 250, 255))
                .bg(Color(40, 60, 90, 255))
                .reactive_style({
                    let live = live.clone();
                    move || Styles::new().h(30. + (live.get() % 4) as f32 * 9.5)
                }),
        )
        .child(
            div()
                .absolute()
                .ml(20.)
                .mt(100.)
                .size(200., 60.)
                .overflow_hidden()
                .bg(Color(30, 30, 30, 255))
                .child(
                    div()
                        .absolute()
                        .mt(10.)
                        .size(50., 30.)
                        .bg(Color(255, 196, 61, 255))
                        .reactive_style(move || {
                            Styles::new().ml(-70. + (slide.get() % 12) as f32 * 30.5)
                        }),
                )
                // And one going from inside it to cut by it and back
                // (small enough to draw as one quad, `push_panel`).
                .child(
                    div()
                        .absolute()
                        .mt(20.)
                        .size(40., 20.)
                        .bg(Color(200, 120, 255, 255))
                        .reactive_style(move || {
                            Styles::new().ml(100. + (swing.get() % 4) as f32 * 25.5)
                        }),
                ),
        )
}

#[test]
fn a_box_patched_alone_is_the_box_drawn_afresh() {
    boxes_patched_alone(false);
}

#[test]
fn a_box_patched_alone_and_drawn_from_the_kept_frame_is_the_box_drawn_afresh() {
    boxes_patched_alone(true);
}

fn boxes_patched_alone(wide: bool) {
    let window = |live: u32| {
        let mut gpu = GpuRenderer::new((SIZE.0 * 2.) as u32, (SIZE.1 * 2.) as u32).unwrap();
        gpu.set_scale_factor(2.);
        let mut ui = Ui::new(SIZE.0, SIZE.1);
        gpu.install_text(&mut ui.scene.borrow_mut());
        let state = ui.signal(live);
        ui.mount(boxes(state.clone(), wide));
        (ui, gpu, state)
    };
    let draw = |ui: &mut Ui, gpu: &mut GpuRenderer| {
        ui.prepare_frame();
        let report = ui.scene.borrow_mut().flush();
        gpu.render(&ui.scene.borrow(), &report.damage).unwrap()
    };
    let (mut ui, mut gpu, live) = window(1);
    draw(&mut ui, &mut gpu);
    let mut patched = 0;
    for value in 2..26 {
        live.set(value);
        let stats = draw(&mut ui, &mut gpu);
        patched += usize::from(stats.patched_items > 0);
        let (mut fresh, mut fresh_gpu, _) = window(value);
        draw(&mut fresh, &mut fresh_gpu);
        assert!(
            gpu.readback().unwrap() == fresh_gpu.readback().unwrap(),
            "frame {value} differs from the boxes drawn afresh (patched {})",
            stats.patched_items
        );
    }
    assert!(patched >= 16, "{patched} frames patched");
}

/// Frames drawn whole whose kept frames are lost before they patch (a
/// fling repaints the window each frame) stop keeping after two, and keep
/// again once frames would patch; each frame as drawn afresh.
fn wholes(whole: Signal<u32>, leaf: Signal<u32>) -> View {
    div()
        .size(SIZE.0, SIZE.1)
        // A change of the root's colour repaints the window, its children
        // with it: a kept frame can't patch it.
        .reactive_style(move || {
            Styles::new().bg(Color(10 + (whole.get() * 9 % 40) as u8, 18, 24, 255))
        })
        .child(
            div()
                .absolute()
                .ml(20.)
                .mt(20.)
                // Large enough that its frames draw from the kept frame
                // (over `KEPT_SHARE` of the window).
                .w(240.)
                .bg(Color(94, 177, 255, 255))
                .reactive_style(move || Styles::new().h(70. + (leaf.get() % 7) as f32 * 9.)),
        )
}

#[test]
fn frames_drawn_whole_stop_keeping_once_their_kept_frames_go_unpatched() {
    let window = |whole: u32, leaf: u32| {
        let mut gpu = GpuRenderer::new((SIZE.0 * 2.) as u32, (SIZE.1 * 2.) as u32).unwrap();
        gpu.set_scale_factor(2.);
        let mut ui = Ui::new(SIZE.0, SIZE.1);
        gpu.install_text(&mut ui.scene.borrow_mut());
        let (a, b) = (ui.signal(whole), ui.signal(leaf));
        ui.mount(wholes(a.clone(), b.clone()));
        (ui, gpu, a, b)
    };
    let draw = |ui: &mut Ui, gpu: &mut GpuRenderer| {
        ui.prepare_frame();
        let report = ui.scene.borrow_mut().flush();
        gpu.render(&ui.scene.borrow(), &report.damage).unwrap()
    };
    let (mut ui, mut gpu, whole, leaf) = window(0, 0);
    draw(&mut ui, &mut gpu);
    let (mut w, mut l) = (0, 0);
    // A frame: the window repainted (`true`) or the bar alone; whether it
    // patched, each checked against the scene drawn afresh.
    let mut frame = |repaint: bool| {
        match repaint {
            true => w += 1,
            false => l += 1,
        }
        whole.set(w);
        leaf.set(l);
        let stats = draw(&mut ui, &mut gpu);
        let (mut fresh, mut fresh_gpu, _, _) = window(w, l);
        draw(&mut fresh, &mut fresh_gpu);
        assert!(
            gpu.readback().unwrap() == fresh_gpu.readback().unwrap(),
            "frame {w}/{l} differs from the scene drawn afresh"
        );
        stats.patched_items > 0
    };
    // One frame drawn whole and the bar's next: patched from it, as before.
    for _ in 0..3 {
        assert!(!frame(true));
        assert!(frame(false), "a bar after one frame drawn whole patches");
    }
    // Frames drawn whole in a row lose their kept frames unpatched: after
    // two, the next keeps none, so the bar's frame after them draws whole.
    for _ in 0..4 {
        assert!(!frame(true));
    }
    assert!(!frame(false), "frames drawn whole in a row stopped keeping");
    // The bar alone again: frames that would patch keep again (after the
    // re-keep backoff of `would_patch`, which those frames drawn whole
    // grew), and the bar's frames patch.
    let patched = (0..10).filter(|_| frame(false)).count();
    assert!(patched >= 5, "{patched} of the bar's 10 frames patched");
    // And a frame drawn whole between them keeps: the bar's next patches.
    assert!(!frame(true));
    assert!(
        frame(false),
        "a bar after one frame drawn whole patches again"
    );
}

/// Boxes layout moves in the same frame something else changes about them
/// (hidden, shown, made relative): each frame as drawn afresh, whatever a
/// patch of the move alone would have drawn.
fn moved_and_changed(live: Signal<u32>) -> View {
    let (hide, place) = (live.clone(), live.clone());
    div()
        .size(SIZE.0, SIZE.1)
        .bg(Color(14, 18, 24, 255))
        .child(
            div()
                .absolute()
                .ml(20.)
                .mt(20.)
                .w(240.)
                .bg(Color(94, 177, 255, 255))
                .reactive_style(move || {
                    let n = hide.get();
                    let styles = Styles::new().h(70. + (n % 5) as f32 * 9.);
                    match n % 4 {
                        0 => styles.invisible(),
                        1 => styles.hidden(),
                        _ => styles.visible(),
                    }
                }),
        )
        .child(
            div()
                .absolute()
                .ml(20.)
                .mt(150.)
                .w(240.)
                .bg(Color(255, 196, 61, 255))
                .reactive_style(move || {
                    let n = place.get();
                    let styles = Styles::new().h(40. + (n % 4) as f32 * 11.);
                    if n % 2 == 0 {
                        styles.relative()
                    } else {
                        styles
                    }
                }),
        )
}

#[test]
fn a_box_moved_as_it_hides_or_changes_is_drawn_as_afresh() {
    let window = |live: u32| {
        let mut gpu = GpuRenderer::new((SIZE.0 * 2.) as u32, (SIZE.1 * 2.) as u32).unwrap();
        gpu.set_scale_factor(2.);
        let mut ui = Ui::new(SIZE.0, SIZE.1);
        gpu.install_text(&mut ui.scene.borrow_mut());
        let state = ui.signal(live);
        ui.mount(moved_and_changed(state.clone()));
        (ui, gpu, state)
    };
    let draw = |ui: &mut Ui, gpu: &mut GpuRenderer| {
        ui.prepare_frame();
        let report = ui.scene.borrow_mut().flush();
        gpu.render(&ui.scene.borrow(), &report.damage).unwrap()
    };
    let (mut ui, mut gpu, live) = window(1);
    draw(&mut ui, &mut gpu);
    for value in 2..20 {
        live.set(value);
        draw(&mut ui, &mut gpu);
        let (mut fresh, mut fresh_gpu, _) = window(value);
        draw(&mut fresh, &mut fresh_gpu);
        assert!(
            gpu.readback().unwrap() == fresh_gpu.readback().unwrap(),
            "frame {value} differs from the boxes drawn afresh"
        );
    }
}

/// Subtrees a frame splices into its kept frame (`splice`): a list whose
/// rows are mounted and removed, a box whose opacity changes over what it
/// holds and a panel recoloured under its children, beside a bar repainting
/// most of the window, so that frames draw from the kept frame. Static
/// boxes keep the subtrees a small part of the window. Each frame as drawn
/// afresh.
fn spliced(live: Signal<u32>) -> View {
    let (bar, rows, fade, tint) = (live.clone(), live.clone(), live.clone(), live.clone());
    div()
        .size(SIZE.0, SIZE.1)
        .bg(Color(14, 18, 24, 255))
        .text_color(Color(229, 237, 247, 255))
        .font_family("Geist")
        .child(div().absolute().size(SIZE.0, 140.).reactive_style(move || {
            Styles::new().bg(Color(30, (bar.get() * 37 % 200) as u8, 90, 255))
        }))
        .children((0..60).map(|at| {
            div()
                .absolute()
                .ml(8. + 23. * (at % 20) as f32)
                .mt(146. + 12. * (at / 20) as f32)
                .size(18., 8.)
                .bg(Color(40, 50, 64, 255))
        }))
        .child(switch(
            move || rows.get() % 3,
            |count, _| {
                div()
                    .absolute()
                    .ml(8.)
                    .mt(184.)
                    .size(220., 70.)
                    .children((0..=count).map(|row| {
                        div()
                            .absolute()
                            .mt(22. * row as f32)
                            .size(220., 20.)
                            .bg(Color(25, 32, 43, 255))
                            .child(
                                text(format!("row {row} of {}", count + 1))
                                    .text_size(12.)
                                    .absolute()
                                    .ml(6.)
                                    .mt(2.)
                                    .size(150., 16.),
                            )
                    }))
            },
        ))
        .child(
            div()
                .absolute()
                .ml(240.)
                .mt(184.)
                .size(110., 70.)
                .bg(Color(60, 80, 110, 255))
                .reactive_style(move || Styles::new().opacity(0.4 + (fade.get() % 4) as f32 * 0.2))
                .child(
                    text("faded")
                        .text_size(12.)
                        .absolute()
                        .ml(8.)
                        .mt(8.)
                        .size(80., 16.),
                )
                .child(
                    div()
                        .absolute()
                        .ml(8.)
                        .mt(40.)
                        .size(60., 12.)
                        .bg(Color(255, 196, 61, 255)),
                ),
        )
        .child(
            div()
                .absolute()
                .ml(360.)
                .mt(184.)
                .size(110., 70.)
                .rounded(6.)
                .reactive_style(move || {
                    Styles::new().bg(Color((tint.get() * 53 % 256) as u8, 60, 90, 255))
                })
                .child(
                    text("panel")
                        .text_size(12.)
                        .absolute()
                        .ml(8.)
                        .mt(8.)
                        .size(80., 16.),
                ),
        )
}

#[test]
fn a_subtree_spliced_into_the_kept_window_is_the_subtree_drawn_afresh() {
    let window = |live: u32| {
        let mut gpu = GpuRenderer::new((SIZE.0 * 2.) as u32, (SIZE.1 * 2.) as u32).unwrap();
        gpu.set_scale_factor(2.);
        FontData::new(GEIST.to_vec())
            .unwrap()
            .install(&mut gpu.text_system().borrow_mut());
        let mut ui = Ui::new(SIZE.0, SIZE.1);
        gpu.install_text(&mut ui.scene.borrow_mut());
        let state = ui.signal(live);
        ui.mount(spliced(state.clone()));
        (ui, gpu, state)
    };
    let draw = |ui: &mut Ui, gpu: &mut GpuRenderer| {
        ui.prepare_frame();
        let report = ui.scene.borrow_mut().flush();
        gpu.render(&ui.scene.borrow(), &report.damage).unwrap()
    };
    let (mut ui, mut gpu, live) = window(1);
    draw(&mut ui, &mut gpu);
    let mut patched = 0;
    for value in 2..26 {
        live.set(value);
        let stats = draw(&mut ui, &mut gpu);
        patched += usize::from(stats.patched_items > 0);
        let (mut fresh, mut fresh_gpu, _) = window(value);
        draw(&mut fresh, &mut fresh_gpu);
        assert!(
            gpu.readback().unwrap() == fresh_gpu.readback().unwrap(),
            "frame {value} differs from the subtrees drawn afresh (patched {})",
            stats.patched_items
        );
    }
    // Each frame remounts the list's rows, fades a box and recolours a
    // panel: patched only by splicing.
    assert!(patched >= 16, "{patched} frames patched");
}

/// Boxes drawn as one quad that layout moves and resizes each frame, and
/// whose colour, corners or border change in some of those frames too
/// (`patch_box_moved` must make those again whole), beside a bar damaging
/// most of the window: each frame as drawn afresh.
fn resized(live: Signal<u32>) -> View {
    let bar = live.clone();
    let boxes = (0..4u32).map(move |at| {
        let live = live.clone();
        div()
            .absolute()
            .ml(12. + 110. * at as f32)
            .mt(170.)
            .reactive_style(move || {
                let n = live.get();
                let styles = Styles::new()
                    .w(30. + ((n + at) % 5) as f32 * 4.5)
                    .h(10. + ((n * 3 + at) % 4) as f32 * 2.5);
                // Every third frame something besides the box changes.
                let paint = n.is_multiple_of(3);
                match at {
                    0 => styles.bg(Color(
                        94,
                        177,
                        if paint { (n * 40 % 256) as u8 } else { 255 },
                        255,
                    )),
                    1 => styles.bg(Color(255, 196, 61, 255)).rounded(if paint {
                        2. + (n % 4) as f32
                    } else {
                        4.
                    }),
                    2 => styles
                        .bg(Color(40, 60, 90, 255))
                        .border(if paint { 1. + (n % 3) as f32 } else { 1. })
                        .border_color(Color(250, 250, 250, 255)),
                    _ => styles.bg(Color(200, 80, 120, if paint { 120 } else { 255 })),
                }
            })
    });
    div()
        .size(SIZE.0, SIZE.1)
        .bg(Color(14, 18, 24, 255))
        .child(div().absolute().size(SIZE.0, 150.).reactive_style(move || {
            Styles::new().bg(Color(30, (bar.get() * 37 % 200) as u8, 90, 255))
        }))
        .children(boxes)
}

#[test]
fn a_box_layout_moved_alone_is_moved_and_one_also_restyled_is_made_again() {
    let window = |live: u32| {
        let mut gpu = GpuRenderer::new((SIZE.0 * 2.) as u32, (SIZE.1 * 2.) as u32).unwrap();
        gpu.set_scale_factor(2.);
        let mut ui = Ui::new(SIZE.0, SIZE.1);
        gpu.install_text(&mut ui.scene.borrow_mut());
        let state = ui.signal(live);
        ui.mount(resized(state.clone()));
        (ui, gpu, state)
    };
    let draw = |ui: &mut Ui, gpu: &mut GpuRenderer| {
        ui.prepare_frame();
        let report = ui.scene.borrow_mut().flush();
        gpu.render(&ui.scene.borrow(), &report.damage).unwrap()
    };
    let (mut ui, mut gpu, live) = window(1);
    draw(&mut ui, &mut gpu);
    let mut patched = 0;
    for value in 2..26 {
        live.set(value);
        let stats = draw(&mut ui, &mut gpu);
        patched += usize::from(stats.patched_items > 0);
        let (mut fresh, mut fresh_gpu, _) = window(value);
        draw(&mut fresh, &mut fresh_gpu);
        assert!(
            gpu.readback().unwrap() == fresh_gpu.readback().unwrap(),
            "frame {value} differs from the boxes drawn afresh (patched {})",
            stats.patched_items
        );
    }
    assert!(patched >= 16, "{patched} frames patched");
}

/// Rows of two cells, painted in turn: on the left a box recoloured each
/// frame (the damage, over a tenth of the window: drawn from the kept
/// frame), on the right a long label that never changes. A batch of the
/// left boxes from the kept frame, which holds every draw, must not span
/// the labels between them: the GPU would set each of their glyphs up only
/// to scissor it away.
fn columns(live: Signal<u32>) -> View {
    div()
        .size(SIZE.0, SIZE.1)
        .bg(Color(14, 18, 24, 255))
        .text_color(Color(229, 237, 247, 255))
        .font_family("Geist")
        .children((0..20u32).map(move |row| {
            let live = live.clone();
            div()
                .absolute()
                .mt(4. + 12.5 * row as f32)
                .size(SIZE.0, 12.)
                .child(div().absolute().size(150., 11.).reactive_style(move || {
                    let shade = ((live.get() * 31 + row * 7) % 200) as u8;
                    Styles::new().bg(Color(40, shade, 120, 255))
                }))
                .child(
                    text(format!(
                        "an unchanging label in the right-hand column, row {row:02}"
                    ))
                    .text_size(10.)
                    .absolute()
                    .ml(170.)
                    .size(300., 12.),
                )
        }))
}

#[test]
fn a_frame_drawn_from_the_kept_window_submits_what_its_damage_reaches() {
    let window = |live: u32| {
        let mut gpu = GpuRenderer::new((SIZE.0 * 2.) as u32, (SIZE.1 * 2.) as u32).unwrap();
        gpu.set_scale_factor(2.);
        FontData::new(GEIST.to_vec())
            .unwrap()
            .install(&mut gpu.text_system().borrow_mut());
        let mut ui = Ui::new(SIZE.0, SIZE.1);
        gpu.install_text(&mut ui.scene.borrow_mut());
        let state = ui.signal(live);
        ui.mount(columns(state.clone()));
        (ui, gpu, state)
    };
    let draw = |ui: &mut Ui, gpu: &mut GpuRenderer| {
        ui.prepare_frame();
        let report = ui.scene.borrow_mut().flush();
        gpu.render(&ui.scene.borrow(), &report.damage).unwrap()
    };
    let (mut ui, mut gpu, live) = window(1);
    // Drawn whole: every label's glyphs.
    let whole = draw(&mut ui, &mut gpu).instances;
    let mut patched = 0;
    for value in 2..12 {
        live.set(value);
        let stats = draw(&mut ui, &mut gpu);
        patched += usize::from(stats.patched_items > 0);
        let (mut fresh, mut fresh_gpu, _) = window(value);
        draw(&mut fresh, &mut fresh_gpu);
        assert!(
            gpu.readback().unwrap() == fresh_gpu.readback().unwrap(),
            "frame {value} differs from the columns drawn afresh"
        );
        // The boxes (20) and the background, not the labels between them.
        assert!(
            stats.instances < whole / 4,
            "frame {value} submits {} instances for its damage ({whole} whole)",
            stats.instances
        );
    }
    assert!(patched >= 8, "{patched} frames patched");
}

/// A text growing every frame (outgrowing its spare room, it moves to the
/// kept frame's end) under a box painted after it, translucent, beside a
/// bar repainting most of the window: it must still paint under the box,
/// each frame as drawn afresh, the frames patched rather than kept again.
fn growing(live: Signal<u32>) -> View {
    let (bar, words) = (live.clone(), live.clone());
    div()
        .size(SIZE.0, SIZE.1)
        .bg(Color(14, 18, 24, 255))
        .text_color(Color(229, 237, 247, 255))
        .font_family("Geist")
        .child(div().absolute().size(SIZE.0, 150.).reactive_style(move || {
            Styles::new().bg(Color(30, (bar.get() * 37 % 200) as u8, 90, 255))
        }))
        .child(
            text_signal(move || "grows ".repeat(1 + words.get() as usize % 12))
                .text_size(12.)
                .absolute()
                .ml(10.)
                .mt(170.)
                .size(460., 60.),
        )
        .child(
            div()
                .absolute()
                .ml(40.)
                .mt(165.)
                .size(160., 40.)
                .bg(Color(255, 196, 61, 160)),
        )
}

#[test]
fn a_text_outgrowing_its_room_still_paints_under_what_is_painted_after_it() {
    let window = |live: u32| {
        let mut gpu = GpuRenderer::new((SIZE.0 * 2.) as u32, (SIZE.1 * 2.) as u32).unwrap();
        gpu.set_scale_factor(2.);
        FontData::new(GEIST.to_vec())
            .unwrap()
            .install(&mut gpu.text_system().borrow_mut());
        let mut ui = Ui::new(SIZE.0, SIZE.1);
        gpu.install_text(&mut ui.scene.borrow_mut());
        let state = ui.signal(live);
        ui.mount(growing(state.clone()));
        (ui, gpu, state)
    };
    let draw = |ui: &mut Ui, gpu: &mut GpuRenderer| {
        ui.prepare_frame();
        let report = ui.scene.borrow_mut().flush();
        gpu.render(&ui.scene.borrow(), &report.damage).unwrap()
    };
    let (mut ui, mut gpu, live) = window(0);
    draw(&mut ui, &mut gpu);
    let mut patched = 0;
    for value in 1..12 {
        live.set(value);
        let stats = draw(&mut ui, &mut gpu);
        patched += usize::from(stats.patched_items > 0);
        let (mut fresh, mut fresh_gpu, _) = window(value);
        draw(&mut fresh, &mut fresh_gpu);
        assert!(
            gpu.readback().unwrap() == fresh_gpu.readback().unwrap(),
            "frame {value} differs from the text drawn afresh (patched {})",
            stats.patched_items
        );
    }
    // Growing every frame, it moves rather than making the window keep
    // again (after the first change gives it room).
    assert!(patched >= 9, "{patched} of 11 frames patched");
}
