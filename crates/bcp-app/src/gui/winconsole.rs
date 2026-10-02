//! Windows console handling. The binary is a console program so that command line prompts
//! work. When it is started with no arguments by double-click, Windows gives it a console of
//! its own; this module detaches from it so no console window stays open behind the GUI. A
//! brief flash while the process starts is accepted (DECISIONS entry 7).
//!
//! This is the only `unsafe` code in `bcp-app`, and none of it touches secrets.

#![allow(unsafe_code)]

use windows_sys::Win32::System::Console::{FreeConsole, GetConsoleProcessList};

/// Frees the console when this process is the only one attached to it, which is the case for
/// a double-click launch. Started from a terminal, the shell is attached as well, so the
/// console is left alone.
pub fn detach_if_alone() {
    let mut list = [0u32; 2];
    // SAFETY: `list` is a valid, writable buffer of the length passed. The call writes at
    // most that many process IDs and returns the number attached (0 on failure).
    let attached = unsafe { GetConsoleProcessList(list.as_mut_ptr(), list.len() as u32) };
    if attached == 1 {
        // SAFETY: FreeConsole takes no arguments and only detaches this process; failure is
        // harmless and ignored.
        unsafe {
            FreeConsole();
        }
    }
}
