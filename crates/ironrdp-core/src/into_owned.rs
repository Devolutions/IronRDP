/// Used to produce an owned version of a given data.
pub trait IntoOwned: Sized {
    /// The resulting type after obtaining ownership.
    type Owned: 'static;

    /// Creates owned data from data.
    fn into_owned(self) -> Self::Owned;
}
