//! Picker-style dialog state shared by overlay selection UIs.

mod buffer_switch;

pub(crate) use buffer_switch::{
    BufferSwitchItem, BufferSwitchPopup, BufferSwitchPopupEntry, BufferSwitchState,
};
