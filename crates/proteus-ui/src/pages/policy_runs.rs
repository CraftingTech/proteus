use crate::api::{self, BackupListItem, CreateRestoreRequest};
use crate::Route;
use dioxus::prelude::*;

fn short_id(id: &str, keep: usize) -> String {
    let id = id.trim();
    if id.is_empty() {
        return "—".into();
    }
    if id.chars().count() <= keep {
        return id.to_string();
    }
    let head: String = id.chars().take(keep).collect();
    format!("{head}…")
}

/// Human run time for restore picking: `2026-08-04 04:50 UTC`.
fn format_run_when(item: &BackupListItem) -> String {
    if let Some(raw) = item
        .started_at
        .as_deref()
        .or(item.created_at.as_deref())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        return format_rfc3339_short(raw);
    }
    stamp_from_run_name(&item.name).unwrap_or_else(|| "Unknown time".into())
}

fn format_rfc3339_short(raw: &str) -> String {
    match raw.split_once('T') {
        Some((date, rest)) => {
            let time = rest.get(..5).unwrap_or(rest);
            format!("{date} {time} UTC")
        }
        None => raw.to_string(),
    }
}

/// Fallback when status/creation times are missing: `{policy}-{YYYYMMDDHHMMSS}`.
fn stamp_from_run_name(name: &str) -> Option<String> {
    let stamp = name.rsplit('-').next()?;
    if stamp.len() != 14 || !stamp.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let y = &stamp[0..4];
    let mo = &stamp[4..6];
    let d = &stamp[6..8];
    let h = &stamp[8..10];
    let mi = &stamp[10..12];
    Some(format!("{y}-{mo}-{d} {h}:{mi} UTC"))
}

fn format_throughput(bps: u64) -> String {
    const KIB: f64 = 1024.0;
    const MIB: f64 = KIB * 1024.0;
    let n = bps as f64;
    if n >= MIB {
        format!("{:.1} MiB/s", n / MIB)
    } else if n >= KIB {
        format!("{:.1} KiB/s", n / KIB)
    } else {
        format!("{bps} B/s")
    }
}

fn confirm_delete_backup(name: &str, namespace: &str) -> bool {
    let msg = format!(
        "Delete backup run {name} in {namespace}?\n\nUnreferenced objects in its repository will be removed (other runs' snapshots are kept)."
    );
    web_sys::window()
        .and_then(|w| w.confirm_with_message(&msg).ok())
        .unwrap_or(false)
}

fn confirm_restore(when: &str, name: &str, namespace: &str) -> bool {
    let msg = format!(
        "Restore backup from {when}\n\nRun: {name}\nNamespace: {namespace}\n\nPVCs with the same names must already exist. A Restore CR will be created."
    );
    web_sys::window()
        .and_then(|w| w.confirm_with_message(&msg).ok())
        .unwrap_or(false)
}

fn matches_policy(item: &BackupListItem, policy_ns: &str, policy_name: &str) -> bool {
    item.namespace == policy_ns
        && item
            .policy_ref
            .as_deref()
            .map(str::trim)
            .is_some_and(|p| p == policy_name)
}

fn sort_runs(mut items: Vec<BackupListItem>) -> Vec<BackupListItem> {
    items.sort_by(|a, b| {
        let rank = |p: Option<&str>| match p {
            Some("Running") => 0,
            Some("Pending") => 1,
            _ => 2,
        };
        rank(a.phase.as_deref())
            .cmp(&rank(b.phase.as_deref()))
            .then_with(|| b.name.cmp(&a.name))
    });
    items
}

