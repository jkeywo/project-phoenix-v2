//! The post-mission report accumulator (issue #1344, PRD #1337).
//!
//! [`crate::core::report`] owns the vocabulary — what a report row IS and what
//! the accumulator promises about order and totals. This module owns the one
//! system that WRITES it, and it lives at the crate root rather than under
//! `core` for the same reason [`crate::narrative`] does: it touches the script
//! boundary's effect queue, which `core` must not depend on.
//!
//! # One writer, and it is not the applier
//!
//! `ctx.effects.report_row(#{ … })` resolves at the script boundary, but the
//! shared dispatch applier cannot write the report there: it is lent plain
//! `&mut Vec<_>` sinks and holds no resources, and the three systems that call
//! it are at Bevy's parameter limit already. So a row rides the #1223
//! effect-queue pattern exactly as an authored narrative beat does —
//! `apply_dispatch_result` buffers a [`crate::core::report::ReportRow`] onto
//! `EffectQueue<ReportRow>`, and [`apply_report_rows`] drains it in queue order
//! into the [`MissionReport`] resource.
//!
//! Having exactly one writer is what makes the accumulator's ordering promise
//! true: rows land in the order the mission produced them, and that order is
//! what the digest folds and what the player reads.
//!
//! # Why a beat only on a real change
//!
//! A scenario may re-state a row every tick — a running count of who got off
//! the rock, re-scored as the window moves. That is a RATE, and
//! [`MissionReport::set_row`] reports whether anything actually moved so this
//! system emits [`NarrativeKind::ReportRowUpdated`] only when it did. The
//! kind is additionally held out of the ndjson stream
//! ([`NarrativeKind::in_timeline_stream`]), so even a genuinely churning row
//! reaches the folded timeline without flooding a reader's scrollback.
//!
//! # Determinism
//!
//! The queue is drained front-to-back and the report is a `Vec` in write order:
//! no `HashMap` walk, no clock, no RNG. [`MissionReport`] is authoritative
//! state — `src/sim_digest.rs` folds it and `src/snapshot.rs` captures and
//! restores it — because a scenario script decides its contents and two
//! instances on the same seed must agree on them.

use bevy::prelude::*;

use crate::core::narrative::{NarrativeEvent, NarrativeKind, NarrativeValue};
use crate::core::report::{MissionReport, ReportRow};
use crate::effect_queue::EffectQueue;

/// The narrative beat one changed row produces.
///
/// `id` is the authored row id — the row's identity, not prose. The detail
/// carries the two String Ids the surface localizes, the semantic state, and
/// the hidden score: the timeline is a DIAGNOSTIC surface (headless only), so
/// the score belongs here even though it never reaches a player.
fn row_event(row: &ReportRow) -> NarrativeEvent {
    NarrativeEvent::new(NarrativeKind::ReportRowUpdated, row.id.clone())
        .text("heading", row.heading_id.clone())
        .text("outcome", row.outcome_id.clone())
        .text("state", row.state.as_str())
        .detail("score", NarrativeValue::Int(i64::from(row.score)))
}

/// The `report_finalized` beat, given the ending's reason String Id.
///
/// Built here rather than inline at the game-over site so the two report beats
/// are shaped in one place and the emitting system stays about broadcasting.
pub fn finalized_event(reason_id: &str, report: &MissionReport) -> NarrativeEvent {
    NarrativeEvent::new(NarrativeKind::ReportFinalized, reason_id.to_string())
        .detail("rows", NarrativeValue::Int(report.rows().len() as i64))
        .detail("total", NarrativeValue::Int(i64::from(report.total())))
}

/// Drain the script boundary's report-row queue into [`MissionReport`], beating
/// every row that actually moved onto the narrative timeline.
///
/// Both resources are `Option` so a bare-`App` fixture that runs this system
/// without the sim's registration does nothing rather than panicking — the same
/// shape [`crate::narrative::emit_authored_and_marked_entity_narrative`] uses
/// for its own queue.
pub fn apply_report_rows(
    queue: Option<ResMut<EffectQueue<ReportRow>>>,
    report: Option<ResMut<MissionReport>>,
    mut out: MessageWriter<NarrativeEvent>,
) {
    let (Some(mut queue), Some(mut report)) = (queue, report) else {
        return;
    };
    if queue.0.is_empty() {
        return;
    }
    // Taken whole, then applied in queue order: two writes to the same row id
    // in one tick settle to the LAST one, which is what "the tick is the unit"
    // means here as much as it does for an authored narrative outcome.
    for row in std::mem::take(&mut queue.0) {
        let event = row_event(&row);
        if report.set_row(row) {
            out.write(event);
        }
    }
}

/// The run boundary for the report: a starting round holds no rows.
///
/// [`MissionReport`] is PER-RUN state, and a session can play many runs — a
/// round ends, `ReturnToLobby` sends everyone back, and another scenario
/// starts in the same process. Without this, round two inherits round one's
/// rows: a combat_test run that authored no report at all would still publish
/// Falling Skyway's saved-Lyra row, be classified `reported`, and beat
/// `ReportFinalized` over a report nobody wrote. Even a second run of the SAME
/// scenario would show what the PREVIOUS crew did — which is precisely the
/// report lying about the mission that [`crate::core::report`]'s module docs
/// forbid.
///
/// Registered on `OnEnter(GamePhase::InProgress)` beside
/// [`crate::command_admission::reset_command_log`], which exists at the same
/// seam for the identical reason.
///
/// The queue is cleared too. It is `ClearedAtFold` and empty at every boundary
/// in practice, but "a row queued before this run started belongs to this run"
/// is not a claim worth leaving to scheduling: clearing it costs nothing and
/// makes the boundary total.
pub fn reset_mission_report(
    mut queue: ResMut<EffectQueue<ReportRow>>,
    mut report: ResMut<MissionReport>,
) {
    queue.0.clear();
    *report = MissionReport::default();
}

#[cfg(test)]
#[path = "mission_report_tests.rs"]
mod tests;
