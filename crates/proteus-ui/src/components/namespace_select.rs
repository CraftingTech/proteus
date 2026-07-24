use dioxus::prelude::*;

/// Searchable select: type to filter, pick from `options` only (no free-form values).
#[component]
pub fn NamespaceSelect(selected: Signal<String>, options: Vec<String>) -> Element {
    let mut open = use_signal(|| false);
    let mut draft = use_signal(String::new);

    let filter = draft().to_lowercase();
    let filtered: Vec<String> = options
        .iter()
        .filter(|name| filter.is_empty() || name.to_lowercase().contains(&filter))
        .cloned()
        .collect();

    let display = if !open() && !selected().is_empty() {
        selected()
    } else if open() {
        draft()
    } else {
        String::new()
    };

    rsx! {
        div { class: "combobox",
            label {
                span { "Namespace" }
                div { class: "combobox-field",
                    input {
                        r#type: "text",
                        role: "combobox",
                        "aria-expanded": if open() { "true" } else { "false" },
                        "aria-autocomplete": "list",
                        placeholder: "All namespaces",
                        value: "{display}",
                        onfocus: move |_| {
                            open.set(true);
                            draft.set(String::new());
                        },
                        oninput: move |evt| {
                            open.set(true);
                            draft.set(evt.value());
                        },
                        onkeydown: move |evt| {
                            if evt.key() == Key::Escape {
                                open.set(false);
                                draft.set(String::new());
                            }
                            if evt.key() == Key::Enter {
                                if let Some(name) = filtered.first().cloned() {
                                    selected.set(name);
                                    open.set(false);
                                    draft.set(String::new());
                                }
                            }
                        },
                        onblur: move |_| {
                            spawn(async move {
                                gloo_timers::future::TimeoutFuture::new(120).await;
                                open.set(false);
                                draft.set(String::new());
                            });
                        },
                    }
                    if !selected().is_empty() {
                        button {
                            class: "combobox-clear",
                            r#type: "button",
                            title: "Clear namespace",
                            onclick: move |_| {
                                selected.set(String::new());
                                draft.set(String::new());
                                open.set(false);
                            },
                            "×"
                        }
                    }
                }
            }

            if open() {
                ul { class: "combobox-menu", role: "listbox",
                    li {
                        class: if selected().is_empty() { "combobox-option active" } else { "combobox-option" },
                        role: "option",
                        onmousedown: move |evt| {
                            evt.prevent_default();
                            selected.set(String::new());
                            open.set(false);
                            draft.set(String::new());
                        },
                        "All namespaces"
                    }
                    if options.is_empty() {
                        li { class: "combobox-option muted", "Loading namespaces…" }
                    } else if filtered.is_empty() {
                        li { class: "combobox-option muted", "No matching namespace" }
                    } else {
                        for name in filtered.iter().cloned() {
                            li {
                                class: if selected() == name { "combobox-option active" } else { "combobox-option" },
                                role: "option",
                                onmousedown: move |evt| {
                                    evt.prevent_default();
                                    selected.set(name.clone());
                                    open.set(false);
                                    draft.set(String::new());
                                },
                                "{name}"
                            }
                        }
                    }
                }
            }
        }
    }
}
