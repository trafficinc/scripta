//! Application boundaries shared by the executable and embedding callers.
pub mod args;
pub mod extensions;
pub mod pipeline;
pub mod playground;

pub mod formatter;
pub mod packages;
mod testing;
mod validation;

pub mod entry;

mod web;
