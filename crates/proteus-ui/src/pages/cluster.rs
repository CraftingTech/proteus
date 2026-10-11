use crate::api::{
    self, BackupListItem, BackupPolicyListItem, ClusterSnapshot, RepositoryListItem,
    RestoreListItem,
};
use crate::Route;
use dioxus::prelude::*;

#[derive(Clone, PartialEq)]
struct HomeData {
    snap: ClusterSnapshot,
    repos: Vec<RepositoryListItem>,
    policies: Vec<BackupPolicyListItem>,
    backups: Vec<BackupListItem>,
    restores: Vec<RestoreListItem>,
}

async fn load_home() -> Result<HomeData, api::ApiClientError> {
    let snap = api::get_cluster().await?;
    let repos = api::list_repositories().await?;
    let policies = api::list_backup_policies().await?;
    let backups = api::list_backups().await?;
    let restores = api::list_restores().await?;
    Ok(HomeData {
        snap,
        repos,
        policies,
        backups,
        restores,
    })
}

#[component]
pub fn Cluster() -> Element {
    let home = use_resource(|| async move { load_home().await });

    rsx! {
        section { class: "page",
            div { class: "page-header",
                h1 { "Cluster" }
            }

            match &*home.read_unchecked() {
                None => rsx! {
                    div { class: "panel",
                        p { class: "muted", "Loading cluster…" }
                    }
                },
                Some(Err(err)) => rsx! {
                    div { class: "banner error",
                        strong { "Failed to load cluster" }
                        p { "{err}" }
                    }
                },
                Some(Ok(data)) => rsx! { ClusterHome { data: data.clone() } },
            }
        }
    }
}

#[component]
fn ClusterHome(data: HomeData) -> Element {
    let notices = attention(&data);
    let policies = sorted_policies(&data.policies);
    let runs = recent_runs(&data.backups, 6);
    let last = data
        .snap
        .last_reconcile_at
        .clone()
        .unwrap_or_else(|| "—".to_string());
    let last = format_when(&last);

    rsx! {
        HomeStats { data: data.clone() }
        if !notices.is_empty() {
            div { class: "section-bar",
                h2 { "Needs attention" }
            }
            div { class: "panel resource-list",
                for notice in notices {
                    div { class: "resource-row",
                        div { class: "resource-id",
                            span { class: "resource-ns", "{notice.kind}" }
                            span { class: "resource-name", "{notice.name}" }
                        }
                        div { class: "resource-detail",
                            div { class: "resource-line",
                                span { "{notice.detail}" }
                            }
                        }
                        div { class: "resource-status",
                            span { class: "{notice.badge}", "{notice.phase}" }
                        }
                    }
                }
            }
        }
        div { class: "overview-split",
            div {
                div { class: "section-bar",
                    h2 { "Policies" }
                    Link { to: Route::Backups {}, class: "muted", "All backups" }
                }
                PolicyList { policies }
            }
            div {
                div { class: "section-bar",
                    h2 { "Recent runs" }
                    Link { to: Route::Backups {}, class: "muted", "All runs" }
                }
                RunList { runs }
            }
        }
        p { class: "muted meta",
            "Version {data.snap.version} · Last reconcile: {last}"
        }
    }
}

