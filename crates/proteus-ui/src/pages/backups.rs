use crate::api::{self, CreateBackupRequest, RepositoryListItem};
use dioxus::prelude::*;

fn confirm_delete(name: &str, namespace: &str) -> bool {
    let msg = format!("Delete backup {name} in namespace {namespace}?");
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

/// Select value: `namespace/name` so cross-namespace repos stay unambiguous.
fn repo_select_value(repo: &RepositoryListItem) -> String {
    format!("{}/{}", repo.namespace, repo.name)
}

fn parse_repo_select(value: &str) -> Option<(String, String)> {
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
            if !ready.iter().any(|r| repo_select_value(r) == selected_repo()) {
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
            gloo_timers::future::TimeoutFuture::new(4_000).await;
            refresh_tick.set(refresh_tick() + 1);
        });
    });

    let restores = use_resource(|| async move { api::list_restores().await });

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
        let Some((repo_ns, repo_name)) = parse_repo_select(&repo_value) else {
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
            h1 { "Backups" }
            p { class: "lede",
                "ProteusBackup jobs and related ProteusRestore objects."
            }

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

            h2 { "Backup jobs" }
            match &*backups.read_unchecked() {
                None => rsx! {
                    div { class: "panel",
                        p { class: "muted", "Loading backups…" }
                    }
                },
                Some(Err(err)) => rsx! {
                    div { class: "banner error",
                        strong { "Failed to load backups" }
                        p { "{err}" }
                    }
                },
                Some(Ok(items)) => rsx! {
                    div { class: "panel",
                        table { class: "table",
                            thead {
                                tr {
                                    th { "Name" }
                                    th { "Namespace" }
                                    th { "Repository" }
                                    th { "PVCs" }
                                    th { "Phase" }
                                    th { "Snapshot" }
                                    th { "Message" }
                                    th { "" }
                                }
                            }
                            tbody {
                                if items.is_empty() {
                                    tr {
                                        td { colspan: 8, class: "muted", "No backups yet." }
                                    }
                                } else {
                                    for item in items.iter() {
                                        {
                                            let item_name = item.name.clone();
                                            let item_ns = item.namespace.clone();
                                            rsx! {
                                                tr {
                                                    td { "{item.name}" }
                                                    td { "{item.namespace}" }
                                                    td { "{item.repository_ref}" }
                                                    td { "{item.pvc_names.join(\", \")}" }
                                                    td {
                                                        span {
                                                            class: match item.phase.as_deref() {
                                                                Some("Succeeded") => "badge phase-ready",
                                                                Some("Failed") => "badge phase-failed",
                                                                _ => "badge",
                                                            },
                                                            "{item.phase.clone().unwrap_or_else(|| \"—\".into())}"
                                                        }
                                                    }
                                                    td {
                                                        {
                                                            let full = item
                                                                .last_snapshot_id
                                                                .clone()
                                                                .unwrap_or_default();
                                                            let short = if full.is_empty() {
                                                                "—".to_string()
                                                            } else {
                                                                short_id(&full, 12)
                                                            };
                                                            rsx! {
                                                                code {
                                                                    class: "mono-id",
                                                                    title: "{full}",
                                                                    "{short}"
                                                                }
                                                            }
                                                        }
                                                    }
                                                    td {
                                                        span {
                                                            class: "cell-clip muted",
                                                            title: "{item.message.clone().unwrap_or_default()}",
                                                            "{item.message.clone().unwrap_or_default()}"
                                                        }
                                                    }
                                                    td {
                                                        button {
                                                            class: "btn btn-danger",
                                                            r#type: "button",
                                                            title: "Delete backup",
                                                            onclick: move |_| {
                                                                if !confirm_delete(&item_name, &item_ns) {
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
                        }
                    }
                },
            }

            h2 { "Restores" }
            match &*restores.read_unchecked() {
                None => rsx! {
                    div { class: "panel",
                        p { class: "muted", "Loading restores…" }
                    }
                },
                Some(Err(err)) => rsx! {
                    div { class: "banner error",
                        strong { "Failed to load restores" }
                        p { "{err}" }
                    }
                },
                Some(Ok(items)) => rsx! {
                    div { class: "panel",
                        table { class: "table",
                            thead {
                                tr {
                                    th { "Name" }
                                    th { "Namespace" }
                                    th { "Backup" }
                                    th { "Target NS" }
                                    th { "Phase" }
                                }
                            }
                            tbody {
                                if items.is_empty() {
                                    tr {
                                        td { colspan: 5, class: "muted", "No restores yet." }
                                    }
                                } else {
                                    for item in items.iter() {
                                        tr {
                                            td { "{item.name}" }
                                            td { "{item.namespace}" }
                                            td { "{item.backup_ref}" }
                                            td { "{item.target_namespace}" }
                                            td { "{item.phase.clone().unwrap_or_else(|| \"—\".into())}" }
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
