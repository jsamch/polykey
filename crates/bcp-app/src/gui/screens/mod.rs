//! The screens of the GUI, one module each. The shell (`app.rs`) dispatches to them.

pub mod recover;
pub mod selftest;
pub mod verify;

#[cfg(test)]
mod tests_recover;
#[cfg(test)]
mod tests_selftest;
#[cfg(test)]
mod tests_verify;
