use std::collections::HashMap;
use std::hash::Hash;
use std::rc::Rc;

// ---------------------------------------------------------------------------
// Convenience alias — the primary use-case.
//
// Stores strings as `Rc<str>`:
//   - Cloning an `Rc` is a single integer increment, never a heap allocation.
//   - `Rc<str>` implements `Borrow<str>`, so every lookup/remove/contains
//     method accepts a plain `&str` with zero allocation.
// ---------------------------------------------------------------------------
pub type StringHistoryList = HistoryList<Rc<str>>;

// ---------------------------------------------------------------------------
// Node stored in a flat Vec arena. Links are slot indices, not pointers.
// ---------------------------------------------------------------------------
struct Node<T> {
    value: T,
    prev: Option<usize>,
    next: Option<usize>,
}

// ---------------------------------------------------------------------------
// HistoryList
//
// Invariants:
//   - Every live value appears exactly once in `nodes` and once as a key in `index`.
//   - `head` is the most-recently-added entry; `tail` is the oldest.
//   - `free_slots` holds indices of tombstoned node slots, ready for reuse.
// ---------------------------------------------------------------------------
pub struct HistoryList<T> {
    nodes: Vec<Option<Node<T>>>, // arena; None = freed slot
    free_slots: Vec<usize>,      // recycled indices
    index: HashMap<T, usize>,    // value -> arena slot  (O(1) lookup)
    head: Option<usize>,
    tail: Option<usize>,
    len: usize,
}

impl<T: Eq + Hash + Clone> HistoryList<T> {
    pub fn new() -> Self {
        Self {
            nodes: Vec::new(),
            free_slots: Vec::new(),
            index: HashMap::new(),
            head: None,
            tail: None,
            len: 0,
        }
    }

    /// Add `value` to the front of the list.
    /// If it is already present anywhere in the list, it is moved to the front
    /// (no duplicate is created). — O(1) amortised.
    pub fn push_front(&mut self, value: T) {
        if let Some(&slot) = self.index.get(&value) {
            // Already exists: detach from current position and reattach at head.
            self.detach(slot);
            self.attach_head(slot);
        } else {
            // Brand-new entry: allocate a node, register in index, attach at head.
            let slot = self.alloc(value.clone());
            self.index.insert(value, slot);
            self.attach_head(slot);
            self.len += 1;
        }
    }

    /// Remove the entry equal to `value` if it exists. — O(1).
    pub fn remove(&mut self, value: &T) -> bool {
        if let Some(slot) = self.index.remove(value) {
            self.detach(slot);
            self.free(slot);
            self.len -= 1;
            true
        } else {
            false
        }
    }

    /// Check whether `value` is present. — O(1).
    pub fn contains(&self, value: &T) -> bool {
        self.index.contains_key(value)
    }

    /// Random-access: return a reference to the stored entry equal to `value`. — O(1).
    pub fn get(&self, value: &T) -> Option<&T> {
        let &slot = self.index.get(value)?;
        self.nodes[slot].as_ref().map(|n| &n.value)
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Iterator from most-recent to oldest.
    pub fn iter(&self) -> Iter<'_, T> {
        Iter {
            nodes: &self.nodes,
            current: self.head,
        }
    }

    // -----------------------------------------------------------------------
    // Private helpers
    // -----------------------------------------------------------------------

    /// Allocate an arena slot, reusing a freed one when available.
    fn alloc(&mut self, value: T) -> usize {
        let node = Node {
            value,
            prev: None,
            next: None,
        };
        if let Some(slot) = self.free_slots.pop() {
            self.nodes[slot] = Some(node);
            slot
        } else {
            self.nodes.push(Some(node));
            self.nodes.len() - 1
        }
    }

    /// Return an arena slot to the free list.
    fn free(&mut self, slot: usize) {
        self.nodes[slot] = None;
        self.free_slots.push(slot);
    }

    /// Unlink a node from the list without freeing its arena slot.
    fn detach(&mut self, slot: usize) {
        let (prev, next) = {
            let n = self.nodes[slot].as_ref().unwrap();
            (n.prev, n.next)
        };
        match prev {
            Some(p) => self.nodes[p].as_mut().unwrap().next = next,
            None => self.head = next, // slot was the head
        }
        match next {
            Some(n) => self.nodes[n].as_mut().unwrap().prev = prev,
            None => self.tail = prev, // slot was the tail
        }
        let n = self.nodes[slot].as_mut().unwrap();
        n.prev = None;
        n.next = None;
    }

    /// Link an already-allocated node as the new head.
    fn attach_head(&mut self, slot: usize) {
        let old_head = self.head;
        if let Some(h) = old_head {
            self.nodes[h].as_mut().unwrap().prev = Some(slot);
        }
        let n = self.nodes[slot].as_mut().unwrap();
        n.next = old_head;
        n.prev = None;
        self.head = Some(slot);
        if self.tail.is_none() {
            self.tail = Some(slot); // first node ever inserted
        }
    }
}

