use super::*;
use crate::perf::profile;

#[test]
fn the_sampler_records_one_tick_sample_per_tick() {
    let mut sampler = TickSampler::new();
    for _ in 0..3 {
        sampler.tick_begin();
        sampler.tick_end();
    }
    let capture = sampler.finish("test-scenario", profile(RUNTIME));
    assert_eq!(capture.summaries[TICK_METRIC].summary.count, 3);
    assert_eq!(capture.summaries[RUN_METRIC].summary.count, 1);
}
