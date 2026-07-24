use dioxus::prelude::*;

#[component]
pub fn Repositories() -> Element {
    rsx! {
        section { class: "page",
            h1 { "Repositories" }
            p { class: "lede",
                "Configure where backups land — S3-compatible buckets or local paths. Wired to ProteusRepository CRs."
            }
            div { class: "panel",
                p { class: "muted", "No repositories yet. UI create-flow lands next." }
            }
        }
    }
}
