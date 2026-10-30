//! `detect()` on the machine running the tests: only sanity, since every machine differs.

use wf_models::hardware::{self, GpuBackend};

#[test]
fn detection_reports_a_plausible_machine() {
    let hw = hardware::detect();
    assert!(hw.total_ram_bytes >= 1 << 30, "{hw:?}");
    assert!(hw.free_ram_bytes <= hw.total_ram_bytes, "{hw:?}");
    assert!(hw.cpu_cores >= 1, "{hw:?}");
    if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        assert!(hw.cpu.neon, "{hw:?}");
        let gpu = hw
            .gpus
            .iter()
            .find(|g| g.backend == GpuBackend::Metal)
            .expect("Metal GPU");
        assert!(gpu.unified_memory && gpu.name.contains("Apple"), "{gpu:?}");
        assert_eq!(gpu.vram_bytes, Some(hw.total_ram_bytes));
    }
    // The rule must run without panicking whatever the machine.
    let _ = hw.recommended_tier();
}
