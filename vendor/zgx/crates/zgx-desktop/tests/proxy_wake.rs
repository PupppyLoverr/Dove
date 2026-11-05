//! Windows: user events sent from another thread reach the event loop,
//! each once and in order, however they are woken for: by the loop's event
//! while it waits (the vendored winit's `ProxyWake`), coalesced when several
//! come before it wakes, or by a posted message while it is busy.
#![cfg(windows)]
use std::time::Duration;
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::platform::run_on_demand::EventLoopExtRunOnDemand;
use winit::platform::windows::EventLoopBuilderExtWindows;
use winit::window::WindowId;

const SENT: u32 = 2000;

#[derive(Default)]
struct App {
    received: Vec<u32>,
}

impl ApplicationHandler<u32> for App {
    fn resumed(&mut self, _: &ActiveEventLoop) {}
    fn window_event(&mut self, _: &ActiveEventLoop, _: WindowId, _: WindowEvent) {}
    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: u32) {
        if event == u32::MAX {
            event_loop.exit();
            return;
        }
        self.received.push(event);
        // Busy now and then, so later sends find the loop away from its
        // wait and post.
        if event.is_multiple_of(97) {
            std::thread::sleep(Duration::from_millis(2));
        }
    }
}

#[test]
fn user_events_from_another_thread_arrive_once_and_in_order() {
    let mut event_loop = EventLoop::<u32>::with_user_event()
        .with_any_thread(true)
        .build()
        .unwrap();
    let proxy = event_loop.create_proxy();
    let sender = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(50));
        for n in 0..SENT {
            proxy.send_event(n).unwrap();
            // Bursts, then pauses long enough for the loop to park.
            if (n + 1).is_multiple_of(50) {
                std::thread::sleep(Duration::from_millis(3));
            }
        }
        std::thread::sleep(Duration::from_millis(20));
        proxy.send_event(u32::MAX).unwrap();
    });
    let mut app = App::default();
    event_loop.run_app_on_demand(&mut app).unwrap();
    sender.join().unwrap();
    assert_eq!(app.received, (0..SENT).collect::<Vec<_>>());
}
