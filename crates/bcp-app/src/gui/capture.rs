//! Screen capture exclusion while the passphrase is on screen (DECISIONS entry 10).
//!
//! On Windows the window gets `SetWindowDisplayAffinity(WDA_EXCLUDEFROMCAPTURE)` for as long
//! as a screen shows the passphrase: screenshots, screen recorders and screen sharing see the
//! desktop behind it instead (Windows 10 version 2004 and later). Older Windows versions
//! reject that value; `WDA_MONITOR` is used there, which shows the window black in captures.
//! The affinity is set back to `WDA_NONE` when the passphrase is wiped.
//!
//! macOS and Linux get nothing here; see the DECISIONS entry for why.

/// Remembers what was last applied, so the system call is made once per change and not in
/// every frame.
#[derive(Default)]
pub struct CaptureGuard {
    applied: Option<bool>,
}

impl CaptureGuard {
    /// Applies `exclude` through `apply` when it differs from what was last applied. `apply`
    /// returns whether the system accepted it; a refusal is not retried every frame.
    pub fn update(&mut self, exclude: bool, apply: impl FnOnce(bool) -> bool) {
        if self.applied == Some(exclude) {
            return;
        }
        // Nothing to undo before anything was ever excluded.
        if self.applied.is_none() && !exclude {
            self.applied = Some(false);
            return;
        }
        let _ = apply(exclude);
        self.applied = Some(exclude);
    }

    /// True when the last change asked for exclusion.
    #[cfg(test)]
    pub fn excluded(&self) -> bool {
        self.applied == Some(true)
    }
}

/// Excludes the window of `frame` from screen capture, or allows it again. Returns whether
/// the system accepted the change; always false where it is not supported.
#[cfg(windows)]
pub fn set_excluded(frame: &eframe::Frame, exclude: bool) -> bool {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use windows_sys::Win32::Foundation::HWND;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SetWindowDisplayAffinity, WDA_EXCLUDEFROMCAPTURE, WDA_MONITOR, WDA_NONE,
    };

    let Ok(handle) = frame.window_handle() else {
        return false;
    };
    let RawWindowHandle::Win32(win32) = handle.as_raw() else {
        return false;
    };
    let hwnd = win32.hwnd.get() as HWND;
    // SAFETY: `hwnd` is the live top-level window of this process, borrowed from eframe for
    // the duration of the call; the function only changes a display attribute of it.
    unsafe {
        if exclude {
            SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE) != 0
                || SetWindowDisplayAffinity(hwnd, WDA_MONITOR) != 0
        } else {
            SetWindowDisplayAffinity(hwnd, WDA_NONE) != 0
        }
    }
}

/// Not supported on this system: does nothing.
#[cfg(not(windows))]
pub fn set_excluded(_frame: &eframe::Frame, _exclude: bool) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_system_is_called_once_per_change() {
        let mut g = CaptureGuard::default();
        let mut calls = Vec::new();
        // Nothing shown yet: no call.
        g.update(false, |e| {
            calls.push(e);
            true
        });
        assert!(calls.is_empty());
        for _ in 0..3 {
            g.update(true, |e| {
                calls.push(e);
                true
            });
        }
        assert!(g.excluded());
        g.update(false, |e| {
            calls.push(e);
            true
        });
        g.update(false, |e| {
            calls.push(e);
            true
        });
        assert_eq!(calls, [true, false]);
        assert!(!g.excluded());
    }

    #[test]
    fn a_refusal_is_not_retried_every_frame() {
        let mut g = CaptureGuard::default();
        let mut calls = 0;
        for _ in 0..5 {
            g.update(true, |_| {
                calls += 1;
                false
            });
        }
        assert_eq!(calls, 1);
    }
}
