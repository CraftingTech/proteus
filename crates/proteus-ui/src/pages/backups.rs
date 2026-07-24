use crate::api::{
    self, BackupListItem, CreateBackupRequest, CreateRestoreRequest, RepositoryListItem,
};
use dioxus::prelude::*;

fn confirm_delete(kind: &str, name: &str, namespace: &str) -> bool {
    let msg = format!("Delete {kind} {name} in namespace {namespace}?");
    web_sys::window()
        .and_then(|w| w.confirm_with_message(&msg).ok())
        .unwrap_or(false)
}

fn confirm_delete_backup(name: &str, namespace: &str, _has_snapshot: bool) -> bool {
    let msg = format!(
        "Delete backup {name} in {namespace}?\n\nUnreferenced objects in its repository will be removed (other backups' snapshots are kept)."
    );
    web_sys::window()
        .and_then(|w| w.confirm_with_message(&msg).ok())
        .unwrap_or(false)
}

fn ready_repositories(repos: &[RepositoryListItem]) -> Vec<&RepositoryListItem> {
    repos
        .iter()
        .filter(|r| r.phase.as_deref() == Some("Ready"))
        .collect()
}

fn succeeded_backups(backups: &[BackupListItem]) -> Vec<&BackupListItem> {
    backups
        .iter()
        .filter(|b| b.phase.as_deref() == Some("Succeeded"))
        .collect()
}

/// Select value: `namespace/name` so cross-namespace resources stay unambiguous.
fn ns_name_value(namespace: &str, name: &str) -> String {
    format!("{namespace}/{name}")
}

fn repo_select_value(repo: &RepositoryListItem) -> String {
    ns_name_value(&repo.namespace, &repo.name)
}

fn backup_select_value(backup: &BackupListItem) -> String {
    ns_name_value(&backup.namespace, &backup.name)
}

fn parse_ns_name_select(value: &str) -> Option<(String, String)> {
    let (ns, name) = value.split_once('/')?;
    let ns = ns.trim();
    let name = name.trim();
    if ns.is_empty() || name.is_empty() {
        None
    } else {
        Some((ns.to_string(), name.to_string()))
    }
}

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

