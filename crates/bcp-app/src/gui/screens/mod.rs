//! The screens of the GUI, one module each. The shell (`app.rs`) dispatches to them.

pub mod create;
mod create_form;
mod create_preview;
mod create_run;
mod create_steps;
pub mod debounce;
pub mod recover;
pub mod selftest;
pub mod verify;

#[cfg(all(test, feature = "gui"))]
mod tests_create;
#[cfg(all(test, feature = "gui"))]
mod tests_create_run;
#[cfg(test)]
mod tests_recover;
#[cfg(test)]
mod tests_selftest;
#[cfg(test)]
mod tests_verify;
