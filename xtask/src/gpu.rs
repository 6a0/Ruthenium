//! Which GPU `cargo xtask bench` runs the game on, and what it samples from
//! that GPU while the game runs (Linux).
//!
//! On a hybrid laptop both GPUs are visible, and OpenGL and Vulkan pick one
//! each in their own way. [`Gpu::env`] pins both to the same GPU through the
//! GL vendor libraries (GLVND), Mesa's `DRI_PRIME` and the Vulkan loader's
//! driver filter. The bench mod reports the device the game actually used,
//! and [`Gpu::matches`] checks it, so a run never lands in the wrong table.
//!
//! - NVIDIA: memory, temperature, clock and throttle reasons from `nvidia-smi`.
//! - Intel (i915 or xe): GPU memory from the kernel's per-client DRM stats in
//!   `/proc/<pid>/fdinfo`, the actual clock and throttle reasons from sysfs.
//!   An integrated GPU has no temperature of its own; it shares the CPU
//!   package sensor.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gpu {
    /// Whatever the game picks with no environment set (how the first
    /// baseline was measured, with the laptop in discrete-GPU mode).
    Auto,
    Nvidia,
    Intel,
}

pub const CHOICES: &str = "auto, nvidia, intel";

impl Gpu {
    pub fn parse(s: &str) -> Result<Self, String> {
        match s {
            "auto" => Ok(Self::Auto),
            "nvidia" => Ok(Self::Nvidia),
            "intel" => Ok(Self::Intel),
            other => Err(format!("unknown GPU {other:?} ({CHOICES})")),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Nvidia => "nvidia",
            Self::Intel => "intel",
        }
    }

    /// Environment for the game: `Some` sets a variable, `None` removes it.
    pub fn env(self) -> Result<Vec<(String, Option<String>)>, String> {
        let set = |k: &str, v: &str| (k.to_owned(), Some(v.to_owned()));
        let unset = |k: &str| (k.to_owned(), None);
        Ok(match self {
            Self::Auto => Vec::new(),
            Self::Nvidia => vec![
                set("__NV_PRIME_RENDER_OFFLOAD", "1"),
                set("__VK_LAYER_NV_optimus", "NVIDIA_only"),
                set("__GLX_VENDOR_LIBRARY_NAME", "nvidia"),
                set(
                    "__EGL_VENDOR_LIBRARY_FILENAMES",
                    &egl_vendor_file("nvidia")?,
                ),
                set("VK_LOADER_DRIVERS_SELECT", "*nvidia*"),
                unset("DRI_PRIME"),
            ],
            Self::Intel => {
                let card = intel_card().ok_or(
                    "no Intel GPU is visible; on a laptop with a GPU switch, set it to hybrid mode",
                )?;
                vec![
                    unset("__NV_PRIME_RENDER_OFFLOAD"),
                    unset("__VK_LAYER_NV_optimus"),
                    set("__GLX_VENDOR_LIBRARY_NAME", "mesa"),
                    set("__EGL_VENDOR_LIBRARY_FILENAMES", &egl_vendor_file("mesa")?),
                    set("DRI_PRIME", &dri_prime_tag(&card.pdev)),
                    set("VK_LOADER_DRIVERS_SELECT", "*intel*"),
                ]
            }
        })
    }

    /// Whether the device the game reported (vendor and name, as the bench
    /// mod records them) is this GPU.
    pub fn matches(self, vendor: &str, name: &str) -> bool {
        let text = format!("{vendor} {name}").to_lowercase();
        match self {
            Self::Auto => true,
            Self::Nvidia => text.contains("nvidia"),
            Self::Intel => text.contains("intel"),
        }
    }

    pub fn sample(self, pid: u32) -> Sample {
        match self {
            Self::Intel => intel_card().map_or_else(Sample::default, |card| Sample {
                temp_c: None,
                reasons: Some(card.reasons()),
                clock_mhz: card.clock_mhz(),
                mem_mib: drm_client_mib(pid, Some(&card.pdev)),
            }),
            Self::Nvidia | Self::Auto => {
                let (temp_c, reasons, clock_mhz) = nvidia_state().unwrap_or_default();
                let mem_mib = nvidia_mem_mib(pid);
                let mem_mib = match self {
                    Self::Auto => mem_mib.or_else(|| drm_client_mib(pid, None)),
                    _ => mem_mib,
                };
                Sample {
                    temp_c,
                    reasons,
                    clock_mhz,
                    mem_mib,
                }
            }
        }
    }
}

