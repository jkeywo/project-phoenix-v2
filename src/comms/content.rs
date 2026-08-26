// Pure comms runtime state and the wire-shaped dialogue vocabulary.
//
// Pure Rust module - no Bevy. This is the state half of the Comms concept
// (issue #816); the Bevy applier (`comms::server`, `comms::scripted`) stays
// dumb.
//
// Issue #985 (Rhai M7) deleted the declarative front-end this module used to
// evaluate. `CommsTemplateState`, `FiredCommsTemplate`, `PendingFollowUp`,
// `evaluate_comms_templates`, `follow_up_trigger_holds` and
// `comms_template_states_from_world` all read `[[comms]]` TOML, which no longer
// parses; the ONE front-end is now `ctx.effects.open_comms(#{...})` and the
// dialogue-node fns behind it (`world::script::comms`). What survives is what
// the script path reuses:
//
//   * `CommsDialogueNode` / `CommsResponse` - the shown node, in wire shape.
//     They lived in `world::config` while they were TOML vocabulary shared with
//     `TriggerCondition`/`TriggerAction`; with the `[[comms]]` parser gone they
//     are runtime projection types with no authored form, so they live here
//     with the rest of the comms runtime state.
//   * `ActiveDialogue` - dialogue state machine entry.
//   * `ScriptedDialogue` - the script half of one.
//   * `OpenCommsRequest` - a scripted request to open a thread.
//   * `response_views` - the node -> wire projection.

use serde::{Deserialize, Serialize};

use crate::core::messages::{CommsPriority, CommsResponseView};

// -- The shown node ---------------------------------------------------------

/// A single response option on the dialogue node currently being shown.
///
/// Text and the confirm flag, and nothing else. The declarative shape carried
/// `actions: Vec<TriggerAction>` and a nested `follow_up: Option<..>` as well,
/// because a TOML response WAS its own effects and its own branch; a scripted
/// response is a name (`on_pick`, parallel in [`ScriptedDialogue::on_pick`]) and
/// the fn behind it supplies both when the player picks it. Issue #985 deleted
/// the two fields with the parser that was their only writer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommsResponse {
    /// Player-facing button text.
    pub text: String,
    /// True when the author marked this response important, so the client
    /// confirms before submitting it (issue #761). Not a strings.csv id - the
    /// authored `text` stays authored content, and this flag rides the wire in
    /// `CommsResponseView`.
    pub important: bool,
}

/// The dialogue node currently being shown for one message.
///
/// Built by [`project_node`](crate::world::script::comms::project_node) from the
/// `#{message, params, responses}` map a node fn returned - the one place script
/// meets wire shape. It carried a `speaker` override and an injection-gating `trigger`
/// while `[[comms]]` authored follow-up trees; both were declarative-only and
/// went with the parser in issue #985 (who is calling is metadata on the OPEN -
/// `OpenCommsRequest::display_name` - and a node in `active_dialogues` has by
/// definition already been injected).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommsDialogueNode {
    /// The message body shown in the inbox, as a `strings.csv` id.
    pub body: String,
    /// Runtime values to interpolate into `body`'s `{placeholder}` tokens. A
    /// `BTreeMap`, so the wire encoding it becomes is key-ordered and
    /// deterministic; see `messages::TEXT_PARAMS_SUFFIX`. Empty for every node
    /// whose copy names no figure, which is all of them but one.
    pub body_params: std::collections::BTreeMap<String, String>,
    /// The response options offered on this node.
    pub responses: Vec<CommsResponse>,
}

/// Project a node's responses onto the wire `CommsResponseView` vector (issue
/// #761). Each view carries the authored `text`, the authored `important` flag,
/// and the current `available` (sender-in-range) flag. All responses in a
/// message share the same availability - a response is "unavailable" exactly
/// when its message's sender is out of comms range, the same authoritative
/// reachability that stamps `CommsMessage::sender_in_range`.
pub fn response_views(responses: &[CommsResponse], available: bool) -> Vec<CommsResponseView> {
    responses
        .iter()
        .map(|r| CommsResponseView {
            text: r.text.clone(),
            important: r.important,
            available,
        })
        .collect()
}

// -- Runtime state ----------------------------------------------------------

