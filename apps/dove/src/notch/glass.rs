//! Real-time glass on Windows: the system's own blurred backdrop (the one
//! Acrylic uses) through Windows.UI.Composition, drawn *under* GPUI's
//! DirectComposition content in the same window and clipped to the island's
//! rounded shapes. It follows moving video at the display's rate and costs
//! the island nothing (DWM does the work).
//!
//! GPUI's DirectComposition target is the window's topmost one, so this
//! takes the non-topmost slot. If any step fails (an older Windows, a
//! driver, transparency effects turned off), [`Glass::new`] returns `None`
//! and the island keeps its captured-and-blurred backdrop instead.

use super::platform::Shape;

#[cfg(windows)]
pub use imp::Glass;

#[cfg(not(windows))]
pub struct Glass;

#[cfg(not(windows))]
impl Glass {
    pub fn new(_: isize) -> Option<Glass> {
        None
    }
    pub fn shape(&self, _: &[Shape], _: f32) {}
}

#[cfg(windows)]
mod imp {
    use std::cell::RefCell;

    use windows_numerics::{Vector2, Vector3};
    use windows::System::DispatcherQueueController;
    use windows::UI::Composition::Desktop::DesktopWindowTarget;
    use windows::UI::Composition::{
        CompositionRoundedRectangleGeometry, Compositor, ContainerVisual, SpriteVisual,
    };
    use windows::Win32::Foundation::HWND;
    use windows::Win32::System::WinRT::Composition::ICompositorDesktopInterop;
    use windows::Win32::System::WinRT::{
        CreateDispatcherQueueController, DQTAT_COM_NONE, DQTYPE_THREAD_CURRENT, DispatcherQueueOptions,
    };
    use windows_core::Interface as _;

    use super::Shape;

    thread_local! {
        /// Composition needs a dispatcher queue on the UI thread; one for
        /// the island's life.
        static QUEUE: RefCell<Option<DispatcherQueueController>> = const { RefCell::new(None) };
    }

    fn ensure_queue() -> windows_core::Result<()> {
        QUEUE.with(|q| {
            if q.borrow().is_some() || windows::System::DispatcherQueue::GetForCurrentThread().is_ok() {
                return Ok(());
            }
            let options = DispatcherQueueOptions {
                dwSize: size_of::<DispatcherQueueOptions>() as u32,
                threadType: DQTYPE_THREAD_CURRENT,
                apartmentType: DQTAT_COM_NONE,
            };
            // SAFETY: creates a queue for this (the UI) thread, kept alive
            // in QUEUE.
            let controller = unsafe { CreateDispatcherQueueController(options)? };
            *q.borrow_mut() = Some(controller);
            Ok(())
        })
    }

    /// One glass pane per island shape (the capsule, and up to two bubbles).
    struct Pane {
        visual: SpriteVisual,
        geometry: CompositionRoundedRectangleGeometry,
    }

    pub struct Glass {
        _compositor: Compositor,
        _target: DesktopWindowTarget,
        _root: ContainerVisual,
        panes: Vec<Pane>,
    }

    impl Glass {
        pub fn new(hwnd: isize) -> Option<Glass> {
            match Self::build(hwnd) {
                Ok(g) => Some(g),
                Err(e) => {
                    tracing::info!("notch: composition glass unavailable ({e}); using captured glass");
                    None
                }
            }
        }

        fn build(hwnd: isize) -> windows_core::Result<Glass> {
            if std::env::var_os("DOVE_NOTCH_CAPTURED_GLASS").is_some() {
                return Err(windows_core::Error::from_hresult(windows_core::HRESULT(-1)));
            }
            ensure_queue()?;
            let compositor = Compositor::new()?;
            let interop: ICompositorDesktopInterop = compositor.cast()?;
            // SAFETY: the island's own window, alive for as long as this.
            let target = unsafe { interop.CreateDesktopWindowTarget(HWND(hwnd as *mut _), false)? };
            // SAFETY: a documented attribute on our window.
            unsafe {
                let on: i32 = 1;
                windows_sys::Win32::Graphics::Dwm::DwmSetWindowAttribute(
                    hwnd as windows_sys::Win32::Foundation::HWND,
                    windows_sys::Win32::Graphics::Dwm::DWMWA_USE_HOSTBACKDROPBRUSH as u32,
                    (&raw const on).cast(),
                    4,
                );
            }
            let root = compositor.CreateContainerVisual()?;
            target.SetRoot(&root)?;
            let mut panes = Vec::new();
            for _ in 0..3 {
                let brush = compositor.CreateHostBackdropBrush()?;
                let visual = compositor.CreateSpriteVisual()?;
                visual.SetBrush(&brush)?;
                let geometry = compositor.CreateRoundedRectangleGeometry()?;
                let clip = compositor.CreateGeometricClipWithGeometry(&geometry)?;
                visual.SetClip(&clip)?;
                visual.SetIsVisible(false)?;
                root.Children()?.InsertAtTop(&visual)?;
                panes.push(Pane { visual, geometry });
            }
            Ok(Glass { _compositor: compositor, _target: target, _root: root, panes })
        }

        /// Puts the glass under `shapes` (window logical pixels).
        pub fn shape(&self, shapes: &[Shape], scale: f32) {
            for (i, pane) in self.panes.iter().enumerate() {
                let _ = (|| -> windows_core::Result<()> {
                    match shapes.get(i) {
                        Some(s) if s.w > 0.5 && s.h > 0.5 => {
                            let (x, y, w, h, r) = (s.x * scale, s.y * scale, s.w * scale, s.h * scale, s.r * scale);
                            pane.visual.SetOffset(Vector3 { X: 0.0, Y: 0.0, Z: 0.0 })?;
                            pane.visual.SetSize(Vector2 { X: x + w + 1.0, Y: y + h + 1.0 })?;
                            pane.geometry.SetOffset(Vector2 { X: x, Y: y })?;
                            pane.geometry.SetSize(Vector2 { X: w, Y: h })?;
                            pane.geometry.SetCornerRadius(Vector2 { X: r, Y: r })?;
                            pane.visual.SetIsVisible(true)?;
                        }
                        _ => pane.visual.SetIsVisible(false)?,
                    }
                    Ok(())
                })();
            }
        }
    }
}
