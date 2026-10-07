fn stamp_json(stamp: &crate::stamp::DeliveryStamp) -> serde_json::Value {
    serde_json::json!({
        "protocol": stamp.protocol,
        "content_id": stamp.content_id,
        "content_epoch": stamp.content_epoch,
    })
}

/// Encode a host's own version stamp — the body of `/host/stamp.json`.
pub fn encode_delivery_stamp(stamp: &crate::stamp::DeliveryStamp) -> String {
    stamp_json(stamp).to_string()
}

/// Advertise the native delivery owner's asset revision without losing u64
/// precision in a pure-JS consumer. This is local delivery metadata, not a
/// simulation message or a cue record.
pub fn encode_native_asset_revision(revision: u64) -> String {
    serde_json::json!({
        "capability": "phoenix-native-asset-revision", "version": 1,
        "revision": revision.to_string(),
    })
    .to_string()
}

/// Encode the browser host's join-handshake verdict (issue #1111).
///
/// The same `StampMismatch::code()` the native host answers
/// `/host/manifest.json` with, shaped for the host page's JS rather than for
/// HTTP: `{"ok":true,…}` or `{"ok":false,"code":…,"detail":…}`, always carrying
/// the host's own stamp so a refused client can see what it should have been.
/// `detail` is operator prose, not player-visible text — see
/// [`crate::stamp::StampMismatch::detail`].
pub fn encode_join_verdict(
    verdict: &Result<(), crate::stamp::StampMismatch>,
    host: &crate::stamp::DeliveryStamp,
) -> String {
    match verdict {
        Ok(()) => serde_json::json!({ "ok": true, "host": stamp_json(host) }).to_string(),
        Err(mismatch) => serde_json::json!({
            "ok": false,
            "code": mismatch.code(),
            "detail": mismatch.detail(),
            "host": stamp_json(host),
        })
        .to_string(),
    }
}

/// Encode the content manifest + catalogue a host publishes.
pub fn encode_delivery_manifest(manifest: &crate::DeliveryManifest) -> String {
    serde_json::json!({
        "stamp": stamp_json(&manifest.stamp),
        "manifest_path": manifest.manifest_path,
        "scenarios": manifest.scenarios,
    })
    .to_string()
}

/// Encode a version-pin refusal — the body of a `409` from either host.
pub fn encode_delivery_refusal(refusal: &crate::DeliveryRefusal) -> String {
    serde_json::json!({
        "error": refusal.mismatch.code(),
        "detail": refusal.mismatch.detail(),
        "host": stamp_json(&refusal.host),
    })
    .to_string()
}
#[cfg(test)]
#[path = "codec_tests.rs"]
mod tests;
