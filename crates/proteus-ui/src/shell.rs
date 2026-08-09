use crate::theme::ColorSchemeToggle;
use crate::Route;
use dioxus::prelude::*;

const MARK_SVG: &str = include_str!("../assets/brand/mark.svg");
const WORDMARK_SVG: &str = include_str!("../assets/brand/wordmark.svg");

#[component]
pub fn Shell() -> Element {
    rsx! {
        div { class: "shell",
            aside { class: "nav",
                div { class: "brand",
                    div {
                        class: "brand-mark",
                        dangerous_inner_html: MARK_SVG,
                    }
                    div { class: "brand-copy",
                        div {
                            class: "brand-wordmark",
                            dangerous_inner_html: WORDMARK_SVG,
                        }
                        p { "Backup control" }
                    }
                }
                nav {
                    Link { to: Route::Cluster {}, "Cluster" }
                    Link { to: Route::Repositories {}, "Repositories" }
                    Link { to: Route::Backups {}, "Backups" }
                    Link { to: Route::Inventory {}, "Inventory" }
                }
                div { class: "nav-footer",
                    ColorSchemeToggle {}
                }
            }
            main { class: "content",
                Outlet::<Route> {}
            }
        }
    }
}
