//! Common definitions for wayland
//!
//! This crate hosts common type and traits used to represent wayland messages
//! and routines in the `wayland-client` and `wayland-server` crates.
//!
//! This notably includes the `Interface` trait, which can exhaustively describe
//! any wayland interface. Its implementations are intended to be generated by the
//! `wayland-scanner` crate.
//!
//! The principal user-facing definition provided by this crate is the `Implementation`
//! trait, which as a user of `wayland-client` or `wayland-server` you will be using
//! to define objects able to handle the messages your program receives. Note that
//! this trait is auto-implemented for closures with appropriate signature, for
//! convenience.

#![warn(missing_docs)]

use std::{ffi::CString, os::unix::io::RawFd};

use wayland_sys::common::wl_interface;

#[doc(hidden)]
pub extern crate smallvec;

pub use wayland_sys as sys;

pub mod client;
pub mod core_interfaces;
pub mod scanner;
pub mod server;

/// Description of the protocol-level information of an object
#[derive(Copy, Clone, Debug)]
pub struct ObjectInfo {
    /// The protocol ID
    pub id: u32,
    /// The interface
    pub interface: &'static Interface,
    /// The version
    pub version: u32,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum AllowNull {
    Yes,
    No,
}

/// Enum of possible argument types as recognized by the wire
#[derive(Copy, Clone, PartialEq, Debug)]
pub enum ArgumentType {
    /// i32
    Int,
    /// u32
    Uint,
    /// fixed point, 1/256 precision
    Fixed,
    /// CString
    Str(AllowNull),
    /// id of a wayland object
    Object(AllowNull),
    /// id of a newly created wayland object
    NewId(AllowNull),
    /// Vec<u8>
    Array(AllowNull),
    /// RawFd
    Fd,
}

impl ArgumentType {
    pub fn same_type(self, other: Self) -> bool {
        std::mem::discriminant(&self) == std::mem::discriminant(&other)
    }
}

/// Enum of possible argument of the protocol
#[derive(Clone, PartialEq, Debug)]
#[allow(clippy::box_vec)]
pub enum Argument<Id> {
    /// i32
    Int(i32),
    /// u32
    Uint(u32),
    /// fixed point, 1/256 precision
    Fixed(i32),
    /// CString
    ///
    /// The value is boxed to reduce the stack size of Argument. The performance
    /// impact is negligible as `string` arguments are pretty rare in the protocol.
    Str(Box<CString>),
    /// id of a wayland object
    Object(Id),
    /// id of a newly created wayland object
    NewId(Id),
    /// Vec<u8>
    ///
    /// The value is boxed to reduce the stack size of Argument. The performance
    /// impact is negligible as `array` arguments are pretty rare in the protocol.
    Array(Box<Vec<u8>>),
    /// RawFd
    Fd(RawFd),
}

impl<Id> Argument<Id> {
    /// Retrieve the type of a given argument instance
    pub fn get_type(&self) -> ArgumentType {
        match *self {
            Argument::Int(_) => ArgumentType::Int,
            Argument::Uint(_) => ArgumentType::Uint,
            Argument::Fixed(_) => ArgumentType::Fixed,
            Argument::Str(_) => ArgumentType::Str(AllowNull::Yes),
            Argument::Object(_) => ArgumentType::Object(AllowNull::Yes),
            Argument::NewId(_) => ArgumentType::NewId(AllowNull::Yes),
            Argument::Array(_) => ArgumentType::Array(AllowNull::Yes),
            Argument::Fd(_) => ArgumentType::Fd,
        }
    }
}

impl<Id: std::fmt::Display> std::fmt::Display for Argument<Id> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Argument::Int(value) => write!(f, "{}", value),
            Argument::Uint(value) => write!(f, "{}", value),
            Argument::Fixed(value) => write!(f, "{}", value),
            Argument::Str(value) => write!(f, "{:?}", value),
            Argument::Object(value) => write!(f, "{}", value),
            Argument::NewId(value) => write!(f, "{}", value),
            Argument::Array(value) => write!(f, "{:?}", value),
            Argument::Fd(value) => write!(f, "{}", value),
        }
    }
}

#[derive(Debug)]
pub struct Interface {
    pub name: &'static str,
    pub version: u32,
    pub requests: &'static [MessageDesc],
    pub events: &'static [MessageDesc],
    pub c_ptr: Option<&'static wl_interface>,
}

/// Wire metadata of a given message
#[derive(Copy, Clone, Debug)]
pub struct MessageDesc {
    /// Name of this message
    pub name: &'static str,
    /// Signature of the message
    pub signature: &'static [ArgumentType],
    /// Minimum required version of the interface
    pub since: u32,
    /// Whether this message is a destructor
    pub is_destructor: bool,
    pub child_interface: Option<&'static Interface>,
    pub arg_interfaces: &'static [&'static Interface],
}

/// A protocol error
///
/// This kind of error is generated by the server if your client didn't respect
/// the protocol, after which the server will kill your connection.
#[derive(Clone, Debug)]
pub struct ProtocolError {
    /// The error code associated with the error
    ///
    /// It should be interpreted as an instance of the `Error` enum of the
    /// associated interface.
    pub code: u32,
    /// The id of the object that caused the error
    pub object_id: u32,
    /// The interface of the object that caused the error
    pub object_interface: String,
    /// The message sent by the server describing the error
    pub message: String,
}

pub const INLINE_ARGS: usize = 4;

#[derive(Debug, Clone, PartialEq)]
pub struct Message<Id> {
    pub sender_id: Id,
    pub opcode: u16,
    pub args: smallvec::SmallVec<[Argument<Id>; INLINE_ARGS]>,
}

#[macro_export]
macro_rules! message {
    ($sender_id: expr, $opcode: expr, [$($args: expr),* $(,)?] $(,)?) => {
        $crate::Message {
            sender_id: $sender_id,
            opcode: $opcode,
            args: $crate::smallvec::smallvec![$($args),*],
        }
    }
}

impl std::error::Error for ProtocolError {}

impl std::fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut ::std::fmt::Formatter) -> Result<(), ::std::fmt::Error> {
        write!(
            f,
            "Protocol error {} on object {}@{}: {}",
            self.code, self.object_interface, self.object_id, self.message
        )
    }
}
/// A type representing an error that cannot occur
#[derive(Clone, Debug)]
pub enum Never {}

impl std::error::Error for Never {}

impl std::fmt::Display for Never {
    fn fmt(&self, _: &mut ::std::fmt::Formatter) -> Result<(), ::std::fmt::Error> {
        match *self {}
    }
}

pub fn check_for_signature<Id>(signature: &[ArgumentType], args: &[Argument<Id>]) -> bool {
    if signature.len() != args.len() {
        return false;
    }
    for (typ, arg) in signature.iter().copied().zip(args.iter()) {
        if !arg.get_type().same_type(typ) {
            return false;
        }
    }
    true
}

#[inline]
pub fn same_interface(a: &'static Interface, b: &'static Interface) -> bool {
    a as *const Interface == b as *const Interface || a.name == b.name
}

#[inline]
pub fn same_interface_or_anonymous(a: &'static Interface, b: &'static Interface) -> bool {
    same_interface(a, b) || same_interface(a, &ANONYMOUS_INTERFACE)
}

/// Special interface representing an anonymous object
pub static ANONYMOUS_INTERFACE: Interface =
    Interface { name: "<anonymous>", version: 0, requests: &[], events: &[], c_ptr: None };
