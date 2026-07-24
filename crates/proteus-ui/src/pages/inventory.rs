use crate::api;
use dioxus::prelude::*;

const KIND_OPTIONS: &[&str] = &[
    "All",
    "Deployment",
    "Pod",
    "Service",
    "PersistentVolumeClaim",
    "ConfigMap",
    "Secret",
];

#[component]
pub fn Inventory() -> Element {
    let mut namespace = use_signal(String::new);
    let mut kind = use_signal(|| "All".to_string());
    let mut query = use_signal(String::new);
    let mut applied_ns = use_signal(String::new);
    let mut applied_kind = use_signal(|| "All".to_string());
    let mut applied_q = use_signal(String::new);

    let rows = use_resource(move || {
        let ns = applied_ns();
        let kind = applied_kind();
        let q = applied_q();
        async move {
            api::get_inventory(
                Some(ns.as_str()).filter(|s| !s.is_empty()),
                Some(kind.as_str()),
                Some(q.as_str()).filter(|s| !s.is_empty()),
            )
            .await
        }
    });

    rsx! {
        section { class: "page",
            h1 { "Inventory" }
            p { class: "lede",
                "Cluster objects Proteus can see — metadata only. Secrets never expose values."
            }

            div { class: "panel filters",
                label {
                    span { "Namespace" }
                    input {
                        r#type: "text",
                        placeholder: "All namespaces",
                        value: "{namespace}",
                        oninput: move |evt| namespace.set(evt.value()),
                    }
                }
                label {
                    span { "Kind" }
                    select {
                        value: "{kind}",
                        onchange: move |evt| kind.set(evt.value()),
                        for option in KIND_OPTIONS {
                            option { value: "{option}", selected: kind() == *option, "{option}" }
                        }
                    }
                }
                label {
                    span { "Name" }
                    input {
                        r#type: "search",
                        placeholder: "Search name…",
                        value: "{query}",
                        oninput: move |evt| query.set(evt.value()),
                    }
                }
                button {
                    class: "btn",
                    r#type: "button",
                    onclick: move |_| {
                        applied_ns.set(namespace());
                        applied_kind.set(kind());
                        applied_q.set(query());
                    },
                    "Apply filters"
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
