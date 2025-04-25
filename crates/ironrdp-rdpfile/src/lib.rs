#![doc = include_str!("../README.md")]
#![doc(html_logo_url = "https://cdnweb.devolutions.net/images/projects/devolutions/logos/devolutions-icon-shadow.svg")]
#![no_std]

extern crate alloc;

use core::fmt;

use alloc::borrow::ToOwned;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use ironrdp_propertyset::{PropertySet, Value};

#[derive(Debug, Clone)]
pub enum ErrorKind {
    UnknownType { ty: String },
    InvalidValue { ty: String, value: String },
    MalformedLine { line: String },
    DuplicatedKey { key: String },
}

#[derive(Debug, Clone)]
pub struct Error {
    pub kind: ErrorKind,
    pub line: usize,
}

impl core::error::Error for Error {}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let line_number = self.line;

        match &self.kind {
            ErrorKind::UnknownType { ty } => write!(f, "unknown type at line {line_number} ({ty})"),
            ErrorKind::InvalidValue { ty, value } => {
                write!(f, "invalid value at line {line_number} for type {ty} ({value})")
            }
            ErrorKind::MalformedLine { line } => write!(f, "malformed line at line {line_number} ({line})"),
            ErrorKind::DuplicatedKey { key } => write!(f, "duplicated key at line {line_number} ({key})"),
        }
    }
}

pub struct ParseResult {
    pub store: PropertySet,
    pub errors: Vec<Error>,
}

pub fn parse(input: &str) -> ParseResult {
    let mut store = PropertySet::new();
    let mut errors = Vec::new();

    for (idx, line) in input.lines().enumerate() {
        let mut split = line.splitn(2, ':');

        if let (Some(key), Some(ty), Some(value)) = (split.next(), split.next(), split.next()) {
            let is_duplicated = match ty {
                "i" => {
                    if let Ok(value) = value.parse::<i64>() {
                        store.insert(key.to_owned(), value).is_some()
                    } else {
                        errors.push(Error {
                            kind: ErrorKind::InvalidValue {
                                ty: ty.to_owned(),
                                value: value.to_owned(),
                            },
                            line: idx,
                        });
                        continue;
                    }
                }
                "s" => store.insert(key.to_owned(), value).is_some(),
                _ => {
                    errors.push(Error {
                        kind: ErrorKind::UnknownType { ty: ty.to_owned() },
                        line: idx,
                    });
                    continue;
                }
            };

            if is_duplicated {
                errors.push(Error {
                    kind: ErrorKind::DuplicatedKey { key: key.to_owned() },
                    line: idx,
                });
            }
        } else {
            errors.push(Error {
                kind: ErrorKind::MalformedLine { line: line.to_owned() },
                line: idx,
            })
        }
    }

    ParseResult { store, errors }
}

pub fn write(store: &PropertySet) -> String {
    let mut buf = String::new();

    for (key, value) in store.iter() {
        buf.push_str(key);

        match value {
            Value::Int(value) => {
                buf.push_str(":i:");
                buf.push_str(&value.to_string());
            }
            Value::Str(value) => {
                buf.push_str(":s:");
                buf.push_str(value);
            }
        }

        buf.push('\n');
    }

    buf
}
