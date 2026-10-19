//! Load, memory and thermal state of the machine a daemon runs on, for the desktop top bar.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct HostStats {
    /// One-minute load average.
    pub load1: f32,
    /// Share of memory in use, percent, when the platform tells.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mem_used_pct: Option<u8>,
    /// `nominal`, or `throttled N%` when the CPU is held back by heat.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thermal: Option<String>,
}

fn pages(text: &str, label: &str) -> Option<u64> {
    let line = text.lines().find(|l| l.starts_with(label))?;
    line.split(':').nth(1)?.trim().trim_end_matches('.').parse().ok()
}
/// Memory in use from `vm_stat` output and the total bytes (`hw.memsize`): active, wired and compressed pages.
pub fn parse_vm_stat(text: &str, total_bytes: u64) -> Option<u8> {
    let size: u64 = text
        .lines()
        .next()?
        .split("page size of ")
        .nth(1)?
        .split_whitespace()
        .next()?
        .parse()
        .ok()?;
    let used = pages(text, "Pages active")?
        + pages(text, "Pages wired down")?
        + pages(text, "Pages occupied by compressor").unwrap_or(0);
    (total_bytes > 0).then(|| ((used * size * 100) / total_bytes).min(100) as u8)
}
/// Memory in use from `/proc/meminfo`: everything but available memory.
pub fn parse_meminfo(text: &str) -> Option<u8> {
    let kb = |name: &str| -> Option<u64> {
        text.lines()
            .find(|l| l.starts_with(name))?
            .split_whitespace()
            .nth(1)?
            .parse()
            .ok()
    };
    let (total, avail) = (kb("MemTotal:")?, kb("MemAvailable:")?);
    (total > 0).then(|| ((total.saturating_sub(avail) * 100) / total) as u8)
}
/// `pmset -g therm`: a CPU speed limit below 100 means the machine is being held back.
pub fn parse_pmset_therm(text: &str) -> String {
    let limit = text
        .lines()
        .find(|l| l.trim_start().starts_with("CPU_Speed_Limit"))
        .and_then(|l| l.split('=').nth(1))
        .and_then(|v| v.trim().parse::<u32>().ok());
    match limit {
        Some(n) if n < 100 => format!("throttled {n}%"),
        _ => "nominal".into(),
    }
}

fn run(program: &str, args: &[&str]) -> Option<String> {
    let out = std::process::Command::new(program)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}
fn load1() -> f32 {
    let mut avg = [0f64; 3];
    // SAFETY: getloadavg writes at most the 3 doubles it is given.
    let n = unsafe { libc::getloadavg(avg.as_mut_ptr(), 3) };
    if n > 0 {
        avg[0] as f32
    } else {
        0.0
    }
}
/// Sample the machine now (spawns `vm_stat`/`sysctl`/`pmset` on macOS; call it rarely).
pub fn sample() -> HostStats {
    #[cfg(target_os = "macos")]
    let (mem, thermal) = {
        let total = run("sysctl", &["-n", "hw.memsize"]).and_then(|s| s.trim().parse().ok());
        let mem = run("vm_stat", &[])
            .zip(total)
            .and_then(|(t, total)| parse_vm_stat(&t, total));
        let thermal = run("pmset", &["-g", "therm"]).map(|t| parse_pmset_therm(&t));
        (mem, thermal)
    };
    #[cfg(not(target_os = "macos"))]
    let (mem, thermal) = (
        std::fs::read_to_string("/proc/meminfo")
            .ok()
            .and_then(|t| parse_meminfo(&t)),
        None,
    );
    let _ = run;
    HostStats {
        load1: load1(),
        mem_used_pct: mem,
        thermal,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const VM: &str = "Mach Virtual Memory Statistics: (page size of 16384 bytes)\nPages free:                                    16846.\nPages active:                                 264965.\nPages inactive:                               263557.\nPages wired down:                             174608.\nPages occupied by compressor:                  50000.\n";
    #[test]
    fn vm_stat_gives_the_share_in_use() {
        // (264965 + 174608 + 50000) pages of 16 KiB against 16 GiB.
        assert_eq!(parse_vm_stat(VM, 17_179_869_184), Some(46));
        assert_eq!(parse_vm_stat("garbage", 1), None);
        assert_eq!(parse_vm_stat(VM, 0), None);
    }
    #[test]
    fn meminfo_gives_the_share_in_use() {
        let t = "MemTotal:       16000000 kB\nMemFree:         1000000 kB\nMemAvailable:    4000000 kB\n";
        assert_eq!(parse_meminfo(t), Some(75));
        assert_eq!(parse_meminfo("nothing"), None);
    }
    #[test]
    fn thermal_state_reads_the_speed_limit() {
        assert_eq!(
            parse_pmset_therm("Note: No thermal warning level has been recorded\n"),
            "nominal"
        );
        assert_eq!(
            parse_pmset_therm("CPU_Scheduler_Limit \t= 100\nCPU_Speed_Limit     \t= 80\n"),
            "throttled 80%"
        );
        assert_eq!(parse_pmset_therm("CPU_Speed_Limit = 100\n"), "nominal");
    }
    #[test]
    fn sampling_this_machine_reports_a_load() {
        let s = sample();
        assert!(s.load1 >= 0.0);
        assert!(s.mem_used_pct.is_none_or(|p| p <= 100));
    }
}
