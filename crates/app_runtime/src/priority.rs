// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

//! Safe Rust wrappers around the two Linux scheduling-priority syscalls
//! the background-job scheduler reaches for: `setpriority` (CPU nice
//! value) and `ioprio_set` (block-device I/O priority).
//!
//! Why a dedicated module:
//!
//! * `ioprio_set` has no userland wrapper in glibc, `nix`, or `rustix`
//!   — every caller has to reach it through `libc::syscall`. Once `libc`
//!   is a required dependency for that, `setpriority` lives in the same
//!   place rather than pulling in a second crate to wrap one function.
//! * The workspace bans `unsafe_code` everywhere else; the audited
//!   `#![allow(unsafe_code)]` at the top of this file is the only
//!   exception, and it covers exactly three `unsafe { libc::… }` calls.
//!   The safe public API takes typed enums so callers cannot pass an
//!   out-of-range nice value or an invalid I/O priority class.
//! * Niceness and ionice are per-thread on Linux: the scheduler spawns
//!   N worker threads, each calls [`apply_to_current_thread`] once at
//!   the top of its loop, and the kernel never sees a thread that
//!   forgot to lower itself. There is no "set priority for the whole
//!   process" code path.
//!
//! Resolution of `BackgroundResourceUsage` to a `(NiceLevel,
//! IoPriorityClass)` pair lives in [`priority_for`], the single
//! authority the rest of the runtime should consult.

#![allow(unsafe_code)]

use std::io;

use sustain_domain::BackgroundResourceUsage;

/// Discrete CPU-scheduling stops the background scheduler uses. The
/// variants map onto a small, hand-picked subset of the kernel's
/// -20..=19 nice range — we deliberately do not expose a full
/// `NiceLevel(i8)` newtype, because the scheduler only ever needs
/// three stops and a typed enum makes the call sites self-explaining.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NiceLevel {
    /// Standard scheduling priority (nice value 0). Used by the
    /// `Aggressive` preset.
    Default,
    /// Moderately deprioritised (nice value 10). Used by the
    /// `Balanced` preset so background workers yield to playback and
    /// UI threads but still make solid progress.
    Background,
    /// Maximally deprioritised (nice value 19). Used by the
    /// `Innocuous` preset; the worker only runs when nothing else
    /// wants the CPU.
    Idle,
}

impl NiceLevel {
    /// Numeric nice value passed to `setpriority`. Range is -20..=19;
    /// every variant here is in 0..=19 (we never request *more* than
    /// default priority).
    fn as_value(self) -> libc::c_int {
        match self {
            Self::Default => 0,
            Self::Background => 10,
            Self::Idle => 19,
        }
    }
}

/// Block-device I/O scheduling class understood by Linux's
/// `ioprio_set`. We expose the three values the scheduler actually
/// uses; the kernel's `RealTime` class is intentionally unreachable.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IoPriorityClass {
    /// Kernel default best-effort class with default data (4). Used by
    /// the `Aggressive` preset — equivalent to not calling
    /// `ioprio_set` at all but kept explicit so a worker that ran
    /// under a different preset earlier resets to default.
    BestEffort,
    /// Best-effort class at the high data value (7). Used by the
    /// `Balanced` preset: still serviced by the disk scheduler but
    /// behind every default-priority caller.
    BestEffortLow,
    /// Idle class: the disk scheduler only services this thread when
    /// the device has nothing else to do. Used by the `Innocuous`
    /// preset.
    Idle,
}

impl IoPriorityClass {
    /// Pack the class + data fields into the single `int` that
    /// `ioprio_set` expects.
    fn as_ioprio(self) -> libc::c_int {
        // <linux/ioprio.h>: IOPRIO_CLASS_SHIFT = 13, and the priority
        // value is (class << 13) | (data & 0x1fff).
        const SHIFT: libc::c_int = 13;
        const CLASS_BE: libc::c_int = 2;
        const CLASS_IDLE: libc::c_int = 3;
        match self {
            // Best-effort, default data (4 — same value the kernel
            // assigns to a process inheriting nothing).
            Self::BestEffort => (CLASS_BE << SHIFT) | 4,
            // Best-effort, lowest priority within the class (7).
            Self::BestEffortLow => (CLASS_BE << SHIFT) | 7,
            // Idle class ignores the data field but we pass 0 for
            // clarity.
            Self::Idle => CLASS_IDLE << SHIFT,
        }
    }
}

/// Failures from the priority syscalls. Carries the `errno` value as a
/// [`std::io::Error`] so callers can decide whether to log-and-continue
/// or escalate.
#[derive(Debug)]
pub enum PriorityError {
    /// `setpriority` returned -1.
    NiceFailed(io::Error),
    /// `ioprio_set` returned -1.
    IoFailed(io::Error),
}

impl std::fmt::Display for PriorityError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NiceFailed(error) => write!(f, "setpriority failed: {error}"),
            Self::IoFailed(error) => write!(f, "ioprio_set failed: {error}"),
        }
    }
}

impl std::error::Error for PriorityError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::NiceFailed(error) | Self::IoFailed(error) => Some(error),
        }
    }
}

/// Apply both the nice value and the I/O priority class to the calling
/// thread. Both syscalls are independent and report errors separately;
/// if the nice call fails we still attempt the I/O call (and report
/// the nice failure), because partially-lowered is better than
/// not-lowered-at-all when the scheduler is trying to keep playback
/// responsive.
pub fn apply_to_current_thread(
    nice: NiceLevel,
    io_class: IoPriorityClass,
) -> Result<(), PriorityError> {
    let nice_outcome = set_thread_nice(nice);
    let io_outcome = set_thread_io_priority(io_class);
    nice_outcome?;
    io_outcome
}

