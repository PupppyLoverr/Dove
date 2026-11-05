//! An IOSurface's pixels on the CPU, read while it is locked read-only: the
//! one place zgx declares IOSurface's lock and reads (a window's shown frame,
//! `GpuRenderer::shown_pixels`, and the desktop's terminal thumbnails).
use std::ffi::c_void;

// SAFETY: IOSurface's declarations (IOSurface.framework, linked by
// objc2-io-surface): `IOSurfaceLockOptions` and the seed are `u32`,
// `kern_return_t` is `i32`.
unsafe extern "C" {
    fn IOSurfaceLock(surface: *const c_void, options: u32, seed: *mut u32) -> i32;
    fn IOSurfaceUnlock(surface: *const c_void, options: u32, seed: *mut u32) -> i32;
    fn IOSurfaceGetBaseAddress(surface: *const c_void) -> *const u8;
    fn IOSurfaceGetBytesPerRow(surface: *const c_void) -> usize;
    fn IOSurfaceGetWidth(surface: *const c_void) -> usize;
    fn IOSurfaceGetHeight(surface: *const c_void) -> usize;
}
/// kIOSurfaceLockReadOnly.
const READ_ONLY: u32 = 1;

/// `read` given `surface`'s pixels while it is locked read-only: its bytes
/// (`height` rows `stride` bytes apart, each at least `width * 4`, BGRA),
/// width, height and stride. `None` when it cannot be locked or holds no
/// pixels. Nothing of the bytes outlives `read`.
///
/// # Safety
///
/// `surface` is a live IOSurface of four bytes a pixel for the whole call.
#[doc(hidden)]
pub unsafe fn with_pixels<R>(
    surface: *const c_void,
    read: impl FnOnce(&[u8], usize, usize, usize) -> R,
) -> Option<R> {
    // SAFETY: the caller's live surface, locked read-only before its base
    // address is read: `height` rows of `stride` bytes are readable (an
    // IOSurface allocates at least that) until it is unlocked below.
    unsafe {
        if IOSurfaceLock(surface, READ_ONLY, std::ptr::null_mut()) != 0 {
            return None;
        }
        let (width, height) = (IOSurfaceGetWidth(surface), IOSurfaceGetHeight(surface));
        let (base, stride) = (
            IOSurfaceGetBaseAddress(surface),
            IOSurfaceGetBytesPerRow(surface),
        );
        let result =
            (!base.is_null() && width > 0 && height > 0 && stride >= width * 4).then(|| {
                read(
                    std::slice::from_raw_parts(base, stride * height),
                    width,
                    height,
                    stride,
                )
            });
        IOSurfaceUnlock(surface, READ_ONLY, std::ptr::null_mut());
        result
    }
}
