# Authoring GM Comms routes

Issue #1317 exposes only the routes declared by the loaded root world. Existing worlds with no `gm_comms_route` offer no transmission controls. All display labels and fictional scripted prose use String Table ids.

```toml
[[gm_comms_route]]
id = "liaison-private"
label = "world.example.route.private"
visibility = "selected_ships"
senders = ["world.example.liaison"]

[[gm_comms_route.hail]]
id = "docking-offer"
label = "world.example.hail.docking"
script_path = "assets/worlds/example.toml#script.setup"
root_fn = "offer_docking"
```

Each sender is an existing authored entity reference name, resolved to its live UUID. It must be alive and carry the ordinary hailable endpoint and Comms range components. Destroyed, missing, or ambiguous identities are refused. Recipients are live Fleet ships with a declared Comms System; their existing Fleet ordinal distinguishes identically named hulls.

`selected_ships` permits an explicit nonempty selection. `fleet` captures every offered live recipient when pressed and refuses if that membership changes before application. Both route kinds deliver through ordinary per-ship Comms projection and history. They do not grant communication range: ordinary range flags and reply gates still apply. No route accepts arbitrary sender names, asset paths, or script functions from the browser.

The text field accepts 1–4096 UTF-8 bytes and retains whitespace, Unicode, markup characters, and String Table-like text exactly. It creates a separate ordinary inbox thread for each selected recipient; recipients cannot read, answer, or clear another ship's private copy. The GM journal records the complete intent and operator, while crew messages identify only the fictional sender.

A hail choice names a compiled root-world function with one context argument. It enters the ordinary `open_comms` queue and uses the existing script budget, node projection, effects, responses, and continuation. Its Applied result means **queued**; ordinary authored evaluation can produce no node. Sender identity and recipient are rechecked when the queue drains. Later dialogue nodes retain their original recipient. The probe in `assets/worlds/probe_gm_comms.toml` provides a complete root and response example.

The panel's `confirmAction` dependency receives category `comms.send` and an immutable preview through the shared #1315 controller. Correlation and Pending state are created only on acceptance, after checking the same operator and session epoch. A captured sender, route, hail, or recipient that becomes unavailable still reaches ordinary canonical admission and produces an attributed Refused result. Cancel creates no action.

[ai] Routing, literal mode, pending sender UUID, dialogue audience, and the captured recipients on durable GM results are authoritative snapshot/digest inputs. New fields default for older catalogue parsing, but snapshot format 32 refuses incompatible continuation. Comms entered at format 31 after Objectives (28), contact overrides (29), and System latches (30); NPC doctrine extends that combined state at 32. Recipient scope stays with operational history even after a ship disappears or a local refusal has no journal grant. Protocol 4 prevents older clients from accidentally localising literal text.
