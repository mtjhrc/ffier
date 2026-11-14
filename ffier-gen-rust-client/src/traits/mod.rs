//! Emit client trait definitions, vtables, and implementations.

mod definition;
mod implementation;
mod vtable;

pub(super) use definition::emit_implementable_trait;
pub(super) use implementation::emit_trait_impl;

#[cfg(test)]
mod tests;
