//! The screens of the GUI, one module each.

pub mod create;
mod create_form;
mod create_preview;
pub mod debounce;

#[cfg(all(test, feature = "gui"))]
mod tests_create;
