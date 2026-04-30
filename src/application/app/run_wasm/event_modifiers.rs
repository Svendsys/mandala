// SPDX-License-Identifier: MPL-2.0

//! `WindowEvent::ModifiersChanged` arm. Mirrors the modifier
//! bitmask from winit into `WasmInputState::modifiers` so
//! subsequent keyboard / mouse arms can read the live ctrl /
//! shift / alt state without a fresh winit query.

#![cfg(target_arch = "wasm32")]

use winit::event::Modifiers;

impl super::WasmApp {
    pub(super) fn handle_modifiers_changed(&mut self, mods: Modifiers) {
        if let Some(input) = self.input.borrow_mut().as_mut() {
            input.modifiers = mods.state();
        }
    }
}
