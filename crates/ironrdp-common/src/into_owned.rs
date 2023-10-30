/// Trait used to produce an owned version of the given type.
pub trait IntoOwned: Sized {
    type Owned: 'static;

    fn into_owned(self) -> Self::Owned;
}
