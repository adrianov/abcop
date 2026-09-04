//! Write lifetime and observation: which assignments always run, and which
//! values are read before the next sure overwrite. Shared by NeverUsed and
//! UsedOnce.

use tree_sitter::Node;

use super::{Entry, Write, WriteKind};

impl Write {
    /// Record a local write at `ident`, marking whether it always executes.
    pub(crate) fn at(ident: Node<'_>, kind: WriteKind, rhs: Option<(usize, usize)>) -> Self {
        Self {
            byte: ident.start_byte(),
            node_id: ident.id(),
            kind,
            rhs,
            unconditional: straight_line(ident),
        }
    }
}

impl Entry {
    /// Whether write `idx` is observed before the next sure overwrite (or ever, if last).
    ///
    /// A following unconditional `op=` consumes the previous value. Reads inside
    /// the RHS of the next plain assignment also see the previous value (`x = x + 1`).
    /// Conditional later writes do not end the lifetime — the prior value may still
    /// be read when the condition is false.
    pub fn write_is_read(&self, idx: usize) -> bool {
        let Some(w) = self.writes.get(idx) else {
            return false;
        };
        if self
            .next_unconditional(idx)
            .is_some_and(|n| n.kind == WriteKind::OpAssign)
        {
            return true;
        }
        let end = self.write_lifetime_end(idx);
        self.reads
            .iter()
            .any(|r| !r.under_defined && r.byte > w.byte && r.byte < end)
    }

    /// Writes whose value is never observed (dead overwrite or trailing unused).
    pub fn unread_writes(&self) -> impl Iterator<Item = &Write> {
        self.writes
            .iter()
            .enumerate()
            .filter(|(i, _)| !self.write_is_read(*i))
            .map(|(_, w)| w)
    }

    /// Exactly one plain write whose value is read; earlier dead overwrites ignored.
    pub fn single_live_plain(&self) -> Option<&Write> {
        let mut live = self
            .writes
            .iter()
            .enumerate()
            .filter(|(i, w)| w.kind == WriteKind::Plain && self.write_is_read(*i));
        let (_, w) = live.next()?;
        live.next().is_none().then_some(w)
    }

    /// Any read that observes the binding's value (not merely `defined?`).
    pub fn has_value_read(&self) -> bool {
        self.reads.iter().any(|r| !r.under_defined)
    }

    fn next_unconditional(&self, idx: usize) -> Option<&Write> {
        self.writes.iter().skip(idx + 1).find(|w| w.unconditional)
    }

    /// End of this write's value: next unconditional overwrite's RHS end (plain)
    /// or left-hand site (other kinds). Conditional writes are skipped.
    fn write_lifetime_end(&self, idx: usize) -> usize {
        match self.next_unconditional(idx) {
            None => usize::MAX,
            Some(next) if next.kind == WriteKind::Plain => {
                next.rhs.map(|(_, end)| end).unwrap_or(next.byte)
            }
            Some(next) => next.byte,
        }
    }
}

/// True when `node` is not under a conditional, loop, or rescue before its scope.
fn straight_line(node: Node<'_>) -> bool {
    const VETO: [&str; 14] = [
        "if",
        "unless",
        "if_modifier",
        "unless_modifier",
        "conditional",
        "while",
        "until",
        "while_modifier",
        "until_modifier",
        "for",
        "rescue",
        "rescue_modifier",
        "in_clause",
        "when",
    ];
    const OWNERS: [&str; 8] = [
        "method",
        "singleton_method",
        "class",
        "module",
        "singleton_class",
        "block",
        "do_block",
        "lambda",
    ];
    let mut cur = Some(node);
    while let Some(n) = cur {
        if VETO.contains(&n.kind()) {
            return false;
        }
        if OWNERS.contains(&n.kind()) {
            return true;
        }
        cur = n.parent();
    }
    true
}
