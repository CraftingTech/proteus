use dioxus::prelude::*;

#[component]
pub fn Pvcs() -> Element {
    rsx! {
        section { class: "page",
            h1 { "PVCs" }
            p { class: "lede",
                "MVP inventory of PersistentVolumeClaims on this cluster — the primary backup targets."
            }
            div { class: "panel",
                table { class: "table",
                    thead {
                        tr {
                            th { "Name" }
                            th { "Namespace" }
                            th { "Storage class" }
                            th { "Size" }
                            th { "" }
                        }
                    }
                    tbody {
                        tr {
                            td { colspan: 5, class: "muted", "PVC listing API not wired yet." }
                        }
                    }
                }
                p {
                    span { class: "badge", "MVP" }
                    " "
                    span { class: "muted", "single cluster · PVC-centric" }
                }
            }
        }
    }
}
