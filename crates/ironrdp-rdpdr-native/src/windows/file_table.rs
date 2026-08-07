//! Bounded RDPDR file identifier allocation.

use std::collections::HashMap;

/// A table that keeps opaque local handles behind RDPDR file identifiers.
#[derive(Debug)]
pub(crate) struct FileTable<T> {
    entries: HashMap<u32, T>,
    available_ids: Vec<u32>,
    next_id: Option<u32>,
    max_entries: usize,
}

impl<T> FileTable<T> {
    pub(crate) fn new(max_entries: usize) -> Self {
        Self {
            entries: HashMap::new(),
            available_ids: Vec::new(),
            next_id: Some(1),
            max_entries,
        }
    }

    pub(crate) fn insert(&mut self, value: T) -> Result<u32, FileTableError> {
        if self.entries.len() == self.max_entries {
            return Err(FileTableError::CapacityExceeded);
        }

        let file_id = if let Some(file_id) = self.available_ids.pop() {
            file_id
        } else {
            let file_id = self.next_id.ok_or(FileTableError::IdSpaceExhausted)?;
            self.next_id = file_id.checked_add(1);
            file_id
        };
        let previous = self.entries.insert(file_id, value);
        debug_assert!(
            previous.is_none(),
            "available RDPDR file IDs never collide with open files"
        );
        Ok(file_id)
    }

    pub(crate) fn get(&self, file_id: u32) -> Option<&T> {
        self.entries.get(&file_id)
    }

    pub(crate) fn get_mut(&mut self, file_id: u32) -> Option<&mut T> {
        self.entries.get_mut(&file_id)
    }

    pub(crate) fn remove(&mut self, file_id: u32) -> Option<T> {
        let value = self.entries.remove(&file_id)?;
        self.available_ids.push(file_id);
        Some(value)
    }

    /// Releases all entries and makes their IDs available to subsequent opens.
    pub(crate) fn clear(&mut self) {
        self.available_ids
            .extend(self.entries.drain().map(|(file_id, _)| file_id));
    }

    /// Retains entries that satisfy `predicate`, dropping all others.
    pub(crate) fn retain(&mut self, mut predicate: impl FnMut(&T) -> bool) {
        let removed_ids = self
            .entries
            .iter()
            .filter_map(|(&file_id, value)| (!predicate(value)).then_some(file_id))
            .collect::<Vec<_>>();
        self.entries.retain(|_, value| predicate(value));
        self.available_ids.extend(removed_ids);
    }
}

/// File-table allocation failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FileTableError {
    /// The backend reached its configured local handle limit.
    CapacityExceeded,
    /// The backend exhausted nonzero `u32` file identifiers.
    IdSpaceExhausted,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closed_file_ids_are_reused() {
        let mut table = FileTable::new(2);
        let first = table.insert("first").expect("first allocation");
        assert_eq!(table.remove(first), Some("first"));

        let second = table.insert("second").expect("second allocation");

        assert_eq!(first, 1);
        assert_eq!(second, first);
        assert_eq!(table.remove(second), Some("second"));
    }

    #[test]
    fn clearing_and_retaining_entries_releases_their_ids() {
        let mut table = FileTable::new(3);
        let first = table.insert("first").expect("first allocation");
        let second = table.insert("second").expect("second allocation");
        table.retain(|value| *value == "first");
        assert_eq!(table.insert("third").expect("reuse retained entry ID"), second);

        table.clear();
        let reused = table.insert("fourth").expect("reuse cleared entry ID");
        assert!(reused == first || reused == second);
    }

    #[test]
    fn allocation_is_bounded() {
        let mut table = FileTable::new(1);
        table.insert(()).expect("first allocation");

        assert_eq!(table.insert(()), Err(FileTableError::CapacityExceeded));
    }
}
