# Acceptance kit — issue #1126, assign and test media devices per bridge surface

**This is the human half of issue #1126.** The automated half is done and is
listed at the bottom; it proves the media-assignment *logic* — the stable device
identity, the per-surface camera/microphone/output assignment, the parse/validate
failure taxonomy, the explicit-consent sharing rule, the deterministic default,
and the missing/denied-device resolution — with pure tests that run in CI on no
hardware. None of that can prove the only questions that matter on a real bridge:
**that each surface actually captures from its assigned camera and microphone and
plays to its assigned output, that previewing/metering/testing a device before
play works, and that a device really is unusable-but-not-fatal when it is missing,
removed or denied.**

So this is a script for one operator at the bridge machine, with **camera + 2
microphones + 2 audio outputs** to hand. It has two parts:

- **Part A — author, validate, persist and reassign.** You can run this now. It
  is the closable half: it exercises the whole pure model against your *real*
  device names, confirms every refusal fires, and confirms the assignment reloads
  and can be changed.
- **Part B — live enumeration, preview/meter/test and teardown.** The native
  adapter is implemented; the multi-device human run remains NOT RUN.
  **Do not tick criterion 3 until the relevant hardware steps pass.**

## Native adapter and test commands

Build with `--features host`. Audio enumeration/testing uses CPAL; Windows camera
preview uses safe WinRT MediaCapture APIs. `--setup` lists exact identities.
Paste those keys into the existing `[[media]]` profile and reload to reassign.
Camera keys always include the OS interface ID; do not invent them from a name.
Duplicate or unnamed audio endpoints are refused by the test commands: give each
endpoint a unique OS name and enumerate again. No default device is substituted.

With `bridge-media.toml` assigning the relevant named surface:

```
phoenix-host --setup --profile bridge-media.toml --preview-camera viewscreen
phoenix-host --setup --profile bridge-media.toml --meter-microphone comms
phoenix-host --setup --profile bridge-media.toml --test-output comms
```

The camera opens a local preview window, closes on Escape/window close or after
30 seconds, and retains only its latest bounded frame. The microphone prints
peak levels for five seconds per assigned input and discards samples. Output
plays a quiet one-second tone per assigned endpoint. Each action tears down its
OS handles before returning. Only one action is accepted per invocation.
Sharing requires the existing explicit consent and prints contention warnings.
Missing, denied, disconnected and stalled devices produce diagnostic failures;
these setup processes do not start or stop a Station or a mission.
No continuous in-play capture, recording or inter-ship calls are introduced.

The camera is Windows-only. A failed enumeration reports the unavailable class
rather than falsely declaring its unqueried assignments missing. Actual permission
changes, unplugging and audible/visible correctness remain hardware checks.

---

## 0. Preconditions — do these first, in this order

- [ ] **A native host build.** The media model compiles into the ordinary host
      binary — no `ultralight` feature is needed for Part A:

      ```
      cargo build --release --features host --bin phoenix-host
      ```

- [ ] **Know your devices.** Enumerate them and copy the identities:

      ```
      ./target/release/phoenix-host --setup
      ```

      The display half lists monitors; **Media devices** lists the successfully
      enumerated device classes and exact profile keys. An unavailable backend
      is named explicitly. Copy keys exactly, including a camera's interface ID.

---

## Part A — author, validate, persist and reassign

### A1. Author a two-surface media profile

Author `bridge-media.toml` (media may share the same file as your display
profile; this kit uses a media-only file for focus). Assign your one camera and
first mic/output to the **viewscreen** surface, and your second mic/output to the
**comms** surface — two distinct endpoints, no shared device:

```toml
version = 1

[[media]]
surface = "viewscreen"
camera = "camera:Logitech BRIO#<paste-interface-id>"      # paste your camera's identity
microphone = ["mic:Blue Yeti"]        # your first microphone
output = ["output:Bridge Speakers"]   # your first output

[[media]]
surface = "comms"
microphone = ["mic:Headset Boom"]     # your second microphone
output = ["output:Comms Headset"]     # your second output
```

Validate it:

```
./target/release/phoenix-host --setup --profile bridge-media.toml
```

