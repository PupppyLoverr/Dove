//! What a frame costs Metal itself, before any renderer work: one render
//! pass into a 2048×1536 BGRA target (load, store), a scissor, end and
//! commit, each frame waiting for the last as zgx's direct frames do.
//! Prints the main thread's retired instructions per call, and the
//! process's and main thread's CPU time per frame: the floor a renderer's
//! own frame costs are measured against (macOS only).
//!
//! `MODE=3` (`MTLCommandQueue`, a command buffer each frame, as zgx's
//! direct path) or `MODE=4` (`MTL4CommandQueue`, a reused command buffer
//! and allocators, a residency set, a shared event). `PACE_MS=17` sleeps
//! between frames as a 60 Hz window does (0: back to back). `FRAMES=n`.
//! `W`, `H`: the target's size.
//!
//! Run: cargo run --release -p zgx-gpu --example mtl_floor
#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("mtl_floor measures Metal: macOS only");
}

#[cfg(target_os = "macos")]
fn main() {
    floor::main();
}

#[cfg(target_os = "macos")]
mod floor {
    use objc2::rc::Retained;
    use objc2::runtime::ProtocolObject;
    use objc2_metal::*;

    unsafe extern "C" {
        fn thread_selfcounts(kind: i32, buf: *mut u64, size: usize) -> i32;
        fn clock_gettime_nsec_np(clock: u32) -> u64;
    }
    /// The calling thread's retired instructions.
    fn instructions() -> u64 {
        let mut counts = [0u64; 2];
        // SAFETY: Two counters' room for kind 1 (instructions, cycles).
        unsafe { thread_selfcounts(1, counts.as_mut_ptr(), 16) };
        counts[0]
    }
    /// The process's and this thread's CPU time, in nanoseconds
    /// (`CLOCK_PROCESS_CPUTIME_ID`, `CLOCK_THREAD_CPUTIME_ID`).
    fn cpu() -> (u64, u64) {
        // SAFETY: Plain clock reads.
        unsafe { (clock_gettime_nsec_np(12), clock_gettime_nsec_np(16)) }
    }
    fn env(name: &str, default: u64) -> u64 {
        std::env::var(name)
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(default)
    }

    /// Instructions to make the command buffer, open the pass, end it and
    /// commit, summed over the frames after the warm-up.
    #[derive(Default)]
    struct Calls([u64; 4]);
    impl Calls {
        fn add(&mut self, marks: [u64; 5]) {
            for (sum, pair) in self.0.iter_mut().zip(marks.windows(2)) {
                *sum += pair[1] - pair[0];
            }
        }
    }

