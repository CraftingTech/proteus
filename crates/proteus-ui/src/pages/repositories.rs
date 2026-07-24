use crate::api;
use dioxus::prelude::*;

#[component]
pub fn Repositories() -> Element {
    let rows = use_resource(|| async move { api::list_repositories().await });

    rsx! {
        section { class: "page",
            h1 { "Repositories" }
            p { class: "lede",
                "ProteusRepository custom resources — S3-compatible or local backends."
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
                                }
                            }
                            tbody {
                                for item in items.iter() {
                                    tr {
                                        td { "{item.name}" }
                                        td { "{item.namespace}" }
                                        td { "{item.backend.clone().unwrap_or_else(|| \"—\".into())}" }
                                        td { "{item.phase.clone().unwrap_or_else(|| \"—\".into())}" }
                                        td { class: "muted", "{item.message.clone().unwrap_or_default()}" }
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
