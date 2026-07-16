//! Stats collection (`argonsysinfo.py` parity): CPU%, RAM, CPU temp, disk
//! usage, RAID status, and local IP. Reads `/proc` and shells out to
//! `df`/`mdadm` the same way the Python version does — these are Linux
//! system-tool wrappers, not something worth reimplementing against raw
//! ioctls (W§1.2).

use std::fs;
use std::net::UdpSocket;
use std::process::Command;

/// Tracks two `/proc/stat` samples to compute CPU% as a delta, matching
/// the Python daemon's approach (a single snapshot can't give a percent).
#[derive(Default)]
pub struct CpuUsage {
    prev: Option<CpuTimes>,
}

#[derive(Clone, Copy)]
struct CpuTimes {
    idle: u64,
    total: u64,
}

impl CpuUsage {
    pub fn new() -> Self {
        CpuUsage::default()
    }

    /// Returns `None` on the first call (needs two samples) or if
    /// `/proc/stat` isn't readable (non-Linux, or sandboxed).
    pub fn sample_percent(&mut self) -> Option<f32> {
        let now = read_cpu_times()?;
        let percent = self.prev.map(|prev| {
            let idle_delta = now.idle.saturating_sub(prev.idle) as f32;
            let total_delta = now.total.saturating_sub(prev.total) as f32;
            if total_delta <= 0.0 {
                0.0
            } else {
                (1.0 - idle_delta / total_delta) * 100.0
            }
        });
        self.prev = Some(now);
        percent
    }
}

fn read_cpu_times() -> Option<CpuTimes> {
    let contents = fs::read_to_string("/proc/stat").ok()?;
    let line = contents.lines().next()?;
    let mut fields = line.split_whitespace();
    if fields.next()? != "cpu" {
        return None;
    }
    let values: Vec<u64> = fields.filter_map(|f| f.parse().ok()).collect();
    if values.len() < 4 {
        return None;
    }
    // user nice system idle iowait irq softirq steal ...
    let idle = values[3] + values.get(4).copied().unwrap_or(0);
    let total: u64 = values.iter().sum();
    Some(CpuTimes { idle, total })
}

/// CPU temperature in Celsius, from the SoC thermal zone.
pub fn read_cpu_temp_c() -> Option<f32> {
    let raw = fs::read_to_string("/sys/class/thermal/thermal_zone0/temp").ok()?;
    parse_cpu_temp_c(&raw)
}

fn parse_cpu_temp_c(raw: &str) -> Option<f32> {
    let millidegrees: i64 = raw.trim().parse().ok()?;
    Some(millidegrees as f32 / 1000.0)
}

#[derive(Debug, Clone, Copy)]
pub struct MemInfo {
    pub total_kb: u64,
    pub available_kb: u64,
}

impl MemInfo {
    pub fn used_percent(&self) -> f32 {
        if self.total_kb == 0 {
            return 0.0;
        }
        let used = self.total_kb.saturating_sub(self.available_kb) as f32;
        used / self.total_kb as f32 * 100.0
    }
}

pub fn read_mem_info() -> Option<MemInfo> {
    let contents = fs::read_to_string("/proc/meminfo").ok()?;
    parse_mem_info(&contents)
}

fn parse_mem_info(contents: &str) -> Option<MemInfo> {
    let mut total_kb = None;
    let mut available_kb = None;
    for line in contents.lines() {
        let (key, rest) = line.split_once(':')?;
        let value: u64 = rest.split_whitespace().next()?.parse().ok()?;
        match key {
            "MemTotal" => total_kb = Some(value),
            "MemAvailable" => available_kb = Some(value),
            _ => {}
        }
    }
    Some(MemInfo {
        total_kb: total_kb?,
        available_kb: available_kb?,
    })
}

#[derive(Debug, Clone)]
pub struct DiskUsage {
    /// `df`'s `Filesystem` column, e.g. `/dev/sda1` — what the storage
    /// page (v0.4.0) matches against a whole-disk device name like
    /// `sda`. Matching on `mount` instead (a plain path like `/` or
    /// `/data`) doesn't work: mount paths essentially never contain the
    /// underlying device name as a substring.
    pub filesystem: String,
    pub mount: String,
    pub used_pct: u8,
}

