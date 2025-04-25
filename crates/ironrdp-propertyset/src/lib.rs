#![doc = include_str!("../README.md")]
#![doc(html_logo_url = "https://cdnweb.devolutions.net/images/projects/devolutions/logos/devolutions-icon-shadow.svg")]
#![no_std]

extern crate alloc;

#[macro_use]
extern crate tracing;

use core::fmt::{self, Display};

use alloc::borrow::Cow;
use alloc::collections::BTreeMap;
use alloc::string::String;

pub type Key = Cow<'static, str>;

/// Key-value store for configuration keys.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct PropertySet {
    inner: BTreeMap<Key, Value>,
}

impl PropertySet {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, key: impl Into<Key>, value: impl Into<Value>) -> Option<Value> {
        let (key, value) = (key.into(), value.into());
        debug!("PropertySet::insert({key}, {value})");
        self.inner.insert(key, value)
    }

    pub fn remove(&mut self, key: &str) -> Option<Value> {
        let value = self.inner.remove(key);

        match &value {
            Some(value) => debug!("PropertySet::remove({key}) = {value}"),
            None => debug!("PropertySet::remove({key}) = None"),
        }

        value
    }

    pub fn get<'a, V: ExtractFrom<&'a Value>>(&'a self, key: &str) -> Option<V> {
        let value = self.inner.get(key);

        match &value {
            Some(value) => debug!("PropertySet::get({key}) = {value}"),
            None => debug!("PropertySet::get({key}) = None"),
        }

        value.and_then(|val| V::extract_from(val, private::Token))
    }

    pub fn iter(&self) -> impl Iterator<Item = (&Key, &Value)> {
        self.inner.iter()
    }
}

impl IntoIterator for PropertySet {
    type Item = (Key, Value);

    type IntoIter = alloc::collections::btree_map::IntoIter<Key, Value>;

    fn into_iter(self) -> Self::IntoIter {
        self.inner.into_iter()
    }
}

impl fmt::Debug for PropertySet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.inner, f)
    }
}

macro_rules! impl_from {
    ($from:ty => $enum:ident :: $variant:ident) => {
        impl From<$from> for $enum {
            fn from(value: $from) -> Self {
                Self::$variant(value.into())
            }
        }
    };
}

macro_rules! impl_extract_from {
    (ref $enum:ident :: as_int => $to:ty) => {
        impl ExtractFrom<&$enum> for $to {
            fn extract_from(value: &$enum, _token: private::Token) -> Option<Self> {
                value.as_int().and_then(|v| v.try_into().ok())
            }
        }
    };
}

pub trait ExtractFrom<Value>: Sized {
    fn extract_from(value: Value, _token: private::Token) -> Option<Self>;
}

/// Represents a value of any type of the .RDP file format.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    /// Numerical value.
    Int(i64),
    /// String value.
    Str(String),
}

impl Value {
    pub fn as_str(&self) -> Option<&str> {
        if let Self::Str(value) = self {
            Some(value.as_str())
        } else {
            None
        }
    }

    pub fn as_int(&self) -> Option<i64> {
        if let Self::Int(value) = self {
            Some(*value)
        } else {
            None
        }
    }
}

impl Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Int(value) => write!(f, "{value}"),
            Value::Str(value) => write!(f, "\"{value}\""),
        }
    }
}

impl_from!(String => Value::Str);
impl_from!(&str => Value::Str);
impl_from!(u8 => Value::Int);
impl_from!(u16 => Value::Int);
impl_from!(u32 => Value::Int);
impl_from!(i8 => Value::Int);
impl_from!(i16 => Value::Int);
impl_from!(i32 => Value::Int);
impl_from!(i64 => Value::Int);
impl_from!(bool => Value::Int);

impl_extract_from!(ref Value::as_int => u8);
impl_extract_from!(ref Value::as_int => u16);
impl_extract_from!(ref Value::as_int => u32);
impl_extract_from!(ref Value::as_int => i8);
impl_extract_from!(ref Value::as_int => i16);
impl_extract_from!(ref Value::as_int => i32);
impl_extract_from!(ref Value::as_int => i64);

impl<'a> ExtractFrom<&'a Value> for &'a str {
    fn extract_from(value: &'a Value, _token: private::Token) -> Option<Self> {
        value.as_str()
    }
}

impl ExtractFrom<&Value> for bool {
    fn extract_from(value: &Value, _token: private::Token) -> Option<Self> {
        value.as_int().map(|value| value != 0)
    }
}

mod private {
    pub struct Token;
}
