//! A window for `scripts/windows_present_probe.py`: a mark moves along a row
//! over 30 frames, each drawing its damage alone, then the window holds its
//! last frame. On screen, every place the mark passed shows the background
//! again and the last shows the mark: a buffer brought up to date wrongly
//! (Windows' swap chain of the renderer's own) leaves a stale
//! mark or a hole. Prints `ready` once the last frame is presented.
//!
//! With `--lose`, the device is then lost: the frame must fail, and a new
//! renderer on a new context, for the same window, draws it whole.
use std::sync::Arc;
use winit::{application::ApplicationHandler, event::WindowEvent, event_loop::*, window::*};
use zgx::scene::*;
use zgx_gpu::GpuRenderer;

const STEPS: u32 = 30;

#[derive(Default)]
struct App {
    window: Option<Arc<Window>>,
    renderer: Option<GpuRenderer>,
    scene: Option<(Scene, NodeId)>,
    step: u32,
}

fn fixed(w: f32, h: f32) -> Style {
    Style {
        width: Some(w),
        height: Some(h),
        ..Default::default()
    }
}

fn renderer_for(window: &Arc<Window>) -> GpuRenderer {
    let mut renderer =
        GpuRenderer::for_window_with_transparency(window.clone(), None, false).unwrap();
    renderer.set_scale_factor(1.);
    renderer.set_background(Color(20, 40, 80, 255));
    renderer
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attributes = Window::default_attributes()
            .with_title("zgx present check")
            .with_inner_size(winit::dpi::PhysicalSize::new(400, 100))
            .with_resizable(false);
        let window = Arc::new(event_loop.create_window(attributes).unwrap());
        let renderer = renderer_for(&window);
        let size = window.inner_size();
        let mut scene = Scene::new(size.width as f32, size.height as f32);
        scene.set_kind(scene.root(), NodeKind::Container(Layout::Overlay));
        scene.append(
            scene.root(),
            NodeKind::Rect(Color(20, 40, 80, 255)),
            fixed(size.width as f32, size.height as f32),
        );
        let mark = scene.append(
            scene.root(),
            NodeKind::Rect(Color(220, 30, 30, 255)),
            fixed(10., 10.),
        );
        self.scene = Some((scene, mark));
        self.renderer = Some(renderer);
        self.window = Some(window);
        event_loop.set_control_flow(ControlFlow::Poll);
    }
    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        if let WindowEvent::CloseRequested = event {
            event_loop.exit();
        }
    }
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let (Some(renderer), Some((scene, mark))) = (&mut self.renderer, &mut self.scene) else {
            return;
        };
        if self.step > STEPS {
            event_loop.set_control_flow(ControlFlow::Wait);
            return;
        }
        scene.set_transform(
            *mark,
            Transform {
                x: 10. + self.step as f32 * 12.,
                y: 45.,
            },
        );
        let damage = scene.flush().damage;
        renderer.render(scene, &damage).unwrap();
        renderer.present().unwrap();
        self.step += 1;
        if self.step > STEPS {
            if std::env::args().any(|arg| arg == "--lose") {
                renderer.context().debug_lose_device();
                let error = renderer.render(scene, &damage).unwrap_err();
                assert!(error.is_device_lost(), "{error}");
                // As the desktop host recovers: the old renderer goes first.
                self.renderer = None;
                let window = self.window.as_ref().unwrap();
                let mut renderer = renderer_for(window);
                let size = window.inner_size();
                let whole = [Rect::new(0., 0., size.width as f32, size.height as f32)];
                renderer.render(scene, &whole).unwrap();
                renderer.present().unwrap();
                self.renderer = Some(renderer);
            }
            println!("ready");
        }
    }
}

fn main() {
    let event_loop = EventLoop::new().unwrap();
    event_loop.run_app(&mut App::default()).unwrap();
}