/// Shells out to `df`, matching the Python daemon's approach — parsing
/// `/proc/partitions` by hand and reimplementing RAID-aware dedup isn't
/// worth it when `df` already does both correctly.
pub fn read_disk_usage() -> Vec<DiskUsage> {
    let Ok(output) = Command::new("df")
        .args(["-P", "-x", "tmpfs", "-x", "devtmpfs"])
        .output()
    else {
        return Vec::new();
    };
    parse_disk_usage(&String::from_utf8_lossy(&output.stdout))
}

fn parse_disk_usage(text: &str) -> Vec<DiskUsage> {
    text.lines()
        .skip(1) // header
        .filter_map(|line| {
            let fields: Vec<&str> = line.split_whitespace().collect();
            let filesystem = fields.first()?.to_string();
            let mount = fields.get(5)?.to_string();
            let pct = fields.get(4)?.trim_end_matches('%').parse().ok()?;
            Some(DiskUsage {
                filesystem,
                mount,
                used_pct: pct,
            })
        })
        .collect()
}

/// Whether a `df` row (identified by its `Filesystem` column) belongs to
/// `device` (a whole-disk name like `sda`, from `lsblk`) — true for
/// `/dev/sda`, `/dev/sda1`, `/dev/mapper/sda1_crypt`, etc.
pub fn filesystem_belongs_to_device(filesystem: &str, device: &str) -> bool {
    filesystem
        .rsplit('/')
        .next()
        .is_some_and(|leaf| leaf.starts_with(device))
}

#[derive(Debug, Clone, PartialEq)]
pub struct RaidDevice {
    pub name: String,
    pub spare: bool,
}

/// A `/proc/mdstat` array entry, parsed from its two-line record — the
/// array-summary line (name, level, member devices) and the immediately
/// following blocks/status line (size, `[raid_disks/working_disks]`,
/// per-device up/down bitmap). Full `mdadm -D` detail (event counts,
/// resync progress) isn't parsed — this is what the storage/RAID web
/// page (v0.4.0, W§3.3) actually needs, not everything `mdadm` reports.
#[derive(Debug, Clone, PartialEq)]
pub struct RaidArray {
    pub name: String,
    pub level: String,
    pub state: String,
    pub devices: Vec<RaidDevice>,
    pub raid_disks: u8,
    pub working_disks: u8,
    pub size_kb: Option<u64>,
}

impl RaidArray {
    pub fn failed_disks(&self) -> u8 {
        self.raid_disks.saturating_sub(self.working_disks)
    }

    pub fn spare_disks(&self) -> usize {
        self.devices.iter().filter(|d| d.spare).count()
    }
}

/// Parses `/proc/mdstat` for per-array RAID status.
pub fn read_raid_status() -> Vec<RaidArray> {
    let Ok(contents) = fs::read_to_string("/proc/mdstat") else {
        return Vec::new();
    };
    parse_raid_status(&contents)
}

fn parse_raid_status(contents: &str) -> Vec<RaidArray> {
    let mut arrays = Vec::new();
    let mut lines = contents.lines().peekable();
    while let Some(line) = lines.next() {
        if !line.starts_with("md") {
            continue;
        }
        let mut fields = line.split_whitespace();
        let Some(name) = fields.next() else { continue };
        let Some(_colon) = fields.next() else {
            continue;
        };
        let Some(activity) = fields.next() else {
            continue;
        };
        let level = fields.next().unwrap_or("").to_string();
        let devices: Vec<RaidDevice> = fields
            .map(|tok| RaidDevice {
                name: tok.split('[').next().unwrap_or(tok).to_string(),
                spare: tok.contains("(S)"),
            })
            .collect();

        let mut size_kb = None;
        let mut raid_disks = 0u8;
        let mut working_disks = 0u8;
        if let Some(detail_line) = lines.peek() {
            size_kb = detail_line
                .split_whitespace()
                .next()
                .and_then(|s| s.parse().ok());
            if let Some(bracket) = detail_line.split('[').nth(1)
                && let Some(counts) = bracket.split(']').next()
                && let Some((total, working)) = counts.split_once('/')
            {
                raid_disks = total.parse().unwrap_or(0);
                working_disks = working.parse().unwrap_or(0);
            }
        }

        let state = if activity == "inactive" {
            "inactive"
        } else if raid_disks > 0 && working_disks < raid_disks {
            "degraded"
        } else {
            "active"
        };

        arrays.push(RaidArray {
            name: name.to_string(),
            level,
            state: state.to_string(),
            devices,
            raid_disks,
            working_disks,
            size_kb,
        });
    }
    arrays
}

