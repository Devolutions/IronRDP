use alloc::vec::Vec;
use core::num::NonZeroUsize;

/// A vector-like collection that is guaranteed to contain at least one element.
///
/// The first element (the [head](NonEmpty::first)) is stored inline, so a single-element
/// `NonEmpty` performs no heap allocation. Additional elements are kept in a growable tail.
///
/// Because the collection can never be empty, [`first`](NonEmpty::first) is infallible and
/// [`len`](NonEmpty::len) returns a [`NonZeroUsize`]: callers never have to branch on an
/// "is it empty?" case.
///
/// Elements are kept in insertion order: the head is the first inserted element.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NonEmpty<T> {
    head: T,
    tail: Vec<T>,
}

impl<T> NonEmpty<T> {
    /// Creates a new collection containing a single element.
    ///
    /// No allocation is performed until a second element is [pushed](NonEmpty::push).
    #[must_use]
    pub const fn new(head: T) -> Self {
        Self { head, tail: Vec::new() }
    }

    /// Appends an element after the existing ones.
    pub fn push(&mut self, value: T) {
        self.tail.push(value);
    }

    /// Returns a reference to the first element.
    ///
    /// This never fails: the collection always contains at least one element.
    #[must_use]
    pub const fn first(&self) -> &T {
        &self.head
    }

    /// Returns the number of elements, which is always at least one.
    #[must_use]
    pub fn len(&self) -> NonZeroUsize {
        // INVARIANT: the head always counts for one, so the total is never zero.
        NonZeroUsize::MIN.saturating_add(self.tail.len())
    }

    /// Returns an iterator over the elements, in insertion order, starting with the head.
    pub fn iter(&self) -> impl Iterator<Item = &T> {
        core::iter::once(&self.head).chain(self.tail.iter())
    }

    /// Consumes the collection, keeping only the elements for which `predicate` returns `true`.
    ///
    /// Returns `None` when no element is kept (a `NonEmpty` cannot represent an empty result).
    #[must_use]
    pub fn filter<F>(self, mut predicate: F) -> Option<Self>
    where
        F: FnMut(&T) -> bool,
    {
        let mut kept = core::iter::once(self.head)
            .chain(self.tail)
            .filter(|value| predicate(value));
        let head = kept.next()?;
        let tail = kept.collect();
        Some(Self { head, tail })
    }
}

impl<T> IntoIterator for NonEmpty<T> {
    type Item = T;
    type IntoIter = core::iter::Chain<core::iter::Once<T>, alloc::vec::IntoIter<T>>;

    fn into_iter(self) -> Self::IntoIter {
        core::iter::once(self.head).chain(self.tail)
    }
}

impl<'a, T> IntoIterator for &'a NonEmpty<T> {
    type Item = &'a T;
    type IntoIter = core::iter::Chain<core::iter::Once<&'a T>, core::slice::Iter<'a, T>>;

    fn into_iter(self) -> Self::IntoIter {
        core::iter::once(&self.head).chain(self.tail.iter())
    }
}
