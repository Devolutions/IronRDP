//! Connection options and the protocol choice.

use std::fmt;

/// The data-control protocol in use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Protocol {
    /// [`ext-data-control-v1`], the wayland-protocols staging protocol.
    ///
    /// [`ext-data-control-v1`]: https://gitlab.freedesktop.org/wayland/wayland-protocols/-/blob/main/staging/ext-data-control/ext-data-control-v1.xml
    Ext,
    /// [`wlr-data-control-unstable-v1`], the wlroots protocol.
    ///
    /// [`wlr-data-control-unstable-v1`]: https://gitlab.freedesktop.org/wlroots/wlr-protocols/-/blob/master/unstable/wlr-data-control-unstable-v1.xml
    Wlr,
}

impl fmt::Display for Protocol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Protocol::Ext => f.write_str("ext-data-control-v1"),
            Protocol::Wlr => f.write_str("wlr-data-control-unstable-v1"),
        }
    }
}

/// Which protocol to prefer when the compositor offers both.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum Preference {
    /// Prefer `ext-data-control-v1`, then `wlr-data-control-unstable-v1`.
    #[default]
    Auto,
    /// Prefer `ext-data-control-v1`.
    Ext,
    /// Prefer `wlr-data-control-unstable-v1`.
    Wlr,
}

/// Options for [`DataControl::connect_with`](super::DataControl::connect_with).
///
/// ```
/// use ironrdp_cliprdr_native::data_control::{Options, Preference};
///
/// let options = Options::new()
///     .preference(Preference::Wlr)
///     .allow_fallback(false);
/// ```
#[derive(Debug, Clone)]
pub struct Options {
    pub(crate) preference: Preference,
    pub(crate) allow_fallback: bool,
    pub(crate) max_read_bytes: usize,
}

impl Options {
    /// The default read size limit: 100 MiB.
    pub const DEFAULT_MAX_READ_BYTES: usize = 100 * 1024 * 1024;

    /// Options with automatic protocol selection and fallback enabled.
    #[must_use]
    pub fn new() -> Self {
        Self {
            preference: Preference::Auto,
            allow_fallback: true,
            max_read_bytes: Self::DEFAULT_MAX_READ_BYTES,
        }
    }

    /// Choose which protocol to prefer.
    #[must_use]
    pub fn preference(mut self, preference: Preference) -> Self {
        self.preference = preference;
        self
    }

    /// Whether to use the other protocol when the preferred one is missing.
    #[must_use]
    pub fn allow_fallback(mut self, allow: bool) -> Self {
        self.allow_fallback = allow;
        self
    }

    /// Limit, in bytes, for one [`read`](super::DataControl::read).
    #[must_use]
    pub fn max_read_bytes(mut self, limit: usize) -> Self {
        self.max_read_bytes = limit;
        self
    }

    /// The protocols to try, in order.
    #[cfg_attr(feature = "__test", visibility::make(pub))]
    pub(crate) fn candidates(&self) -> Vec<Protocol> {
        let (first, second) = match self.preference {
            Preference::Auto | Preference::Ext => (Protocol::Ext, Protocol::Wlr),
            Preference::Wlr => (Protocol::Wlr, Protocol::Ext),
        };
        if self.allow_fallback {
            vec![first, second]
        } else {
            vec![first]
        }
    }
}

impl Default for Options {
    fn default() -> Self {
        Self::new()
    }
}