    pub fn main() {
        let mode = env("MODE", 3);
        let frames = env("FRAMES", 600).max(61) as usize;
        let pace = env("PACE_MS", 0);
        let warm = 60;
        let device = MTLCreateSystemDefaultDevice().expect("a Metal device");
        // SAFETY: A plain 2D descriptor.
        let desc = unsafe {
            MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(
                MTLPixelFormat::BGRA8Unorm,
                env("W", 2048) as usize,
                env("H", 1536) as usize,
                false,
            )
        };
        desc.setUsage(MTLTextureUsage::RenderTarget | MTLTextureUsage::ShaderRead);
        desc.setStorageMode(MTLStorageMode::Private);
        let target = device.newTextureWithDescriptor(&desc).expect("a target");
        let scissor = MTLScissorRect {
            x: 10,
            y: 10,
            width: 40,
            height: 40,
        };
        let mut calls = Calls::default();
        let (mut cpu_from, mut wall_from) = ((0, 0), std::time::Instant::now());
        let mut frame_start = |frame: usize| {
            if frame == warm {
                cpu_from = cpu();
                wall_from = std::time::Instant::now();
            }
            if pace > 0 {
                std::thread::sleep(std::time::Duration::from_micros(pace * 1000 - 300));
            }
        };
        if mode == 4 {
            let queue = device.newMTL4CommandQueue().expect("a Metal 4 queue");
            let allocators = [
                device.newCommandAllocator().expect("an allocator"),
                device.newCommandAllocator().expect("an allocator"),
            ];
            let buffer = device.newCommandBuffer().expect("a Metal 4 command buffer");
            let event = device.newSharedEvent().expect("an event");
            let residency = device
                .newResidencySetWithDescriptor_error(&MTLResidencySetDescriptor::new())
                .expect("a residency set");
            residency.addAllocation(ProtocolObject::from_ref(&*target));
            residency.commit();
            queue.addResidencySet(&residency);
            let pass = MTL4RenderPassDescriptor::new();
            // SAFETY: Index 0 of a render pass descriptor's colour attachments.
            let color = unsafe { pass.colorAttachments().objectAtIndexedSubscript(0) };
            color.setTexture(Some(&target));
            color.setLoadAction(MTLLoadAction::Load);
            color.setStoreAction(MTLStoreAction::Store);
            for frame in 0..frames {
                frame_start(frame);
                // The frame two back used this allocator: done before reuse.
                if frame >= 2 {
                    event.waitUntilSignaledValue_timeoutMS(frame as u64 - 1, 1000);
                }
                let mut marks = [instructions(), 0, 0, 0, 0];
                allocators[frame % 2].reset();
                buffer.beginCommandBufferWithAllocator(&allocators[frame % 2]);
                marks[1] = instructions();
                let encoder = buffer
                    .renderCommandEncoderWithDescriptor(&pass)
                    .expect("an encoder");
                marks[2] = instructions();
                encoder.setScissorRect(scissor);
                encoder.endEncoding();
                buffer.endCommandBuffer();
                marks[3] = instructions();
                let mut list = [std::ptr::NonNull::from(&*buffer)];
                // SAFETY: One live command buffer.
                unsafe { queue.commit_count(std::ptr::NonNull::from(&mut list).cast(), 1) };
                queue.signalEvent_value(ProtocolObject::from_ref(&*event), frame as u64 + 1);
                marks[4] = instructions();
                if frame >= warm {
                    calls.add(marks);
                }
            }
            event.waitUntilSignaledValue_timeoutMS(frames as u64, 1000);
        } else {
            let queue = device.newCommandQueue().expect("a queue");
            let mut last: Option<Retained<ProtocolObject<dyn MTLCommandBuffer>>> = None;
            for frame in 0..frames {
                frame_start(frame);
                if let Some(last) = last.take() {
                    last.waitUntilCompleted();
                }
                let mut marks = [instructions(), 0, 0, 0, 0];
                let buffer = queue.commandBuffer().expect("a command buffer");
                marks[1] = instructions();
                let pass = MTLRenderPassDescriptor::new();
                // SAFETY: Index 0 of a render pass descriptor's colour attachments.
                let color = unsafe { pass.colorAttachments().objectAtIndexedSubscript(0) };
                color.setTexture(Some(&target));
                color.setLoadAction(MTLLoadAction::Load);
                color.setStoreAction(MTLStoreAction::Store);
                let encoder = buffer
                    .renderCommandEncoderWithDescriptor(&pass)
                    .expect("an encoder");
                marks[2] = instructions();
                encoder.setScissorRect(scissor);
                encoder.endEncoding();
                marks[3] = instructions();
                buffer.commit();
                marks[4] = instructions();
                if frame >= warm {
                    calls.add(marks);
                }
                last = Some(buffer);
            }
            if let Some(last) = last {
                last.waitUntilCompleted();
            }
        }
        let (process, thread) = cpu();
        let measured = (frames - warm) as f64;
        let k = |sum: u64| sum as f64 / measured / 1e3;
        let [buffer, encoder, end, commit] = calls.0;
        println!(
            "Metal {mode}, paced {pace} ms: K instructions a frame (each includes a ~1.7 K read): \
             buffer {:.1}  encoder {:.1}  end {:.1}  commit {:.1}  = {:.1}",
            k(buffer),
            k(encoder),
            k(end),
            k(commit),
            k(buffer + encoder + end + commit),
        );
        println!(
            "CPU a frame: process {:.1} µs, main thread {:.1} µs; wall {:.1} µs",
            (process - cpu_from.0) as f64 / measured / 1e3,
            (thread - cpu_from.1) as f64 / measured / 1e3,
            wall_from.elapsed().as_secs_f64() * 1e6 / measured,
        );
    }
}
