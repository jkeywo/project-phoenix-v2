//! Endpoint preferences, sharing the browser's six-bus gain contract.
use serde::{Deserialize, Serialize};

pub const CATEGORIES: [&str; 5] = ["music", "ambience", "effects", "alerts", "interface"];

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Bus {
    pub level: f32,
    pub muted: bool,
}
impl Default for Bus {
    fn default() -> Self {
        Self {
            level: 1.0,
            muted: false,
        }
    }
}
impl Bus {
    pub fn gain(self) -> f32 {
        if self.muted {
            0.0
        } else {
            level(self.level)
        }
    }
}
pub fn level(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(0.0, 1.0)
    } else {
        1.0
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AudioMix {
    pub master: Bus,
    pub music: Bus,
    pub ambience: Bus,
    pub effects: Bus,
    pub alerts: Bus,
    pub interface: Bus,
}
impl AudioMix {
    pub fn bus(&self, id: &str) -> Option<Bus> {
        Some(match id {
            "master" => self.master,
            "music" => self.music,
            "ambience" => self.ambience,
            "effects" => self.effects,
            "alerts" => self.alerts,
            "interface" => self.interface,
            _ => return None,
        })
    }
    pub fn set(&mut self, id: &str, mut bus: Bus) -> bool {
        bus.level = level(bus.level);
        *match id {
            "master" => &mut self.master,
            "music" => &mut self.music,
            "ambience" => &mut self.ambience,
            "effects" => &mut self.effects,
            "alerts" => &mut self.alerts,
            "interface" => &mut self.interface,
            _ => return false,
        } = bus;
        true
    }
    pub fn sanitised(mut self) -> Self {
        for id in ["master"].into_iter().chain(CATEGORIES) {
            self.set(id, self.bus(id).unwrap());
        }
        self
    }
    pub fn gain(&self, category: &str, authored: f32) -> f32 {
        self.master.gain() * self.bus(category).map_or(0.0, Bus::gain) * level(authored)
    }
}
