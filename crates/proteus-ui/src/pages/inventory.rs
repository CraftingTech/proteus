use crate::api;
use crate::components::NamespaceSelect;
use dioxus::prelude::*;

const KIND_OPTIONS: &[(&str, &str)] = &[
    ("All", "All kinds"),
    ("Deployment", "Deployments"),
    ("Pod", "Pods"),
    ("Service", "Services"),
    ("PersistentVolumeClaim", "PVCs"),
    ("ConfigMap", "ConfigMaps"),
    ("Secret", "Secrets"),
];

const SEARCH_DEBOUNCE_MS: u32 = 280;

#[component]
pub fn Inventory() -> Element {
    let namespace = use_signal(String::new);
    let mut kind = use_signal(|| "All".to_string());
    let mut query = use_signal(String::new);
    let mut namespace_options = use_signal(Vec::<String>::new);

    let ns_list = use_resource(|| async move { api::list_namespaces().await });

    use_effect(move || {
        if let Some(Ok(items)) = ns_list.read_unchecked().as_ref() {
            let mut names: Vec<String> = items.iter().map(|n| n.name.clone()).collect();
            names.sort();
            names.dedup();
            namespace_options.set(names);
        }
    });

    let rows = use_resource(move || {
        let ns = namespace();
        let kind = kind();
        let q = query();
        async move {
            let delay = if q.is_empty() { 40 } else { SEARCH_DEBOUNCE_MS };
            gloo_timers::future::TimeoutFuture::new(delay).await;
            api::get_inventory(
                Some(ns.as_str()).filter(|s| !s.is_empty()),
                Some(kind.as_str()),
                Some(q.as_str()).filter(|s| !s.is_empty()),
            )
            .await
        }
    });

    // Fallback: derive namespaces from inventory rows if /namespaces is unavailable.
    use_effect(move || {
        if !namespace_options().is_empty() {
            return;
        }
        if let Some(Ok(items)) = rows.read_unchecked().as_ref() {
            let mut names: Vec<String> = items
                .iter()
                .map(|i| i.namespace.clone())
                .filter(|n| !n.is_empty())
                .collect();
            names.sort();
            names.dedup();
            if !names.is_empty() {
                namespace_options.set(names);
            }
        }
    });

    rsx! {
        section { class: "page",
            h1 { "Inventory" }
            p { class: "lede",
                "Cluster objects Proteus can see — metadata only. Secrets never expose values."
            }

            div { class: "panel filters",
                div { class: "filter-row",
                    NamespaceSelect {
                        selected: namespace,
                        options: namespace_options(),
                    }

                    label { class: "filter-kind",
                        span { "Kind" }
                        select {
                            value: "{kind}",
                            onchange: move |evt| kind.set(evt.value()),
                            for (value, label) in KIND_OPTIONS {
                                option {
                                    value: "{value}",
                                    selected: kind() == *value,
                                    "{label}"
                                }
                            }
                        }
                    }

                    label { class: "filter-search",
                        span { "Name" }
                        input {
                            r#type: "search",
                            placeholder: "Filter by name…",
                            value: "{query}",
                            oninput: move |evt| query.set(evt.value()),
                        }
                    }
                }
            }

            match &*rows.read_unchecked() {
                None => rsx! {
                    div { class: "panel",
                        p { class: "muted", "Loading inventory…" }
                    }
                },
                Some(Err(err)) => rsx! {
                    div { class: "banner error",
                        strong { "Failed to load inventory" }
                        p { "{err}" }
                    }
                },
                Some(Ok(items)) => rsx! {
                    div { class: "panel",
                        p { class: "meta muted", "{items.len()} objects" }
                        table { class: "table",
                            thead {
                                tr {
                                    th { "Kind" }
                                    th { "Name" }
                                    th { "Namespace" }
                                    th { "Extra" }
                                }
                            }
                            tbody {
                                if items.is_empty() {
                                    tr {
                                        td { colspan: 4, class: "muted", "No matching objects." }
                                    }
                                } else {
                                    for item in items.iter() {
                                        tr {
                                            td { "{item.kind}" }
                                            td { "{item.name}" }
                                            td { "{item.namespace}" }
                                            td { class: "muted", "{item.extra.clone().unwrap_or_default()}" }
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
