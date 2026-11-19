//! Unofficial client library for the Microsoft Teams chat protocol.
//!
//! The protocol facts come from purple-teams (GPL-3.0) and from observing the Teams web client.
//! Every endpoint used here is internal and undocumented, so it can change without notice.

pub mod actions;
pub mod activity;
pub mod ams;
pub mod auth;
pub mod broker;
pub mod card;
pub mod cards;
pub mod chatsvc;
pub mod client;
pub mod client_info;
pub mod csa;
pub mod engage;
pub mod error;
pub mod files;
pub mod graph;
pub mod http;
pub mod images;
pub mod lists;
pub mod mentions;
pub mod model;
pub mod mt;
pub mod onenote;
pub mod people;
pub mod planner;
pub mod presence;
pub mod receipts;
pub mod tags;
pub mod teams;
pub mod tenants;
pub mod threads;
pub mod time;
pub mod translate;
pub mod trouter;
pub mod urlpreview;
pub mod workflows;

pub use client::{Client, Session};
pub use error::{Error, Result};
