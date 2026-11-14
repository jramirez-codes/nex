//! The shared UI kit: colors, animation, painting primitives, icons, toasts and small widgets.
//! Everything here is stateless (or owns its own tiny state, like `Glide`) and is reused by
//! the window chrome, the panes and the file explorer.

pub mod glide;
pub mod icons;
pub mod paint;
pub mod theme;
pub mod toast;
pub mod widgets;
