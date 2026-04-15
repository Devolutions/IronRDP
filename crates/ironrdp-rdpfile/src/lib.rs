#![cfg_attr(doc, doc = include_str!("../README.md"))]
#![doc(html_logo_url = "https://cdnweb.devolutions.net/images/projects/devolutions/logos/devolutions-icon-shadow.svg")]
#![no_std]

extern crate alloc;

use alloc::borrow::ToOwned as _;
use alloc::string::{String, ToString as _};
use alloc::vec::Vec;
use core::fmt;

use ironrdp_propertyset::{PropertySet, Value};

#[derive(Debug, Clone)]
pub enum ErrorKind {
    UnknownType { key: String, ty: String },
    InvalidValue { key: String, ty: String },
    MalformedLine,
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
            ErrorKind::UnknownType { key, ty } => {
                write!(f, "unknown type at line {line_number} for key '{key}' ({ty})")
            }
            ErrorKind::InvalidValue { key, ty } => {
                write!(f, "invalid value at line {line_number} for key '{key}' (type {ty})")
            }
            ErrorKind::MalformedLine => write!(f, "malformed line at line {line_number}"),
        }
    }
}

pub fn load(properties: &mut PropertySet, input: &str) -> Result<(), Vec<Error>> {
    let mut errors = Vec::new();

    for (idx, line) in input.lines().enumerate() {
        let line_number = idx + 1;
        let mut split = line.splitn(3, ':');

        if let (Some(key), Some(ty), Some(value)) = (split.next(), split.next(), split.next()) {
            match ty {
                "i" => {
                    if let Ok(value) = value.parse::<i64>() {
                        properties.insert(key.to_owned(), value);
                    } else {
                        errors.push(Error {
                            kind: ErrorKind::InvalidValue {
                                key: key.to_owned(),
                                ty: ty.to_owned(),
                            },
                            line: line_number,
                        });
                    }
                }
                "s" => {
                    properties.insert(key.to_owned(), value);
                }
                _ => {
                    errors.push(Error {
                        kind: ErrorKind::UnknownType {
                            key: key.to_owned(),
                            ty: ty.to_owned(),
                        },
                        line: line_number,
                    });
                }
            }
        } else {
            errors.push(Error {
                kind: ErrorKind::MalformedLine,
                line: line_number,
            })
        }
    }

    if errors.is_empty() { Ok(()) } else { Err(errors) }
}

pub struct ParseResult {
    pub properties: PropertySet,
    pub errors: Vec<Error>,
}

pub fn parse(input: &str) -> ParseResult {
    let mut properties = PropertySet::new();

    let errors = match load(&mut properties, input) {
        Ok(()) => Vec::new(),
        Err(errors) => errors,
    };

    ParseResult { properties, errors }
}

pub fn write(properties: &PropertySet) -> String {
    let mut buf = String::new();

    for (key, value) in properties.iter() {
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
