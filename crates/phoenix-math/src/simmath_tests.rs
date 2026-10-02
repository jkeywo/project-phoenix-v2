use super::*;

/// Pin a handful of wrapper outputs to exact bit patterns.
///
/// These are the values `libm` 0.2 produces; they must be identical on
/// every platform (that is the whole point of the module). If this test
/// fails, either a wrapper silently fell back to a `std` method whose
/// system libm disagrees with these bits on this platform, or a `libm`
/// upgrade changed an answer — both mean cross-target lockstep would
/// desync, and both need a deliberate decision, not a re-bless. The
/// full cross-target vector battery is issue #909; this is only the
/// tripwire.
#[test]
fn wrapper_outputs_are_bit_exact() {
    let cases: [(f32, u32); 14] = [
        (sin(1.0), 0x3f576aa4),
        (sin(-2.5), 0xbf193578),
        (cos(1.0), 0x3f0a5140),
        (cos(-2.5), 0xbf4d17bf),
        (tan(0.7), 0x3f57a036),
        (asin(0.6), 0x3f24bc7e),
        (acos(0.6), 0x3f6d6338),
        (atan(1.5), 0x3f7b985f),
        (atan2(1.0, -2.0), 0x402b6374),
        (powf(2.5, 1.3), 0x40529f03),
        (exp(1.0), 0x402df854),
        (ln(2.0), 0x3f317218),
        (log10(10.0), 0x3f800000),
        (log10(100.0), 0x40000000),
    ];
    for (i, (got, want)) in cases.iter().enumerate() {
        assert_eq!(
            got.to_bits(),
            *want,
            "case {i}: got {got} ({:#010x}), want bits {want:#010x}",
            got.to_bits(),
        );
    }
}

/// `sin_cos` must agree bit-for-bit with the individual `sin`/`cos`
/// wrappers, so call sites can use either form interchangeably.
#[test]
fn sin_cos_matches_sin_and_cos() {
    for x in [0.0_f32, 1.0, -2.5, std::f32::consts::PI, 100.0] {
        let (s, c) = sin_cos(x);
        assert_eq!(s.to_bits(), sin(x).to_bits());
        assert_eq!(c.to_bits(), cos(x).to_bits());
    }
}

/// Keep distance calculations finite and nonzero when squaring would
/// overflow or underflow, as well as pinning the ordinary result.
#[test]
fn hypot_preserves_scaled_distances() {
    for scale in [2.0_f32.powi(-100), 1.0, 2.0_f32.powi(100)] {
        assert_eq!(
            hypot(3.0 * scale, 4.0 * scale).to_bits(),
            (5.0 * scale).to_bits(),
        );
    }
}
