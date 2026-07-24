use crate::Route;
use dioxus::prelude::*;

#[component]
pub fn Shell() -> Element {
    rsx! {
        div { class: "shell",
            aside { class: "nav",
                div { class: "brand",
                    span { class: "brand-mark", "P" }
                    div {
                        strong { "Proteus" }
                        p { "Backup control" }
                    }
                }
                nav {
                    Link { to: Route::Cluster {}, "Cluster" }
                    Link { to: Route::Repositories {}, "Repositories" }
                    Link { to: Route::Backups {}, "Backups" }
                    Link { to: Route::Pvcs {}, "PVCs" }
                }
            }
            main { class: "content",
                Outlet::<Route> {}
            }
        }
    }
}
