# Brainstorm: Proteus visual identity

- **Approved**: 2026-08-08
- **Issue**: [#57](https://github.com/CraftingTech/proteus/issues/57)
- **Scope**: identity only (mark, wordmark, tokens, in-app theme compare). UX page polish = later.

## Refined idea

Proteus’s Launch identity is a **shield inside a filled heptagon** (Kubernetes-inspired badge), using the K8s-adjacent blue `#3069de` as the primary brand color. Existing SVGs under `~/workspace/tmp/proteus` (`shield`, `name`, `full` × black/white/`3069de`) are raw material, not the final logo.

In the ops shell nav: **heptagon+shield mark** beside the **wordmark letterforms from the name SVG** (the **O is a heptagon**, echoing Kube) — not IBM Plex text for the brand word.

Operators compared three accent themes in-app; **`kube` (`#3069de`) locked** as the product accent (switcher removed). UX redesign of pages is out of scope for this pass.

## Open assumptions (locked for planning unless revised)

| Item | Assumption |
| ---- | ---------- |
| Default theme | `kube` (`#3069de`) |
| Theme depth | Accent (+ brand mark fill) via CSS variables; no light-mode / full surface redesign |
| Persistence | `localStorage` key for selected theme |
| Wordmark | Ship `name` SVG (or cleaned derivative); if O is not clearly heptagonal, redraw that glyph |
| Favicon | Heptagon+shield mark |
| Source kit | Copy curated assets into repo (`crates/proteus-ui/assets/brand/`); tmp path is not the source of truth after import |

## Risks

- Traced shield paths may muddy at 16–28px → may need simplified mark for favicon/nav
- Composing heptagon + shield is new artwork, not a drop-in of current files
- Wordmark as SVG vs font: SVG is required to keep the heptagon O
