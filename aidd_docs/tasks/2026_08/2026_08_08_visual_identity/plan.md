---
objective: "Shell shows heptagon+shield mark and name wordmark; three accent themes switchable in-app; brand assets and design.md are the repo source of truth."
status: implemented
issue: https://github.com/CraftingTech/proteus/issues/57
---

# Plan: Visual identity (#57)

## Overview

| Field      | Value |
| ---------- | ----- |
| **Goal**   | Ship Proteus brand kit (heptagon+shield, wordmark) + 3 in-app accent themes for compare |
| **Source** | [`brainstorm.md`](./brainstorm.md), [#57](https://github.com/CraftingTech/proteus/issues/57) |

## Phases

| #   | Phase | File |
| --- | ----- | ---- |
| 1   | Brand assets (heptagon mark + wordmark + favicon) | [`phase-1.md`](./phase-1.md) |
| 2   | Theme tokens + in-app switcher | [`phase-2.md`](./phase-2.md) |
| 3   | Shell brand chrome | [`phase-3.md`](./phase-3.md) |
| 4   | Docs + README surface | [`phase-4.md`](./phase-4.md) |

## Resources

| Source | Verified |
| ------ | -------- |
| https://github.com/CraftingTech/proteus/issues/57 | Brand kit + apply in UI + design.md |
| `~/workspace/tmp/proteus/{shield,name,full}/{black,white,3069de}.svg` | Source artwork (shield / wordmark / lockup) |

## Decisions

| Decision | Why |
| -------- | --- |
| Mark = filled heptagon + shield (not shield alone) | Product intent; Kube-inspired badge |
| Nav = mark + name SVG wordmark (heptagon O), not Plex text | Keep letterforms; O recalls Kube |
| Three themes for in-app compare, then **lock `kube`** | Compare done 2026-08-08; teal/amber switcher removed |
| Persist theme in `localStorage` | Dropped once kube locked |
| Curated assets live under `crates/proteus-ui/assets/brand/` | Repo owns the kit; tmp is import only |
