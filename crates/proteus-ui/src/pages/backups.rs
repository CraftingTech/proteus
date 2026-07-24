use crate::api;
use dioxus::prelude::*;

#[component]
pub fn Backups() -> Element {
    let backups = use_resource(|| async move { api::list_backups().await });
    let restores = use_resource(|| async move { api::list_restores().await });

    rsx! {
        section { class: "page",
            h1 { "Backups" }
            p { class: "lede",
                "ProteusBackup jobs and related ProteusRestore objects."
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
                                    th { "Schedule" }
                                    th { "Phase" }
                                }
                            }
                            tbody {
                                if items.is_empty() {
                                    tr {
                                        td { colspan: 5, class: "muted", "No backups yet." }
                                    }
                                } else {
                                    for item in items.iter() {
                                        tr {
                                            td { "{item.name}" }
                                            td { "{item.namespace}" }
                                            td { "{item.repository_ref}" }
                                            td { "{item.schedule.clone().unwrap_or_else(|| \"—\".into())}" }
                                            td { "{item.phase.clone().unwrap_or_else(|| \"—\".into())}" }
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