/// Runtime state for one active dialogue conversation.
#[derive(Clone, Debug)]
pub struct ActiveDialogue {
    /// The current dialogue node being presented - the *projected* node
    /// ([`project_node`](crate::world::script::comms::project_node)): the
    /// authored body and response texts, because a response's effects and
    /// follow-up come from calling its `on_pick` fn, not from the node.
    pub current_node: CommsDialogueNode,
    /// Thread identifier shared by all messages in this dialogue tree.
    /// Set when the first message is injected; follow-ups inherit the same id.
    pub thread_id: String,
    /// The Rhai dialogue tree driving this thread (issue #984).
    ///
    /// It was `Option` while declarative `[[comms]]` threads coexisted, with
    /// `None` meaning "answered by the declarative arm of
    /// `handle_respond_to_message`". Issue #985 deleted that arm and every
    /// writer of `None` with it, so the plain field is the honest shape: every
    /// live dialogue is entered through `open_scripted_comms_threads` or
    /// advanced by an `on_pick`, and both carry the script.
    pub script: ScriptedDialogue,
}
/// The script half of an [`ActiveDialogue`] — what a scripted thread needs to
/// answer the next `RespondToMessage` (issue #984).
///
/// Strings only, so the dialogue state round-trips through a save exactly as
/// [`OpenCommsRequest`] does: no `AST`, no closure, no `Entity`. Both of them DO
/// travel, and travel together — S8 put `CommsRuntime::active_dialogues` and
/// `WorldScriptRuntime::pending_comms_opens` into
/// [`PhoenixSnapshot::comms`](crate::snapshot::PhoenixSnapshot::comms) in one
/// slice, because a restored `pending_comms_opens` without its
/// `active_dialogues` would re-open threads the player had already answered.
///
/// The restore resolves `node_fn` and `on_pick` against the **recompiled**
/// script set, and what makes that the same tree the capture read is issue
/// #864's content binding rather than anything stored here: the load returns the
/// compiled set's `content_hash` as a ledger record for its caller to apply
/// (issue #1241),
/// `snapshot::content_digest` folds it, and `Versions::check` refuses a save
/// whose scripts moved — so an edited `on_pick` body refuses the save instead of
/// resolving the name against a different fn.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScriptedDialogue {
    /// Content-relative path of the unit defining this thread's node fns — the
    /// same `(script_path, fn_name)` key a
    /// [`ScheduledCall`](crate::world::script::schedule::ScheduledCall) carries,
    /// and for the same reason (short and anonymous fn names are not unique
    /// across units).
    pub script_path: String,
    /// Supporting-world layer whose handler opened this thread. `None` is the
    /// base world. The path alone is not ownership: sibling script units may be
    /// shared by several layers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin_layer: Option<String>,
    /// The fn that produced the node currently shown.
    pub node_fn: String,
    /// The `on_pick` fn name for each response, PARALLEL to
    /// [`ActiveDialogue::current_node`]'s `responses` — so the index the player
    /// submits addresses both the shown button and the fn that answers it.
    pub on_pick: Vec<String>,
}

/// A script's request to open a comms thread — what
/// `ctx.effects.open_comms(#{…})` buffers (issue #984).
///
/// The metadata lives on the OPEN, not on the node, mirroring the declarative
/// split between [`CommsTemplate`] (`from` / `display_name` / `thread_id` /
/// `urgent`) and [`CommsDialogueNode`] (`message` / `responses`): follow-up
/// nodes inherit the thread rather than restating who is calling.
///
/// Strings and one bool, no `Entity` and no closures — so the queue this lands
/// on round-trips through a save (issue #864) as plainly as
/// [`ScheduledCall`](crate::world::script::schedule::ScheduledCall) does. `from`
/// is the sender's authored reference id, resolved through `name_to_uuid` when
/// the request is materialised; `root_fn` is the dialogue node fn to enter, and
/// `script_path` the unit that defines it — the pair is exactly the
/// `(script_path, fn_name)` key a scheduled callback carries, and for the same
/// reason (anonymous and short fn names are not unique across units).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenCommsRequest {
    /// Sender entity **reference id** (resolved to the sender UUID at open).
    pub from: String,
    /// The dialogue node fn to enter — the thread's root node.
    pub root_fn: String,
    /// Optional player-facing sender display text, independent of `from`.
    /// `None` falls back to `from`, as a template's `display_name` does.
    pub display_name: Option<String>,
    /// Joins an existing thread when set; a fresh id is minted when absent.
    pub thread_id: Option<String>,
    /// Generic authoritative priority. `Routine` plus legacy `urgent = true`
    /// decodes as Urgent through [`effective_priority`](Self::effective_priority).
    #[serde(default)]
    pub priority: CommsPriority,
    /// Compatibility fallback retained for snapshots authored before priority.
    /// New script opens project this boolean from `priority`.
    pub urgent: bool,
    /// Content-relative path of the unit defining `root_fn`. Stamped at the
    /// host's drain boundary, not by the script — the effect sink cannot know
    /// which unit is running, exactly as `ScheduleSink::drain` stamps a
    /// callback's path there.
    pub script_path: String,
    /// Supporting-world layer whose call queued this open. Stamped by the host
    /// alongside `script_path`, never authored by Rhai.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin_layer: Option<String>,
}

impl OpenCommsRequest {
    pub fn effective_priority(&self) -> CommsPriority {
        if self.priority == CommsPriority::Routine && self.urgent {
            CommsPriority::Urgent
        } else {
            self.priority
        }
    }
}
