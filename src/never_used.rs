//! NeverUsed rule: local variables that are assigned but never read.
//!
//! Complements UsedOnce (which requires exactly one read): here the read
//! count is zero, meaning every write to the binding is dead code. Reported
//! once per binding, located at the first write. Underscore-prefixed names
//! are never tracked. Parameters have no writes and never qualify.

use crate::model::FileModel;

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct NeverUsedOffense {
    pub line: usize,
    pub column: usize,
    pub name: String,
}

pub fn analyze(fm: &FileModel) -> Vec<NeverUsedOffense> {
    let mut out = Vec::new();

    for scope in &fm.scopes {
        for (name, e) in &scope.entries {
            if !e.reads.is_empty() || e.writes.is_empty() {
                continue;
            }
            let first = e.writes.iter().map(|w| w.byte).min().unwrap_or(0);
            let (line, column) = fm.line_col(first);
            out.push(NeverUsedOffense {
                line,
                column,
                name: name.to_string(),
            });
        }
    }
    out.sort_by_key(|o| (o.line, o.column));
    out.dedup_by(|a, b| a.line == b.line && a.column == b.column && a.name == b.name);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::build_from_str;

    fn flags(src: &str) -> Vec<NeverUsedOffense> {
        analyze(&build_from_str(src))
    }

    #[test]
    fn dead_local_is_flagged_at_write_line() {
        let f = flags("def k\n  gone = compute\n  p :done\nend\n");
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].name, "gone");
        assert_eq!(f[0].line, 2);
    }

    #[test]
    fn multiple_writes_without_reads_reported_once() {
        let f = flags("def k\n  x = 1\n  x += 2\nend\n");
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].line, 2);
    }

    #[test]
    fn read_variable_not_flagged() {
        let f = flags("def k\n  x = 1\n  p x\nend\n");
        assert!(f.is_empty());
    }

    #[test]
    fn underscore_names_exempt() {
        let f = flags("def k\n  _tmp = 1\nend\n");
        assert!(f.is_empty());
    }

    #[test]
    fn unused_rescue_variable_flagged() {
        let f = flags("def k\n  risky\nrescue => e\n  :handled\nend\n");
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].name, "e");
    }

    #[test]
    fn parameters_never_flagged() {
        let f = flags("def k(unused_arg)\n  :body\nend\n");
        assert!(f.is_empty());
    }
}
