//! Unofficial client library for the Microsoft Teams chat protocol.
//!
//! The protocol facts come from purple-teams (GPL-3.0) and from observing the Teams web client.
//! Every endpoint used here is internal and undocumented, so it can change without notice.

pub mod actions;
pub mod ams;
pub mod auth;
pub mod broker;
pub mod card;
pub mod cards;
pub mod chatsvc;
pub mod client;
pub mod client_info;
pub mod csa;
pub mod error;
pub mod files;
pub mod http;
pub mod mentions;
pub mod model;
pub mod mt;
pub mod people;
pub mod presence;
pub mod threads;
pub mod time;
pub mod trouter;

pub use client::{Client, Session};
pub use error::{Error, Result};