#[derive(Debug, Clone, PartialEq)]
pub struct BlockDevice {
    pub name: String,
    pub size_bytes: u64,
    pub model: Option<String>,
}

/// Whole-disk devices (no partitions), via `lsblk -d` — the storage page
/// (v0.4.0, W§3.3) lists per-disk rows, not per-filesystem-mount rows
/// like [`read_disk_usage`].
pub fn read_block_devices() -> Vec<BlockDevice> {
    let Ok(output) = Command::new("lsblk")
        .args(["-d", "-n", "-b", "-J", "-o", "NAME,SIZE,MODEL,TYPE"])
        .output()
    else {
        return Vec::new();
    };
    parse_block_devices(&String::from_utf8_lossy(&output.stdout))
}

fn parse_block_devices(json: &str) -> Vec<BlockDevice> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(json) else {
        return Vec::new();
    };
    let Some(devices) = value.get("blockdevices").and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    devices
        .iter()
        .filter(|d| d.get("type").and_then(|v| v.as_str()) == Some("disk"))
        .filter_map(|d| {
            let name = d.get("name")?.as_str()?.to_string();
            let size_bytes = d.get("size")?.as_u64()?;
            let model = d
                .get("model")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string);
            Some(BlockDevice {
                name,
                size_bytes,
                model,
            })
        })
        .collect()
}

/// Disk temperature in Celsius via `smartctl`'s JSON output, matching the
/// documented approach (W§1.2) — no safe Rust crate reads S.M.A.R.T.
/// registers directly for arbitrary drives, shelling out is the pragmatic
/// choice here too. Returns `None` on any failure (missing `smartmontools`,
/// unsupported drive, permission denied) rather than erroring — a status
/// page showing "temp: —" is fine, a crashed daemon is not.
pub fn read_disk_temp_c(device: &str) -> Option<f32> {
    let output = Command::new("smartctl")
        .args(["-A", "-j", &format!("/dev/{device}")])
        .output()
        .ok()?;
    parse_smartctl_temp(&String::from_utf8_lossy(&output.stdout))
}

fn parse_smartctl_temp(json: &str) -> Option<f32> {
    let value: serde_json::Value = serde_json::from_str(json).ok()?;
    value
        .get("temperature")
        .and_then(|t| t.get("current"))
        .and_then(|v| v.as_f64())
        .map(|v| v as f32)
}

/// A block device paired with its S.M.A.R.T. temperature, for the
/// storage page (v0.4.0) and the HDD fan curve's temperature input.
#[derive(Debug, Clone)]
pub struct DiskInfo {
    pub device: BlockDevice,
    pub temp_c: Option<f32>,
}

/// [`read_block_devices`] + a [`read_disk_temp_c`] call per device — runs
/// on a blocking-pool thread via [`tokio::task::spawn_blocking`] since
/// `lsblk`/`smartctl` are synchronous subprocess calls that would
/// otherwise stall a tokio worker thread if called directly from async
/// code (the fan control loop's poll tick, or a storage-page request
/// handler).
pub async fn read_storage_snapshot() -> Vec<DiskInfo> {
    tokio::task::spawn_blocking(|| {
        read_block_devices()
            .into_iter()
            .map(|device| {
                let temp_c = read_disk_temp_c(&device.name);
                DiskInfo { device, temp_c }
            })
            .collect()
    })
    .await
    .unwrap_or_default()
}

