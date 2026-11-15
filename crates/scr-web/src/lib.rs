//! Bounded HTTP hosting with persistent, supervised interpreter processes.
#![forbid(unsafe_code)]
mod server;
mod worker;
pub use server::{Config, serve};
pub use worker::run_worker;
