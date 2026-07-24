use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use kube::api::{Patch, PatchParams};
use kube::Api;
use proteus_crd::{BackupPhase, ProteusBackup, ProteusBackupStatus};

/// Patches `progressPercent` + `message` while a backup is Running.
pub struct BackupProgressSink {
    api: Api<ProteusBackup>,
    name: String,
    carry: ProteusBackupStatus,
    last_percent: Mutex<u8>,
    last_message: Mutex<String>,
    cancelled: AtomicBool,
}

impl BackupProgressSink {
    pub fn new(api: Api<ProteusBackup>, name: String, obj: &ProteusBackup) -> Self {
        Self {
            api,
            name,
            carry: ProteusBackupStatus {
                last_snapshot_id: obj.status.as_ref().and_then(|s| s.last_snapshot_id.clone()),
                last_success_at: obj.status.as_ref().and_then(|s| s.last_success_at.clone()),
                last_failure_at: obj.status.as_ref().and_then(|s| s.last_failure_at.clone()),
                last_bytes: obj.status.as_ref().and_then(|s| s.last_bytes),
                retained_snapshots: obj.status.as_ref().and_then(|s| s.retained_snapshots),
                ..Default::default()
            },
            last_percent: Mutex::new(0),
            last_message: Mutex::new(String::new()),
            cancelled: AtomicBool::new(false),
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed)
    }

    /// Returns `Err` when the backup CR was deleted (user cancelled via UI/API).
    pub async fn report(&self, percent: u8, message: impl Into<String>) -> Result<(), String> {
        if self.is_cancelled() {
            return Err("backup deleted; cancelling".into());
        }
        let percent = percent.min(99);
        let message = message.into();
        {
            let mut last = self.last_percent.lock().unwrap_or_else(|e| e.into_inner());
            let mut last_msg = self.last_message.lock().unwrap_or_else(|e| e.into_inner());
            // Never go backwards on percent.
            if percent < *last {
                return Ok(());
            }
            // Same percent + same message → skip (progress patches must not spam).
            if percent == *last && message == *last_msg {
                return Ok(());
            }
            *last = percent;
            *last_msg = message.clone();
        }

        let status = ProteusBackupStatus {
            phase: Some(BackupPhase::Running),
            message: Some(message),
            progress_percent: Some(percent),
            ..self.carry.clone()
        };
        let patch = serde_json::json!({ "status": status });
        match self
            .api
            .patch_status(&self.name, &PatchParams::default(), &Patch::Merge(&patch))
            .await
        {
            Ok(_) => Ok(()),
            Err(kube::Error::Api(err)) if err.code == 404 => {
                self.cancelled.store(true, Ordering::Relaxed);
                Err("backup deleted; cancelling".into())
            }
            Err(err) => {
                tracing::warn!(error = %err, "failed to patch backup progress");
                Ok(())
            }
        }
    }
}

/// Map a fraction within `[range_start, range_end]` to a percent.
pub fn map_range(range_start: u8, range_end: u8, done: u64, total: u64) -> u8 {
    let start = u32::from(range_start);
    let end = u32::from(range_end.max(range_start));
    if total == 0 {
        return range_start;
    }
    let span = end.saturating_sub(start);
    let frac = (done.min(total) as u128 * span as u128) / total as u128;
    (start + frac as u32).min(99) as u8
}

pub fn format_bytes(n: u64) -> String {
    const KIB: f64 = 1024.0;
    const MIB: f64 = KIB * 1024.0;
    const GIB: f64 = MIB * 1024.0;
    let n = n as f64;
    if n >= GIB {
        format!("{:.1} GiB", n / GIB)
    } else if n >= MIB {
        format!("{:.1} MiB", n / MIB)
    } else if n >= KIB {
        format!("{:.1} KiB", n / KIB)
    } else {
        format!("{n:.0} B")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_range_midpoint() {
        assert_eq!(map_range(10, 50, 50, 100), 30);
    }

    #[test]
    fn map_range_complete() {
        assert_eq!(map_range(10, 50, 100, 100), 50);
    }
}
