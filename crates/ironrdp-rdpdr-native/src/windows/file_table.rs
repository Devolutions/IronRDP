//! Bounded RDPDR file identifier allocation.

use std::collections::HashMap;

/// A table that keeps opaque local handles behind RDPDR file identifiers.
#[derive(Debug)]
pub(crate) struct FileTable<T> {
    entries: HashMap<u32, T>,
    available_ids: Vec<u32>,
    next_id: Option<u32>,
    max_entries: usize,
    active_ids: usize,
}

/// An opaque file ID reserved before opening a native handle.
#[derive(Debug)]
pub(crate) struct FileIdReservation(u32);

impl FileIdReservation {
    pub(crate) fn file_id(&self) -> u32 {
        self.0
    }
}

impl<T> FileTable<T> {
    pub(crate) fn new(max_entries: usize) -> Self {
        Self {
            entries: HashMap::new(),
            available_ids: Vec::new(),
            next_id: Some(1),
            max_entries,
            active_ids: 0,
        }
    }

    pub(crate) fn reserve_file_id(&mut self) -> Result<FileIdReservation, FileTableError> {
        if self.active_ids == self.max_entries {
            return Err(FileTableError::CapacityExceeded);
        }

        let file_id = if let Some(file_id) = self.available_ids.pop() {
            file_id
        } else {
            let file_id = self.next_id.ok_or(FileTableError::IdSpaceExhausted)?;
            self.next_id = file_id.checked_add(1);
            file_id
        };
        self.active_ids += 1;
        Ok(FileIdReservation(file_id))
    }

    pub(crate) fn insert(&mut self, reservation: FileIdReservation, value: T) {
        let file_id = reservation.0;
        let previous = self.entries.insert(file_id, value);
        debug_assert!(
            previous.is_none(),
            "released RDPDR file IDs never collide with open files"
        );
    }

    pub(crate) fn release_file_id(&mut self, reservation: FileIdReservation) {
        self.available_ids.push(reservation.0);
        self.active_ids -= 1;
    }

    pub(crate) fn get(&self, file_id: u32) -> Option<&T> {
        self.entries.get(&file_id)
    }

    pub(crate) fn remove(&mut self, file_id: u32) -> Option<T> {
        let value = self.entries.remove(&file_id)?;
        self.available_ids.push(file_id);
        self.active_ids -= 1;
        Some(value)
    }

    pub(crate) fn clear(&mut self) {
        self.available_ids
            .extend(self.entries.drain().map(|(file_id, _)| file_id));
        self.active_ids = 0;
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
    fn released_file_ids_are_reused() {
        let mut table = FileTable::new(1);
        let reservation = table.reserve_file_id().expect("first allocation");
        let file_id = reservation.file_id();
        table.insert(reservation, ());
        assert!(table.remove(file_id).is_some());
        assert_eq!(table.reserve_file_id().expect("reuse allocation").file_id(), file_id);
    }

    #[test]
    fn reservations_enforce_capacity_before_a_handle_is_inserted() {
        let mut table = FileTable::<()>::new(1);
        let reservation = table.reserve_file_id().expect("reserve file ID");
        let file_id = reservation.file_id();

        assert!(matches!(table.reserve_file_id(), Err(FileTableError::CapacityExceeded)));

        table.release_file_id(reservation);
        assert_eq!(table.reserve_file_id().expect("reuse reservation").file_id(), file_id);
    }
}
