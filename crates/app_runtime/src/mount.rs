// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

//! Safe Rust wrapper around `statvfs(2)`, used to read a sync device's
//! filesystem capacity for the disk-occupation bar.
//!
//! Why a dedicated module: the workspace bans `unsafe_code` everywhere
//! except small audited files like this one. `statvfs` has no safe
//! wrapper in glibc-via-`libc`, `nix`, or `rustix` at our pinned
//! versions, so it is reached through `libc::statvfs` behind the
//! `#![allow(unsafe_code)]` below. The single public function takes a
//! `&Path` and returns plain `u64` byte counts, so callers never touch a
//! raw struct or pointer.

#![allow(unsafe_code)]

use std::path::Path;

/// Total and available bytes of the filesystem mounted at `path`.
///
/// Returns `None` if the path cannot be `statvfs`'d — typically because
/// the device was unplugged between discovery and this call. `available`
/// is the space an unprivileged process may use (`f_bavail`), not the
/// root-reserved `f_bfree`.
pub fn capacity(path: &Path) -> Option<(u64, u64)> {
    use std::os::unix::ffi::OsStrExt;

    let c_path = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut stat = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    // SAFETY: `c_path` is a valid NUL-terminated C string for the
    // lifetime of the call, and `stat` is owned, suitably-aligned storage
    // that `statvfs` fully initialises when it returns 0. We read the
    // struct only on that success path.
    let stat = unsafe {
        if libc::statvfs(c_path.as_ptr(), stat.as_mut_ptr()) != 0 {
            return None;
        }
        stat.assume_init()
    };

    let block = stat.f_frsize as u64;
    let total = (stat.f_blocks as u64).saturating_mul(block);
    let available = (stat.f_bavail as u64).saturating_mul(block);
    Some((total, available))
}
