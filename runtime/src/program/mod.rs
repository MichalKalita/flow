//! Compiles a `.flow` source file into the program the runtime executes.
//! Owns declaration shape and static checks. Does not open a database or run a request.

mod check;
mod compile;
mod grammar;
mod model;

pub use grammar::{ident, literal, valid_id};
pub use model::*;