/// Resolve a [`BackgroundResourceUsage`] preset to its `(nice, io)`
/// pair. Single authority — every caller that needs to know what
/// "Balanced" means in scheduling terms should go through here so the
/// preset can be tuned without scattering magic numbers.
pub fn priority_for(usage: BackgroundResourceUsage) -> (NiceLevel, IoPriorityClass) {
    match usage {
        BackgroundResourceUsage::Innocuous => (NiceLevel::Idle, IoPriorityClass::Idle),
        BackgroundResourceUsage::Balanced => {
            (NiceLevel::Background, IoPriorityClass::BestEffortLow)
        }
        BackgroundResourceUsage::Aggressive => (NiceLevel::Default, IoPriorityClass::BestEffort),
    }
}

/// Number of worker threads to spawn for the given preset on this
/// machine. Wraps [`std::thread::available_parallelism`] so callers do
/// not have to handle the fallback; a kernel that cannot report the
/// CPU count (extremely rare on Linux) collapses to a single worker
/// rather than crashing the scheduler.
pub fn resolve_worker_count(usage: BackgroundResourceUsage) -> usize {
    let cores = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);
    usage.worker_count(cores)
}

fn set_thread_nice(nice: NiceLevel) -> Result<(), PriorityError> {
    // PRIO_PROCESS with `who = 0` targets the calling thread on Linux
    // (each task has its own task_struct.nice), exactly what the
    // scheduler wants: each worker lowers itself.
    let value = nice.as_value();
    // SAFETY: `setpriority` is a libc function with no preconditions
    // beyond well-defined argument values. `PRIO_PROCESS` is the
    // constant the libc crate vends; `0` for `who` is the documented
    // self-targeting value; `value` is in the kernel's -20..=19 range
    // by construction of `NiceLevel::as_value`. Unlike `getpriority`,
    // `setpriority` returns 0 on success and -1 on error — there is no
    // ambiguous "-1 is a valid result" case to disambiguate via errno.
    let rc = unsafe { libc::setpriority(libc::PRIO_PROCESS, 0, value) };
    if rc == -1 {
        return Err(PriorityError::NiceFailed(io::Error::last_os_error()));
    }
    Ok(())
}

fn set_thread_io_priority(class: IoPriorityClass) -> Result<(), PriorityError> {
    // `ioprio_set(which, who, ioprio)`:
    //   which = IOPRIO_WHO_PROCESS (1) targets a TID when who != 0,
    //   the calling thread when who == 0.
    const IOPRIO_WHO_PROCESS: libc::c_int = 1;
    let ioprio = class.as_ioprio();
    // SAFETY: `SYS_ioprio_set` is the documented syscall number on
    // every Linux ABI we ship to (we do not target any other
    // kernel). The three arguments below are all C-compatible
    // integers; no pointers are passed across the boundary.
    let rc = unsafe {
        libc::syscall(
            libc::SYS_ioprio_set,
            IOPRIO_WHO_PROCESS as libc::c_long,
            0 as libc::c_long,
            ioprio as libc::c_long,
        )
    };
    if rc == -1 {
        return Err(PriorityError::IoFailed(io::Error::last_os_error()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        BackgroundResourceUsage, IoPriorityClass, NiceLevel, priority_for, resolve_worker_count,
    };

    #[test]
    fn nice_values_cover_the_three_presets() {
        // Locked-in numeric values: changing these would silently
        // alter the scheduler's behavior on every shipping install.
        assert_eq!(NiceLevel::Default.as_value(), 0);
        assert_eq!(NiceLevel::Background.as_value(), 10);
        assert_eq!(NiceLevel::Idle.as_value(), 19);
    }

    #[test]
    fn ioprio_packs_class_and_data_correctly() {
        // CLASS_BE (2) << 13 | 4 = 16388
        assert_eq!(IoPriorityClass::BestEffort.as_ioprio(), (2 << 13) | 4);
        // CLASS_BE (2) << 13 | 7 = 16391
        assert_eq!(IoPriorityClass::BestEffortLow.as_ioprio(), (2 << 13) | 7);
        // CLASS_IDLE (3) << 13 = 24576
        assert_eq!(IoPriorityClass::Idle.as_ioprio(), 3 << 13);
    }

    #[test]
    fn priority_presets_map_correctly() {
        assert_eq!(
            priority_for(BackgroundResourceUsage::Innocuous),
            (NiceLevel::Idle, IoPriorityClass::Idle)
        );
        assert_eq!(
            priority_for(BackgroundResourceUsage::Balanced),
            (NiceLevel::Background, IoPriorityClass::BestEffortLow)
        );
        assert_eq!(
            priority_for(BackgroundResourceUsage::Aggressive),
            (NiceLevel::Default, IoPriorityClass::BestEffort)
        );
    }

    #[test]
    fn resolve_worker_count_uses_available_parallelism() {
        // `available_parallelism` is documented to never return 0,
        // and at minimum we want one worker; we cannot assert an exact
        // value (test runner doesn't know the machine), only that the
        // helper produces something sensible for every preset.
        let innocuous = resolve_worker_count(BackgroundResourceUsage::Innocuous);
        let balanced = resolve_worker_count(BackgroundResourceUsage::Balanced);
        let aggressive = resolve_worker_count(BackgroundResourceUsage::Aggressive);

        assert_eq!(innocuous, 1, "Innocuous always spawns exactly one worker");
        assert!(balanced >= 1, "Balanced must spawn at least one worker");
        assert!(
            aggressive >= balanced,
            "Aggressive ({aggressive}) must be >= Balanced ({balanced})",
        );
    }
}
