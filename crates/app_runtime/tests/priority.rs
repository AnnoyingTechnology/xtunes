// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

//! Integration test for the Linux priority syscall wrappers in
//! `sustain_app_runtime::priority`. The unit tests in that module only
//! cover the numeric/typed conversions; this file actually invokes the
//! syscalls on a spawned thread to confirm the kernel accepts them and
//! that the resulting priority is observable through `getpriority` /
//! `ioprio_get`. Sustain is Linux-only, so the file is gated on the
//! target_os for clarity rather than to support non-Linux builds.

#![cfg(target_os = "linux")]
// The workspace denies `unsafe_code`. This test file invokes
// `getpriority` and the `ioprio_get` syscall to read back the values
// the wrapper just wrote — both are unavoidably unsafe FFI. Opting in
// here keeps the audited unsafe surface to one production module
// (`sustain_app_runtime::priority`) plus this test file, which only
// runs under `cargo test` and never ships in the binary.
#![allow(unsafe_code)]

use std::thread;

use sustain_app_runtime::priority::{
    IoPriorityClass, NiceLevel, apply_to_current_thread, priority_for, resolve_worker_count,
};
use sustain_domain::BackgroundResourceUsage;

fn current_nice() -> i32 {
    // SAFETY: thin libc call with no preconditions beyond the
    // documented argument values.
    unsafe { libc::getpriority(libc::PRIO_PROCESS, 0) }
}

fn current_ioprio() -> libc::c_int {
    const IOPRIO_WHO_PROCESS: libc::c_long = 1;
    // SAFETY: `SYS_ioprio_get` is the documented syscall number; the
    // arguments are C-integers, no pointers across the boundary.
    let rc = unsafe { libc::syscall(libc::SYS_ioprio_get, IOPRIO_WHO_PROCESS, 0 as libc::c_long) };
    rc as libc::c_int
}

#[test]
fn apply_to_current_thread_lowers_nice() {
    let handle = thread::spawn(|| {
        apply_to_current_thread(NiceLevel::Background, IoPriorityClass::BestEffortLow)
            .expect("Linux setpriority + ioprio_set must succeed for unprivileged self-nicing");
        // Asking for a nicer value than baseline is always allowed for
        // an unprivileged process, so the kernel honours +10 verbatim
        // when no CAP_SYS_NICE is required.
        assert_eq!(
            current_nice(),
            10,
            "Background preset should land at nice 10"
        );
    });
    handle.join().expect("thread join");
}

#[test]
fn idle_preset_lands_at_nice_19() {
    let handle = thread::spawn(|| {
        apply_to_current_thread(NiceLevel::Idle, IoPriorityClass::Idle)
            .expect("Idle preset must apply on Linux");
        assert_eq!(current_nice(), 19);
    });
    handle.join().expect("thread join");
}

#[test]
fn ioprio_class_round_trips_through_syscall() {
    // After applying the idle class, ioprio_get should report the
    // same class back. The data field is allowed to wobble (kernel
    // sometimes returns class-specific defaults for fields it does not
    // use), so we only assert on the high bits.
    let handle = thread::spawn(|| {
        apply_to_current_thread(NiceLevel::Idle, IoPriorityClass::Idle)
            .expect("ioprio_set must succeed");
        let raw = current_ioprio();
        const SHIFT: libc::c_int = 13;
        const CLASS_IDLE: libc::c_int = 3;
        assert_eq!(raw >> SHIFT, CLASS_IDLE);
    });
    handle.join().expect("thread join");
}

#[test]
fn priority_for_matches_preset() {
    assert_eq!(
        priority_for(BackgroundResourceUsage::Innocuous),
        (NiceLevel::Idle, IoPriorityClass::Idle)
    );
}

#[test]
fn resolve_worker_count_is_consistent_across_calls() {
    let a = resolve_worker_count(BackgroundResourceUsage::Balanced);
    let b = resolve_worker_count(BackgroundResourceUsage::Balanced);
    assert_eq!(
        a, b,
        "available_parallelism is stable for the duration of a process"
    );
}
