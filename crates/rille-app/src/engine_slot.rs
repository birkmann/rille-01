//! The current engine handle. Replaced when the audio device changes.

use std::sync::{Arc, RwLock};

use rille_engine::EngineHandle;

#[derive(Default)]
pub struct EngineSlot(RwLock<Option<Arc<EngineHandle>>>);

impl EngineSlot {
    pub fn get(&self) -> Option<Arc<EngineHandle>> {
        self.0.read().expect("engine slot lock").clone()
    }

    pub fn set(&self, h: Option<Arc<EngineHandle>>) {
        *self.0.write().expect("engine slot lock") = h;
    }
}
