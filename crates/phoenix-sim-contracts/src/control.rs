//! Facts used by all command authority paths.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ControlSource {
    #[default]
    Human,
    Ai,
    /// The fine-system policy is the sole actuator writer. Its station holder
    /// may change explicitly supported summary intents at Admission.
    Simplified,
    /// Explicit offline marker. A system with this source behaves as if it were
    /// in the `offline_systems` set: both `accept_human_input` and `operate_ai`
    /// return `false`. Set by the station-rating system when a rating marks a
    /// system as explicitly offline (distinct from damage-driven offline).
    Offline,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ControlTickPolicy {
    pub accept_human_input: bool,
    /// A holder can direct supported policy inputs without owning actuators.
    pub accept_summary_input: bool,
    pub operate_ai: bool,
    pub coordinate: bool,
}

pub fn control_tick_policy(source: ControlSource) -> ControlTickPolicy {
    match source {
        ControlSource::Human => ControlTickPolicy {
            accept_human_input: true,
            accept_summary_input: false,
            operate_ai: false,
            coordinate: true,
        },
        ControlSource::Simplified => ControlTickPolicy {
            accept_human_input: false,
            accept_summary_input: true,
            operate_ai: true,
            coordinate: true,
        },
        ControlSource::Ai => ControlTickPolicy {
            accept_human_input: false,
            accept_summary_input: false,
            operate_ai: true,
            coordinate: true,
        },
        ControlSource::Offline => ControlTickPolicy {
            accept_human_input: false,
            accept_summary_input: false,
            operate_ai: false,
            coordinate: false,
        },
    }
}
