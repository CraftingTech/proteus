use dioxus::prelude::*;

#[component]
pub fn Cluster() -> Element {
    rsx! {
        section { class: "page",
            h1 { "Cluster" }
            p { class: "lede",
                "Single-cluster overview. Live metrics come from the embedded API once the controller is running."
            }
            div { class: "panel grid",
                div { class: "stat",
                    strong { "—" }
                    span { "Repositories" }
                }
                div { class: "stat",
                    strong { "—" }
                    span { "Backups" }
                }
                div { class: "stat",
                    strong { "—" }
                    span { "Restores" }
                }
            }
        }
    }
}
