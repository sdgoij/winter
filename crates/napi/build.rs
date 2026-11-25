//! The addon's build script.
//!
//! `napi_build::setup` is what makes the cdylib a Node addon on Windows: it
//! links the delay-load hook and the Node import library that the ABI needs
//! (ADR 084).

fn main() {
    napi_build::setup();
}
