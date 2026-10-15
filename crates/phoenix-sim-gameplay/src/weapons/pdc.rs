//! Three-dimensional straight-bolt geometry. Observations belong to callers;
//! physical collision uses the same actual projectile vector after launch.

/// Constant-velocity lead from an already permitted measured XYZ sample.
/// Unknown elevation is not accepted here; callers must withhold the launch.
/// A kinematically unreachable or nonfinite solution is absent, never a private
/// position fallback. Angles use the existing heading/-Z and Y-up convention.
pub fn lead_angles(
    origin: [f32; 3],
    position: [f32; 3],
    velocity: [f32; 3],
    speed: f32,
) -> Option<[f32; 2]> {
    if !origin
        .into_iter()
        .chain(position)
        .chain(velocity)
        .all(f32::is_finite)
        || !speed.is_finite()
        || speed <= 0.0
    {
        return None;
    }
    let r = std::array::from_fn::<_, 3, _>(|i| f64::from(position[i]) - f64::from(origin[i]));
    let v = velocity.map(f64::from);
    let dot = |a: [f64; 3], b: [f64; 3]| a.into_iter().zip(b).map(|(a, b)| a * b).sum::<f64>();
    let c = dot(r, r);
    if c == 0.0 {
        return None;
    }
    let a = dot(v, v) - f64::from(speed) * f64::from(speed);
    let b = 2.0 * dot(r, v);
    let time = if a == 0.0 {
        let t = -c / b;
        (t.is_finite() && t > 0.0).then_some(t)?
    } else {
        let discriminant = b * b - 4.0 * a * c;
        if discriminant < 0.0 || !discriminant.is_finite() {
            return None;
        }
        // Stable quadratic roots avoid subtracting near-equal positive terms.
        let q = -0.5 * (b + libm::copysign(libm::sqrt(discriminant), b));
        [q / a, c / q]
            .into_iter()
            .filter(|t| t.is_finite() && *t > 0.0)
            .min_by(f64::total_cmp)?
    };
    let d = std::array::from_fn::<_, 3, _>(|i| r[i] + v[i] * time);
    if !d.into_iter().all(f64::is_finite) {
        return None;
    }
    let heading = libm::atan2(d[0], -d[2]) as f32;
    let pitch = libm::atan2(d[1], libm::sqrt(d[0] * d[0] + d[2] * d[2])) as f32;
    (heading.is_finite() && pitch.is_finite()).then_some([heading, pitch])
}

/// First physical sphere contact along an actual last-step bolt segment.
/// Closest-point/half-chord arithmetic avoids the cancellation of a quadratic
/// discriminant for tiny bodies crossed by long links. Ties are left to the
/// existing owner (stable target ID), not hidden faction or selected target.
pub fn swept_sphere_fraction(
    start: [f32; 3],
    end: [f32; 3],
    center: [f32; 3],
    radius: f32,
) -> Option<f32> {
    if !start
        .into_iter()
        .chain(end)
        .chain(center)
        .all(f32::is_finite)
        || !radius.is_finite()
        || radius < 0.0
    {
        return None;
    }
    let relative_start = std::array::from_fn(|i| f64::from(start[i]) - f64::from(center[i]));
    let relative_end = std::array::from_fn(|i| f64::from(end[i]) - f64::from(center[i]));
    relative_sphere_fraction(relative_start, relative_end, f64::from(radius))
}

/// Both objects traverse this same fixed interval. Widen before subtraction
/// and radius addition so finite f32 positions/radii cannot overflow first.
pub fn moving_sphere_fraction(
    start: [f32; 3],
    end: [f32; 3],
    radius: f32,
    target_start: [f32; 3],
    target_end: [f32; 3],
    target_radius: f32,
) -> Option<f32> {
    if !start
        .into_iter()
        .chain(end)
        .chain(target_start)
        .chain(target_end)
        .all(f32::is_finite)
        || !radius.is_finite()
        || !target_radius.is_finite()
        || radius < 0.0
        || target_radius < 0.0
    {
        return None;
    }
    let relative_start = std::array::from_fn(|i| f64::from(start[i]) - f64::from(target_start[i]));
    let relative_end = std::array::from_fn(|i| f64::from(end[i]) - f64::from(target_end[i]));
    relative_sphere_fraction(
        relative_start,
        relative_end,
        f64::from(radius) + f64::from(target_radius),
    )
}

/// Intersect only the common live part of two same-step travelled segments.
/// The result is a fraction of the original bolt segment, so static body hits
/// and guided hits share one ordering. New emissions have no travelled interval
/// yet and cannot collide retrospectively with the other object's earlier path.
#[allow(clippy::too_many_arguments)]
pub fn moving_sphere_interval_fraction(
    start: [f32; 3],
    end: [f32; 3],
    radius: f32,
    seconds: f32,
    target_start: [f32; 3],
    target_end: [f32; 3],
    target_radius: f32,
    target_seconds: f32,
) -> Option<f32> {
    if !start
        .into_iter()
        .chain(end)
        .chain(target_start)
        .chain(target_end)
        .all(f32::is_finite)
        || !seconds.is_finite()
        || seconds <= 0.0
        || !target_seconds.is_finite()
        || target_seconds <= 0.0
        || !radius.is_finite()
        || radius < 0.0
        || !target_radius.is_finite()
        || target_radius < 0.0
    {
        return None;
    }
    let common = f64::from(seconds.min(target_seconds));
    let bolt_fraction = common / f64::from(seconds);
    let target_fraction = common / f64::from(target_seconds);
    let relative_start = std::array::from_fn(|i| f64::from(start[i]) - f64::from(target_start[i]));
    let relative_end = std::array::from_fn(|i| {
        let bolt = f64::from(start[i]) + (f64::from(end[i]) - f64::from(start[i])) * bolt_fraction;
        let target = f64::from(target_start[i])
            + (f64::from(target_end[i]) - f64::from(target_start[i])) * target_fraction;
        bolt - target
    });
    relative_sphere_fraction(
        relative_start,
        relative_end,
        f64::from(radius) + f64::from(target_radius),
    )
    .map(|fraction| fraction * bolt_fraction as f32)
}

fn relative_sphere_fraction(start: [f64; 3], end: [f64; 3], radius: f64) -> Option<f32> {
    let d = std::array::from_fn::<_, 3, _>(|i| end[i] - start[i]);
    let r = start.map(|value| -value);
    let length_sq = d.into_iter().map(|v| v * v).sum::<f64>();
    let radius_sq = radius * radius;
    if length_sq == 0.0 {
        return (r.into_iter().map(|v| v * v).sum::<f64>() <= radius_sq).then_some(0.0);
    }
    let t = d.into_iter().zip(r).map(|(d, r)| d * r).sum::<f64>() / length_sq;
    let perpendicular = std::array::from_fn::<_, 3, _>(|i| r[i] - d[i] * t);
    let height_sq = radius_sq - perpendicular.into_iter().map(|v| v * v).sum::<f64>();
    if height_sq < 0.0 {
        return None;
    }
    let half = libm::sqrt(height_sq / length_sq);
    let enter = t - half;
    let leave = t + half;
    (leave >= 0.0 && enter <= 1.0).then_some(enter.max(0.0) as f32)
}

#[cfg(test)]
#[path = "pdc_tests.rs"]
mod tests;
