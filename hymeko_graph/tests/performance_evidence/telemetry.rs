//! Process-local telemetry used only by the isolated performance-evidence client.

/// Current and peak resident working-set observations for this process.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MemorySnapshot {
    pub working_set_bytes: usize,
    pub peak_working_set_bytes: usize,
}

#[cfg(windows)]
mod platform {
    use std::ffi::c_void;
    use std::mem::size_of;

    use super::MemorySnapshot;

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct FileTime {
        low: u32,
        high: u32,
    }

    impl FileTime {
        fn ticks_100ns(self) -> u64 {
            (u64::from(self.high) << 32) | u64::from(self.low)
        }
    }

    #[repr(C)]
    struct ProcessMemoryCounters {
        cb: u32,
        page_fault_count: u32,
        peak_working_set_size: usize,
        working_set_size: usize,
        quota_peak_paged_pool_usage: usize,
        quota_paged_pool_usage: usize,
        quota_peak_non_paged_pool_usage: usize,
        quota_non_paged_pool_usage: usize,
        pagefile_usage: usize,
        peak_pagefile_usage: usize,
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        #[link_name = "GetCurrentProcess"]
        fn get_current_process() -> *mut c_void;
        #[link_name = "GetProcessTimes"]
        fn get_process_times(
            process: *mut c_void,
            creation: *mut FileTime,
            exit: *mut FileTime,
            kernel: *mut FileTime,
            user: *mut FileTime,
        ) -> i32;
    }

    #[link(name = "psapi")]
    unsafe extern "system" {
        #[link_name = "GetProcessMemoryInfo"]
        fn get_process_memory_info(
            process: *mut c_void,
            counters: *mut ProcessMemoryCounters,
            size: u32,
        ) -> i32;
    }

    pub fn process_cpu_us() -> Option<f64> {
        let mut creation = FileTime::default();
        let mut exit = FileTime::default();
        let mut kernel = FileTime::default();
        let mut user = FileTime::default();
        // SAFETY: the pseudo-handle is valid in the current process, and every output points to a
        // correctly sized, writable `FILETIME`-compatible value for the duration of the call.
        let ok = unsafe {
            get_process_times(
                get_current_process(),
                &mut creation,
                &mut exit,
                &mut kernel,
                &mut user,
            )
        };
        (ok != 0).then(|| (kernel.ticks_100ns() + user.ticks_100ns()) as f64 / 10.0)
    }

    pub fn memory_snapshot() -> Option<MemorySnapshot> {
        let mut counters = ProcessMemoryCounters {
            cb: size_of::<ProcessMemoryCounters>() as u32,
            page_fault_count: 0,
            peak_working_set_size: 0,
            working_set_size: 0,
            quota_peak_paged_pool_usage: 0,
            quota_paged_pool_usage: 0,
            quota_peak_non_paged_pool_usage: 0,
            quota_non_paged_pool_usage: 0,
            pagefile_usage: 0,
            peak_pagefile_usage: 0,
        };
        // SAFETY: the pseudo-handle is valid and `counters` has the C layout/size declared in `cb`.
        let ok = unsafe {
            get_process_memory_info(
                get_current_process(),
                &mut counters,
                size_of::<ProcessMemoryCounters>() as u32,
            )
        };
        (ok != 0).then_some(MemorySnapshot {
            working_set_bytes: counters.working_set_size,
            peak_working_set_bytes: counters.peak_working_set_size,
        })
    }
}

#[cfg(not(windows))]
mod platform {
    use super::MemorySnapshot;

    pub fn process_cpu_us() -> Option<f64> {
        None
    }

    pub fn memory_snapshot() -> Option<MemorySnapshot> {
        None
    }
}

pub use platform::{memory_snapshot, process_cpu_us};
