/// How often a broadcast registration fires.
#[derive(Clone, Debug)]
pub enum Cadence {
    Hz(f32),
    Period(std::time::Duration),
    OnEvent,
    Once,
}
