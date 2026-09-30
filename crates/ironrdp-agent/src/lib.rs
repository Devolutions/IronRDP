#![cfg_attr(doc, doc = include_str!("../README.md"))]
#![doc(html_logo_url = "https://cdnweb.devolutions.net/images/projects/devolutions/logos/devolutions-icon-shadow.svg")]
#![allow(unused_crate_dependencies)]

//! A CLI-driven agentic RDP client.

pub mod cli;

pub(crate) mod attach;
pub(crate) mod daemon_lifecycle;
pub(crate) mod gw_forward;
pub(crate) mod help;
pub(crate) mod terminal_image;

#[cfg(windows)]
pub(crate) mod sandbox;
#[cfg(windows)]
pub(crate) mod sandbox_grpc;
