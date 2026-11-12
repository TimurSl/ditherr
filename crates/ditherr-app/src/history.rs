//! Unlimited undo and redo with a visible list of steps (spec §8, "History").
//!
//! Every step stores a full snapshot of the state. Studio's state is a look
//! and a few layers, so snapshots are small and undo needs no inverse logic.

/// One entry in the step list.
struct Step<T> {
    label: String,
    state: T,
}

pub struct History<T> {
    /// `steps[0]` is the state the history started from.
    steps: Vec<Step<T>>,
    current: usize,
    /// While set, further records with the same key edit the current step
    /// instead of adding one, so a slider drag is one undo step.
    open_key: Option<String>,
}

impl<T: Clone + PartialEq> History<T> {
    pub fn new(label: impl Into<String>, state: T) -> Self {
        Self {
            steps: vec![Step {
                label: label.into(),
                state,
            }],
            current: 0,
            open_key: None,
        }
    }

    /// Forgets everything and starts over from `state`.
    pub fn reset(&mut self, label: impl Into<String>, state: T) {
        *self = Self::new(label, state);
    }

    pub fn current(&self) -> &T {
        &self.steps[self.current].state
    }

    /// Records `state` as a new step and drops any redo tail. Returns whether
    /// anything changed; recording the current state again is a no-op.
    ///
    /// With a `merge_key`, a record that follows another with the same key
    /// (and no [`close`](Self::close) in between) replaces that step's state.
    pub fn record(&mut self, label: impl Into<String>, state: T, merge_key: Option<&str>) -> bool {
        if *self.current() == state {
            return false;
        }
        let merge = match (merge_key, &self.open_key) {
            (Some(k), Some(open)) => k == open && self.current > 0,
            _ => false,
        };
        if merge {
            self.steps[self.current].state = state;
        } else {
            self.steps.truncate(self.current + 1);
            self.steps.push(Step {
                label: label.into(),
                state,
            });
            self.current += 1;
            self.open_key = merge_key.map(str::to_owned);
        }
        true
    }

    /// Ends the current merge run; the next record starts a new step.
    pub fn close(&mut self) {
        self.open_key = None;
    }

    pub fn can_undo(&self) -> bool {
        self.current > 0
    }

    pub fn can_redo(&self) -> bool {
        self.current + 1 < self.steps.len()
    }

    pub fn undo(&mut self) -> Option<&T> {
        self.jump(self.current.checked_sub(1)?)
    }

    pub fn redo(&mut self) -> Option<&T> {
        self.jump(self.current + 1)
    }

    /// Moves to step `index` (0 is the starting state).
    pub fn jump(&mut self, index: usize) -> Option<&T> {
        if index >= self.steps.len() {
            return None;
        }
        self.open_key = None;
        self.current = index;
        Some(self.current())
    }

    pub fn len(&self) -> usize {
        self.steps.len()
    }

    pub fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }

    pub fn position(&self) -> usize {
        self.current
    }

    /// Step labels in order, oldest first.
    pub fn labels(&self) -> impl Iterator<Item = &str> {
        self.steps.iter().map(|s| s.label.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn undo_and_redo_walk_the_steps() {
        let mut h = History::new("Open", 0);
        assert!(!h.can_undo() && !h.can_redo());
        h.record("a", 1, None);
        h.record("b", 2, None);
        assert_eq!(*h.current(), 2);
        assert_eq!(h.undo(), Some(&1));
        assert_eq!(h.undo(), Some(&0));
        assert_eq!(h.undo(), None);
        assert_eq!(h.redo(), Some(&1));
        assert_eq!(h.redo(), Some(&2));
        assert_eq!(h.redo(), None);
        assert_eq!(h.labels().collect::<Vec<_>>(), ["Open", "a", "b"]);
    }

    #[test]
    fn recording_after_undo_drops_the_redo_tail() {
        let mut h = History::new("Open", 0);
        h.record("a", 1, None);
        h.record("b", 2, None);
        h.undo();
        h.record("c", 9, None);
        assert!(!h.can_redo());
        assert_eq!(h.labels().collect::<Vec<_>>(), ["Open", "a", "c"]);
        assert_eq!(*h.current(), 9);
    }

    #[test]
    fn recording_the_same_state_is_a_no_op() {
        let mut h = History::new("Open", 5);
        assert!(!h.record("nothing", 5, None));
        assert_eq!(h.len(), 1);
    }

    #[test]
    fn same_key_merges_until_closed() {
        let mut h = History::new("Open", 0);
        h.record("Strength", 1, Some("strength"));
        h.record("Strength", 2, Some("strength"));
        h.record("Strength", 3, Some("strength"));
        assert_eq!(h.len(), 2, "a drag is one step");
        assert_eq!(*h.current(), 3);
        // Undo goes back to before the drag, not to a mid-drag value.
        assert_eq!(h.undo(), Some(&0));

        h.redo();
        h.close();
        h.record("Strength", 4, Some("strength"));
        assert_eq!(h.len(), 3, "a closed run starts a new step");
    }

    #[test]
    fn different_keys_do_not_merge() {
        let mut h = History::new("Open", 0);
        h.record("A", 1, Some("a"));
        h.record("B", 2, Some("b"));
        h.record("B again", 3, Some("b"));
        assert_eq!(h.labels().collect::<Vec<_>>(), ["Open", "A", "B"]);
    }

    #[test]
    fn undo_ends_a_merge_run() {
        let mut h = History::new("Open", 0);
        h.record("x", 1, Some("k"));
        h.undo();
        h.redo();
        h.record("x", 2, Some("k"));
        assert_eq!(h.len(), 3);
    }

    #[test]
    fn the_starting_state_is_never_overwritten_by_a_merge() {
        let mut h = History::new("Open", 0);
        h.record("x", 1, Some("k"));
        h.jump(0);
        // A merge key left open would otherwise rewrite step 0.
        h.record("x", 2, Some("k"));
        assert_eq!(h.labels().next(), Some("Open"));
        h.undo();
        assert_eq!(*h.current(), 0);
    }

    #[test]
    fn jump_and_reset() {
        let mut h = History::new("Open", 0);
        h.record("a", 1, None);
        h.record("b", 2, None);
        assert_eq!(h.jump(1), Some(&1));
        assert_eq!(h.position(), 1);
        assert!(h.can_redo());
        assert_eq!(h.jump(7), None);
        h.reset("New", 42);
        assert_eq!((h.len(), *h.current(), h.position()), (1, 42, 0));
    }

    #[test]
    fn is_unlimited() {
        let mut h = History::new("Open", 0u32);
        for i in 1..=5000 {
            h.record("step", i, None);
        }
        assert_eq!(h.len(), 5001);
        for _ in 0..5000 {
            assert!(h.undo().is_some());
        }
        assert_eq!(*h.current(), 0);
    }
}
