use std::collections::HashMap;
use std::rc::Rc;

use ironrdp_graphics::pointer::DecodedPointer;

#[derive(Debug, Clone, Default)]
pub struct PointerCache {
    // TODO(@pacancoder) maybe use Vec<Optional<...>> instead?
    cache: HashMap<usize, Rc<DecodedPointer>>,
}

impl PointerCache {
    pub fn insert(&mut self, id: usize, pointer: Rc<DecodedPointer>) -> Option<Rc<DecodedPointer>> {
        self.cache.insert(id, pointer)
    }

    pub fn get(&self, id: usize) -> Option<Rc<DecodedPointer>> {
        self.cache.get(&id).cloned()
    }

    pub fn is_cached(&self, id: usize) -> bool {
        self.cache.contains_key(&id)
    }
}
