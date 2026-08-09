//! Bounded RDPDR file identifier allocation.

use std::collections::HashMap;

/// A table that keeps opaque local handles behind RDPDR file identifiers.
#[derive(Debug)]
pub(crate) struct FileTable<T> {
    entries: HashMap<u32, T>,
    available_ids: Vec<u32>,
    reserved_ids: HashMap<u32, ()>,
    next_id: Option<u32>,
    max_entries: usize,
}

/// A file ID reserved before a filesystem operation that can mutate local state.
#[derive(Debug)]
pub(crate) struct FileIdReservation {
    file_id: u32,
}

impl<T> FileTable<T> {
    pub(crate) fn new(max_entries: usize) -> Self {
        Self {
            entries: HashMap::new(),
            available_ids: Vec::new(),
            reserved_ids: HashMap::new(),
            next_id: Some(1),
            max_entries,
        }
    }

    #[cfg(test)]
    fn insert(&mut self, value: T) -> Result<u32, FileTableError> {
        let reservation = self.reserve_file_id()?;
        Ok(self.insert_reserved(reservation, value))
    }

    /// Reserves an ID before an operation that can mutate the filesystem.
    pub(crate) fn reserve_file_id(&mut self) -> Result<FileIdReservation, FileTableError> {
        if self.entries.len() + self.reserved_ids.len() == self.max_entries {
            return Err(FileTableError::CapacityExceeded);
        }

        let file_id = if let Some(file_id) = self.available_ids.pop() {
            file_id
        } else {
            let file_id = self.next_id.ok_or(FileTableError::IdSpaceExhausted)?;
            self.next_id = file_id.checked_add(1);
            file_id
        };
        let previous = self.reserved_ids.insert(file_id, ());
        debug_assert!(
            previous.is_none(),
            "reserved RDPDR file IDs never collide with open or reserved files"
        );
        Ok(FileIdReservation { file_id })
    }

    /// Records a successfully opened file under a previously reserved ID.
    pub(crate) fn insert_reserved(&mut self, reservation: FileIdReservation, value: T) -> u32 {
        let file_id = reservation.file_id;
        let reserved = self.reserved_ids.remove(&file_id);
        debug_assert!(reserved.is_some(), "file IDs are inserted only once after reservation");
        let previous = self.entries.insert(file_id, value);
        debug_assert!(
            previous.is_none(),
            "reserved RDPDR file IDs never collide with open files"
        );
        file_id
    }

    /// Releases an ID after the operation it guarded failed.
    pub(crate) fn release_file_id(&mut self, reservation: FileIdReservation) {
        let released = self.reserved_ids.remove(&reservation.file_id);
        debug_assert!(released.is_some(), "file IDs are released only once after reservation");
        if released.is_some() {
            self.available_ids.push(reservation.file_id);
        }
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
        let mut removed_ids = Vec::new();
        self.entries.retain(|file_id, value| {
            let retain = predicate(value);
            if !retain {
                removed_ids.push(*file_id);
            }
            retain
        });
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

    #[test]
    fn reservations_prevent_mutating_operations_beyond_capacity() {
        let mut table = FileTable::new(1);
        let reservation = table.reserve_file_id().expect("first reservation");

        assert!(matches!(table.reserve_file_id(), Err(FileTableError::CapacityExceeded)));

        table.release_file_id(reservation);
        assert_eq!(table.insert(()).expect("released reservation is reusable"), 1);
    }

    #[test]
    fn retaining_entries_evaluates_each_predicate_once() {
        let mut table = FileTable::new(2);
        table.insert("first").expect("first allocation");
        table.insert("second").expect("second allocation");
        let mut evaluations = 0;

        table.retain(|_| {
            evaluations += 1;
            evaluations == 1
        });

        assert_eq!(evaluations, 2);
    }
}
