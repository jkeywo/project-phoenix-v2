pub mod scan;
pub use scan::*;
/// The blackboard channel key a ship's last scan is published under.
///
/// **Not a system id.** No `[[system]]` block declares it, no station owns it,
/// it registers no `ControlSource` and no `ControlSystem` message may target it
/// — the thing that can be commanded and damaged is `sensors`, and that is what
/// the scan command targets. This is where the *result* is carried, and it is
/// a channel for `operations`' reason: the blackboard map and the
/// `BlackboardUpdate` wire message are typed as `SystemId`.
///
/// A field on `SensorsBlackboard` was the obvious alternative and was rejected:
/// that payload is the radar's live configuration, republished as contacts
/// move, and hanging a rarely-changing reading off it would re-broadcast the
/// reading every time a blip did.
pub const SCAN_BLACKBOARD_KEY: &str = "scan";
