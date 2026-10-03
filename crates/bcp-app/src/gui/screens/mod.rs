//! The screens of the GUI, one module each. The shell (`app.rs`) dispatches to them.

pub mod create;
mod create_form;
mod create_preview;
pub mod debounce;
pub mod recover;

#[cfg(all(test, feature = "gui"))]
mod tests_create;
#[cfg(test)]
mod tests_recover;