/// One sample of the GPU the game runs on.
#[derive(Debug, Default)]
pub struct Sample {
    pub temp_c: Option<f64>,
    /// NVIDIA: the throttle-reason bit mask in hex (`0x...`). Intel:
    /// `intel:` and the active throttle reasons, comma-separated.
    pub reasons: Option<String>,
    pub clock_mhz: Option<f64>,
    pub mem_mib: Option<f64>,
}

/// Thermal and hardware slowdown bits of NVIDIA's throttle reasons (not
/// idle, not the normal software power cap).
const NVIDIA_SLOWDOWN_BITS: u64 = 0x08 | 0x20 | 0x40 | 0x80;

/// Intel's throttle reasons that mean a thermal or hardware slowdown. The
/// power limits (pl1, pl2, pl4) are the normal cap, like NVIDIA's software
/// power cap, and are left out.
const INTEL_SLOWDOWN: &[&str] = &["thermal", "prochot", "ratl", "vr_thermalert", "vr_tdc"];
const INTEL_REASONS: &[&str] = &[
    "pl1",
    "pl2",
    "pl4",
    "thermal",
    "prochot",
    "ratl",
    "vr_thermalert",
    "vr_tdc",
];

/// Whether a recorded reasons value means a thermal or hardware slowdown.
pub fn is_slowdown(reasons: &str) -> Option<bool> {
    if let Some(list) = reasons.strip_prefix("intel:") {
        return Some(list.split(',').any(|r| INTEL_SLOWDOWN.contains(&r)));
    }
    let bits = u64::from_str_radix(reasons.strip_prefix("0x")?, 16).ok()?;
    Some(bits & NVIDIA_SLOWDOWN_BITS != 0)
}

#[allow(clippy::type_complexity)]
fn nvidia_state() -> Option<(Option<f64>, Option<String>, Option<f64>)> {
    let out = Command::new("nvidia-smi")
        .args([
            "--query-gpu=temperature.gpu,clocks_event_reasons.active,clocks.gr",
            "--format=csv,noheader,nounits",
        ])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let mut f = text.lines().next()?.split(',').map(str::trim);
    Some((
        f.next().and_then(|v| v.parse().ok()),
        f.next().map(str::to_owned),
        f.next().and_then(|v| v.parse().ok()),
    ))
}

/// GPU memory of one process from `nvidia-smi`'s process table.
fn nvidia_mem_mib(pid: u32) -> Option<f64> {
    let out = Command::new("nvidia-smi").output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let pid = pid.to_string();
    let line = text
        .lines()
        .find(|l| l.split_whitespace().nth(4) == Some(pid.as_str()))?;
    let mib = line.split_whitespace().rev().nth(1)?;
    mib.strip_suffix("MiB")?.parse().ok()
}

/// One DRM client, from one `fdinfo` file (several file descriptors can
/// share a client).
#[derive(Debug, Default, PartialEq)]
struct DrmClient {
    pdev: Option<String>,
    id: String,
    resident_kib: Option<u64>,
    total_kib: Option<u64>,
}

/// Parses the kernel's DRM usage stats (Documentation/gpu/drm-usage-stats.rst):
/// `drm-resident-<region>` and `drm-total-<region>`, in bytes, KiB or MiB.
fn parse_fdinfo(text: &str) -> Option<DrmClient> {
    let mut c = DrmClient::default();
    let mut seen = false;
    for line in text.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim();
        match key {
            "drm-client-id" => {
                c.id = value.to_owned();
                seen = true;
            }
            "drm-pdev" => c.pdev = Some(value.to_owned()),
            _ => {
                let slot = if key.starts_with("drm-resident-") {
                    &mut c.resident_kib
                } else if key.starts_with("drm-total-") {
                    &mut c.total_kib
                } else {
                    continue;
                };
                let mut parts = value.split_whitespace();
                let Some(n) = parts.next().and_then(|n| n.parse::<u64>().ok()) else {
                    continue;
                };
                let kib = match parts.next() {
                    Some("KiB") => n,
                    Some("MiB") => n * 1024,
                    _ => n / 1024,
                };
                *slot = Some(slot.unwrap_or(0) + kib);
            }
        }
    }
    seen.then_some(c)
}