/// Board model string, for the OLED splash screen's version label (W§1.5's
/// resolution). Sourced from the device tree rather than `/proc/cpuinfo`'s
/// `Revision` code table, which would need a lookup table to decode.
pub fn read_pi_model() -> Option<String> {
    for path in [
        "/proc/device-tree/model",
        "/sys/firmware/devicetree/base/model",
    ] {
        if let Ok(raw) = fs::read_to_string(path) {
            let model = raw.trim_end_matches('\0').trim().to_string();
            if !model.is_empty() {
                return Some(model);
            }
        }
    }
    None
}

/// Local IP via the classic UDP-connect trick: connecting a UDP socket
/// doesn't send any packets, it just makes the kernel pick a source
/// address/route, which `local_addr()` then reports.
pub fn read_local_ip() -> Option<std::net::IpAddr> {
    let socket = UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("8.8.8.8:80").ok()?;
    Some(socket.local_addr().ok()?.ip())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mem_used_percent_handles_zero_total() {
        let mem = MemInfo {
            total_kb: 0,
            available_kb: 0,
        };
        assert_eq!(mem.used_percent(), 0.0);
    }

    #[test]
    fn mem_used_percent_computes_correctly() {
        let mem = MemInfo {
            total_kb: 1000,
            available_kb: 250,
        };
        assert_eq!(mem.used_percent(), 75.0);
    }

    #[test]
    fn parse_cpu_temp_c_converts_millidegrees() {
        assert_eq!(parse_cpu_temp_c("45123\n"), Some(45.123));
    }

    #[test]
    fn parse_cpu_temp_c_rejects_garbage() {
        assert_eq!(parse_cpu_temp_c("not-a-number"), None);
    }

    #[test]
    fn parse_mem_info_reads_total_and_available() {
        let mem = parse_mem_info(
            "MemTotal:       16384000 kB\nMemFree:         1000000 kB\nMemAvailable:    8192000 kB\n",
        )
        .unwrap();
        assert_eq!(mem.total_kb, 16384000);
        assert_eq!(mem.available_kb, 8192000);
    }

    #[test]
    fn parse_mem_info_none_when_fields_missing() {
        assert!(parse_mem_info("MemFree: 1000 kB\n").is_none());
    }

    #[test]
    fn parse_disk_usage_skips_header_and_parses_rows() {
        let text = "Filesystem 1024-blocks Used Available Capacity Mounted\n\
                     /dev/root    100000    50000  50000      50%    /\n\
                     /dev/sda1    200000   180000  20000      90%    /data\n";
        let disks = parse_disk_usage(text);
        assert_eq!(disks.len(), 2);
        assert_eq!(disks[0].filesystem, "/dev/root");
        assert_eq!(disks[0].mount, "/");
        assert_eq!(disks[0].used_pct, 50);
        assert_eq!(disks[1].filesystem, "/dev/sda1");
        assert_eq!(disks[1].mount, "/data");
        assert_eq!(disks[1].used_pct, 90);
    }

    #[test]
    fn filesystem_belongs_to_device_matches_partitions_not_mount_paths() {
        // The actual bug: a whole-disk device name like "sda" is never a
        // substring of a mount path like "/" or "/data" — matching has to
        // go through df's Filesystem column instead.
        assert!(filesystem_belongs_to_device("/dev/sda1", "sda"));
        assert!(filesystem_belongs_to_device("/dev/sda", "sda"));
        assert!(!filesystem_belongs_to_device("/dev/sdb1", "sda"));
        assert!(!filesystem_belongs_to_device("/", "sda"));
        assert!(!filesystem_belongs_to_device("/data", "sda"));
    }

    #[test]
    fn filesystem_belongs_to_device_handles_nvme_and_mmc_partition_naming() {
        assert!(filesystem_belongs_to_device("/dev/nvme0n1p1", "nvme0n1"));
        assert!(filesystem_belongs_to_device("/dev/mmcblk0p1", "mmcblk0"));
        assert!(!filesystem_belongs_to_device("/dev/mmcblk1p1", "mmcblk0"));
    }

    #[test]
    fn parse_raid_status_reports_active_array() {
        let mdstat = "Personalities : [raid1]\nmd0 : active raid1 sda1[0] sdb1[1]\n      1000000 blocks [2/2] [UU]\n";
        let arrays = parse_raid_status(mdstat);
        assert_eq!(arrays.len(), 1);
        assert_eq!(arrays[0].name, "md0");
        assert_eq!(arrays[0].state, "active");
    }

    #[test]
    fn parse_raid_status_reports_inactive_array() {
        let mdstat = "md0 : inactive sda1[0]\n";
        let arrays = parse_raid_status(mdstat);
        assert_eq!(arrays[0].state, "inactive");
    }

    #[test]
    fn parse_raid_status_extracts_devices_size_and_counts() {
        let mdstat = "Personalities : [raid1]\nmd0 : active raid1 sda1[0] sdb1[1]\n      1000000 blocks super 1.2 [2/2] [UU]\n";
        let arrays = parse_raid_status(mdstat);
        assert_eq!(arrays[0].level, "raid1");
        assert_eq!(arrays[0].raid_disks, 2);
        assert_eq!(arrays[0].working_disks, 2);
        assert_eq!(arrays[0].size_kb, Some(1000000));
        assert_eq!(arrays[0].failed_disks(), 0);
        assert_eq!(
            arrays[0].devices,
            vec![
                RaidDevice {
                    name: "sda1".to_string(),
                    spare: false
                },
                RaidDevice {
                    name: "sdb1".to_string(),
                    spare: false
                },
            ]
        );
    }

    #[test]
    fn parse_raid_status_detects_degraded_array_from_working_count() {
        let mdstat = "md0 : active raid1 sda1[0]\n      1000000 blocks super 1.2 [2/1] [U_]\n";
        let arrays = parse_raid_status(mdstat);
        assert_eq!(arrays[0].state, "degraded");
        assert_eq!(arrays[0].failed_disks(), 1);
    }

    #[test]
    fn parse_raid_status_counts_spare_devices() {
        let mdstat = "md0 : active raid1 sda1[0] sdb1[1] sdc1[2](S)\n      1000000 blocks super 1.2 [2/2] [UU]\n";
        let arrays = parse_raid_status(mdstat);
        assert_eq!(arrays[0].spare_disks(), 1);
    }

    #[test]
    fn parse_block_devices_filters_to_whole_disks() {
        let json = r#"{
           "blockdevices": [
              {"name":"sda", "size":4000787030016, "model":"IronWolf ", "type":"disk"},
              {"name":"sda1", "size":4000785932288, "model":null, "type":"part"},
              {"name":"mmcblk0", "size":63864569856, "model":null, "type":"disk"}
           ]
        }"#;
        let devices = parse_block_devices(json);
        assert_eq!(devices.len(), 2);
        assert_eq!(devices[0].name, "sda");
        assert_eq!(devices[0].size_bytes, 4000787030016);
        assert_eq!(devices[0].model.as_deref(), Some("IronWolf"));
        assert_eq!(devices[1].name, "mmcblk0");
        assert_eq!(devices[1].model, None);
    }

    #[test]
    fn parse_block_devices_handles_malformed_json() {
        assert_eq!(parse_block_devices("not json"), vec![]);
    }

    #[test]
    fn parse_smartctl_temp_reads_current_temperature() {
        let json = r#"{"temperature": {"current": 34}}"#;
        assert_eq!(parse_smartctl_temp(json), Some(34.0));
    }

    #[test]
    fn parse_smartctl_temp_none_when_field_missing() {
        assert_eq!(parse_smartctl_temp(r#"{"some_other_field": 1}"#), None);
        assert_eq!(parse_smartctl_temp("not json"), None);
    }

    #[test]
    fn cpu_usage_needs_two_samples() {
        // On a non-Linux CI runner /proc/stat won't exist, so this should
        // stay None rather than panic either way.
        let mut usage = CpuUsage::new();
        let first = usage.sample_percent();
        if cfg!(target_os = "linux") {
            assert!(
                first.is_none(),
                "first sample has no delta to compare against"
            );
        } else {
            assert!(first.is_none());
        }
    }
}
