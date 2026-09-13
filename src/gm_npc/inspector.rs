//! Constrained Live reading tokens over the exact field and selected authored
//! choice. Derived AI scores, hull, position and unrelated choices do not stale
//! a doctrine edit. This is optimistic concurrency, never an authority token.
use super::*;
use crate::inspector::{FieldDescriptor, FieldOrigin, LiveMutability};

pub fn valid_revision(revision: &str) -> bool {
    revision.len() == 16
        && revision
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

pub fn revision(
    target: &str,
    doctrine: &[DoctrineObjective],
    state: &NpcDoctrineState,
    choice: &NpcDoctrinePaletteEntry,
) -> String {
    // Reuse the existing canonical doctrine representation: the TOML schema's
    // flattened fields cannot be serialized directly through postcard.
    let fields = (
        "npc-live-inspector-v1",
        target,
        doctrine
            .iter()
            .map(doctrine_digest_fields)
            .collect::<Vec<_>>(),
        state.0.as_ref().map(applied_digest_fields),
        &choice.id,
        &choice.label,
        &choice.targets,
        &choice.origin_layer,
        choice
            .doctrine
            .iter()
            .map(doctrine_digest_fields)
            .collect::<Vec<_>>(),
    );
    format!("{:016x}", vellum_digest::digest_postcard(&fields))
}

pub fn descriptor(kind: &str, mutability: LiveMutability, schema_path: &str) -> FieldDescriptor {
    FieldDescriptor {
        kind: kind.into(),
        default_source: None,
        live_mutability: mutability,
        origin: FieldOrigin {
            schema_path: schema_path.into(),
            // Runtime retains the origin layer, not the exact source member or
            // scalar span. Do not fabricate a filename or capture live source.
            document: None,
            line: None,
            layer: None,
        },
        validation: Vec::new(),
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NpcInspector {
    pub current: FieldDescriptor,
    pub intent: FieldDescriptor,
    pub definition: FieldDescriptor,
}

impl Default for NpcInspector {
    fn default() -> Self {
        let mut current = descriptor("enum", LiveMutability::NamedAction, "behaviour.doctrine");
        current.validation = vec![
            "inspector.validation.authored_choice".into(),
            "inspector.validation.npc_compatible".into(),
            "inspector.validation.current_reading".into(),
        ];
        Self {
            current,
            intent: descriptor(
                "string",
                LiveMutability::Derived,
                "scored_objectives.chosen",
            ),
            definition: descriptor(
                "string",
                LiveMutability::RecreateRequired,
                "gm_npc_doctrine_palette.doctrine",
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const REQUEST: &str = include_str!("../../tests/fixtures/npc-live-request.json");

    #[test]
    fn live_inspector_codec_accepts_only_the_named_bounded_checked_action() {
        let request = crate::core::codec::decode_gm_action_request(REQUEST).unwrap();
        assert!(
            matches!(request.action, crate::gm_action::GmAction::SetNpcDoctrineChecked { target, doctrine, expected_revision }
            if target == "courier" && doctrine == "north" && expected_revision == "0123456789abcdef")
        );
        for invalid in [
            REQUEST.replace("0123456789abcdef", "NaN"),
            REQUEST.replace("0123456789abcdef", "0123456789ABCDEF"),
            REQUEST.replace("\"north\"", "\"\""),
            REQUEST.replace("\"target\":", "\"raw_ecs\":{},\"target\":"),
            REQUEST.replace("\"expected_revision\":\"0123456789abcdef\",", ""),
        ] {
            assert!(
                crate::core::codec::decode_gm_action_request(&invalid).is_none(),
                "{invalid}"
            );
        }
    }

    #[cfg(all(feature = "host", not(target_arch = "wasm32")))]
    #[test]
    fn live_inspector_native_bridge_delivers_the_same_checked_request_only_from_its_active_pane() {
        use crate::native_host::{
            native_gm::{bridge::NativeGmBridge, NativeGmRecord},
            panes::{registry::PaneId, surface::RecordingSurface},
        };
        let bridge = NativeGmBridge::default();
        bridge.activate(PaneId(41));
        let mut page = RecordingSurface::ready();
        let record = serde_json::json!({"kind":"action", "request":REQUEST}).to_string();
        page.queue_record(record.clone());
        bridge.pump(PaneId(41), &mut page);
        let records = bridge.take_records();
        assert_eq!(records.len(), 1);
        let NativeGmRecord::Action { request } =
            crate::core::codec::decode_native_gm_record(&records[0]).unwrap()
        else {
            panic!("checked action record")
        };
        assert!(crate::core::codec::decode_gm_action_request(&request).is_some());
        bridge.activate(PaneId(42));
        page.queue_record(record);
        bridge.pump(PaneId(41), &mut page);
        assert!(bridge.take_records().is_empty());
    }
}
