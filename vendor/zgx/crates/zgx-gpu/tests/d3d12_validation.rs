//! Windows: frames drawn through the repository's wgpu-hal raise no error in
//! the Direct3D 12 debug layer: every resource is in the state each command
//! needs, the vertex ring's buffers and a window's own swap chain
//! (dxgi_present.rs) included. Test builds ask for the debug
//! layer (wgpu's default flags in debug builds); where it isn't installed
//! (Windows' Graphics Tools), the test says so and checks nothing, unless
//! `ZGX_REQUIRE_DEBUG_LAYER` is set, as in the manual `d3d12-debug-layer`
//! workflow, which installs it.
#![cfg(all(windows, zgx_wgpu_hal_patched))]
use windows::{
    Win32::System::LibraryLoader::{GetModuleHandleW, LoadLibraryW},
    core::w,
};
use zgx::scene::*;
use zgx_gpu::GpuRenderer;

fn fixed(w: f32, h: f32) -> Style {
    Style {
        width: Some(w),
        height: Some(h),
        ..Default::default()
    }
}

/// The debug layer's reports are one list for the process: the tests take
/// turns.
static LAYER: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Whether the debug layer is there to check against: else the test says
/// so, unless it must be (`ZGX_REQUIRE_DEBUG_LAYER`).
fn debug_layer() -> bool {
    let required = std::env::var_os("ZGX_REQUIRE_DEBUG_LAYER").is_some();
    // SAFETY: loads a system library by name.
    if unsafe { LoadLibraryW(w!("d3d12SDKLayers.dll")) }.is_err() {
        assert!(!required, "no Direct3D 12 debug layer (Graphics Tools)");
        eprintln!("skipped: no Direct3D 12 debug layer (Windows' Graphics Tools) on this machine");
        return false;
    }
    true
}

#[test]
fn frames_raise_no_debug_layer_error() {
    let _turn = LAYER
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let required = std::env::var_os("ZGX_REQUIRE_DEBUG_LAYER").is_some();
    if !debug_layer() {
        return;
    }
    let _ = wgpu_hal::VALIDATION_CANARY.get_and_reset();
    let mut gpu = GpuRenderer::new(160, 80).expect("an adapter");
    if required {
        // SAFETY: looks a loaded module up by name.
        assert!(
            unsafe { GetModuleHandleW(w!("d3d12SDKLayers.dll")) }.is_ok(),
            "the debug layer is on"
        );
    }
    let mut scene = Scene::new(160., 80.);
    scene.set_kind(scene.root(), NodeKind::Container(Layout::Overlay));
    scene.append(
        scene.root(),
        NodeKind::Rect(Color(20, 40, 60, 255)),
        fixed(160., 80.),
    );
    let moving = scene.append(
        scene.root(),
        NodeKind::Rect(Color(200, 0, 0, 128)),
        fixed(20., 20.),
    );
    let label = scene.append(
        scene.root(),
        NodeKind::Text {
            text: "barriers".into(),
            font_size: 14.,
            color: Color(255, 255, 255, 255),
        },
        fixed(120., 24.),
    );
    // Frames back to back go round the vertex ring while earlier ones are
    // in flight; some redraw whole, some are read back.
    for frame in 0..40 {
        scene.set_transform(
            moving,
            Transform {
                x: (frame * 3) as f32,
                y: 10.,
            },
        );
        if frame % 5 == 0 {
            scene.set_kind(
                label,
                NodeKind::Text {
                    text: format!("frame {frame}").into(),
                    font_size: 14.,
                    color: Color(255, 255, 255, 255),
                },
            );
        }
        let damage = scene.flush().damage;
        gpu.render(&scene, &damage).unwrap();
        if frame % 10 == 9 {
            gpu.render(&scene, &[Rect::new(0., 0., 160., 80.)]).unwrap();
            gpu.readback().unwrap();
        }
        gpu.present().unwrap();
    }
    gpu.readback().unwrap();
    let errors = wgpu_hal::VALIDATION_CANARY.get_and_reset();
    assert!(errors.is_empty(), "the debug layer reported: {errors:#?}");
}

/// A window's frames through its own swap chain: drawn with damage, caught
/// up from the buffer shown, resized and presented.
#[test]
fn window_frames_raise_no_debug_layer_error() {
    use std::sync::Arc;
    use winit::{
        application::ApplicationHandler,
        event::WindowEvent,
        event_loop::{ActiveEventLoop, EventLoop},
        platform::{
            run_on_demand::EventLoopExtRunOnDemand,
            windows::{EventLoopBuilderExtWindows, WindowAttributesExtWindows},
        },
        window::{Window, WindowId},
    };
    let _turn = LAYER
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if !debug_layer() {
        return;
    }
    let _ = wgpu_hal::VALIDATION_CANARY.get_and_reset();
    struct App;
    impl ApplicationHandler for App {
        fn resumed(&mut self, event_loop: &ActiveEventLoop) {
            let attributes = Window::default_attributes()
                .with_title("zgx debug layer test")
                .with_inner_size(winit::dpi::PhysicalSize::new(200, 120))
                .with_no_redirection_bitmap(true);
            let window = Arc::new(event_loop.create_window(attributes).unwrap());
            let mut gpu = GpuRenderer::for_window_with_transparency(window, None, false).unwrap();
            let mut scene = Scene::new(200., 120.);
            scene.set_kind(scene.root(), NodeKind::Container(Layout::Overlay));
            let background = scene.append(
                scene.root(),
                NodeKind::Rect(Color(20, 40, 60, 255)),
                fixed(200., 120.),
            );
            let mark = scene.append(
                scene.root(),
                NodeKind::Rect(Color(200, 0, 0, 255)),
                fixed(10., 10.),
            );
            for frame in 0..24 {
                if frame == 12 {
                    scene.resize(260., 150.);
                    scene.set_style(background, fixed(260., 150.));
                    gpu.resize(260, 150);
                }
                scene.set_transform(
                    mark,
                    Transform {
                        x: (frame * 7) as f32,
                        y: 20.,
                    },
                );
                let damage = scene.flush().damage;
                gpu.render(&scene, &damage).unwrap();
                gpu.present().unwrap();
            }
            event_loop.exit();
        }
        fn window_event(&mut self, _: &ActiveEventLoop, _: WindowId, _: WindowEvent) {}
    }
    let mut event_loop = EventLoop::builder().with_any_thread(true).build().unwrap();
    event_loop.run_app_on_demand(&mut App).unwrap();
    let errors = wgpu_hal::VALIDATION_CANARY.get_and_reset();
    assert!(errors.is_empty(), "the debug layer reported: {errors:#?}");
}