// ---------------------------------------------------------------------------
// String-optimised methods on StringHistoryList (= HistoryList<Rc<str>>)
//
// All methods accept `&str` directly.
//
// Because `Rc<str>` implements `Borrow<str>`, the HashMap can be probed with
// a `&str` key — no allocation needed for lookups, removals, or membership
// tests.  A heap allocation only occurs when a *new* string is inserted for
// the first time (the `Rc::from(s)` call), which is unavoidable.
// ---------------------------------------------------------------------------
impl HistoryList<Rc<str>> {
    /// Push from a `&str`. Allocates one `Rc<str>` only if the string is new.
    pub fn push_str(&mut self, s: &str) {
        if let Some(&slot) = self.index.get(s) {
            // Reuse the existing Rc — no allocation.
            self.detach(slot);
            self.attach_head(slot);
        } else {
            let rc: Rc<str> = Rc::from(s); // single allocation
            let slot = self.alloc(rc.clone()); // Rc::clone = integer bump
            self.index.insert(rc, slot);
            self.attach_head(slot);
            self.len += 1;
        }
    }

    /// Remove by `&str`. No allocation. — O(1).
    pub fn remove_str(&mut self, s: &str) -> bool {
        if let Some(slot) = self.index.remove(s) {
            self.detach(slot);
            self.free(slot);
            self.len -= 1;
            true
        } else {
            false
        }
    }

    /// Membership test by `&str`. No allocation. — O(1).
    pub fn contains_str(&self, s: &str) -> bool {
        self.index.contains_key(s)
    }

    /// Random access by `&str`. No allocation. — O(1).
    pub fn get_str(&self, s: &str) -> Option<&str> {
        let &slot = self.index.get(s)?;
        self.nodes[slot].as_ref().map(|n| n.value.as_ref())
    }

    /// Iterator that yields `&str` instead of `&Rc<str>`.
    pub fn str_iter(&self) -> impl Iterator<Item = &str> {
        self.iter().map(|rc| rc.as_ref())
    }
}

// ---------------------------------------------------------------------------
// FromIterator impls
// ---------------------------------------------------------------------------
impl<T: Eq + Hash + Clone> FromIterator<T> for HistoryList<T> {
    /// Builds a `HistoryList` from an iterator.
    ///
    /// Items are inserted via `push_front` in iteration order, so the *last*
    /// yielded item ends up at the front of the list — matching the semantics
    /// of "most recently added is at the front". Duplicates within the iterator
    /// are handled exactly as they would be by repeated `push_front` calls.
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        let mut list = HistoryList::new();
        for item in iter {
            list.push_front(item);
        }
        list
    }
}

/// Collect from `&str` slices — no intermediate `String` allocation.
impl<'a> FromIterator<&'a str> for StringHistoryList {
    fn from_iter<I: IntoIterator<Item = &'a str>>(iter: I) -> Self {
        let mut list = StringHistoryList::new();
        for s in iter {
            list.push_str(s);
        }
        list
    }
}

/// Collect from owned `String`s — avoids a second clone by using `push_str`.
impl FromIterator<String> for StringHistoryList {
    fn from_iter<I: IntoIterator<Item = String>>(iter: I) -> Self {
        let mut list = StringHistoryList::new();
        for s in iter {
            list.push_str(&s);
        }
        list
    }
}

impl<T: Eq + Hash + Clone> Default for HistoryList<T> {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Sequential iterator (front → back, i.e. newest → oldest)
// ---------------------------------------------------------------------------
pub struct Iter<'a, T> {
    nodes: &'a [Option<Node<T>>],
    current: Option<usize>,
}

impl<'a, T> Iterator for Iter<'a, T> {
    type Item = &'a T;

    fn next(&mut self) -> Option<Self::Item> {
        let slot = self.current?;
        let node = self.nodes[slot].as_ref().unwrap();
        self.current = node.next;
        Some(&node.value)
    }
}

// ---------------------------------------------------------------------------
// Index<usize> for StringHistoryList
//
// O(n) linked-list traversal. History lists are small (dozens of entries)
// and index access only happens on individual keypresses, so this is fine.
// A parallel Vec<usize> of ordered slot indices would give O(1) but adds
// bookkeeping complexity that isn't worth it here.
// ---------------------------------------------------------------------------
impl std::ops::Index<usize> for StringHistoryList {
    type Output = str;

