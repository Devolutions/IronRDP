use alloc::collections::BTreeMap;
use core::any::TypeId;

use ironrdp_core::NonEmpty;

use crate::DvcProcessor;

mod sealed {
    use super::CardinalityKind;

    pub trait Sealed {
        fn kind() -> CardinalityKind;
    }
    impl Sealed for super::Singleton {
        fn kind() -> CardinalityKind {
            CardinalityKind::Singleton
        }
    }
    impl Sealed for super::Multi {
        fn kind() -> CardinalityKind {
            CardinalityKind::Multi
        }
    }
}

/// Runtime image of a [`ChannelCardinality`] marker, used to branch on cardinality where a
/// type-level decision must drive runtime storage.
#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CardinalityKind {
    Singleton,
    Multi,
}

/// Returns the [`CardinalityKind`] of the cardinality marker `C`.
pub(crate) fn kind_of<C: ChannelCardinality>() -> CardinalityKind {
    <C as sealed::Sealed>::kind()
}

/// Marker trait for the cardinality of a dynamic virtual channel processor type.
///
/// A processor type is either a [`Singleton`] (at most one channel per type) or [`Multi`]
/// (several channels may share the same processor type, e.g. one channel per USB device in
/// [MS-RDPEUSB]). This trait is sealed and implemented only by those two marker types.
pub trait ChannelCardinality: sealed::Sealed {}

/// Cardinality marker for processor types backing at most one channel.
pub enum Singleton {}

/// Cardinality marker for processor types that may back more than one channel.
pub enum Multi {}

impl ChannelCardinality for Singleton {}
impl ChannelCardinality for Multi {}

/// Associates a dynamic virtual channel processor type with its [`ChannelCardinality`].
///
/// This is kept separate from the role traits (`DvcServerProcessor` / `DvcClientProcessor`)
/// so that cardinality is stated once for a channel regardless of the RDP side, and from
/// [`DvcProcessor`] so the latter stays object-safe (it is stored type-erased as
/// `Box<dyn DvcProcessor>`). It is only ever used as a generic bound, never as a trait
/// object.
pub trait DvcChannelCardinality: DvcProcessor {
    /// Whether this processor type backs a single channel ([`Singleton`]) or possibly many
    /// ([`Multi`]).
    type Cardinality: ChannelCardinality;
}

/// The set of channel ids registered for a given processor type, tagged by cardinality.
#[derive(Debug)]
pub(crate) enum ChannelIds {
    /// Exactly one channel is registered for the type.
    Singleton(u32),
    /// One or more channels are registered for the type, in registration order.
    Multi(NonEmpty<u32>),
}

impl ChannelIds {
    /// Records `channel_id` for `type_id` in `map`, using the storage variant dictated by
    /// `kind`.
    ///
    /// For [`CardinalityKind::Multi`] the id is appended to any existing registration. For
    /// [`CardinalityKind::Singleton`] any existing entry is replaced; callers that must reject
    /// a second singleton registration have to check for an existing entry beforehand.
    pub(crate) fn insert_into(
        map: &mut BTreeMap<TypeId, ChannelIds>,
        kind: CardinalityKind,
        type_id: TypeId,
        channel_id: u32,
    ) {
        match kind {
            CardinalityKind::Singleton => {
                map.insert(type_id, ChannelIds::Singleton(channel_id));
            }
            CardinalityKind::Multi => match map.entry(type_id) {
                alloc::collections::btree_map::Entry::Vacant(entry) => {
                    entry.insert(ChannelIds::Multi(NonEmpty::new(channel_id)));
                }
                alloc::collections::btree_map::Entry::Occupied(mut entry) => match entry.get_mut() {
                    ChannelIds::Multi(ids) => ids.push(channel_id),
                    // Unreachable: a type's cardinality is fixed, so a `Multi` registration
                    // never meets a `Singleton` entry for the same type.
                    ChannelIds::Singleton(_) => unreachable!("multi type stored as Singleton"),
                },
            },
        }
    }

    /// Consumes the set, removing `channel_id`. Returns the remaining set, or `None` when it
    /// becomes empty (and its map entry should therefore be dropped).
    pub(crate) fn without(self, channel_id: u32) -> Option<Self> {
        match self {
            Self::Singleton(id) => (id != channel_id).then_some(Self::Singleton(id)),
            Self::Multi(ids) => ids.filter(|&id| id != channel_id).map(Self::Multi),
        }
    }
}