/// The GPU memory of a process's DRM clients on one device (`pdev`, e.g.
/// `0000:00:02.0`) or on all of them: resident memory where the driver
/// reports it, else all allocated memory.
fn drm_client_mib(pid: u32, pdev: Option<&str>) -> Option<f64> {
    let dir = PathBuf::from(format!("/proc/{pid}/fdinfo"));
    let mut clients: HashMap<(Option<String>, String), DrmClient> = HashMap::new();
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let Ok(text) = std::fs::read_to_string(entry.path()) else {
            continue;
        };
        if let Some(c) = parse_fdinfo(&text)
            && pdev.is_none_or(|p| c.pdev.as_deref() == Some(p))
        {
            clients.insert((c.pdev.clone(), c.id.clone()), c);
        }
    }
    sum_clients(clients.values())
}

fn sum_clients<'a>(clients: impl Iterator<Item = &'a DrmClient> + Clone) -> Option<f64> {
    let resident: Vec<u64> = clients.clone().filter_map(|c| c.resident_kib).collect();
    let kib: Vec<u64> = if resident.is_empty() {
        clients.filter_map(|c| c.total_kib).collect()
    } else {
        resident
    };
    (!kib.is_empty()).then(|| kib.iter().sum::<u64>() as f64 / 1024.0)
}

/// An Intel GPU's DRM card in sysfs.
#[derive(Debug)]
pub struct IntelCard {
    path: PathBuf,
    /// PCI address, e.g. `0000:00:02.0`.
    pub pdev: String,
    /// `i915` or `xe`.
    pub driver: String,
}

pub fn intel_card() -> Option<IntelCard> {
    drm_cards()
        .into_iter()
        .find(|c| c.vendor == "0x8086")
        .map(|c| IntelCard {
            path: c.path,
            pdev: c.pdev,
            driver: c.driver,
        })
}

#[derive(Debug)]
pub struct DrmCard {
    path: PathBuf,
    pub pdev: String,
    pub vendor: String,
    pub device: String,
    pub driver: String,
}

/// The DRM cards (`/sys/class/drm/cardN`, not their connectors).
pub fn drm_cards() -> Vec<DrmCard> {
    let Ok(entries) = std::fs::read_dir("/sys/class/drm") else {
        return Vec::new();
    };
    let mut cards: Vec<DrmCard> = entries
        .flatten()
        .filter(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            name.strip_prefix("card")
                .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
        })
        .filter_map(|e| {
            let path = e.path();
            let device = path.join("device");
            let base = |p: PathBuf| {
                std::fs::read_link(p)
                    .ok()?
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
            };
            Some(DrmCard {
                pdev: base(device.clone())?,
                vendor: read(&device.join("vendor"))?,
                device: read(&device.join("device")).unwrap_or_default(),
                driver: base(device.join("driver")).unwrap_or_default(),
                path,
            })
        })
        .collect();
    cards.sort_by(|a, b| a.path.cmp(&b.path));
    cards
}

impl IntelCard {
    fn clock_mhz(&self) -> Option<f64> {
        let file = match self.driver.as_str() {
            "xe" => self.path.join("device/tile0/gt0/freqs/act_freq"),
            _ => self.path.join("gt_act_freq_mhz"),
        };
        read(&file)?.parse().ok()
    }

    /// `intel:` and the active throttle reasons.
    fn reasons(&self) -> String {
        let (dir, prefix) = match self.driver.as_str() {
            "xe" => (self.path.join("device/tile0/gt0/freqs/throttle"), "reason_"),
            _ => (self.path.join("gt/gt0"), "throttle_reason_"),
        };
        let active: Vec<&str> = INTEL_REASONS
            .iter()
            .copied()
            .filter(|r| read(&dir.join(format!("{prefix}{r}"))).as_deref() == Some("1"))
            .collect();
        format!("intel:{}", active.join(","))
    }
}

fn read(path: &Path) -> Option<String> {
    std::fs::read_to_string(path)
        .ok()
        .map(|s| s.trim().to_owned())
}

