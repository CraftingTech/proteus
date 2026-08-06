//! keepLast selection for Succeeded backup runs of a policy.

/// A Succeeded run candidate for retention pruning.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SucceededRunRef {
    pub namespace: String,
    pub name: String,
    /// Sort key: prefer `last_success_at`, else creation timestamp (RFC3339 / kube time).
    pub sort_key: String,
}

/// Oldest Succeeded runs beyond `keep_last` (newest kept). Empty when nothing to prune.
pub fn select_prunable(mut runs: Vec<SucceededRunRef>, keep_last: u32) -> Vec<SucceededRunRef> {
    if keep_last == 0 {
        return runs;
    }
    runs.sort_by(|a, b| b.sort_key.cmp(&a.sort_key));
    let keep = keep_last as usize;
    if runs.len() <= keep {
        return Vec::new();
    }
    runs.into_iter().skip(keep).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(name: &str, key: &str) -> SucceededRunRef {
        SucceededRunRef {
            namespace: "ns".into(),
            name: name.into(),
            sort_key: key.into(),
        }
    }

    #[test]
    fn keep_last_two_drops_oldest() {
        let prunable = select_prunable(
            vec![
                run("a", "2026-01-01T00:00:00Z"),
                run("b", "2026-01-02T00:00:00Z"),
                run("c", "2026-01-03T00:00:00Z"),
            ],
            2,
        );
        assert_eq!(prunable.len(), 1);
        assert_eq!(prunable[0].name, "a");
    }

    #[test]
    fn nothing_to_prune_when_within_limit() {
        assert!(select_prunable(vec![run("a", "2026-01-01T00:00:00Z")], 2).is_empty());
    }
}
