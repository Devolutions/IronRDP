#![cfg_attr(doc, doc = include_str!("../README.md"))]
#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;
extern crate windows_sys;

pub mod pdu;