#[component]
pub fn Backups() -> Element {
    let mut refresh_tick = use_signal(|| 0u32);
    let mut show_form = use_signal(|| false);
    let mut name = use_signal(String::new);
    let mut namespace = use_signal(|| "default".to_string());
    let mut selected_repo = use_signal(String::new);
    let mut selected_pvcs = use_signal(Vec::<String>::new);
    let mut form_error = use_signal(|| Option::<String>::None);
    let mut form_busy = use_signal(|| false);
    let mut action_error = use_signal(|| Option::<String>::None);
    let mut namespace_options = use_signal(Vec::<String>::new);

    let ns_list = use_resource(|| async move { api::list_namespaces().await });
    use_effect(move || {
        if let Some(Ok(items)) = ns_list.read_unchecked().as_ref() {
            let mut names: Vec<String> = items.iter().map(|n| n.name.clone()).collect();
            names.sort();
            names.dedup();
            namespace_options.set(names.clone());
            if !names.iter().any(|n| n == &namespace()) {
                let fallback = names
                    .iter()
                    .find(|n| *n == "default")
                    .cloned()
                    .or_else(|| names.first().cloned())
                    .unwrap_or_else(|| "default".into());
                namespace.set(fallback);
            }
        }
    });

    let repos = use_resource(|| async move { api::list_repositories().await });
    use_effect(move || {
        if let Some(Ok(items)) = repos.read_unchecked().as_ref() {
            let ready = ready_repositories(items);
            if !ready
                .iter()
                .any(|r| repo_select_value(r) == selected_repo())
            {
                selected_repo.set(
                    ready
                        .first()
                        .map(|r| repo_select_value(r))
                        .unwrap_or_default(),
                );
            }
        }
    });

    // PVC candidates for the chosen namespace — refetched whenever `namespace` changes.
    let pvcs = use_resource(move || {
        let ns = namespace();
        async move {
            api::get_inventory(
                Some(ns.as_str()).filter(|s| !s.is_empty()),
                Some("PersistentVolumeClaim"),
                None,
            )
            .await
        }
    });
    use_effect(move || {
        // Drop selections that no longer exist in the current namespace's PVC list.
        if let Some(Ok(items)) = pvcs.read_unchecked().as_ref() {
            let names: Vec<String> = items.iter().map(|i| i.name.clone()).collect();
            let kept: Vec<String> = selected_pvcs()
                .into_iter()
                .filter(|p| names.contains(p))
                .collect();
            if kept != selected_pvcs() {
                selected_pvcs.set(kept);
            }
        }
    });

    let backups = use_resource(move || {
        let _ = refresh_tick();
        async move { api::list_backups().await }
    });

    // Poll while any backup is still Pending/Running.
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
            // Faster poll while Running so progress % stays fresh.
            gloo_timers::future::TimeoutFuture::new(2_000).await;
            refresh_tick.set(refresh_tick() + 1);
        });
    });

    let restores = use_resource(move || {
        let _ = refresh_tick();
        async move { api::list_restores().await }
    });

    // Poll while any restore is still Pending/Running.
    use_effect(move || {
        let needs_poll = match &*restores.read_unchecked() {
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
            gloo_timers::future::TimeoutFuture::new(4_000).await;
            refresh_tick.set(refresh_tick() + 1);
        });
    });

    let mut show_restore_form = use_signal(|| false);
    let mut restore_name = use_signal(String::new);
    let mut restore_namespace = use_signal(|| "default".to_string());
    let mut selected_backup = use_signal(String::new);
    let mut restore_snapshot_id = use_signal(String::new);
    let mut restore_overwrite = use_signal(|| false);
    let mut restore_form_error = use_signal(|| Option::<String>::None);
    let mut restore_form_busy = use_signal(|| false);

    use_effect(move || {
        if let Some(Ok(items)) = backups.read_unchecked().as_ref() {
            let succeeded = succeeded_backups(items);
            if !succeeded
                .iter()
                .any(|b| backup_select_value(b) == selected_backup())
            {
                selected_backup.set(
                    succeeded
                        .first()
                        .map(|b| backup_select_value(b))
                        .unwrap_or_default(),
                );
            }
        }
        if !namespace_options()
            .iter()
            .any(|n| n == &restore_namespace())
        {
            let fallback = namespace_options()
                .iter()
                .find(|n| *n == "default")
                .cloned()
                .or_else(|| namespace_options().first().cloned())
                .unwrap_or_else(|| "default".into());
            restore_namespace.set(fallback);
        }
    });

    let on_create_restore = move |_| {
        if restore_form_busy() {
            return;
        }
        restore_form_error.set(None);
        action_error.set(None);

        let r_name = restore_name().trim().to_string();
        let r_ns = restore_namespace().trim().to_string();
        let backup_value = selected_backup().trim().to_string();
        let snapshot_id = restore_snapshot_id().trim().to_string();

        if r_name.is_empty() {
            restore_form_error.set(Some("name is required".into()));
            return;
        }
        if r_ns.is_empty() {
            restore_form_error.set(Some("target namespace is required".into()));
            return;
        }
        let Some((backup_ns, backup_name)) = parse_ns_name_select(&backup_value) else {
            restore_form_error.set(Some("pick a Succeeded backup".into()));
            return;
        };

        let req = CreateRestoreRequest {
            name: r_name,
            namespace: r_ns.clone(),
            backup_ref: backup_name,
            backup_namespace: Some(backup_ns),
            snapshot_id: if snapshot_id.is_empty() {
                None
            } else {
                Some(snapshot_id)
            },
            target_namespace: r_ns,
            overwrite: restore_overwrite(),
        };

        restore_form_busy.set(true);
        spawn(async move {
            match api::create_restore(&req).await {
                Ok(_) => {
                    restore_name.set(String::new());
                    restore_snapshot_id.set(String::new());
                    restore_overwrite.set(false);
                    show_restore_form.set(false);
                    restore_form_busy.set(false);
                    refresh_tick.set(refresh_tick() + 1);
                }
                Err(err) => {
                    restore_form_error.set(Some(err.message));
                    restore_form_busy.set(false);
                }
            }
        });
    };

    let on_create = move |_| {
        if form_busy() {
            return;
        }
        form_error.set(None);
        action_error.set(None);

        let backup_name = name().trim().to_string();
        let backup_ns = namespace().trim().to_string();
        let repo_value = selected_repo().trim().to_string();
        let pvcs = selected_pvcs();

        if backup_name.is_empty() {
            form_error.set(Some("name is required".into()));
            return;
        }
        if backup_ns.is_empty() {
            form_error.set(Some("namespace is required".into()));
            return;
        }
        let Some((repo_ns, repo_name)) = parse_ns_name_select(&repo_value) else {
            form_error.set(Some("pick a Ready repository".into()));
            return;
        };
        if pvcs.is_empty() {
            form_error.set(Some("pick at least one PVC".into()));
            return;
        }

        let req = CreateBackupRequest {
            name: backup_name,
            namespace: backup_ns.clone(),
            repository_ref: repo_name,
            repository_namespace: Some(repo_ns),
            target_namespace: backup_ns,
            pvc_names: pvcs,
        };

        form_busy.set(true);
        spawn(async move {
            match api::create_backup(&req).await {
                Ok(_) => {
                    name.set(String::new());
                    selected_pvcs.set(Vec::new());
                    show_form.set(false);
                    form_busy.set(false);
                    refresh_tick.set(refresh_tick() + 1);
                }
                Err(err) => {
                    form_error.set(Some(err.message));
                    form_busy.set(false);
                }
            }
        });
    };

    rsx! {
        section { class: "page",
            div { class: "page-header",
                h1 { "Backups" }
                div { class: "toolbar",
                    button {
                        class: "btn",
                        r#type: "button",
                        onclick: move |_| {
                            show_form.set(!show_form());
                            form_error.set(None);
                        },
                        if show_form() { "Cancel" } else { "+ New backup" }
                    }
                    button {
                        class: "btn",
                        r#type: "button",
                        onclick: move |_| refresh_tick.set(refresh_tick() + 1),
                        "Refresh"
                    }
                }
            }

            if show_form() {
                div { class: "panel form-panel",
                    h2 { "New backup" }

                    div { class: "form-grid",
                        label {
                            span { "Name" }
                            input {
                                r#type: "text",
                                value: "{name}",
                                placeholder: "nightly-data",
                                oninput: move |evt| name.set(evt.value()),
                            }
                        }
                        label {
                            span { "Namespace" }
                            select {
                                value: "{namespace}",
                                onchange: move |evt| namespace.set(evt.value()),
                                for ns in namespace_options().iter() {
                                    option {
                                        value: "{ns}",
                                        selected: namespace() == *ns,
                                        "{ns}"
                                    }
                                }
                            }
                            span { class: "field-hint muted", "Also where PVCs are looked up." }
                        }
                        label {
                            span { "Repository (Ready only)" }
                            match &*repos.read_unchecked() {
                                Some(Ok(items)) => {
                                    let ready = ready_repositories(items);
                                    rsx! {
                                        select {
                                            value: "{selected_repo}",
                                            onchange: move |evt| selected_repo.set(evt.value()),
                                            if ready.is_empty() {
                                                option { value: "", "No Ready repositories" }
                                            }
                                            for repo in ready.iter() {
                                                {
                                                    let value = repo_select_value(repo);
                                                    let selected = selected_repo() == value;
                                                    rsx! {
                                                        option {
                                                            value: "{value}",
                                                            selected: selected,
                                                            "{repo.name} ({repo.namespace})"
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                                Some(Err(_)) => rsx! { span { class: "muted", "Failed to load repositories" } },
                                None => rsx! { span { class: "muted", "Loading repositories…" } },
                            }
                        }

                        div { class: "form-span-2 field-block",
                            span { class: "field-label", "PVCs in {namespace}" }
                            match &*pvcs.read_unchecked() {
                                Some(Ok(items)) if items.is_empty() => rsx! {
                                    span { class: "muted", "No PVCs found in this namespace." }
                                },
                                Some(Ok(items)) => rsx! {
                                    div { class: "checkbox-list",
                                        for item in items.iter() {
                                            {
                                                let pvc_name = item.name.clone();
                                                let checked = selected_pvcs().contains(&pvc_name);
                                                rsx! {
                                                    label { class: "checkbox checkbox-row",
                                                        input {
                                                            r#type: "checkbox",
                                                            checked: checked,
                                                            onchange: move |evt| {
                                                                let mut current = selected_pvcs();
                                                                if evt.checked() {
                                                                    if !current.contains(&pvc_name) {
                                                                        current.push(pvc_name.clone());
                                                                    }
                                                                } else {
                                                                    current.retain(|p| p != &pvc_name);
                                                                }
                                                                selected_pvcs.set(current);
                                                            },
                                                        }
                                                        span { "{item.name}" }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                },
                                Some(Err(err)) => rsx! { span { class: "muted", "Failed to load PVCs: {err}" } },
                                None => rsx! { span { class: "muted", "Loading PVCs…" } },
                            }
                        }
                    }

                    if let Some(err) = form_error() {
                        div { class: "banner error",
                            strong { "Create failed" }
                            p { "{err}" }
                        }
                    }

                    div { class: "form-actions",
                        button {
                            class: "btn",
                            r#type: "button",
                            disabled: form_busy(),
                            onclick: on_create,
                            if form_busy() { "Creating…" } else { "Create backup" }
                        }
                    }
                }
            }

            if let Some(err) = action_error() {
                div { class: "banner error",
                    strong { "Action failed" }
                    p { "{err}" }
                }
            }

            div { class: "section-bar",
                h2 { "Backup jobs" }
            }
            match &*backups.read_unchecked() {
                None => rsx! {
                    div { class: "panel",
                        p { class: "muted empty-state", "Loading backups…" }
                    }
                },
                Some(Err(err)) => rsx! {
                    div { class: "banner error",
                        strong { "Failed to load backups" }
                        p { "{err}" }
                    }
                },
                Some(Ok(items)) => rsx! {
                    div { class: "panel resource-list",
                        if items.is_empty() {
                            p { class: "muted empty-state", "No backups yet." }
                        } else {
                            for item in items.iter() {
                                {
                                    let item_name = item.name.clone();
                                    let item_ns = item.namespace.clone();
                                    let has_snapshot = item
                                        .last_snapshot_id
                                        .as_deref()
                                        .map(str::trim)
                                        .is_some_and(|s| !s.is_empty());
                                    let progress = item.progress_percent.unwrap_or(0);
                                    let show_progress = matches!(
                                        item.phase.as_deref(),
                                        Some("Running") | Some("Pending")
                                    );
                                    let message = item.message.clone().unwrap_or_default();
                                    let snap_full = item
                                        .last_snapshot_id
                                        .clone()
                                        .unwrap_or_default();
                                    let snap_short = if snap_full.is_empty() {
                                        String::new()
                                    } else {
                                        short_id(&snap_full, 12)
                                    };
                                    let pvcs = item.pvc_names.join(", ");
                                    rsx! {
                                        div { class: "resource-row",
                                            div { class: "resource-id",
                                                span { class: "resource-ns", "{item.namespace}" }
                                                span { class: "resource-name", "{item.name}" }
                                            }
                                            div { class: "resource-detail",
                                                div { class: "resource-line",
                                                    span { title: "Repository", "{item.repository_ref}" }
                                                    if !pvcs.is_empty() {
                                                        span {
                                                            class: "pill",
                                                            title: "PVCs: {pvcs}",
                                                            "{pvcs}"
                                                        }
                                                    }
                                                }
                                                div { class: "resource-line",
                                                    if snap_short.is_empty() {
                                                        span { "No snapshot yet" }
                                                    } else {
                                                        code {
                                                            class: "mono-id",
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
                                                    if let Some(bps) = item.throughput_bytes_per_sec {
                                                        span {
                                                            class: "pill",
                                                            title: "Throughput",
                                                            "{format_throughput(bps)}"
                                                        }
                                                    }
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
                                                if !message.is_empty() {
                                                    span { class: "status-msg muted", "{message}" }
                                                }
                                            }
                                            div { class: "resource-actions",
                                                button {
                                                    class: "btn btn-danger",
                                                    r#type: "button",
                                                    title: "Delete backup and its snapshot data",
                                                    onclick: move |_| {
                                                        if !confirm_delete_backup(
                                                            &item_name,
                                                            &item_ns,
                                                            has_snapshot,
                                                        ) {
                                                            return;
                                                        }
                                                        let name = item_name.clone();
                                                        let ns = item_ns.clone();
                                                        spawn(async move {
                                                            match api::delete_backup(&ns, &name).await {
                                                                Ok(()) => {
                                                                    action_error.set(None);
                                                                    refresh_tick.set(refresh_tick() + 1);
                                                                }
                                                                Err(err) => {
                                                                    action_error.set(Some(err.message));
                                                                }
                                                            }
                                                        });
                                                    },
                                                    "✕"
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

            div { class: "section-bar",
                h2 { "Restores" }
                div { class: "toolbar",
                    button {
                        class: "btn",
                        r#type: "button",
                        onclick: move |_| {
                            show_restore_form.set(!show_restore_form());
                            restore_form_error.set(None);
                        },
                        if show_restore_form() { "Cancel" } else { "+ New restore" }
                    }
                }
            }

            if show_restore_form() {
                div { class: "panel form-panel",
                    h2 { "New restore" }

                    div { class: "form-grid",
                        label {
                            span { "Name" }
                            input {
                                r#type: "text",
                                value: "{restore_name}",
                                placeholder: "nightly-data-restore",
                                oninput: move |evt| restore_name.set(evt.value()),
                            }
                        }
                        label {
                            span { "Target namespace" }
                            select {
                                value: "{restore_namespace}",
                                onchange: move |evt| restore_namespace.set(evt.value()),
                                for ns in namespace_options().iter() {
                                    option {
                                        value: "{ns}",
                                        selected: restore_namespace() == *ns,
                                        "{ns}"
                                    }
                                }
                            }
                            span { class: "field-hint muted",
                                "PVCs here must already exist, with the names the backup used."
                            }
                        }
                        label {
                            span { "Backup (Succeeded only)" }
                            match &*backups.read_unchecked() {
                                Some(Ok(items)) => {
                                    let succeeded = succeeded_backups(items);
                                    rsx! {
                                        select {
                                            value: "{selected_backup}",
                                            onchange: move |evt| selected_backup.set(evt.value()),
                                            if succeeded.is_empty() {
                                                option { value: "", "No Succeeded backups" }
                                            }
                                            for backup in succeeded.iter() {
                                                {
                                                    let value = backup_select_value(backup);
                                                    let selected = selected_backup() == value;
                                                    rsx! {
                                                        option {
                                                            value: "{value}",
                                                            selected: selected,
                                                            "{backup.name} ({backup.namespace})"
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                                Some(Err(_)) => rsx! { span { class: "muted", "Failed to load backups" } },
                                None => rsx! { span { class: "muted", "Loading backups…" } },
                            }
                        }
                        label {
                            span { "Snapshot id (optional)" }
                            input {
                                r#type: "text",
                                value: "{restore_snapshot_id}",
                                placeholder: "leave blank to use the backup's latest snapshot",
                                oninput: move |evt| restore_snapshot_id.set(evt.value()),
                            }
                        }
                        label { class: "checkbox form-span-2",
                            input {
                                r#type: "checkbox",
                                checked: restore_overwrite(),
                                onchange: move |evt| restore_overwrite.set(evt.checked()),
                            }
                            span { "Overwrite existing data in target PVCs" }
                        }
                    }

                    if let Some(err) = restore_form_error() {
                        div { class: "banner error",
                            strong { "Create failed" }
                            p { "{err}" }
                        }
                    }

                    div { class: "form-actions",
                        button {
                            class: "btn",
                            r#type: "button",
                            disabled: restore_form_busy(),
                            onclick: on_create_restore,
                            if restore_form_busy() { "Creating…" } else { "Create restore" }
                        }
                    }
                }
            }

            match &*restores.read_unchecked() {
                None => rsx! {
                    div { class: "panel",
                        p { class: "muted empty-state", "Loading restores…" }
                    }
                },
                Some(Err(err)) => rsx! {
                    div { class: "banner error",
                        strong { "Failed to load restores" }
                        p { "{err}" }
                    }
                },
                Some(Ok(items)) => rsx! {
                    div { class: "panel resource-list",
                        if items.is_empty() {
                            p { class: "muted empty-state", "No restores yet." }
                        } else {
                            for item in items.iter() {
                                {
                                    let item_name = item.name.clone();
                                    let item_ns = item.namespace.clone();
                                    let message = item.message.clone().unwrap_or_default();
                                    let snap_full = item
                                        .restored_snapshot_id
                                        .clone()
                                        .unwrap_or_default();
                                    let snap_short = if snap_full.is_empty() {
                                        String::new()
                                    } else {
                                        short_id(&snap_full, 12)
                                    };
                                    rsx! {
                                        div { class: "resource-row",
                                            div { class: "resource-id",
                                                span { class: "resource-ns", "{item.namespace}" }
                                                span { class: "resource-name", "{item.name}" }
                                            }
                                            div { class: "resource-detail",
                                                div { class: "resource-line",
                                                    span { title: "Source backup", "{item.backup_ref}" }
                                                    span {
                                                        class: "pill",
                                                        title: "Target namespace",
                                                        "→ {item.target_namespace}"
                                                    }
                                                }
                                                div { class: "resource-line",
                                                    if snap_short.is_empty() {
                                                        span { "No snapshot" }
                                                    } else {
                                                        code {
                                                            class: "mono-id",
                                                            title: "{snap_full}",
                                                            "{snap_short}"
                                                        }
                                                    }
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
                                                if !message.is_empty() {
                                                    span { class: "status-msg muted", "{message}" }
                                                }
                                            }
                                            div { class: "resource-actions",
                                                button {
                                                    class: "btn btn-danger",
                                                    r#type: "button",
                                                    title: "Delete restore",
                                                    onclick: move |_| {
                                                        if !confirm_delete(
                                                            "restore",
                                                            &item_name,
                                                            &item_ns,
                                                        ) {
                                                            return;
                                                        }
                                                        let name = item_name.clone();
                                                        let ns = item_ns.clone();
                                                        spawn(async move {
                                                            match api::delete_restore(&ns, &name)
                                                                .await
                                                            {
                                                                Ok(()) => {
                                                                    action_error.set(None);
                                                                    refresh_tick
                                                                        .set(refresh_tick() + 1);
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
