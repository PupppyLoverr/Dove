//! A dithered image (`ImageData::dithered`): Zeron's two levels in dots on
//! the screen's grid, each lit to its colour's full value or dimmed to 8% by
//! its 4x4 Bayer threshold; the same on the wgpu path and with Metal directly,
//! moved or scaled.
use std::sync::Arc;
use zgx::{affine::Affine, image::ImageData, scene::*};
use zgx_gpu::GpuRenderer;

fn gpu_fixture() -> std::sync::MutexGuard<'static, ()> {
    static DEVICE_TEST: std::sync::Mutex<()> = std::sync::Mutex::new(());
    DEVICE_TEST
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}
const SIZE: u32 = 64;
fn pixel(bytes: &[u8], x: u32, y: u32) -> [u8; 4] {
    let at = ((y * SIZE + x) * 4) as usize;
    bytes[at..at + 4].try_into().unwrap()
}
fn near(actual: [u8; 4], expected: [u8; 4]) -> bool {
    actual
        .iter()
        .zip(expected)
        .all(|(a, e)| (*a as i32 - e as i32).abs() <= 1)
}
fn fixed(w: f32, h: f32) -> Style {
    Style {
        width: Some(w),
        height: Some(h),
        ..Default::default()
    }
}

#[test]
fn dithered_images_light_or_dim_each_dot_by_its_bayer_threshold() {
    let _gpu_fixture = gpu_fixture();
    // One flat colour: its peak, 200 of 255, passes thresholds 0 to 12.
    let flat = Arc::new(ImageData::new(8, 8, [200u8, 100, 50, 255].repeat(64)).unwrap());
    let (lit, dim) = ([255, 128, 64, 255], [16, 8, 4, 255]);
    for direct in [true, false] {
        let mut gpu = GpuRenderer::new(SIZE, SIZE).unwrap();
        gpu.set_direct_metal(direct);
        let mut scene = Scene::new(SIZE as f32, SIZE as f32);
        scene.set_kind(scene.root(), NodeKind::Container(Layout::Overlay));
        let node = scene.append(
            scene.root(),
            NodeKind::Image(Arc::new(flat.dithered(2.))),
            fixed(SIZE as f32, SIZE as f32),
        );
        for (case, image) in [
            ("plain", flat.dithered(2.)),
            // Scaled about its centre (as `fit = cover` draws): the dots stay
            // two pixels on the screen's grid.
            (
                "scaled",
                flat.dithered(2.).transformed(Affine::scale(1.5, 1.5)),
            ),
        ] {
            scene.set_kind(node, NodeKind::Image(Arc::new(image)));
            let damage = scene.flush().damage;
            gpu.render(&scene, &damage).unwrap();
            let p = gpu.readback().unwrap();
            let check = |x: u32, y: u32, expected: [u8; 4], what: &str| {
                assert!(
                    near(pixel(&p, x, y), expected),
                    "direct {direct}, {case}, {what} at ({x}, {y}): {:?}",
                    pixel(&p, x, y)
                );
            };
            // Dot (0, 0), threshold 0: lit, both its pixels alike.
            check(0, 0, lit, "threshold 0");
            check(1, 1, lit, "the same dot");
            // Dot (0, 3), threshold 15, and (2, 1), 14: dimmed.
            check(0, 6, dim, "threshold 15");
            check(1, 7, dim, "the same dot");
            check(4, 2, dim, "threshold 14");
            // Dot (1, 1), threshold 4: lit. And the pattern repeats every 4 dots.
            check(2, 2, lit, "threshold 4");
            check(8, 6, dim, "threshold 15, a tile on");
            // Thirteen of every sixteen dots lit.
            let lit_dots = (0..4)
                .flat_map(|y| (0..4).map(move |x| (x, y)))
                .filter(|(x, y)| near(pixel(&p, x * 2, y * 2), lit))
                .count();
            assert_eq!(lit_dots, 13, "direct {direct}, {case}");
        }
        // Undithered, the colour itself.
        scene.set_kind(node, NodeKind::Image(flat.clone()));
        let damage = scene.flush().damage;
        gpu.render(&scene, &damage).unwrap();
        let p = gpu.readback().unwrap();
        assert!(
            near(pixel(&p, 0, 6), [200, 100, 50, 255]),
            "direct {direct}"
        );
        if direct && cfg!(target_os = "macos") {
            assert!(gpu.debug_metal_frames() > 0, "never drew with Metal");
        }
    }
}

