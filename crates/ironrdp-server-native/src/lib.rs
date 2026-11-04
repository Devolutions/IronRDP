#![cfg_attr(doc, doc = include_str!("../README.md"))]

#[cfg(all(windows, feature = "cursor"))]
pub mod cursor;
