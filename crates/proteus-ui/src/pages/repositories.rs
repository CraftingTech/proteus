use crate::api::{self, CreateRepositoryBackend, CreateRepositoryRequest};
use dioxus::prelude::*;

#[derive(Clone, Copy, PartialEq, Eq)]
enum BackendKind {
    Local,
    S3,
}

fn confirm_delete(name: &str, namespace: &str) -> bool {
    let msg = format!("Delete repository {name} in namespace {namespace}?");
    web_sys::window()
        .and_then(|w| w.confirm_with_message(&msg).ok())
        .unwrap_or(false)
}

#[component]
pub fn Repositories() -> Element {
    let mut refresh_tick = use_signal(|| 0u32);
    let mut show_form = use_signal(|| false);
    let mut backend_kind = use_signal(|| BackendKind::Local);
    let mut name = use_signal(String::new);
    let mut namespace = use_signal(|| "proteus-system".to_string());
    let mut description = use_signal(String::new);
    let mut local_path = use_signal(|| "/var/lib/proteus/repo".to_string());
    let mut bucket = use_signal(String::new);
    let mut endpoint = use_signal(String::new);
    let mut region = use_signal(|| "us-east-1".to_string());
    let mut prefix = use_signal(String::new);
    let mut credentials_secret_ref = use_signal(String::new);
    let mut force_path_style = use_signal(|| true);
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
            if !names.iter().any(|n| n == "proteus-system") {
                names.insert(0, "proteus-system".to_string());
            }
            namespace_options.set(names);
        }
    });

    let rows = use_resource(move || {
        let _ = refresh_tick();
        async move { api::list_repositories().await }
    });

    // Poll while any row is still reconciling (missing / Pending phase).
    use_effect(move || {
        let needs_poll = match &*rows.read_unchecked() {
            Some(Ok(items)) => items
                .iter()
                .any(|item| matches!(item.phase.as_deref(), None | Some("Pending"))),
            _ => false,
        };
        if !needs_poll {
            return;
        }
        spawn(async move {
            gloo_timers::future::TimeoutFuture::new(7_000).await;
            refresh_tick.set(refresh_tick() + 1);
        });
    });

    let on_create = move |_| {
        if form_busy() {
            return;
        }
        form_error.set(None);
        action_error.set(None);

        let repo_name = name().trim().to_string();
        let repo_ns = namespace().trim().to_string();
        if repo_name.is_empty() {
            form_error.set(Some("name is required".into()));
            return;
        }
        if repo_ns.is_empty() {
            form_error.set(Some("namespace is required".into()));
            return;
        }

        let backend = match backend_kind() {
            BackendKind::Local => {
                let path = local_path().trim().to_string();
                if path.is_empty() {
                    form_error.set(Some("backend.path is required".into()));
                    return;
                }
                CreateRepositoryBackend::Local { path }
            }
            BackendKind::S3 => {
                let bucket = bucket().trim().to_string();
                let secret = credentials_secret_ref().trim().to_string();
                if bucket.is_empty() {
                    form_error.set(Some("backend.bucket is required".into()));
                    return;
                }
                if secret.is_empty() {
                    form_error.set(Some("backend.credentialsSecretRef is required".into()));
                    return;
                }
                let endpoint = endpoint().trim().to_string();
                let region = region().trim().to_string();
                let prefix = prefix().trim().to_string();
                CreateRepositoryBackend::S3 {
                    bucket,
                    prefix: (!prefix.is_empty()).then_some(prefix),
                    endpoint: (!endpoint.is_empty()).then_some(endpoint),
                    region: (!region.is_empty()).then_some(region),
                    credentials_secret_ref: secret,
                    force_path_style: force_path_style(),
                }
            }
        };

        let desc = description().trim().to_string();
        let req = CreateRepositoryRequest {
            name: repo_name,
            namespace: repo_ns,
            description: (!desc.is_empty()).then_some(desc),
            encryption_enabled: false,
            backend,
        };

        form_busy.set(true);
        spawn(async move {
            match api::create_repository(&req).await {
                Ok(_) => {
                    name.set(String::new());
                    description.set(String::new());
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
            h1 { "Repositories" }
            p { class: "lede",
                "Create and manage ProteusRepository custom resources — local path or S3-compatible."
            }

            div { class: "toolbar",
                button {
                    class: "btn",
                    r#type: "button",
                    onclick: move |_| {
                        show_form.set(!show_form());
                        form_error.set(None);
                    },
                    if show_form() { "Cancel" } else { "+ New repository" }
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
                    h2 { "New repository" }

                    div { class: "backend-toggle",
                        label { class: "radio",
                            input {
                                r#type: "radio",
                                name: "backend",
                                checked: backend_kind() == BackendKind::Local,
                                onchange: move |_| backend_kind.set(BackendKind::Local),
                            }
                            " Local"
                        }
                        label { class: "radio",
                            input {
                                r#type: "radio",
                                name: "backend",
                                checked: backend_kind() == BackendKind::S3,
                                onchange: move |_| backend_kind.set(BackendKind::S3),
                            }
                            " S3"
                        }
                    }

                    div { class: "form-grid",
                        label {
                            span { "Name" }
                            input {
                                r#type: "text",
                                value: "{name}",
                                placeholder: "local-repo",
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
                        }
                        label { class: "form-span-2",
                            span { "Description (optional)" }
                            input {
                                r#type: "text",
                                value: "{description}",
                                oninput: move |evt| description.set(evt.value()),
                            }
                        }

                        if backend_kind() == BackendKind::Local {
                            label { class: "form-span-2",
                                span { "Local path" }
                                input {
                                    r#type: "text",
                                    value: "{local_path}",
                                    placeholder: "/var/lib/proteus/repo",
                                    oninput: move |evt| local_path.set(evt.value()),
                                }
                            }
                        } else {
                            label {
                                span { "Bucket" }
                                input {
                                    r#type: "text",
                                    value: "{bucket}",
                                    oninput: move |evt| bucket.set(evt.value()),
                                }
                            }
                            label {
                                span { "Credentials Secret ref" }
                                input {
                                    r#type: "text",
                                    value: "{credentials_secret_ref}",
                                    placeholder: "minio-creds",
                                    oninput: move |evt| credentials_secret_ref.set(evt.value()),
                                }
                            }
                            label {
                                span { "Endpoint" }
                                input {
                                    r#type: "text",
                                    value: "{endpoint}",
                                    placeholder: "http://minio:9000",
                                    oninput: move |evt| endpoint.set(evt.value()),
                                }
                            }
                            label {
                                span { "Region" }
                                input {
                                    r#type: "text",
                                    value: "{region}",
                                    oninput: move |evt| region.set(evt.value()),
                                }
                            }
                            label {
                                span { "Prefix (optional)" }
                                input {
                                    r#type: "text",
                                    value: "{prefix}",
                                    oninput: move |evt| prefix.set(evt.value()),
                                }
                            }
                            label { class: "checkbox",
                                input {
                                    r#type: "checkbox",
                                    checked: force_path_style(),
                                    onchange: move |evt| force_path_style.set(evt.checked()),
                                }
                                " Force path-style (MinIO)"
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
                            if form_busy() { "Creating…" } else { "Create" }
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

            match &*rows.read_unchecked() {
                None => rsx! {
                    div { class: "panel",
                        p { class: "muted", "Loading repositories…" }
                    }
                },
                Some(Err(err)) => rsx! {
                    div { class: "banner error",
                        strong { "Failed to load repositories" }
                        p { "{err}" }
                    }
                },
                Some(Ok(items)) if items.is_empty() => rsx! {
                    div { class: "panel",
                        p { class: "muted", "No repositories yet." }
                    }
                },
                Some(Ok(items)) => rsx! {
                    div { class: "panel",
                        table { class: "table",
                            thead {
                                tr {
                                    th { "Name" }
                                    th { "Namespace" }
                                    th { "Backend" }
                                    th { "Phase" }
                                    th { "Message" }
                                    th { "" }
                                }
                            }
                            tbody {
                                for item in items.iter() {
                                    {
                                        let item_name = item.name.clone();
                                        let item_ns = item.namespace.clone();
                                        rsx! {
                                            tr {
                                                td { "{item.name}" }
                                                td { "{item.namespace}" }
                                                td { "{item.backend.clone().unwrap_or_else(|| \"—\".into())}" }
                                                td {
                                                    span {
                                                        class: match item.phase.as_deref() {
                                                            Some("Ready") => "badge phase-ready",
                                                            Some("Failed") => "badge phase-failed",
                                                            _ => "badge",
                                                        },
                                                        "{item.phase.clone().unwrap_or_else(|| \"—\".into())}"
                                                    }
                                                }
                                                td { class: "muted", "{item.message.clone().unwrap_or_default()}" }
                                                td {
                                                    button {
                                                        class: "btn btn-danger",
                                                        r#type: "button",
                                                        title: "Delete repository",
                                                        onclick: move |_| {
                                                            if !confirm_delete(&item_name, &item_ns) {
                                                                return;
                                                            }
                                                            let name = item_name.clone();
                                                            let ns = item_ns.clone();
                                                            spawn(async move {
                                                                match api::delete_repository(&ns, &name).await {
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
                },
            }
        }
    }
}