#[test]
fn characters_draw_a_glyph_a_cell_over_the_image_and_reveal_cell_by_cell() {
    let _gpu_fixture = gpu_fixture();
    // Its brightness (0.81) picks `8`, whose second row starts lit.
    let flat = Arc::new(ImageData::new(8, 8, [240u8, 200, 160, 255].repeat(64)).unwrap());
    let (dim, glyph) = ([211, 176, 141, 255], [240, 200, 160, 255]);
    let clear = [0, 0, 0, 0];
    for direct in [true, false] {
        let mut gpu = GpuRenderer::new(SIZE, SIZE).unwrap();
        gpu.set_direct_metal(direct);
        let mut scene = Scene::new(SIZE as f32, SIZE as f32);
        scene.set_kind(scene.root(), NodeKind::Container(Layout::Overlay));
        let node = scene.append(
            scene.root(),
            NodeKind::Image(Arc::new(flat.characters(1.))),
            fixed(SIZE as f32, SIZE as f32),
        );
        let mut render = |scene: &mut Scene| {
            let damage = scene.flush().damage;
            gpu.render(scene, &damage).unwrap();
            gpu.readback().unwrap()
        };
        let at = |p: &[u8], x: u32, y: u32, expected: [u8; 4], what: &str| {
            assert!(
                near(pixel(p, x, y), expected),
                "direct {direct}, {what} at ({x}, {y}): {:?}",
                pixel(p, x, y)
            );
        };
        // No reveal: every cell, its glyph over the image dimmed.
        let p = render(&mut scene);
        at(&p, 0, 1, glyph, "a lit font pixel");
        at(&p, 1, 1, dim, "an unlit one");
        at(&p, 0, 0, dim, "its top row's first, unlit");
        at(&p, 6, 1, glyph, "the next cell's");
        let reveal = |scene: &mut Scene, front: f32, band: f32| {
            scene.set_shader_uniforms(
                node,
                zgx::decoration::ShaderUniforms {
                    time: 0.,
                    params: [[front, band, 0., 0.], [0.; 4], [0.; 4], [0.; 4]],
                },
            );
        };
        // The front short of the middle: nothing.
        reveal(&mut scene, -0.1, 0.2);
        let p = render(&mut scene);
        at(&p, 32, 32, clear, "ahead of the front");
        at(&p, 0, 1, clear, "ahead of the front");
        // Past every corner: all of it, as with no reveal.
        reveal(&mut scene, 3., 0.2);
        let p = render(&mut scene);
        at(&p, 0, 1, glyph, "behind the front");
        // Part way: the middle shows (characters, the picture fading in
        // behind them), a corner cell nothing yet.
        reveal(&mut scene, 0.45, 0.2);
        let p = render(&mut scene);
        assert!(
            (24..40).any(|x| (24..40).any(|y| pixel(&p, x, y)[3] > 0)),
            "direct {direct}: the middle shows"
        );
        at(&p, 0, 1, clear, "a corner cell, not reached");
    }
}

#[test]
fn a_characters_reveal_is_measured_in_the_window_at_any_scale() {
    let _gpu_fixture = gpu_fixture();
    let flat = Arc::new(ImageData::new(8, 8, [200u8, 100, 50, 255].repeat(64)).unwrap());
    for direct in [true, false] {
        // 32 points a side at scale 2: 64 pixels.
        let mut gpu = GpuRenderer::new(SIZE, SIZE).unwrap();
        gpu.set_direct_metal(direct);
        gpu.set_scale_factor(2.);
        let mut scene = Scene::new(32., 32.);
        scene.set_kind(scene.root(), NodeKind::Container(Layout::Overlay));
        let node = scene.append(
            scene.root(),
            NodeKind::Image(Arc::new(flat.characters(0.5))),
            fixed(32., 32.),
        );
        // A small front at the middle: the middle shows, the corners don't.
        scene.set_shader_uniforms(
            node,
            zgx::decoration::ShaderUniforms {
                time: 0.,
                params: [[0.3, 0.05, 0., 0.], [0.; 4], [0.; 4], [0.; 4]],
            },
        );
        let damage = scene.flush().damage;
        gpu.render(&scene, &damage).unwrap();
        let p = gpu.readback().unwrap();
        let shown = |x: u32, y: u32| pixel(&p, x, y)[3] > 0;
        assert!(
            (28..36).any(|x| (28..36).any(|y| shown(x, y))),
            "direct {direct}: the middle"
        );
        for (x, y) in [(2, 2), (61, 2), (2, 61), (61, 61), (16, 16)] {
            assert!(
                !shown(x, y),
                "direct {direct}: ({x}, {y}) is past the front"
            );
        }
    }
}