#[component]
fn HomeStats(data: HomeData) -> Element {
    let repos_ready = data
        .repos
        .iter()
        .filter(|r| r.phase.as_deref() == Some("Ready"))
        .count();
    let repos_bad = data.repos.len() - repos_ready;
    let scheduled = data
        .policies
        .iter()
        .filter(|p| has_schedule(p) && !p.paused)
        .count();
    let failed_runs = data
        .backups
        .iter()
        .filter(|b| b.phase.as_deref() == Some("Failed"))
        .count();
    let running_runs = data
        .backups
        .iter()
        .filter(|b| matches!(b.phase.as_deref(), Some("Running") | Some("Pending")))
        .count();
    let active_restores = data
        .restores
        .iter()
        .filter(|r| !matches!(r.phase.as_deref(), Some("Succeeded") | None))
        .count();

    let repo_class = if repos_bad > 0 {
        "stat stat-alert"
    } else {
        "stat"
    };
    let run_class = if failed_runs > 0 {
        "stat stat-alert"
    } else {
        "stat"
    };
    let repo_sub = if data.repos.is_empty() {
        "none yet".to_string()
    } else if repos_bad > 0 {
        format!("{repos_bad} not ready")
    } else {
        format!("{repos_ready} ready")
    };
    let policy_sub = if data.policies.is_empty() {
        "none yet".to_string()
    } else {
        format!("{scheduled} scheduled")
    };
    let run_sub = if data.backups.is_empty() {
        "none yet".to_string()
    } else if failed_runs > 0 {
        format!("{failed_runs} failed")
    } else if running_runs > 0 {
        format!("{running_runs} running")
    } else {
        "all succeeded".to_string()
    };
    let restore_sub = if data.restores.is_empty() {
        "none yet".to_string()
    } else if active_restores > 0 {
        format!("{active_restores} active")
    } else {
        "all succeeded".to_string()
    };

    rsx! {
        div { class: "panel grid",
            Link {
                to: Route::Repositories {},
                class: "{repo_class}",
                strong { "{data.repos.len()}" }
                span { "Repositories" }
                span { class: "stat-sub", "{repo_sub}" }
            }
            Link {
                to: Route::Backups {},
                class: "stat",
                strong { "{data.policies.len()}" }
                span { "Policies" }
                span { class: "stat-sub", "{policy_sub}" }
            }
            Link {
                to: Route::Backups {},
                class: "{run_class}",
                strong { "{data.backups.len()}" }
                span { "Runs" }
                span { class: "stat-sub", "{run_sub}" }
            }
            Link {
                to: Route::Backups {},
                class: "stat",
                strong { "{data.restores.len()}" }
                span { "Restores" }
                span { class: "stat-sub", "{restore_sub}" }
            }
        }
    }
}

#[component]
fn PolicyList(policies: Vec<BackupPolicyListItem>) -> Element {
    rsx! {
        div { class: "panel resource-list",
            if policies.is_empty() {
                p { class: "muted empty-state",
                    "No backup policies yet. "
                    Link { to: Route::Backups {}, "Create one" }
                }
            } else {
                for item in policies {
                    PolicyRow { item }
                }
            }
        }
    }
}

#[component]
fn PolicyRow(item: BackupPolicyListItem) -> Element {
    let ns = item.namespace.clone();
    let name = item.name.clone();
    let pvcs = if item.pvc_names.is_empty() {
        "no PVCs".to_string()
    } else {
        item.pvc_names.join(", ")
    };
    let schedule = item
        .schedule
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("manual");
    let next = item
        .next_run_at
        .as_deref()
        .map(format_when)
        .unwrap_or_else(|| "—".into());
    let phase = item.phase.clone().unwrap_or_else(|| "—".into());
    let badge = phase_badge(item.phase.as_deref());

    rsx! {
        div { class: "resource-row",
            div { class: "resource-id",
                span { class: "resource-ns", "{item.namespace}" }
                Link {
                    to: Route::PolicyRuns { namespace: ns, name },
                    class: "resource-name resource-name-link",
                    "{item.name}"
                }
            }
            div { class: "resource-detail",
                div { class: "resource-line",
                    span { "{pvcs}" }
                    span { class: "pill", "{schedule}" }
                    if has_schedule(&item) {
                        span { "next {next}" }
                    }
                    span { "keep {item.keep_last}" }
                }
            }
            div { class: "resource-status",
                if item.paused {
                    span { class: "badge", "Paused" }
                }
                span { class: "{badge}", "{phase}" }
            }
        }
    }
}

#[component]
fn RunList(runs: Vec<BackupListItem>) -> Element {
    rsx! {
        div { class: "panel resource-list",
            if runs.is_empty() {
                p { class: "muted empty-state", "No backup runs yet." }
            } else {
                for item in runs {
                    RunRow { item }
                }
            }
        }
    }
}

#[component]
fn RunRow(item: BackupListItem) -> Element {
    let when = item
        .started_at
        .as_deref()
        .or(item.created_at.as_deref())
        .map(format_when)
        .or_else(|| stamp_from_run_name(&item.name))
        .unwrap_or_else(|| "—".into());
    let pvcs = if item.pvc_names.is_empty() {
        "no PVCs".to_string()
    } else {
        item.pvc_names.join(", ")
    };
    let phase = item.phase.clone().unwrap_or_else(|| "—".into());
    let badge = phase_badge(item.phase.as_deref());
    let policy = item.policy_ref.clone();
    let ns = item.namespace.clone();
    let plane = item.data_plane.clone();

    rsx! {
        div { class: "resource-row",
            div { class: "resource-id",
                span { class: "resource-ns", "{item.namespace}" }
                if let Some(policy_name) = policy.clone() {
                    Link {
                        to: Route::PolicyRuns {
                            namespace: ns.clone(),
                            name: policy_name,
                        },
                        class: "resource-name resource-name-link",
                        "{item.name}"
                    }
                } else {
                    span { class: "resource-name", "{item.name}" }
                }
            }
            div { class: "resource-detail",
                div { class: "resource-line",
                    span { "{pvcs}" }
                    span { "{when}" }
                    if let Some(plane) = plane {
                        span { class: "pill", "{plane}" }
                    }
                }
            }
            div { class: "resource-status",
                span { class: "{badge}", "{phase}" }
            }
        }
    }
}