- [ ] **The report echoes both surfaces** with their camera/mic(s)/output(s), and
      says the media assignments are valid (no "invalid" line, no "Media
      problems"). If a backend is present it also says whether they match the
      connected devices.

      **PASS:** both surfaces listed, no error line, exit code 0.
      **On failure:** re-check each identity against `--setup`'s device list (or
      Windows' exact device name) and the `kind:` tag. A typo in a name is the
      usual cause; the error names the surface and the id.

### A2. Confirm the failure taxonomy fires on your real profile

Make each of these edits **one at a time**, run `--setup --profile
bridge-media.toml`, confirm the refusal, then undo it:

- [ ] **Wrong kind in a slot.** Put a `mic:` identity in the viewscreen's
      `camera = ` field. Expect a refusal naming the surface, the camera slot and
      the microphone you put there. **PASS:** non-zero exit, "invalid" line naming
      `viewscreen` and `camera`.
- [ ] **Malformed id.** Drop the `camera:` tag — write `camera = "Logitech BRIO"`.
      Expect a refusal that the id carries no known device kind. **PASS:** the
      report says so and exits non-zero.
- [ ] **Duplicate on one surface.** List the same mic twice in one surface's
      `microphone = [ … ]`. Expect a refusal that the device is assigned more than
      once. **PASS:** refusal names the surface and the device.
- [ ] **Duplicate surface.** Add a second `[[media]]` with `surface = "comms"`.
      Expect a refusal that a surface is named twice. **PASS:** refusal names
      `comms`.

### A3. Explicit sharing — the contention rule (acceptance criterion 2)

- [ ] **A share without consent is refused.** Assign your **first** microphone
      (`mic:Blue Yeti`) to *both* surfaces, with no `allow_shared` on either.
      Expect a refusal that sharing is an explicit choice, naming both surfaces
      and telling you to add it to `allow_shared` or give each its own device.

      **PASS:** non-zero exit, refusal names both surfaces and `allow_shared`.

- [ ] **A consented share validates with a warning.** Add
      `allow_shared = ["mic:Blue Yeti"]` to **both** surfaces. Re-validate. Expect
      it to pass, with a warning line that the device is shared by explicit
      consent across the two surfaces and the OS/driver may refuse the second use.

      **PASS:** exit 0, and a `warning:` line about the contention appears.
      **On failure:** consent must be on *every* surface using the device — a
      one-sided `allow_shared` is still refused (that is the point). Undo the
      share when done.

### A4. Persistence — the assignment reloads unchanged

- [ ] **The profile round-trips.** Launch the authoritative host with the profile
      and read the boot log:

      ```
      ./target/release/phoenix-host --world assets/worlds/combat_test.toml \
          --profile bridge-media.toml --solo
      ```

      The log prints `bridge display profile — … , N media surface(s)` with N = 2,
      and re-prints any contention warning. Close the window.

      **PASS:** the surface count and any warning match what A1/A3 showed — the
      assignment survived load exactly, with nothing re-ordered or dropped.
      **On failure:** capture the boot log (`--log info`) and note which surface
      or device changed.

### A5. Reassignment — changing a surface's devices takes effect

- [ ] **Swap the outputs between surfaces.** Edit `bridge-media.toml` so the
      viewscreen now uses `output:Comms Headset` and comms uses
      `output:Bridge Speakers`. Re-run A1's `--setup --profile` and A4's launch.

      **PASS:** the report and boot log show the swapped assignment — the change
      is picked up on reload, not cached from the previous run. (Part B confirms audio actually follows the swap.)

**Part A passes** when every box above is ticked with your real device names.
That exercises acceptance criteria 1 (assignment), 2 (explicit sharing +
warning), 5 (two distinct endpoints), and the persistence/failure half of 6, on
real multi-device hardware — everything that does not require live capture.

---

## Part B — live enumeration, preview/meter/test and teardown

Run the commands above separately for each assigned surface. These are explicit
setup diagnostics with a named surface, not persistent in-play media streams.
The operator judges image/audio correspondence and tests actual unplug/denial.

- [ ] **Live enumeration lists your real devices.** `--setup` prints each camera,
      microphone and output with an identity you can paste verbatim. Test commands
      use the explicit assignment and report OS access errors when opening it;
      enumeration alone does not establish capture permission.
      **PASS:** all 5 of your devices appear under the right kind, with correct
      names.
- [ ] **Camera preview on the viewscreen surface.** Before entering play, the
      viewscreen surface previews its assigned camera. **PASS:** you see the
      camera's live image; switching the assignment previews the other camera (if
      you have one).
