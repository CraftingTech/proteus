use dioxus::prelude::*;

const STORAGE_KEY: &str = "proteus-color-scheme";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorScheme {
    Dark,
    Light,
}

impl ColorScheme {
    pub fn id(self) -> &'static str {
        match self {
            ColorScheme::Dark => "dark",
            ColorScheme::Light => "light",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        match id {
            "dark" => Some(ColorScheme::Dark),
            "light" => Some(ColorScheme::Light),
            _ => None,
        }
    }

    pub fn toggle(self) -> Self {
        match self {
            ColorScheme::Dark => ColorScheme::Light,
            ColorScheme::Light => ColorScheme::Dark,
        }
    }
}

pub fn load_color_scheme() -> ColorScheme {
    read_stored_scheme()
        .or_else(system_preference)
        .unwrap_or(ColorScheme::Dark)
}

pub fn apply_color_scheme(scheme: ColorScheme) {
    if let Some(document) = web_sys::window().and_then(|w| w.document()) {
        if let Some(root) = document.document_element() {
            let _ = root.set_attribute("data-theme", scheme.id());
        }
    }
    if let Some(storage) = web_sys::window()
        .and_then(|w| w.local_storage().ok())
        .flatten()
    {
        let _ = storage.set_item(STORAGE_KEY, scheme.id());
    }
}

fn read_stored_scheme() -> Option<ColorScheme> {
    let storage = web_sys::window()
        .and_then(|w| w.local_storage().ok())
        .flatten()?;
    let value = storage.get_item(STORAGE_KEY).ok().flatten()?;
    ColorScheme::from_id(&value)
}

fn system_preference() -> Option<ColorScheme> {
    let list = web_sys::window()?
        .match_media("(prefers-color-scheme: light)")
        .ok()
        .flatten()?;
    Some(if list.matches() {
        ColorScheme::Light
    } else {
        ColorScheme::Dark
    })
}

#[derive(Clone, Copy)]
pub struct ColorSchemeCtx(pub Signal<ColorScheme>);

pub fn use_color_scheme() -> Signal<ColorScheme> {
    use_context::<ColorSchemeCtx>().0
}

#[component]
pub fn ColorSchemeProvider(children: Element) -> Element {
    let scheme = use_signal(|| {
        let initial = load_color_scheme();
        apply_color_scheme(initial);
        initial
    });

    use_effect(move || {
        apply_color_scheme(scheme());
    });

    use_context_provider(|| ColorSchemeCtx(scheme));

    rsx! { {children} }
}

#[component]
pub fn ColorSchemeToggle() -> Element {
    let mut scheme = use_color_scheme();
    let is_dark = scheme() == ColorScheme::Dark;

    rsx! {
        label {
            class: "scheme-toggle-wrap",
            "for": "proteus-scheme-toggle",
            span { class: "visually-hidden",
                if is_dark { "Enable light mode" } else { "Enable dark mode" }
            }
            div {
                class: if is_dark { "scheme-toggle enabled" } else { "scheme-toggle" },
                div { class: "scheme-toggle-icons", "aria-hidden": "true",
                    // sun
                    svg {
                        class: "scheme-icon scheme-icon-sun",
                        view_box: "0 0 24 24",
                        fill: "none",
                        stroke: "currentColor",
                        stroke_width: "2",
                        stroke_linecap: "round",
                        stroke_linejoin: "round",
                        circle { cx: "12", cy: "12", r: "4" }
                        path { d: "M12 2v2M12 20v2M4.93 4.93l1.41 1.41M17.66 17.66l1.41 1.41M2 12h2M20 12h2M4.93 19.07l1.41-1.41M17.66 6.34l1.41-1.41" }
                    }
                    // moon
                    svg {
                        class: "scheme-icon scheme-icon-moon",
                        view_box: "0 0 24 24",
                        fill: "currentColor",
                        path { d: "M21 14.3A8.5 8.5 0 0 1 9.7 3a7 7 0 1 0 11.3 11.3z" }
                    }
                }
                input {
                    id: "proteus-scheme-toggle",
                    name: "proteus-scheme-toggle",
                    r#type: "checkbox",
                    checked: is_dark,
                    onchange: move |_| {
                        let next = scheme().toggle();
                        scheme.set(next);
                    },
                }
            }
        }
    }
}