struct Notice {
    kind: &'static str,
    name: String,
    detail: String,
    phase: String,
    badge: &'static str,
}

fn attention(data: &HomeData) -> Vec<Notice> {
    let mut out = Vec::new();
    for repo in &data.repos {
        if repo.phase.as_deref() != Some("Ready") {
            out.push(Notice {
                kind: "Repository",
                name: format!("{}/{}", repo.namespace, repo.name),
                detail: repo
                    .message
                    .clone()
                    .unwrap_or_else(|| repo.backend.clone().unwrap_or_default()),
                phase: repo.phase.clone().unwrap_or_else(|| "Unknown".into()),
                badge: phase_badge(repo.phase.as_deref()),
            });
        }
    }
    for policy in &data.policies {
        if matches!(policy.phase.as_deref(), Some("Invalid") | Some("Failed")) {
            out.push(Notice {
                kind: "Policy",
                name: format!("{}/{}", policy.namespace, policy.name),
                detail: policy.message.clone().unwrap_or_default(),
                phase: policy.phase.clone().unwrap_or_else(|| "Unknown".into()),
                badge: phase_badge(policy.phase.as_deref()),
            });
        }
    }
    for run in &data.backups {
        if run.phase.as_deref() == Some("Failed") {
            out.push(Notice {
                kind: "Run",
                name: format!("{}/{}", run.namespace, run.name),
                detail: run.message.clone().unwrap_or_default(),
                phase: run.phase.clone().unwrap_or_else(|| "Unknown".into()),
                badge: phase_badge(run.phase.as_deref()),
            });
        }
    }
    for restore in &data.restores {
        if !matches!(restore.phase.as_deref(), Some("Succeeded")) {
            out.push(Notice {
                kind: "Restore",
                name: format!("{}/{}", restore.namespace, restore.name),
                detail: restore.message.clone().unwrap_or_default(),
                phase: restore.phase.clone().unwrap_or_else(|| "Unknown".into()),
                badge: phase_badge(restore.phase.as_deref()),
            });
        }
    }
    out
}

fn sorted_policies(policies: &[BackupPolicyListItem]) -> Vec<BackupPolicyListItem> {
    let mut items = policies.to_vec();
    items.sort_by(|a, b| {
        let rank = |p: &BackupPolicyListItem| {
            let scheduled = has_schedule(p);
            match (scheduled, p.paused) {
                (true, false) => 0,
                (true, true) => 1,
                _ => 2,
            }
        };
        rank(a)
            .cmp(&rank(b))
            .then_with(|| {
                a.next_run_at
                    .as_deref()
                    .unwrap_or("9999")
                    .cmp(b.next_run_at.as_deref().unwrap_or("9999"))
            })
            .then_with(|| a.name.cmp(&b.name))
    });
    items
}

fn recent_runs(backups: &[BackupListItem], limit: usize) -> Vec<BackupListItem> {
    let mut items = backups.to_vec();
    items.sort_by(|a, b| run_sort_key(b).cmp(&run_sort_key(a)));
    items.truncate(limit);
    items
}

fn run_sort_key(item: &BackupListItem) -> String {
    item.started_at
        .clone()
        .or_else(|| item.created_at.clone())
        .unwrap_or_else(|| item.name.clone())
}

fn has_schedule(policy: &BackupPolicyListItem) -> bool {
    policy
        .schedule
        .as_deref()
        .is_some_and(|s| !s.trim().is_empty())
}

fn phase_badge(phase: Option<&str>) -> &'static str {
    match phase {
        Some("Ready" | "Succeeded") => "badge phase-ready",
        Some("Failed" | "Invalid") => "badge phase-failed",
        Some("Running" | "Pending") => "badge phase-running",
        _ => "badge",
    }
}

fn format_when(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed == "—" {
        return "—".into();
    }
    match trimmed.split_once('T') {
        Some((date, rest)) => {
            let time = rest.get(..5).unwrap_or(rest);
            format!("{date} {time} UTC")
        }
        None => trimmed.to_string(),
    }
}

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
