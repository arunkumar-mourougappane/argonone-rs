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
    let text = String::from_utf8_lossy(&output.stdout);
    text.lines()
        .skip(1) // header
        .filter_map(|line| {
            let fields: Vec<&str> = line.split_whitespace().collect();
            let mount = fields.get(5)?.to_string();
            let pct = fields.get(4)?.trim_end_matches('%').parse().ok()?;
            Some(DiskUsage {
                mount,
                used_pct: pct,
            })
        })
        .collect()
}

#[derive(Debug, Clone)]
pub struct RaidArray {
    pub name: String,
    pub state: String,
}

/// Parses `/proc/mdstat` for a coarse "array name + active/degraded"
/// summary. Full detail (`mdadm -D`) is left for when the storage/RAID
/// web page (v0.4.0) actually needs more than this.
pub fn read_raid_status() -> Vec<RaidArray> {
    let Ok(contents) = fs::read_to_string("/proc/mdstat") else {
        return Vec::new();
    };
    contents
        .lines()
        .filter(|l| l.starts_with("md"))
        .filter_map(|line| {
            let name = line.split_whitespace().next()?.to_string();
            let state = if line.contains("inactive") {
                "inactive"
            } else if contents.contains("_") {
                // crude degraded-array signal: mdstat prints '_' in the
                // bitmap for a missing/failed member.
                "degraded"
            } else {
                "active"
            };
            Some(RaidArray {
                name,
                state: state.to_string(),
            })
        })
        .collect()
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
