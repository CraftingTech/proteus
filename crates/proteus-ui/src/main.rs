mod api;
mod components;
mod pages;
mod shell;
mod theme;

use dioxus::prelude::*;
use pages::{Backups, Cluster, Inventory, PolicyRuns, Repositories};
use shell::Shell;
use theme::ColorSchemeProvider;

const STYLES: Asset = asset!("/assets/styles.css");
const FAVICON: Asset = asset!("/assets/favicon.svg");

fn main() {
    dioxus::launch(App);
}

#[derive(Clone, Routable, Debug, PartialEq)]
#[rustfmt::skip]
pub enum Route {
    #[layout(Shell)]
      #[route("/")]
      Cluster {},
      #[route("/repositories")]
      Repositories {},
      // More specific than /backups — must be registered first.
      #[route("/backups/policy/:namespace/:name")]
      PolicyRuns { namespace: String, name: String },
      #[route("/backups")]
      Backups {},
      #[route("/inventory")]
      Inventory {},
}

#[component]
fn App() -> Element {
    rsx! {
        document::Link { rel: "stylesheet", href: STYLES }
        document::Link { rel: "icon", r#type: "image/svg+xml", href: FAVICON }
        ColorSchemeProvider {
            Router::<Route> {}
        }
    }
}