/// Mesa's `DRI_PRIME` tag for a PCI address: `0000:00:02.0` is
/// `pci-0000_00_02_0`.
fn dri_prime_tag(pdev: &str) -> String {
    format!("pci-{}", pdev.replace([':', '.'], "_"))
}

/// GLVND's EGL vendor file for `vendor` (`mesa` or `nvidia`).
fn egl_vendor_file(vendor: &str) -> Result<String, String> {
    let dirs = [
        "/etc/glvnd/egl_vendor.d",
        "/usr/share/glvnd/egl_vendor.d",
        "/usr/local/share/glvnd/egl_vendor.d",
    ];
    dirs.iter()
        .filter_map(|d| std::fs::read_dir(d).ok())
        .flat_map(|e| e.flatten())
        .map(|e| e.path())
        .find(|p| {
            p.file_name()
                .is_some_and(|n| n.to_string_lossy().ends_with(&format!("_{vendor}.json")))
        })
        .map(|p| p.to_string_lossy().into_owned())
        .ok_or_else(|| {
            format!(
                "no GLVND EGL vendor file for {vendor} in {}",
                dirs.join(", ")
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_i915_fdinfo() {
        let text = "pos:\t0\nflags:\t02100002\ndrm-driver:\ti915\ndrm-client-id:\t7\n\
            drm-pdev:\t0000:00:02.0\ndrm-total-system0:\t300 MiB\ndrm-shared-system0:\t0\n\
            drm-active-system0:\t0\ndrm-resident-system0:\t204800 KiB\n\
            drm-total-stolen-system0:\t0\ndrm-resident-stolen-system0:\t4194304\n\
            drm-engine-render:\t2000 ns\n";
        let c = parse_fdinfo(text).unwrap();
        assert_eq!(c.id, "7");
        assert_eq!(c.pdev.as_deref(), Some("0000:00:02.0"));
        assert_eq!(c.resident_kib, Some(204_800 + 4096));
        assert_eq!(c.total_kib, Some(300 * 1024));
    }

    #[test]
    fn ignores_non_drm_fds_and_counts_shared_clients_once() {
        assert_eq!(parse_fdinfo("pos:\t0\nflags:\t02\nmnt_id:\t3\n"), None);
        let a = parse_fdinfo("drm-client-id: 1\ndrm-resident-gtt: 1024 KiB\n").unwrap();
        let b = parse_fdinfo("drm-client-id: 2\ndrm-resident-vram0: 2 MiB\n").unwrap();
        let mut clients = HashMap::new();
        for c in [
            a,
            b,
            parse_fdinfo("drm-client-id: 1\ndrm-resident-gtt: 1024 KiB\n").unwrap(),
        ] {
            clients.insert((c.pdev.clone(), c.id.clone()), c);
        }
        assert_eq!(sum_clients(clients.values()), Some(3.0));
    }

    #[test]
    fn falls_back_to_total_without_resident_stats() {
        let c = parse_fdinfo("drm-client-id: 1\ndrm-total-system: 512 KiB\n").unwrap();
        assert_eq!(sum_clients([c].iter()), Some(0.5));
    }

    #[test]
    fn slowdown_reasons() {
        assert_eq!(is_slowdown("0x0000000000000004"), Some(false));
        assert_eq!(is_slowdown("0x0000000000000020"), Some(true));
        assert_eq!(is_slowdown("intel:"), Some(false));
        assert_eq!(is_slowdown("intel:pl1,pl2"), Some(false));
        assert_eq!(is_slowdown("intel:pl1,thermal"), Some(true));
        assert_eq!(is_slowdown("[N/A]"), None);
    }

    #[test]
    fn device_checks() {
        assert!(Gpu::Intel.matches("Intel", "Mesa Intel(R) Graphics (RPL-S)"));
        assert!(!Gpu::Intel.matches("NVIDIA Corporation", "NVIDIA GeForce RTX 4060 Laptop GPU"));
        assert!(Gpu::Nvidia.matches("NVIDIA Corporation", "NVIDIA GeForce RTX 4060 Laptop GPU"));
        assert!(Gpu::Auto.matches("", ""));
        assert_eq!(dri_prime_tag("0000:00:02.0"), "pci-0000_00_02_0");
    }
}
