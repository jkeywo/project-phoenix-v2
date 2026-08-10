// A pack-carried script using only capabilities the deny-by-default vellum
// sandbox grants (issue #988): integer arithmetic inside a plain function.
fn on_alarm(ctx) {
    let n = 2 + 2;
    n
}
