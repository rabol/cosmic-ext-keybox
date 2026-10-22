// SPDX-License-Identifier: GPL-3.0-only

//! Asynchronous, provider-neutral access to the freedesktop Secret Service.

mod client;
mod crypto;
mod error;
mod proxy;
mod types;

pub use client::{Bus, Client, PromptOutcome};
pub use crypto::CryptoError;
pub use error::Error;
pub use types::{CollectionInfo, ItemInfo, ObjectId, Provider, RetrievedSecret, ServiceEvent};