#[component]
pub fn PolicyRuns(namespace: String, name: String) -> Element {
    let policy_ns = namespace.clone();
    let policy_name = name.clone();
    let mut refresh_tick = use_signal(|| 0u32);
    let mut action_error = use_signal(|| Option::<String>::None);
    let mut restore_busy = use_signal(|| false);

    let backups = use_resource(move || {
        let _ = refresh_tick();
        let ns = policy_ns.clone();
        let policy = policy_name.clone();
        async move {
            let all = api::list_backups().await?;
            Ok::<_, api::ApiClientError>(sort_runs(
                all.into_iter()
                    .filter(|b| matches_policy(b, &ns, &policy))
                    .collect(),
            ))
        }
    });

    use_effect(move || {
        let needs_poll = match &*backups.read_unchecked() {
            Some(Ok(items)) => items.iter().any(|item| {
                matches!(
                    item.phase.as_deref(),
                    None | Some("Pending") | Some("Running")
                )
            }),
            _ => false,
        };
        if !needs_poll {
            return;
        }
        spawn(async move {
            gloo_timers::future::TimeoutFuture::new(2_000).await;
            refresh_tick.set(refresh_tick() + 1);
        });
    });

    rsx! {
        section { class: "page",
            div { class: "page-header",
                div {
                    h1 { "Runs · {name}" }
                    p { class: "muted",
                        "All runs for policy {namespace}/{name}. Pick by date, then Restore on any Succeeded run."
                    }
                }
                div { class: "toolbar",
                    Link { to: Route::Backups {}, class: "btn", "← Backups" }
                    button {
                        class: "btn",
                        r#type: "button",
                        onclick: move |_| refresh_tick.set(refresh_tick() + 1),
                        "Refresh"
                    }
                }
            }

            if let Some(err) = action_error() {
                div { class: "banner error",
                    strong { "Action failed" }
                    p { "{err}" }
                }
            }

            match &*backups.read_unchecked() {
                None => rsx! {
                    div { class: "panel",
                        p { class: "muted empty-state", "Loading runs…" }
                    }
                },
                Some(Err(err)) => rsx! {
                    div { class: "banner error",
                        strong { "Failed to load runs" }
                        p { "{err}" }
                    }
                },
                Some(Ok(items)) => rsx! {
                    div { class: "panel resource-list",
                        if items.is_empty() {
                            p { class: "muted empty-state", "No runs for this policy yet." }
                        } else {
                            for item in items.iter() {
                                {
                                    let item_name = item.name.clone();
                                    let item_ns = item.namespace.clone();
                                    let restore_name = item.name.clone();
                                    let restore_ns = item.namespace.clone();
                                    let can_restore = item.phase.as_deref() == Some("Succeeded");
                                    let progress = item.progress_percent.unwrap_or(0);
                                    let show_progress = matches!(
                                        item.phase.as_deref(),
                                        Some("Running") | Some("Pending")
                                    );
                                    let message = item.message.clone().unwrap_or_default();
                                    let plane = item.data_plane.clone().unwrap_or_default();
                                    let plane_title = match item.assigned_node.as_deref() {
                                        Some(node) if !plane.is_empty() => {
                                            format!("dataPlane={plane} node={node}")
                                        }
                                        _ if !plane.is_empty() => format!("dataPlane={plane}"),
                                        _ => String::new(),
                                    };
                                    let snap_full =
                                        item.last_snapshot_id.clone().unwrap_or_default();
                                    let snap_short = if snap_full.is_empty() {
                                        String::new()
                                    } else {
                                        short_id(&snap_full, 12)
                                    };
                                    let pvcs = item.pvc_names.join(", ");
                                    let when = format_run_when(item);
                                    let restore_when = when.clone();
                                    rsx! {
                                        div { class: "resource-row",
                                            div { class: "resource-id",
                                                span { class: "resource-ns", "{item.namespace}" }
                                                span {
                                                    class: "resource-name",
                                                    title: "Run time (UTC)",
                                                    "{when}"
                                                }
                                                span {
                                                    class: "resource-sub mono-id",
                                                    title: "Run name",
                                                    "{item.name}"
                                                }
                                            }
                                            div {
                                                class: "resource-status",
                                                title: "{message}",
                                                span {
                                                    class: match item.phase.as_deref() {
                                                        Some("Succeeded") => "badge phase-ready",
                                                        Some("Failed") => "badge phase-failed",
                                                        Some("Running") => "badge phase-running",
                                                        _ => "badge",
                                                    },
                                                    "{item.phase.clone().unwrap_or_else(|| \"—\".into())}"
                                                }
                                                if !plane.is_empty() {
                                                    span {
                                                        class: "pill",
                                                        title: "{plane_title}",
                                                        "{plane}"
                                                    }
                                                }
                                                if show_progress {
                                                    div { class: "progress",
                                                        div { class: "progress-track",
                                                            div {
                                                                class: "progress-bar",
                                                                style: "width: {progress}%",
                                                            }
                                                        }
                                                        span { class: "progress-label", "{progress}%" }
                                                    }
                                                }
                                            }
                                            div { class: "resource-actions",
                                                button {
                                                    class: "btn",
                                                    r#type: "button",
                                                    disabled: !can_restore || restore_busy(),
                                                    title: if can_restore {
                                                        "Create a restore from this run"
                                                    } else {
                                                        "Only Succeeded runs can be restored"
                                                    },
                                                    onclick: move |_| {
                                                        if !can_restore {
                                                            return;
                                                        }
                                                        if !confirm_restore(
                                                            &restore_when,
                                                            &restore_name,
                                                            &restore_ns,
                                                        ) {
                                                            return;
                                                        }
                                                        action_error.set(None);
                                                        restore_busy.set(true);
                                                        let name = restore_name.clone();
                                                        let ns = restore_ns.clone();
                                                        let restore_cr =
                                                            format!("{name}-restore");
                                                        spawn(async move {
                                                            let req = CreateRestoreRequest {
                                                                name: restore_cr,
                                                                namespace: ns.clone(),
                                                                backup_ref: name,
                                                                backup_namespace: Some(
                                                                    ns.clone(),
                                                                ),
                                                                snapshot_id: None,
                                                                target_namespace: ns,
                                                                overwrite: false,
                                                            };
                                                            match api::create_restore(&req).await
                                                            {
                                                                Ok(_) => {
                                                                    restore_busy.set(false);
                                                                    refresh_tick.set(
                                                                        refresh_tick() + 1,
                                                                    );
                                                                }
                                                                Err(err) => {
                                                                    action_error
                                                                        .set(Some(err.message));
                                                                    restore_busy.set(false);
                                                                }
                                                            }
                                                        });
                                                    },
                                                    "Restore"
                                                }
                                                button {
                                                    class: "btn btn-icon btn-danger",
                                                    r#type: "button",
                                                    title: "Delete this run",
                                                    onclick: move |_| {
                                                        if !confirm_delete_backup(
                                                            &item_name,
                                                            &item_ns,
                                                        ) {
                                                            return;
                                                        }
                                                        action_error.set(None);
                                                        let name = item_name.clone();
                                                        let ns = item_ns.clone();
                                                        spawn(async move {
                                                            match api::delete_backup(
                                                                &ns, &name,
                                                            )
                                                            .await
                                                            {
                                                                Ok(()) => {
                                                                    refresh_tick.set(
                                                                        refresh_tick() + 1,
                                                                    );
                                                                }
                                                                Err(err) => {
                                                                    action_error
                                                                        .set(Some(err.message));
                                                                }
                                                            }
                                                        });
                                                    },
                                                    "✕"
                                                }
                                            }
                                            div { class: "resource-detail",
                                                div { class: "resource-line",
                                                    span {
                                                        class: "pill",
                                                        title: "Repository",
                                                        "{item.repository_ref}"
                                                    }
                                                    if !pvcs.is_empty() {
                                                        span {
                                                            class: "pill",
                                                            title: "PVCs: {pvcs}",
                                                            "{pvcs}"
                                                        }
                                                    }
                                                    if snap_short.is_empty() {
                                                        span { class: "pill", "No snapshot yet" }
                                                    } else {
                                                        code {
                                                            class: "mono-id pill",
                                                            title: "{snap_full}",
                                                            "{snap_short}"
                                                        }
                                                    }
                                                    if let Some(secs) = item.duration_seconds {
                                                        span {
                                                            class: "pill",
                                                            title: "Duration",
                                                            "{secs}s"
                                                        }
                                                    }
                                                    if let Some(bps) =
                                                        item.throughput_bytes_per_sec
                                                    {
                                                        span {
                                                            class: "pill",
                                                            title: "Throughput",
                                                            "{format_throughput(bps)}"
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                },
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stamp_from_run_name_parses_suffix() {
        assert_eq!(
            stamp_from_run_name("test-20260804045042").as_deref(),
            Some("2026-08-04 04:50 UTC")
        );
    }

    #[test]
    fn format_run_when_prefers_started_at() {
        let item = BackupListItem {
            name: "test-20260804045042".into(),
            namespace: "ns".into(),
            policy_ref: Some("test".into()),
            repository_ref: "repo".into(),
            target_namespace: "ns".into(),
            pvc_names: vec![],
            schedule: None,
            phase: Some("Succeeded".into()),
            message: None,
            last_snapshot_id: None,
            progress_percent: None,
            duration_seconds: None,
            throughput_bytes_per_sec: None,
            started_at: Some("2026-08-04T04:50:42Z".into()),
            created_at: Some("2026-08-04T04:49:00Z".into()),
            data_plane: None,
            assigned_node: None,
        };
        assert_eq!(format_run_when(&item), "2026-08-04 04:50 UTC");
    }

    #[test]
    fn format_run_when_falls_back_to_name_stamp() {
        let item = BackupListItem {
            name: "daily-data-20260803120000".into(),
            namespace: "ns".into(),
            policy_ref: None,
            repository_ref: "repo".into(),
            target_namespace: "ns".into(),
            pvc_names: vec![],
            schedule: None,
            phase: None,
            message: None,
            last_snapshot_id: None,
            progress_percent: None,
            duration_seconds: None,
            throughput_bytes_per_sec: None,
            started_at: None,
            created_at: None,
            data_plane: None,
            assigned_node: None,
        };
        assert_eq!(format_run_when(&item), "2026-08-03 12:00 UTC");
    }
}
