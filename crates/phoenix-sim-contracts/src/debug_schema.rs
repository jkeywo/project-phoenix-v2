/// The wire-schema version every top-level debug payload stamps.
///
/// See convention 2 in the module docs. Bump on a breaking shape change to any
/// payload in this module; a consumer compares it against the version it was
/// built for.
pub const DEBUG_SCHEMA_VERSION: u32 = 1;
