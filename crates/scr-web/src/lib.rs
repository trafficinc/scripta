//! Bounded HTTP hosting with persistent, supervised interpreter processes.
#![forbid(unsafe_code)]
mod dev;
mod health;
mod observe;
mod proxy;
mod response;
mod server;
mod uploads;
mod worker;
pub use observe::LogFormat;
pub use server::{Config, serve};
pub use uploads::UploadConfig;
pub use worker::run_worker;
