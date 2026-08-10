// Reaches for the wall clock at load time. The deny-by-default vellum sandbox
// has no `timestamp`, so this is a denied capability and the whole pack is
// rejected on upload (issue #988).
let now = timestamp();