    fn index(&self, mut idx: usize) -> &str {
        let mut current = self.head;
        loop {
            let slot = current.expect("StringHistoryList: index out of bounds");
            if idx == 0 {
                return self.nodes[slot].as_ref().unwrap().value.as_ref();
            }
            current = self.nodes[slot].as_ref().unwrap().next;
            idx -= 1;
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;

    fn to_strs(h: &StringHistoryList) -> Vec<&str> {
        h.str_iter().collect()
    }

    // --- Generic behaviour (unchanged) ------------------------------------

    #[test]
    fn index_access() {
        let h: StringHistoryList = ["a", "b", "c"].into_iter().collect();
        // collect pushes a, b, c → order is c (0), b (1), a (2)
        assert_eq!(&h[0], "c");
        assert_eq!(&h[1], "b");
        assert_eq!(&h[2], "a");
    }

    #[test]
    fn index_after_move_to_front() {
        let mut h = StringHistoryList::new();
        h.push_str("a");
        h.push_str("b");
        h.push_str("c");
        h.push_str("a"); // moves "a" to front
        assert_eq!(&h[0], "a");
        assert_eq!(&h[1], "c");
        assert_eq!(&h[2], "b");
    }

    #[test]
    fn push_and_order() {
        let mut h = StringHistoryList::new();
        h.push_str("a");
        h.push_str("b");
        h.push_str("c");
        assert_eq!(to_strs(&h), vec!["c", "b", "a"]);
        assert_eq!(h.len(), 3);
    }

    #[test]
    fn duplicate_moves_to_front() {
        let mut h = StringHistoryList::new();
        h.push_str("a");
        h.push_str("b");
        h.push_str("c");
        h.push_str("a");
        assert_eq!(to_strs(&h), vec!["a", "c", "b"]);
        assert_eq!(h.len(), 3);
    }

    #[test]
    fn duplicate_of_head_is_noop() {
        let mut h = StringHistoryList::new();
        h.push_str("a");
        h.push_str("b");
        h.push_str("b");
        assert_eq!(to_strs(&h), vec!["b", "a"]);
        assert_eq!(h.len(), 2);
    }

    #[test]
    fn remove_str() {
        let mut h = StringHistoryList::new();
        h.push_str("a");
        h.push_str("b");
        h.push_str("c");
        assert!(h.remove_str("b"));
        assert_eq!(to_strs(&h), vec!["c", "a"]);
        assert_eq!(h.len(), 2);
        assert!(!h.remove_str("b"));
    }

    #[test]
    fn remove_head_and_tail() {
        let mut h = StringHistoryList::new();
        h.push_str("a");
        h.push_str("b");
        h.push_str("c");
        h.remove_str("c");
        assert_eq!(to_strs(&h), vec!["b", "a"]);
        h.remove_str("a");
        assert_eq!(to_strs(&h), vec!["b"]);
        h.remove_str("b");
        assert!(h.is_empty());
    }

    #[test]
    fn contains_and_get_str() {
        let mut h = StringHistoryList::new();
        h.push_str("hello");
        assert!(h.contains_str("hello"));
        assert!(!h.contains_str("world"));
        assert_eq!(h.get_str("hello"), Some("hello"));
        assert_eq!(h.get_str("world"), None);
    }

    // --- FromIterator -----------------------------------------------------

    #[test]
    fn from_iter_str_slices() {
        let h: StringHistoryList = ["a", "b", "c"].into_iter().collect();
        assert_eq!(to_strs(&h), vec!["c", "b", "a"]);
    }

    #[test]
    fn from_iter_strings() {
        let input = vec!["x".to_string(), "y".to_string(), "z".to_string()];
        let h: StringHistoryList = input.into_iter().collect();
        assert_eq!(to_strs(&h), vec!["z", "y", "x"]);
    }

    #[test]
    fn from_iter_deduplicates() {
        let h: StringHistoryList = ["a", "b", "c", "b"].into_iter().collect();
        assert_eq!(to_strs(&h), vec!["b", "c", "a"]);
        assert_eq!(h.len(), 3);
    }

    #[test]
    fn from_iter_empty() {
        let h: StringHistoryList = std::iter::empty::<&str>().collect();
        assert!(h.is_empty());
    }

    #[test]
    fn from_iter_all_duplicates() {
        let h: StringHistoryList = ["x", "x", "x"].into_iter().collect();
        assert_eq!(to_strs(&h), vec!["x"]);
        assert_eq!(h.len(), 1);
    }

    // --- Arena slot reuse ------------------------------------------------

    #[test]
    fn slot_reuse() {
        let mut h = StringHistoryList::new();
        h.push_str("a");
        h.push_str("b");
        h.remove_str("a");
        h.push_str("c");
        assert_eq!(to_strs(&h), vec!["c", "b"]);
        assert_eq!(h.nodes.len(), 2);
    }

    // --- Rc clone is the only copy on re-insert --------------------------

    #[test]
    fn rc_reuse_on_move_to_front() {
        let mut h = StringHistoryList::new();
        h.push_str("shared");
        // Grab a handle to the stored Rc before re-inserting.
        let rc_before = h.iter().next().unwrap().clone();
        h.push_str("shared"); // move to front — must reuse existing Rc
        let rc_after = h.iter().next().unwrap().clone();
        // Same Rc allocation: pointer equality confirms no new string was created.
        assert!(Rc::ptr_eq(&rc_before, &rc_after));
    }
}
