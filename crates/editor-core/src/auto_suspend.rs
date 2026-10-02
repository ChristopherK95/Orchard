//! Auto-suspend (ticket 12): when the `claude` processes use more than the memory limit, or the OS
//! runs low, the longest-Idle sessions are Suspended until back under it; optionally, sessions Idle
//! longer than N minutes are too. Working and Needs you sessions are never touched.
//!
//! There's no telling which `claude` process belongs to which session, so the memory is taken as
//! split evenly between the running sessions: enough are suspended now to get back under on that
//! estimate, and the next check (after a short cooldown, so the closed processes are gone from the
//! reading) suspends more if that wasn't enough.
//!
//! The memory reading and the clock are injectable, so the choice can be tested exactly.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::session::SessionId;

/// What the memory monitor sees.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MemorySample {
    /// Memory of the adapter's direct children: the `claude` processes, one per running session
    /// (resident set on Linux, working set on Windows). Not the adapter itself, whose memory
    /// suspending doesn't free, nor the tools an Agent runs (a build), which it can't free either.
    pub agents_bytes: u64,
    /// Memory the OS can still hand out, and its total.
    pub available_bytes: u64,
    pub total_bytes: u64,
}

/// Reads memory use; `adapter_pid` is the shared adapter's process, if it's running.
pub trait MemoryProbe: Send + Sync {
    fn sample(&self, adapter_pid: Option<u32>) -> MemorySample;
}

pub trait Clock: Send + Sync {
    fn now(&self) -> Instant;
}

/// The real clock.
pub(crate) struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

/// The real memory reading, from the OS's process table.
pub(crate) struct SystemProbe {
    system: Mutex<sysinfo::System>,
}

impl Default for SystemProbe {
    fn default() -> Self {
        Self {
            system: Mutex::new(sysinfo::System::new()),
        }
    }
}

impl MemoryProbe for SystemProbe {
    fn sample(&self, adapter_pid: Option<u32>) -> MemorySample {
        use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate};
        // (A panic mid-read mustn't switch auto-suspend off for good.)
        let mut system = self.system.lock().unwrap_or_else(|e| e.into_inner());
        system.refresh_memory();
        let mut agents_bytes = 0;
        if let Some(adapter) = adapter_pid.map(Pid::from_u32) {
            system.refresh_processes_specifics(
                ProcessesToUpdate::All,
                true,
                ProcessRefreshKind::nothing().with_memory(),
            );
            let started = system.process(adapter).map(|p| p.start_time());
            agents_bytes = system
                .processes()
                .values()
                // A child of the adapter (not of a process that reused a dead one's id).
                .filter(|p| p.parent() == Some(adapter) && Some(p.start_time()) >= started)
                .map(|p| p.memory())
                .sum();
        }
        MemorySample {
            agents_bytes,
            available_bytes: system.available_memory(),
            total_bytes: system.total_memory(),
        }
    }
}

/// Why sessions were suspended, for the toast.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum AutoSuspendReason {
    /// The `claude` processes used `used_bytes`, over the `limit_bytes` setting.
    #[serde(rename_all = "camelCase")]
    MemoryLimit { used_bytes: u64, limit_bytes: u64 },
    /// The OS reported little memory left.
    #[serde(rename_all = "camelCase")]
    LowMemory { available_bytes: u64 },
    /// Idle longer than the idle auto-suspend setting.
    #[serde(rename_all = "camelCase")]
    Idle { minutes: u64 },
}

/// An Idle session that could be suspended, and since when it's been Idle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Candidate {
    pub(crate) id: SessionId,
    pub(crate) idle_since: Instant,
}

/// The rules in force for one check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Limits {
    /// `None` when there's no limit, or just after a suspend (the reading may not show it yet).
    pub(crate) memory_limit_bytes: Option<u64>,
    /// Whether to act on the OS being low on memory (not just after a suspend either).
    pub(crate) low_memory: bool,
    /// `None` when idle auto-suspend is off.
    pub(crate) idle_after: Option<Duration>,
}

/// The OS counts as low on memory below this much available (or 5% of it, if that's more).
const LOW_MEMORY_BYTES: u64 = 512 * 1024 * 1024;

/// Which Idle sessions to suspend, and why. `running` is how many sessions have an Agent process
/// (Idle, Working or Needs you): the memory is split evenly between them, as there's no telling
/// which `claude` process is whose. Longest-Idle first; only `candidates` are ever chosen.
pub(crate) fn choose(
    candidates: &[Candidate],
    running: usize,
    sample: MemorySample,
    limits: Limits,
    now: Instant,
) -> Vec<(SessionId, AutoSuspendReason)> {
    let mut by_idle = candidates.to_vec();
    by_idle.sort_by_key(|c| c.idle_since);
    let mut chosen: Vec<(SessionId, AutoSuspendReason)> = vec![];

    if let Some(limit) = limits.memory_limit_bytes {
        if sample.agents_bytes > limit && running > 0 {
            let each = sample.agents_bytes.div_ceil(running as u64).max(1);
            let over = sample.agents_bytes - limit;
            let needed = over.div_ceil(each) as usize;
            let reason = AutoSuspendReason::MemoryLimit {
                used_bytes: sample.agents_bytes,
                limit_bytes: limit,
            };
            chosen.extend(by_idle.iter().take(needed).map(|c| (c.id, reason)));
        }
    }

    // Low memory only counts when the Agents are a real part of it: suspending them won't help
    // much if another program (a game, a VM) is what's using it up.
    let low_at = LOW_MEMORY_BYTES.max(sample.total_bytes / 20);
    let in_use = sample.total_bytes.saturating_sub(sample.available_bytes);
    let agents_matter = sample.agents_bytes.saturating_mul(4) >= in_use;
    if limits.low_memory
        && sample.total_bytes > 0
        && sample.available_bytes < low_at
        && agents_matter
        && chosen.is_empty()
    {
        // One at a time: the next check sees whether that was enough.
        let reason = AutoSuspendReason::LowMemory {
            available_bytes: sample.available_bytes,
        };
        chosen.extend(by_idle.first().map(|c| (c.id, reason)));
    }

    if let Some(after) = limits.idle_after {
        let reason = AutoSuspendReason::Idle {
            minutes: after.as_secs() / 60,
        };
        for c in &by_idle {
            let idle_for = now.saturating_duration_since(c.idle_since);
            if idle_for >= after && !chosen.iter().any(|(id, _)| *id == c.id) {
                chosen.push((c.id, reason));
            }
        }
    }
    chosen
}

