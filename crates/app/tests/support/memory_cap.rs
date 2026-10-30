//! Ends a UI test binary that uses more memory than any window test has a reason to, before it can
//! take the machine with it. Tests run in parallel, so one careless helper multiplies: full-size
//! fake models once put about 20 GB behind this binary. Fake a model with `set_len`, never with
//! its bytes.

use std::io::{self, Write};
use std::sync::Once;
use std::thread;
use std::time::Duration;

const LOOK_EVERY: Duration = Duration::from_millis(50);

/// Starts watching. Later calls do nothing.
pub fn install(limit_mb: u64) {
    static WATCH: Once = Once::new();
    WATCH.call_once(|| {
        thread::spawn(move || {
            loop {
                let used = footprint_mb();
                if used > limit_mb {
                    // Not `eprintln!`: the test harness would hold that back, and abort loses it.
                    let _ = writeln!(
                        io::stderr(),
                        "this test binary uses {used} MB, over its limit of {limit_mb} MB"
                    );
                    std::process::abort();
                }
                thread::sleep(LOOK_EVERY);
            }
        });
    });
}

#[cfg(target_os = "macos")]
fn footprint_mb() -> u64 {
    #[repr(C)]
    #[derive(Default)]
    struct RusageInfoV0 {
        uuid: [u8; 16],
        user_time: u64,
        system_time: u64,
        pkg_idle_wkups: u64,
        interrupt_wkups: u64,
        pageins: u64,
        wired_size: u64,
        resident_size: u64,
        phys_footprint: u64,
        proc_start_abstime: u64,
        proc_exit_abstime: u64,
    }
    unsafe extern "C" {
        fn proc_pid_rusage(pid: i32, flavor: i32, buffer: *mut RusageInfoV0) -> i32;
    }
    let mut info = RusageInfoV0::default();
    unsafe { proc_pid_rusage(std::process::id() as i32, 0, &mut info) };
    info.phys_footprint / (1024 * 1024)
}

#[cfg(not(target_os = "macos"))]
fn footprint_mb() -> u64 {
    0
}
