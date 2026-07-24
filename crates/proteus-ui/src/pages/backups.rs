use dioxus::prelude::*;

#[component]
pub fn Backups() -> Element {
    rsx! {
        section { class: "page",
            h1 { "Backups" }
            p { class: "lede",
                "Schedule and inspect backup jobs. Backed by ProteusBackup custom resources."
            }
            div { class: "panel",
                table { class: "table",
                    thead {
                        tr {
                            th { "Name" }
                            th { "Namespace" }
                            th { "Schedule" }
                            th { "Phase" }
                        }
                    }
                    tbody {
                        tr {
                            td { colspan: 4, class: "muted", "No backups yet." }
                        }
                    }
                }
            }
        }
    }
}
