//! Pacemaker takes charge of leader election, timing and branch synchoronizing.
//!
//!

pub mod data;
pub mod elector;
pub mod liveness_storage;
pub mod network;
pub mod pacemaker;
mod timer;
mod utils;
