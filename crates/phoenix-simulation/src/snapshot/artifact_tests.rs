use super::*;

#[test]
fn format_17_without_apply_outcomes_or_pending_station_commands_is_refused() {
    let previous = vellum_save::Versions::new(17, SIMULATION_RULES, 0);
    let current = vellum_save::Versions::new(SNAPSHOT_FORMAT, SIMULATION_RULES, 0);
    assert!(matches!(
        previous
            .check(&current)
            .expect_err("format 17 cannot distinguish Applied from an undelivered payload"),
        vellum_save::Moved::Format { .. },
    ));
}

#[test]
fn format_21_without_the_armed_direct_effects_is_refused() {
    let previous = vellum_save::Versions::new(21, SIMULATION_RULES, 0);
    let current = vellum_save::Versions::new(SNAPSHOT_FORMAT, SIMULATION_RULES, 0);
    assert!(matches!(
        previous
            .check(&current)
            .expect_err("format 21 records an Applied hit nothing will ever land"),
        vellum_save::Moved::Format { .. },
    ));
}
