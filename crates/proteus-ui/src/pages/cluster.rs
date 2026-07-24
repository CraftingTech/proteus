use crate::api::{self, ClusterSnapshot};
use dioxus::prelude::*;

#[component]
pub fn Cluster() -> Element {
    let snapshot = use_resource(|| async move { api::get_cluster().await });

    rsx! {
        section { class: "page",
            div { class: "page-header",
                h1 { "Cluster" }
            }

            match &*snapshot.read_unchecked() {
                None => rsx! {
                    div { class: "panel",
                        p { class: "muted", "Loading cluster snapshot…" }
                    }
                },
                Some(Err(err)) => rsx! {
                    div { class: "banner error",
                        strong { "Failed to load cluster" }
                        p { "{err}" }
                    }
                },
                Some(Ok(snap)) => rsx! { ClusterStats { snap: snap.clone() } },
            }
        }
    }
}

#[component]
fn ClusterStats(snap: ClusterSnapshot) -> Element {
    let last = snap
        .last_reconcile_at
        .clone()
        .unwrap_or_else(|| "—".to_string());

    rsx! {
        div { class: "panel grid",
            div { class: "stat",
                strong { "{snap.repositories}" }
                span { "Repositories" }
            }
            div { class: "stat",
                strong { "{snap.backups}" }
                span { "Backups" }
            }
            div { class: "stat",
                strong { "{snap.restores}" }
                span { "Restores" }
            }
        }
        p { class: "muted meta",
            "Version {snap.version} · Last reconcile: {last}"
        }
    }
}