- [ ] **Microphone level/test per surface.** Each surface meters its assigned
      microphone — speak, and the level moves on that surface's meter and no
      other. **PASS:** the viewscreen meter follows `mic:Blue Yeti`, the comms
      meter follows `mic:Headset Boom`, independently.
- [ ] **Output test per surface.** Each surface plays a test tone to its assigned
      output only. **PASS:** the viewscreen tone comes from `output:Bridge
      Speakers`, the comms tone from `output:Comms Headset`; neither leaks to the
      other.
- [ ] **Reassignment follows to real audio (Part A A5, live).** After swapping the
      outputs, the test tones swap devices too. **PASS:** audio follows the new
      assignment with no restart beyond the reload.
- [ ] **A shared device (A3), on hardware.** With `mic:Blue Yeti` consented to
      both surfaces, confirm the OS behaviour the warning predicted: either both
      surfaces meter it, or the second open is refused with a clear
      contention/capability message — **never** a crash. **PASS:** the behaviour
      matches the warning; note exactly what the OS did.
- [ ] **Missing device is reported, not fatal (acceptance criterion 4).** Unplug
      the camera during its preview. The diagnostic reports the failure and
      closes its handles. **PASS:** a useful failure, no crash, and an independently
      running Station remains usable. Re-enumeration reports the absent endpoint.
- [ ] **Denied device is reported, not fatal.** In Windows privacy settings, deny
      microphone access, then launch. The assigned mic resolves as denied and is
      reported; the surface stays usable for its camera and output. **PASS:** a
      "denied" report, no crash. Restore access afterward.
- [ ] **No ship-to-ship call, recording or Discord (acceptance criterion 5).**
      Confirm this build does exactly the above and nothing more — the two
      endpoints exist and are testable, but there is no inter-ship audio/video, no
      recording and no third-party integration. **PASS:** none of those behaviours
      is present; #1126 leaves them as separate future endpoints.

**Part B passes** — and only then may acceptance criterion 3 (and the live half of
1, 2 and 4) be ticked — when every box has been confirmed on real hardware with
the backend wired.

---

## Where to record results

Add a dated note to the issue #1126 thread (or the batch's acceptance log):

- which part you ran (A now, and whether B was run on the backend or deferred),
- your device identities and the `bridge-media.toml` you used,
- each box: pass / fail / not-run, with a line on anything that surprised you,
- for any failure, the operator log around it (`--log info`) and a screenshot for
  a visual issue.

If Part B has not been run, record NOT RUN and leave its criteria unticked.

---

## What the automated half already covers (no hardware)

The pure media model in `src/native_host/bridge_media.rs` is tested by
`src/native_host/bridge_media_tests.rs`, which runs in ordinary CI:

- **stable device identity** — `kind:name`, recovered back to its kind; stable
  across a re-enumeration in another order; a camera and a mic of the same name
  kept distinct by kind; two identical devices disambiguated by a hardware id or
  an enumeration ordinal; a nameless device given a stable placeholder;
- **assignment and the failure taxonomy** — a well-formed assignment validates to
  kind-checked ids; a wrong-kind device in a slot, a malformed id, a device listed
  twice on one surface, and two surfaces of one name are each refused with an
  authored message;
- **explicit sharing** — a device shared without consent from every using surface
  is refused; a fully consented share validates and warns; a one-sided consent is
  still refused;
- **resolution against present devices** — a missing device and a present-but-
  denied device are each named distinctly and leave the surface usable with its
  other devices; a present-but-unassigned device is not a problem;
- **the deterministic default** — the OS default (else first) of each kind is
  chosen when the operator has not; a denied device is skipped; a many-surface
  default consents to the forced share so it validates;
- **persistence** — the `[[media]]` tables round-trip through TOML unchanged
  inside the display profile (`bridge_profile_tests.rs`), and validate to the same
  assignments on reload;
- **the setup report** — lists devices by kind, validates hand-authored
  assignments even with no backend, and surfaces a missing device and a contention
  warning when devices are present.

The host-gated adapters are `media_camera.rs`, `media_microphone.rs` and
`media_output.rs`. Their tests cover explicit selection, missing/ambiguous
preflight, bounded samples, camera identity, refusal before native window
creation and terminal failure preservation. Hardware behavior remains Part B.
