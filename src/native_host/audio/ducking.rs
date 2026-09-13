//! Room-only sample-clock envelope. No pending cues or wall-clock catchup.
use serde::Deserialize;

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DuckingSpec {
    pub gain: f64,
    pub attack_seconds: f64,
    pub hold_seconds: f64,
    pub max_hold_seconds: f64,
    pub release_seconds: f64,
}
impl Default for DuckingSpec {
    fn default() -> Self {
        toml::from_str(include_str!("../../../assets/audio/room-ducking.toml"))
            .expect("authored-room-ducking-valid")
    }
}
#[derive(Clone, Copy)]
struct Window {
    from: f64,
    floor: f64,
    attack_start: f64,
    attack_end: f64,
    cap: f64,
    hold: f64,
    end: f64,
}
#[derive(Default)]
pub struct Ducking {
    enabled: bool,
    spec: DuckingSpec,
    window: Option<Window>,
}
impl Ducking {
    pub fn set_enabled(&mut self, enabled: bool, now: f64) {
        if self.enabled && !enabled {
            let from = f64::from(self.gain(now));
            self.window = (from < 1.0).then_some(Window {
                from,
                floor: from,
                attack_start: now,
                attack_end: now,
                cap: now,
                hold: now,
                end: now + self.spec.release_seconds,
            });
        }
        self.enabled = enabled;
    }
    pub fn clear(&mut self) {
        self.window = None;
    }
    pub fn gain(&self, now: f64) -> f32 {
        let Some(window) = self.window.filter(|window| now < window.end) else {
            return 1.0;
        };
        if now < window.attack_end {
            return (window.from
                + (window.floor - window.from)
                    * ((now - window.attack_start) / (window.attack_end - window.attack_start))
                        .max(0.0)) as f32;
        }
        if now <= window.hold {
            return window.floor as f32;
        }
        (window.floor + (1.0 - window.floor) * (now - window.hold) / self.spec.release_seconds)
            as f32
    }
    pub fn trigger(&mut self, now: f64) {
        if !self.enabled {
            return;
        }
        let previous = self.window.filter(|window| now < window.end);
        if previous.is_some_and(|window| now >= window.cap) {
            return;
        }
        let cap = previous.map_or(now + self.spec.max_hold_seconds, |window| window.cap);
        let hold = cap.min(
            previous
                .map_or(now, |window| window.hold)
                .max(now + self.spec.hold_seconds),
        );
        self.window = Some(Window {
            from: f64::from(self.gain(now)),
            floor: self.spec.gain,
            attack_start: now,
            attack_end: (now + self.spec.attack_seconds).min(hold),
            cap,
            hold,
            end: hold + self.spec.release_seconds,
        });
    }
}