#[cfg(test)]
mod tests {
    use super::*;

    const GB: u64 = 1024 * 1024 * 1024;

    fn limits(limit_gb: u64, idle_minutes: Option<u64>) -> Limits {
        Limits {
            memory_limit_bytes: Some(limit_gb * GB),
            low_memory: true,
            idle_after: idle_minutes.map(|m| Duration::from_secs(m * 60)),
        }
    }

    fn sample(agents_gb: f64) -> MemorySample {
        MemorySample {
            agents_bytes: (agents_gb * GB as f64) as u64,
            available_bytes: 8 * GB,
            total_bytes: 32 * GB,
        }
    }

    fn candidates(start: Instant, idle_ages_secs: &[u64]) -> Vec<Candidate> {
        idle_ages_secs
            .iter()
            .enumerate()
            .map(|(i, age)| Candidate {
                id: SessionId(i as u64 + 1),
                idle_since: start - Duration::from_secs(*age),
            })
            .collect()
    }

    fn ids(chosen: &[(SessionId, AutoSuspendReason)]) -> Vec<u64> {
        chosen.iter().map(|(id, _)| id.0).collect()
    }

    #[test]
    fn the_real_probe_reads_the_os_and_counts_only_the_adapters_descendants() {
        let probe = SystemProbe::default();
        let none = probe.sample(None);
        assert!(none.total_bytes > 0 && none.available_bytes > 0);
        assert_eq!(none.agents_bytes, 0, "no adapter, no Agents");
        // A child of this process stands in for a `claude` under the adapter.
        let mut child = std::process::Command::new(if cfg!(windows) { "cmd" } else { "sleep" })
            .args(if cfg!(windows) {
                &["/C", "ping -n 3 127.0.0.1 >NUL"][..]
            } else {
                &["2"][..]
            })
            .spawn()
            .unwrap();
        let with_child = probe.sample(Some(std::process::id()));
        let _ = child.kill();
        let _ = child.wait();
        assert!(with_child.agents_bytes > 0, "the child's memory is counted");
    }

    #[test]
    fn under_the_limit_nothing_is_suspended() {
        let now = Instant::now() + Duration::from_secs(10_000);
        let c = candidates(now, &[300, 200, 100]);
        assert!(choose(&c, 3, sample(3.9), limits(4, None), now).is_empty());
    }

    #[test]
    fn over_the_limit_the_longest_idle_go_first_until_under() {
        let now = Instant::now() + Duration::from_secs(10_000);
        let c = candidates(now, &[100, 300, 200]);
        // 5 GB over 4 sessions: about 1.25 GB each, 1 GB over -> one session.
        assert_eq!(ids(&choose(&c, 4, sample(5.0), limits(4, None), now)), [2]);
        // 7 GB over 4: 1.75 each, 3 GB over -> two.
        assert_eq!(
            ids(&choose(&c, 4, sample(7.0), limits(4, None), now)),
            [2, 3]
        );
        // Far over: every candidate, but never more.
        assert_eq!(
            ids(&choose(&c, 4, sample(40.0), limits(4, None), now)),
            [2, 3, 1]
        );
    }

    #[test]
    fn low_os_memory_suspends_the_longest_idle_one() {
        let now = Instant::now() + Duration::from_secs(10_000);
        let c = candidates(now, &[100, 300]);
        let low = MemorySample {
            agents_bytes: 6 * GB,
            available_bytes: 200 * 1024 * 1024,
            total_bytes: 16 * GB,
        };
        let chosen = choose(&c, 2, low, limits(8, None), now);
        assert_eq!(ids(&chosen), [2]);
        assert!(matches!(chosen[0].1, AutoSuspendReason::LowMemory { .. }));
    }

    #[test]
    fn low_memory_that_isnt_the_agents_doing_suspends_nothing() {
        let now = Instant::now() + Duration::from_secs(10_000);
        let c = candidates(now, &[100, 300]);
        let low_from_elsewhere = MemorySample {
            agents_bytes: GB / 4,
            available_bytes: 200 * 1024 * 1024,
            total_bytes: 16 * GB,
        };
        assert!(choose(&c, 2, low_from_elsewhere, limits(8, None), now).is_empty());
    }

    #[test]
    fn idle_auto_suspend_takes_only_sessions_idle_long_enough() {
        let now = Instant::now() + Duration::from_secs(10_000);
        let c = candidates(now, &[31 * 60, 10 * 60, 45 * 60]);
        assert_eq!(
            ids(&choose(&c, 3, sample(1.0), limits(4, Some(30)), now)),
            [3, 1]
        );
        assert!(
            choose(&c, 3, sample(1.0), limits(4, None), now).is_empty(),
            "off"
        );
    }
}
