# Coding Assertions

The checks that must pass for code to count as done — and the craft standard behind them.

## Craft (non-negotiable)

We follow **Clean Code** (Uncle Bob), **SOLID**, Refactoring / patterns (Martin Fowler), and GoF where a pattern earns its keep. These are review and merge criteria, not aspirations.

### SOLID (required)

| Principle | Expectation |
| --------- | ----------- |
| **S** — Single Responsibility | One reason to change per type/module; no god files or mixed concerns. |
| **O** — Open/Closed | Extend via new types/traits; avoid editing cores for every variant. |
| **L** — Liskov Substitution | Trait impls honor the contract; no surprise failures or weaker invariants. |
| **I** — Interface Segregation | Narrow traits; callers must not depend on methods they do not use. |
| **D** — Dependency Inversion | Depend on abstractions (`trait`s); wire concretes at the edge. |

### Clean Code

- **Names over comments.** If you need a paragraph to explain a block, rename, extract, or simplify.
- **Comments explain *why*, never *what*.** No narrating the next line. No module essays.
- **Small units.** Prefer focused types and functions over god modules (SRP).
- **Patterns are tools, not décor.** Use Strategy/Factory/etc. when the code hurts without them — never to show off.
- **Boy Scout rule.** Leave the file clearer than you found it.
- **No `missing_docs` theatre.**
- **100% Rust** including UI (Dioxus). No Node/React application code.

## Before commit

| Order | Command | Checks |
| ----- | ------- | ------ |
| 1 | `just fmt-check` | formatting |
| 2 | `just clippy` | lints |
| 3 | `just test` | unit tests |
| 4 | `just build-ui` when touching the UI | WASM bundle |

Or: `just check`

## Before push

| Order | Command | Checks |
| ----- | ------- | ------ |
| 1 | `just build-release` | release build of the operator |
| 2 | `just image` | image build (optional locally) |

## Behavior

If a fix is needed, spawn 1 agent per assertion category (fmt / clippy / tests / UI).
