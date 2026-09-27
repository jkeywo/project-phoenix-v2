# Host setup and recovery localisation (#1538)

The existing landing, scenario and ship stages already select String Ids in
`gui/host-landing-view.js` and `gui/host-scenarios.js`. The shared renderers use
the host's private locale. The AI launch control is `server.launch_ai_ship`.
`tests/client/host-journey-localisation.test.js` mounts the real host markup,
renders setup and selection in German, keeps the selected scenario's machine id
unchanged, observes English fallback for an untranslated ship heading, and
resolves the launch control in German.

| Prior gap | Display edge and outcome |
| --- | --- |
| Browser save/restore status came from Rust as English prose. | `wasm_snapshot_status()` keeps its one-shot, tab-separated envelope, but its message is now a semantic JSON outcome with numeric ticks and technical parameters. `gui/snapshot-status.js` resolves the sentence through the String Table at the host display edge. |
| Saved-session staging and import exceptions were literal English arguments to `showSnapshotStatus()`. | The host renders stable IDs and retains the version or browser error as `{detail}`. Existing refusal decisions and URL cleanup are unchanged. |
| A save click before a simulation existed reported a hardcoded English sentence. | Both the direct host check and Rust's fixed-tick refusal select `server.snapshot.no_run`. |
| Native mod-pack shelf failures included host-authored English sentences. | The landing renderer resolves shelf scan, vanished archive and unreadable archive outcomes through IDs. Archive names and OS errors remain parameters. Validator findings keep their exact technical detail. |
| The string checker saw `.textContent` and markup but missed status helper arguments. | `untranslatedSnapshotStatus()` reports direct prose literals sent to the host status sink; its focused test pins this shape. It does not scan identifiers or developer logs. |

The browser host's mod-pack validator details and native validator findings
remain technical reports from the active content and filesystem. Their headings,
severity and outcome frames use existing String Ids; #1546 owns Workshop
diagnostic authoring. New T5 surfaces own their own display text.
