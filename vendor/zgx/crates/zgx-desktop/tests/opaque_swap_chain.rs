//! Windows: an opaque window's swap chain is opaque from the start, so the
//! compositor copies its frames rather than blending them over what lies
//! behind (7 to 14% of its GPU time a frame); a transparent window's has
//! alpha; and one made opaque whose background lets what is behind show is
//! made again with alpha at its first frame.
#![cfg(windows)]
use std::sync::Arc;
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::platform::run_on_demand::EventLoopExtRunOnDemand;
use winit::platform::windows::{EventLoopBuilderExtWindows, WindowAttributesExtWindows};
use winit::window::{Window, WindowId};
use zgx::scene::{Color, Rect, Scene};
use zgx_gpu::{GpuContext, GpuRenderer};

/// A window made as zgx-desktop makes its windows (`window_info`), whether
/// `transparent`, drawn once on `background`: whether its swap chain
/// presents opaque when made and after the frame.
fn drawn(
    event_loop: &ActiveEventLoop,
    context: &mut Option<GpuContext>,
    transparent: bool,
    background: Color,
) -> (Option<bool>, Option<bool>) {
    let attributes = Window::default_attributes()
        .with_title("zgx opaque swap chain test")
        .with_inner_size(winit::dpi::PhysicalSize::new(200, 120))
        .with_transparent(transparent)
        .with_no_redirection_bitmap(true);
    let window = Arc::new(event_loop.create_window(attributes).unwrap());
    let mut renderer =
        GpuRenderer::for_window_with_transparency(window, context.as_ref(), transparent).unwrap();
    context.get_or_insert_with(|| renderer.context());
    renderer.set_background(background);
    let made = renderer.presents_opaque();
    let scene = Scene::new(200., 120.);
    renderer
        .render(&scene, &[Rect::new(0., 0., 200., 120.)])
        .unwrap();
    renderer.present().unwrap();
    (made, renderer.presents_opaque())
}

#[derive(Default)]
struct App {
    windows: Vec<(Option<bool>, Option<bool>)>,
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let mut context = None;
        let (clear, dark) = (Color(0, 0, 0, 0), Color(30, 30, 30, 255));
        for (transparent, background) in [(false, dark), (true, clear), (false, clear)] {
            let window = drawn(event_loop, &mut context, transparent, background);
            self.windows.push(window);
        }
        event_loop.exit();
    }
    fn window_event(&mut self, _: &ActiveEventLoop, _: WindowId, _: WindowEvent) {}
}

#[test]
fn an_opaque_windows_swap_chain_is_opaque_and_a_see_through_ones_is_not() {
    let mut event_loop = EventLoop::builder().with_any_thread(true).build().unwrap();
    let mut app = App::default();
    event_loop.run_app_on_demand(&mut app).unwrap();
    assert_eq!(
        app.windows,
        [
            // Opaque, on an opaque background: opaque from the start.
            (Some(true), Some(true)),
            // Transparent: alpha.
            (Some(false), Some(false)),
            // Made opaque, its background clear: alpha from the first frame.
            (Some(true), Some(false)),
        ],
        "(opaque when made, after the first frame), for each window"
    );
}
