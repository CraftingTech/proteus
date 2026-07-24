mod pages;
mod shell;

use dioxus::prelude::*;
use pages::{Backups, Cluster, Pvcs, Repositories};
use shell::Shell;

const STYLES: Asset = asset!("/assets/styles.css");

fn main() {
    dioxus::launch(App);
}

#[derive(Clone, Routable, Debug, PartialEq)]
#[rustfmt::skip]
enum Route {
    #[layout(Shell)]
      #[route("/")]
      Cluster {},
      #[route("/repositories")]
      Repositories {},
      #[route("/backups")]
      Backups {},
      #[route("/pvcs")]
      Pvcs {},
}

#[component]
fn App() -> Element {
    rsx! {
        document::Link { rel: "stylesheet", href: STYLES }
        Router::<Route> {}
    }
}
