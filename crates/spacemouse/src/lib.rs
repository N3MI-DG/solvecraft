//! 3D mouse input for SolveCraft: which HID devices are SpaceMice ([`is_space_mouse`],
//! [`model_name`]) and what their input reports say ([`Decoder`]): six axes of motion and the
//! buttons.
//!
//! This crate knows nothing about SolveCraft's documents, camera or UI. Decoding does no I/O and
//! builds for every target, wasm included; the `native` feature adds `Reader`, a background
//! thread that finds the device over HID and hands its [`Event`]s over.
//!
//! The report layouts and device ids follow the devices' own reports, the PySpaceMouse device
//! table (MIT, github.com/JakubAndrysek/PySpaceMouse) and the USB ID repository
//! (www.linux-usb.org/usb.ids).
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

mod decode;
mod devices;
#[cfg(feature = "native")]
mod reader;

pub use decode::{Buttons, Decoder, Event, FULL_SCALE, Motion};
pub use devices::{USAGE_MULTI_AXIS, USAGE_PAGE_GENERIC_DESKTOP, VENDOR_3DCONNEXION, VENDOR_LOGITECH, is_space_mouse, model_name};
#[cfg(feature = "native")]
pub use reader::Reader;
