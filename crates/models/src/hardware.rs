//! What the machine can run. [`HardwareInfo`] is a plain value so the tier logic can be tested
//! with any machine.

use std::process::{Command, Output};

use sysinfo::{CpuRefreshKind, System};

use crate::Tier;

#[derive(Debug, Clone, PartialEq)]
pub struct HardwareInfo {
    pub total_ram_bytes: u64,
    pub free_ram_bytes: u64,
    /// Logical cores.
    pub cpu_cores: usize,
    pub cpu: CpuFeatures,
    pub gpus: Vec<Gpu>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CpuArch {
    X86_64,
    Aarch64,
    Other,
}

/// The SIMD features ggml and onnxruntime care about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CpuFeatures {
    pub arch: CpuArch,
    pub avx2: bool,
    pub fma: bool,
    pub avx512: bool,
    pub neon: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpuBackend {
    Metal,
    Cuda,
    Vulkan,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Gpu {
    pub backend: GpuBackend,
    pub name: String,
    /// Dedicated video memory; `None` when unknown. Unified-memory GPUs report system RAM here.
    pub vram_bytes: Option<u64>,
    /// Apple Silicon and other GPUs that share system RAM.
    pub unified_memory: bool,
}

const GIB: u64 = 1 << 30;

/// Thresholds sit below the nominal sizes because firmware and integrated GPUs reserve some RAM:
/// a "16 GB" laptop can report 15.4 GiB.
const BALANCED_MIN_RAM: u64 = 14 * GIB;
/// Apple Silicon "32 GB+" (an M-Pro with 36 GB qualifies, an 18 GB one doesn't).
const HIGH_MIN_UNIFIED_RAM: u64 = 28 * GIB;
/// "≥ 8 GB VRAM"; nvidia-smi reports 8 GB cards as 8188 MiB.
const HIGH_MIN_VRAM: u64 = 7_680 << 20;

impl HardwareInfo {
    /// Spec §6 tier table.
    pub fn recommended_tier(&self) -> Tier {
        // "Older CPU": x86 without AVX2 (pre-2013) runs ggml several times slower.
        let old_cpu = (self.cpu.arch == CpuArch::X86_64 && !self.cpu.avx2) || self.cpu_cores < 4;
        if old_cpu || self.total_ram_bytes < BALANCED_MIN_RAM {
            return Tier::Lite;
        }
        // High needs GPU acceleration: Whisper-large-v3-turbo can't meet the 1 s target on CPU.
        let high_gpu = self.gpus.iter().any(|gpu| {
            if gpu.unified_memory {
                gpu.backend == GpuBackend::Metal && self.total_ram_bytes >= HIGH_MIN_UNIFIED_RAM
            } else {
                gpu.vram_bytes.is_some_and(|vram| vram >= HIGH_MIN_VRAM)
            }
        });
        if high_gpu { Tier::High } else { Tier::Balanced }
    }
}

/// Probes this machine. Spawns `nvidia-smi` or `reg`, so call it from a worker thread. Anything
/// that can't be read is left out, which at worst recommends a lower tier.
pub fn detect() -> HardwareInfo {
    let (total_ram, free_ram, cpu_brand) = match System::new() {
        Ok(mut system) => {
            system.refresh_memory();
            system.refresh_cpu_list(CpuRefreshKind::nothing());
            let brand = system
                .cpus()
                .first()
                .map(|c| c.brand().to_owned())
                .unwrap_or_default();
            (system.total_memory(), system.available_memory(), brand)
        }
        Err(_) => (0, 0, String::new()),
    };
    HardwareInfo {
        total_ram_bytes: total_ram,
        free_ram_bytes: free_ram,
        cpu_cores: std::thread::available_parallelism().map_or(1, |n| n.get()),
        cpu: detect_cpu(),
        gpus: detect_gpus(&cpu_brand, total_ram),
    }
}

fn detect_cpu() -> CpuFeatures {
    #[cfg(target_arch = "x86_64")]
    return CpuFeatures {
        arch: CpuArch::X86_64,
        avx2: std::arch::is_x86_feature_detected!("avx2"),
        fma: std::arch::is_x86_feature_detected!("fma"),
        avx512: std::arch::is_x86_feature_detected!("avx512f"),
        neon: false,
    };
    #[cfg(target_arch = "aarch64")]
    return CpuFeatures {
        arch: CpuArch::Aarch64,
        avx2: false,
        fma: false,
        avx512: false,
        neon: std::arch::is_aarch64_feature_detected!("neon"),
    };
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    CpuFeatures {
        arch: CpuArch::Other,
        avx2: false,
        fma: false,
        avx512: false,
        neon: false,
    }
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
fn detect_gpus(cpu_brand: &str, total_ram: u64) -> Vec<Gpu> {
    vec![Gpu {
        backend: GpuBackend::Metal,
        name: cpu_brand.to_owned(),
        vram_bytes: Some(total_ram),
        unified_memory: true,
    }]
}

/// Intel Macs run on the CPU tiers: ggml's Metal backend isn't worth it on their GPUs.
#[cfg(all(target_os = "macos", not(target_arch = "aarch64")))]
fn detect_gpus(_cpu_brand: &str, _total_ram: u64) -> Vec<Gpu> {
    vec![]
}

#[cfg(target_os = "linux")]
fn detect_gpus(_cpu_brand: &str, _total_ram: u64) -> Vec<Gpu> {
    let mut gpus = nvidia_gpus();
    gpus.extend(amd_sysfs_gpus());
    gpus
}

#[cfg(windows)]
fn detect_gpus(_cpu_brand: &str, _total_ram: u64) -> Vec<Gpu> {
    let mut gpus = nvidia_gpus();
    let key = r"HKLM\SYSTEM\CurrentControlSet\Control\Class\{4d36e968-e325-11ce-bfc1-08002be10318}";
    if let Some(output) = run(
        "reg",
        &["query", key, "/s", "/f", "HardwareInformation.qwMemorySize"],
    ) {
        let has_cuda = !gpus.is_empty();
        let others = parse_reg_adapters(&output, |key| {
            run("reg", &["query", key, "/v", "DriverDesc"])
        });
        gpus.extend(
            others
                .into_iter()
                .filter(|g| !(has_cuda && g.name.contains("NVIDIA"))),
        );
    }
    gpus
}

#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
fn detect_gpus(_cpu_brand: &str, _total_ram: u64) -> Vec<Gpu> {
    vec![]
}

#[cfg(any(target_os = "linux", windows))]
fn nvidia_gpus() -> Vec<Gpu> {
    let args = [
        "--query-gpu=name,memory.total",
        "--format=csv,noheader,nounits",
    ];
    run("nvidia-smi", &args)
        .map(|out| parse_nvidia_smi(&out))
        .unwrap_or_default()
}

/// Radeon cards expose VRAM in sysfs; Mesa gives them Vulkan.
#[cfg(target_os = "linux")]
fn amd_sysfs_gpus() -> Vec<Gpu> {
    let Ok(cards) = std::fs::read_dir("/sys/class/drm") else {
        return vec![];
    };
    cards
        .flatten()
        .filter(|card| {
            let name = card.file_name();
            let name = name.to_string_lossy();
            name.starts_with("card") && !name.contains('-')
        })
        .filter_map(|card| {
            let device = card.path().join("device");
            let vendor = std::fs::read_to_string(device.join("vendor")).ok()?;
            let vram = std::fs::read_to_string(device.join("mem_info_vram_total")).ok()?;
            (vendor.trim() == "0x1002").then(|| Gpu {
                backend: GpuBackend::Vulkan,
                name: "AMD Radeon".into(),
                vram_bytes: vram.trim().parse().ok(),
                unified_memory: false,
            })
        })
        .collect()
}

/// `nvidia-smi` CSV: `NVIDIA GeForce RTX 3070, 8192` per GPU, memory in MiB.
#[cfg_attr(not(any(target_os = "linux", windows, test)), allow(dead_code))]
fn parse_nvidia_smi(output: &str) -> Vec<Gpu> {
    output
        .lines()
        .filter_map(|line| {
            let (name, mib) = line.rsplit_once(',')?;
            Some(Gpu {
                backend: GpuBackend::Cuda,
                name: name.trim().to_owned(),
                vram_bytes: Some(mib.trim().parse::<u64>().ok()? << 20),
                unified_memory: false,
            })
        })
        .collect()
}

/// `reg query ... /s /f HardwareInformation.qwMemorySize` prints each adapter's key followed by
/// an indented `HardwareInformation.qwMemorySize  REG_QWORD  0x200000000` line. The adapter's
/// name comes from a second query, passed in as `describe`.
#[cfg_attr(not(any(windows, test)), allow(dead_code))]
fn parse_reg_adapters(output: &str, describe: impl Fn(&str) -> Option<String>) -> Vec<Gpu> {
    let mut gpus = vec![];
    let mut key = "";
    for line in output.lines() {
        if line.starts_with("HKEY_") {
            key = line.trim();
        }
        let Some(rest) = line.trim().strip_prefix("HardwareInformation.qwMemorySize") else {
            continue;
        };
        let Some(hex) = rest
            .split_whitespace()
            .last()
            .and_then(|v| v.strip_prefix("0x"))
        else {
            continue;
        };
        let name = describe(key).and_then(|out| {
            let line = out
                .lines()
                .find(|l| l.trim().starts_with("DriverDesc"))?
                .to_owned();
            Some(line.split("REG_SZ").nth(1)?.trim().to_owned())
        });
        gpus.push(Gpu {
            backend: GpuBackend::Vulkan,
            name: name.unwrap_or_else(|| "GPU".into()),
            vram_bytes: u64::from_str_radix(hex, 16).ok(),
            unified_memory: false,
        });
    }
    gpus
}

/// Runs a probe tool; `None` when it's missing or fails. No console window on Windows.
#[cfg_attr(not(any(target_os = "linux", windows)), allow(dead_code))]
fn run(program: &str, args: &[&str]) -> Option<String> {
    let mut command = Command::new(program);
    command.args(args);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let Output { status, stdout, .. } = command.output().ok()?;
    status
        .success()
        .then(|| String::from_utf8_lossy(&stdout).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    const GIB: u64 = 1 << 30;

    #[test]
    fn nvidia_smi_lines_become_cuda_gpus() {
        let gpus = parse_nvidia_smi("NVIDIA GeForce RTX 3070, 8192\nNVIDIA GeForce GT 710, 2048\n");
        assert_eq!(gpus.len(), 2);
        assert_eq!(gpus[0].backend, GpuBackend::Cuda);
        assert_eq!(gpus[0].name, "NVIDIA GeForce RTX 3070");
        assert_eq!(gpus[0].vram_bytes, Some(8 * GIB));
        assert_eq!(gpus[1].vram_bytes, Some(2 * GIB));
        assert!(parse_nvidia_smi("").is_empty());
    }

    #[test]
    fn registry_adapters_become_vulkan_gpus_with_64_bit_vram() {
        let output = "\r
HKEY_LOCAL_MACHINE\\SYSTEM\\CurrentControlSet\\Control\\Class\\{4d36e968-e325-11ce-bfc1-08002be10318}\\0000\r
    HardwareInformation.qwMemorySize    REG_QWORD    0x400000000\r
\r
End of search: 1 match(es) found.\r
";
        let gpus = parse_reg_adapters(output, |key| {
            assert!(key.ends_with("\\0000"), "{key}");
            Some("    DriverDesc    REG_SZ    AMD Radeon RX 6800\r\n".to_owned())
        });
        assert_eq!(gpus.len(), 1);
        assert_eq!(gpus[0].backend, GpuBackend::Vulkan);
        assert_eq!(gpus[0].name, "AMD Radeon RX 6800");
        assert_eq!(gpus[0].vram_bytes, Some(16 * GIB));
    }
}
