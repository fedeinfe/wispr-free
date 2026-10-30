//! Spec §6: detected hardware → recommended pipeline tier. Hardware is a plain value, so any
//! machine can be described here without owning it.

use wf_models::Tier;
use wf_models::hardware::{CpuArch, CpuFeatures, Gpu, GpuBackend, HardwareInfo};

const GB: u64 = 1_000_000_000;
const GIB: u64 = 1 << 30;

fn x86_laptop(total_ram: u64) -> HardwareInfo {
    HardwareInfo {
        total_ram_bytes: total_ram,
        free_ram_bytes: total_ram / 2,
        cpu_cores: 8,
        cpu: CpuFeatures {
            arch: CpuArch::X86_64,
            avx2: true,
            fma: true,
            avx512: false,
            neon: false,
        },
        gpus: vec![],
    }
}

#[test]
fn an_8_gb_laptop_without_gpu_gets_lite() {
    assert_eq!(x86_laptop(8 * GIB).recommended_tier(), Tier::Lite);
}

#[test]
fn a_16_gb_laptop_gets_balanced_even_when_firmware_reserves_some_ram() {
    assert_eq!(x86_laptop(16 * GIB).recommended_tier(), Tier::Balanced);
    // Integrated graphics carve-outs: a "16 GB" machine reporting 15.4 GiB.
    assert_eq!(
        x86_laptop(15_400 * GIB / 1000).recommended_tier(),
        Tier::Balanced
    );
    assert_eq!(x86_laptop(16 * GB).recommended_tier(), Tier::Balanced);
}

fn apple_silicon(total_ram: u64) -> HardwareInfo {
    HardwareInfo {
        total_ram_bytes: total_ram,
        free_ram_bytes: total_ram / 2,
        cpu_cores: 10,
        cpu: CpuFeatures {
            arch: CpuArch::Aarch64,
            avx2: false,
            fma: false,
            avx512: false,
            neon: true,
        },
        gpus: vec![Gpu {
            backend: GpuBackend::Metal,
            name: "Apple M-series".into(),
            vram_bytes: Some(total_ram),
            unified_memory: true,
        }],
    }
}

fn with_discrete_gpu(mut hw: HardwareInfo, backend: GpuBackend, vram: u64) -> HardwareInfo {
    hw.gpus.push(Gpu {
        backend,
        name: "discrete".into(),
        vram_bytes: Some(vram),
        unified_memory: false,
    });
    hw
}

#[test]
fn apple_silicon_follows_unified_memory_size() {
    assert_eq!(apple_silicon(8 * GIB).recommended_tier(), Tier::Lite);
    assert_eq!(apple_silicon(16 * GIB).recommended_tier(), Tier::Balanced);
    assert_eq!(apple_silicon(36 * GIB).recommended_tier(), Tier::High);
}

#[test]
fn a_gpu_with_8_gb_vram_gets_high() {
    // nvidia-smi reports 8 GB cards as 8188 MiB.
    let rtx = with_discrete_gpu(x86_laptop(16 * GIB), GpuBackend::Cuda, 8188 << 20);
    assert_eq!(rtx.recommended_tier(), Tier::High);
    let radeon = with_discrete_gpu(x86_laptop(32 * GIB), GpuBackend::Vulkan, 12 * GIB);
    assert_eq!(radeon.recommended_tier(), Tier::High);
}

#[test]
fn small_or_unknown_vram_and_cpu_only_workstations_stay_balanced() {
    let small = with_discrete_gpu(x86_laptop(16 * GIB), GpuBackend::Vulkan, 4 * GIB);
    assert_eq!(small.recommended_tier(), Tier::Balanced);
    let mut unknown = with_discrete_gpu(x86_laptop(32 * GIB), GpuBackend::Vulkan, 0);
    unknown.gpus[0].vram_bytes = None;
    assert_eq!(unknown.recommended_tier(), Tier::Balanced);
    // Whisper-large-v3-turbo can't meet High's 1 s target on CPU alone.
    assert_eq!(x86_laptop(64 * GIB).recommended_tier(), Tier::Balanced);
}

#[test]
fn a_big_gpu_cannot_lift_an_8_gb_machine_out_of_lite() {
    let hw = with_discrete_gpu(x86_laptop(8 * GIB), GpuBackend::Cuda, 8 * GIB);
    assert_eq!(hw.recommended_tier(), Tier::Lite);
}

#[test]
fn an_older_cpu_gets_lite_whatever_the_ram() {
    let mut no_avx2 = with_discrete_gpu(x86_laptop(32 * GIB), GpuBackend::Cuda, 8 * GIB);
    no_avx2.cpu.avx2 = false;
    assert_eq!(no_avx2.recommended_tier(), Tier::Lite);

    let mut dual_core = x86_laptop(16 * GIB);
    dual_core.cpu_cores = 2;
    assert_eq!(dual_core.recommended_tier(), Tier::Lite);
}
