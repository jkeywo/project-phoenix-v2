# Acceptance kit — issue #1335, the native bridge lobby on real monitors

**This is the human half of PRD #1324.** The automated half is done and is listed
at the bottom; between them, issues #1325–#1334 prove the *logic* — the layout
law's three rules, the view models the button rows are a map over, the reveal
state machine, the join-URL decision, the placement rule a rebuilt console
obeys, the saved layout's round trip — with pure tests that run in CI on no
hardware, plus three `#[ignore]`d tests that reach a real Ultralight view and
real winit windows on this machine.

None of that can prove the only questions that finally matter here: **that
pressing a button on one screen actually moves the viewscreen onto another; that
a console opened from a station card lands on the glass the operator chose,
crewable by whoever walks up to it; that two consoles on one physical screen are
both operable and legible from across a room; that a cable coming out mid-mission
degrades instead of ending the night; and that the bridge an operator built last
week is the bridge they get when they pick that hull again.**

So this is a script for one operator at the bridge machine, in the order a game
night actually happens: boot, pick, invite, arrange, crew, fly, break something,
come back. Work through it in order — later sections assume the bridge the
earlier ones built.

Two parts of the feature cannot be settled on this rig at all, and they are
**parked explicitly in §9** rather than quietly skipped: touch operation of the
lobby (no touch hardware, the #1124 pattern), and the OS accessibility
preferences (the live OS read is a documented stub — see §8). Do not tick those
criteria until §9's conditions are met.

---

## 0. Preconditions — do these first, in this order

- [ ] **Two or more monitors on the bridge machine**, on the OS's extended
      desktop. One-monitor behaviour is covered too (§3d and §4e), but everything
      that makes this feature exist needs a second screen.

- [ ] **A trunk-built `dist/`.** The lobby surface is assembled from the **host**
      page's own markup and renders with the `gui/` modules beside it, so what it
      needs on disk is `dist/index.html` and `dist/gui/` — trunk's output, not
      the phone bundle's. `run-native.bat` does **not** run trunk (it is a wasm
      release build), so run it once by hand from this checkout, and again after
      any change to `server.html` or `gui/`:

      ```
      TRUNK_BUILD_RELEASE=true trunk build --release
      ```

      Without it `run-native.bat` stops with
      `[ERROR] dist\index.html not found` before taking the port.

- [ ] **Nothing else on port 8080**, and Windows' firewall prompt answered
      *allow* on first run — the host binds `0.0.0.0:8080` so phones on the room's
      Wi-Fi can reach the bundle.

- [ ] **Know your monitors** (optional, but §7 reads better with it). The
      launcher's default mode forwards flags verbatim, so this is the enumeration
      through the same script the rest of the kit uses:

      ```
      run-native.bat --setup
      ```

      It prints each display's stable `id` (e.g. `\\.\DISPLAY2@1920x1080`), its
      geometry and its scale. Those ids are what the saved-layout file in §7 is
      written in. (`--setup` refuses `lobby`; it is an enumerate-and-exit
      diagnostic.)

- [ ] **Start each section from a clean bridge unless it says otherwise.** §7
      depends on `%APPDATA%\ProjectPhoenix\bridge-layouts\` — open that folder in
      Explorer now and leave it open; you will watch files appear in it.

Everything below runs through **`run-native.bat lobby`**, which builds the host
(`cargo build --release --features ultralight --bin phoenix-host`) and the phone
bundle, then launches `phoenix-host --client-dir dist --lobby`. Arguments after
the mode word are forwarded verbatim; where a step needs one, the exact line is
given.

---

## 1. Boot to the lobby, and pick the mission on the viewscreen

```
run-native.bat lobby
```

No `--world` and no `--ship`: both are chosen on screen. The first run builds;
subsequent ones are a fast no-op.

- [ ] **The viewscreen window opens where the OS put it**, ordinary and
      windowed — *not* borderless fullscreen and *not* moved. A host with no
      `--profile` and no press describes its bridge rather than instructing it,
      and this is that promise on screen. (After §3's first press it stays
      borderless fullscreen for the rest of the run; that is expected.)
- [ ] **The scenario panel is on it.** The world list draws over the viewscreen —
      the host page's own picker, not a terminal prompt. If the surface is blank
      and the log says the SDK or the bundle is missing, fix that first: chrome
      never refuses to start a mission, so a blank surface is a *reported* state
      rather than an error dialog.
- [ ] **A mouse picks the world.** Click **Combat Test**. The panel advances to
      the hull stage.
- [ ] **A mouse picks the hull.** Click the **destroyer**. The world loads (a few
      seconds), the picker closes, and the **crew lobby** appears underneath:
      scenario title, crew counter, and one station card per station on the
      destroyer's roster — Captain, Helm, Tactical, Navigation, Comms,
      Engineering, Command.
- [ ] **The lobby says nobody can join.** With no `--rendezvous` the terminal
      carries `no --rendezvous, so nobody can join this host …` and the join panel
      on the viewscreen reads **"Crew joining is off — this host has no join
      service."** That is §2's first PASS as well; note it here and move on.
- [ ] **Nothing needed the command line.** You typed one word (`lobby`) and made
      every mission decision on the glass.

Leave this host running — §3 and §4 use it.

---

## 2. The join QR

The QR is the crew's whole route in, so it is worth its own section and its own
prerequisites. Read §2a before running §2b: two of the three ways a native host's
QR can be useless are configuration outside this repository.

### 2a. Joining is off, in words (no `--rendezvous`)

From the host still running in §1:

- [ ] **The join panel says joining is off**, in words, with **no framed QR**:
      *"Crew joining is off — this host has no join service."* A framed empty
      code would have a crew standing in front of the viewscreen scanning
      something that can never work, and "the code has not arrived yet" and
      "there will never be a code" look identical on a wall.
- [ ] The same holds for `run-native.bat lobby --solo` — one condition, two
      spellings.

### 2b. A real code, and a phone that reaches it

> **Prerequisites, and they are outside this checkout.** The rendezvous service
> must be **deployed** (`docs/delivery-checklist.md` §3a — as of writing,
> `worker-rendezvous/` has never been deployed), and its `ALLOWED_ORIGIN` must
> carry **two** origins: the one you pass as `--origin`, *and the origin the
> phone's page is served from*, which for a native host is the bridge machine's
> own LAN address (`http://192.168.x.y:8080`). A browser-hosted game does not
> hit this, because the phones load the same public page the operator is on.
> Until both are true this leg cannot pass, and **that is a park, not a fail** —
> record it in §9 and carry on. Run
> `node scripts/check-rendezvous.mjs --rendezvous <URL> --origin <URL>` first;
> a non-zero exit means stop here.

```
run-native.bat lobby --rendezvous https://phoenix-rendezvous.project-phoenix.workers.dev --origin https://pp-dev.kiwigamedesign.co.uk
```

Use the **built-in** service URL above. A *different* one is silently swapped for
the built-in one by a scanning phone (the client's `?rendezvous=` gate reads the
parameter's own host), and the host says so at the prompt — see §2c.

- [ ] **The terminal prints the code**: `phoenix-host: crew join code XXXXX
      (full: …)`, and beside it `the join QR is on the viewscreen, pointing phones
      at http://<address>`.
- [ ] **The same code is on the viewscreen**, as a framed QR with the five
      letters under it ("or type this code"). Not a terminal-only code: this is
      the whole of #1329.
- [ ] **The QR draws with no CDN.** The encoder is vendored
      (`gui/vendor/qrcode.js`) and served by this process, so there is no external
      request to fail — a bridge machine is not assumed to have internet. If you
      can run this room LAN-only (no route out, the service reachable or not),
      confirm the code still *draws*; otherwise note that you could not, and that
      the claim rests on the vendored file plus `tests/client/qr-encoder.test.js`,
      which loads it from disk with no network.
- [ ] **A phone's camera opens the link.** Scan it. The phone lands on the client
      page with the code already in the fragment — no five letters typed — and
      goes through the ordinary join flow.
- [ ] **The phone claims a station.** Tap a station card on the phone. On the
      **viewscreen**, that station's card fills in with the holder's initials and
      the crew counter advances. A phone and a wall console are the same kind of
      participant; this is the baseline the consoles in §4 are measured against.
- [ ] **Typing the five letters works too**, from a second phone — the QR and the
      code are two routes to one string.

### 2c. When the address the QR names cannot work

The host classifies the address it puts in the QR and says at the prompt when no
phone in the room can open it. Reproduce **one** of these — a VPN is the easiest:

- [ ] **Bring up a VPN (Tailscale or similar) and relaunch §2b's command.** The
      terminal carries a second line after the "pointing phones at" one:
      `…which is a carrier-grade NAT address (100.64.0.0/10) — usually a VPN
      interface such as Tailscale — so a phone on the room's Wi-Fi has no path to
      it. Bind the LAN address explicitly with --addr <ip>:<port> if this machine
      has one.`
- [ ] **The recourse works.** Relaunch naming the room's own address, and the
      warning is gone and the QR points at the LAN:

      ```
      run-native.bat lobby --addr 192.168.1.5:8080 --rendezvous https://phoenix-rendezvous.project-phoenix.workers.dev --origin https://pp-dev.kiwigamedesign.co.uk
      ```

      (Substitute your machine's own LAN address. An explicit `--addr` is the
      operator's own answer and is used exactly as given.)
- [ ] **Note which classes you actually exercised.** The other three sentences —
      loopback (`--addr 127.0.0.1:8080`), link-local (a network with no DHCP), and
      "not a private LAN address" — are the same mechanism; one is enough for this
      box, and the rest are pinned by unit tests. Say in your write-up which you saw.
- [ ] **Optional, 30 seconds.** Pass a non-default `--rendezvous` (a staging
      worker URL) and confirm the extra line: `…but a scanning phone will IGNORE
      the --rendezvous service in that QR and dial the client bundle's built-in
      one … instead, so it will not find this host.` That is the gate #1112 owns,
      reported rather than papered over.

### 2d. The toggle, in the lobby and in play

- [ ] **In the lobby, the surface's own control toggles the QR.** The viewscreen
      window has no settings cog, so the surface carries one control of its own.
      Click it: the join panel hides. Click again: it comes back.
- [ ] **A phone toggles it too.** From a joined phone's settings menu, press the
      QR toggle. The panel on the viewscreen flips. (Two presses are two flips —
      it is an edge, not a state.)
- [ ] **The QR is above the picker.** Relaunch and confirm the code is readable
      *while you are still choosing the scenario*: the crew join while the
      operator picks.
- [ ] **Mission start hides it.** Start the mission (§4 or the AI-launch control).
      The QR goes, and so does the rest of the lobby chrome — the viewscreen is
      clean.
- [ ] **F9 is the in-play gesture.** Press **F9**. The lobby surface comes back
      *over* the mission (it composites into an opaque texture, so revealing it
      covers the view — that is the native answer, not a bug). The QR toggle is
      there; press it and the code shows for a late arrival. Press **F9** again to
      hide the surface.
- [ ] **A phone cannot uncover the surface.** With the surface hidden mid-mission,
      press a phone's QR toggle. The viewscreen does **not** open — a phone in
      somebody's pocket must not drop a sheet over a running mission. It sets what
      the operator finds when they next press F9; confirm that too.

---

## 3. The viewscreen's monitor row

Back to a plain `run-native.bat lobby` with the destroyer picked (§1).

### 3a. The row names your monitors

- [ ] **A row of buttons headed "Viewscreen monitor"** sits in the lobby, one
      button per connected display.
- [ ] **Each button names its display recognisably** — the OS name plus the native
      resolution (`DELL U2718Q · 3840×2160`), or `Unnamed display · 1920×1080` for
      a display the OS named nothing. You can tell your TV from your side panel
      without counting.
- [ ] **The current viewscreen is marked in words**, not only in colour: the
      button carries **showing the viewscreen**. The OS's own primary display
      carries **primary**. Squint or imagine the row in greyscale — you can still
      read which is which.

### 3b. A press moves the window, live

- [ ] **Press the button for your other monitor.** The viewscreen window moves
      there **immediately**, borderless fullscreen, filling that display. No
      restart, no dialog.
- [ ] **The row re-marks itself.** The **showing the viewscreen** mark is now on
      the button you pressed and has left the one it was on — one frame, one
      repaint, no stale marker.
- [ ] **Press it back.** The viewscreen returns to the first monitor, borderless
      fullscreen this time. (It does not go back to being a windowed window; once
      the layout has been told, it is told.)
- [ ] **Time it.** Note roughly how long the move takes and whether anything
      flickers on the other display. This is a stopwatch judgement about a room,
      which is why it is here.

### 3c. Pressing the monitor it is already on does nothing

- [ ] **Press the button already marked "showing the viewscreen".** Nothing
      happens: the window does not move, does not re-enter fullscreen, and **no
      notice appears**. Asking for what already holds is accepted and is a no-op —
      that rule exists so an ordinary double-press, or two operators pressing at
      once, does not look like a fault.

### 3d. One monitor (only if you ever run one-screen)

Unplug all but one display, or run this on a single-screen laptop.

- [ ] **The row is still there**, with exactly one button, marked both **primary**
      and **showing the viewscreen**.
- [ ] **Pressing it does nothing at all** — same no-op as §3c. A one-monitor bridge
      has nowhere for the viewscreen to go and the row says so by having one
      button rather than by disappearing.
- [ ] **Every station card says why it offers no screens** — see §4e.

---

## 4. A station console on a chosen screen

Two or more monitors, the destroyer picked, the viewscreen on the monitor you
want it on.

### 4a. Open one

- [ ] **Every station card carries a row headed "Console screen"** — an **Off**
      button first, then one button per *eligible* monitor.
- [ ] **The viewscreen's own display is not among them.** Count the buttons: one
      fewer than the monitor row has. A console never covers the shared view, so
      that screen is not offered at all rather than offered and refused.
- [ ] **Press a screen on the Helm card.** A borderless-fullscreen window opens on
      that monitor showing the ordinary client page — the same document a phone
      loads. The Helm card's row now shows that button as selected (`aria-pressed`
      as well as a border, so it reads without colour).
- [ ] **It joins and claims like a phone.** At that monitor, claim **Helm** from
      the console's own station grid. The Helm card on the viewscreen fills in
      with its initials, exactly as §2b's phone did. Admission cannot tell the
      console from a phone.
- [ ] **Anyone may sit there.** Release the station from the console and claim it
      from a *phone* instead, then claim it back. The station id in the layout
      decides which console opens on which glass and **never** who may sit there.

### 4b. Move it

- [ ] **Press a different screen on the same card.** The console closes on the
      first monitor and opens on the second — one click, not a restart.
- [ ] **The seat survives the move.** The card's avatar drops to its placeholder
      for the length of one page load and then comes back with **the same
      initials**: the rebuilt page reconnects on the same session token and the
      lobby restores the station it held.
- [ ] **Nobody is told off for their own press.** Moving a console you pressed
      raises **no notice** on the monitor row. (A console *somebody else's* press
      re-tiled does — that is §5.)
- [ ] **Time the gap.** Note how long the console spends reloading. Its station is
      on AI control for exactly that long — and stays there if somebody else
      claims the station during the load, which is a real window rather than a
      theoretical one.

### 4c. Close it

- [ ] **Press Off on the Helm card.** The console's window closes and the monitor
      is free.
- [ ] **The screen comes back everywhere.** Look at another station's row — the
      button for that monitor is no longer greyed. One law, one answer, on every
      card at once.

### 4d. Mid-mission, after a replug

This one needs a running mission; §6 uses it too. Crew the ship (a console
readies, or use the lobby's AI-launch control with nobody connected), start the
mission, then:

- [ ] **F9 reveals the lobby over the mission**, station cards and screen rows
      included.
- [ ] **A screen row press works mid-mission.** Open a console on a free monitor
      from the revealed surface. It opens; the crew member at it joins and claims,
      mid-mission, without the mission ending.
- [ ] **F9 hides it again** and the mission is unobstructed.

### 4e. One monitor

- [ ] With a single display, every station card's row shows no buttons and one
      sentence instead: **"Consoles need a second monitor — crew join from
      phones."** An empty strip would read as broken; this reads as an answer.

---

## 5. Two consoles on one screen

The claim `MAX_PANES_PER_STATION = 2` exists to bound is a judgement about a
room, so this section is the one the ignored `tests/native_bridge_displays.rs`
explicitly hands over: *both halves operable*, *legible at bridge distance*, and
*the re-tile blink tolerable*. Take your time here.

### 5a. The second console splits the screen

- [ ] **Open Helm on monitor 2** (§4a), then **open Tactical on monitor 2 as
      well.** The screen divides itself **side by side**, automatically — you
      never position a window.
- [ ] **No gap, no overlap.** The two halves meet cleanly at the seam and together
      cover the whole display.
- [ ] **The neighbour says it is re-tiling.** The monitor row carries a notice
      naming the console nobody asked to move: *"Monitor <id>'s split changed, so
      helm's console is re-tiling and will reconnect."* Confirm it names **helm**
      (the one that was already there), not tactical.
- [ ] **The re-tile blink is tolerable — time it.** Helm's page reloads: how long
      is its screen blank, and does the operator's notice arrive **before** the
      crew member at it asks what happened? Write the number down; this is the
      question only a stopwatch on real hardware answers.
- [ ] **The rebuilt console keeps its seat.** Helm's card returns to the same
      initials after the reload. (If somebody else claimed Helm during the gap, it
      stays on AI control — that is what the notice promises, and it is true.)

### 5b. Both halves are fully operable

With a crew member (or you) at each half:

- [ ] **The mouse operates each half.** Move the pointer to the left half and
      press a control; move to the right half and press one. One ordinary pointer,
      no mode switch, each half responding only to clicks over its own rectangle.
- [ ] **The seam is clean.** Sweep the pointer slowly across the middle. Response
      hands over exactly at the seam — no dead strip, no half answering a click on
      the other's side.
- [ ] **The keyboard reaches each half.** Press **Ctrl+Tab** to move the focus
      reticle onto one console and type into a field on it; Ctrl+Tab again for the
      other. Characters land in the focused half only. Plain **Tab** stays the
      page's own field-to-field traversal.
- [ ] **Both are genuinely playable.** Claim a station on each half and actually
      operate it for a minute — not "the page rendered", but "somebody could sit
      here for an hour".

### 5c. Legible at bridge distance

- [ ] **Stand where the crew stands.** From the position a person at that screen
      would actually be, can you read the console's labels, numbers and buttons on
      **both** halves? Say yes or no, with the monitor's size and resolution and
      the distance. A no here is the finding this cap exists to catch.
- [ ] **Try it on your largest and smallest non-viewscreen display**, if you have
      more than one. Note where the answer changes.

### 5d. A third is refused, by name

- [ ] **Press monitor 2 on the Navigation card.** The button is **greyed and not
      pressable**, and it names what is holding the screen: **"full — helm,
      tactical"**.
- [ ] **The names are in the order they are drawn.** Left half first on a
      side-by-side screen: the button reads the way the glass reads, not merely
      the same set.
- [ ] **The button is still there.** It is greyed, not vanished — "free a slot
      here" is a different fact from "this screen is not offered", and a monitor
      that disappeared from the row would read as one that had been unplugged.
- [ ] **The greyed screen un-greys everywhere at once.** Press **Off** on the
      Tactical card. Navigation's button for that monitor is offerable again, and
      so is every other station's.

### 5e. The survivor regrows

- [ ] **After that Off**, the remaining console (helm) **grows back to fill the
      whole monitor** — it does not sit in half a screen with black beside it.
- [ ] **It keeps its seat** across that rebuild, same initials, same rule as §4b.
- [ ] **It settles.** The notice appears **once**; the console is not rebuilt
      again on the next frame, or the one after. Watch it for ten seconds.

### 5f. A hand-authored console on the same screen (optional, 5 minutes)

This is the case the row cannot free, and it is worth seeing once. Author a
`bridge.toml` from the ids `run-native.bat --setup` printed — one `viewscreen`,
and a `station` display carrying one participant pane:

```toml
version = 1

[[display]]
id = '\\.\DISPLAY1@1920x1080'   # your viewscreen monitor's id
role = "viewscreen"

[[display]]
id = '\\.\DISPLAY2@1920x1080'   # a second monitor
role = "station"
split = "side_by_side"
[[display.pane]]
label = "Ada"
```

```
run-native.bat lobby --profile bridge.toml --pane Ada
```

- [ ] **Ada's console opens at boot** on monitor 2, and the screen has **one** free
      slot rather than two: seat a station on it and it tiles *beside* Ada rather
      than over her.
- [ ] **A third is refused and says whose slot it cannot take**: the greyed button
      reads *"full — Ada, helm; opened by the host's own settings and not closable
      from here: Ada"*. Ada has no station card and no Off button anywhere, so the
      row says the slot is not the lobby's to free instead of sending you hunting.
- [ ] **The authored order is the drawn order.** Ada is on the half her
      `[[display]]` entry gave her, and stays there — the boot frame is not
      re-tiled out from under her.

---

## 6. A cable comes out mid-mission

Get into a running mission with **at least one console open on a non-viewscreen
monitor** and somebody (or a phone) at it. Run the host with `--log info` so the
operator log carries the lobby's own lines:

```
run-native.bat lobby --log info
```

### 6a. Unplug

- [ ] **Physically unplug the monitor a console is on** (or switch its input, or
      disable it in Windows display settings) while the mission is running.
- [ ] **Nothing crashes.** The host keeps running, the viewscreen keeps drawing,
      the ship keeps flying. That is the headline.
- [ ] **The console closes.** Its window is gone rather than re-homed onto the
      viewscreen — the never-silently-re-home doctrine, held at runtime.
- [ ] **Its crew drops to Backfill.** The station is flown by the AI from that
      moment; the log carries the disconnect at info.
- [ ] **The lobby tells the truth.** Press **F9**: that station's screen row shows
      **Off** selected, and its card's avatar is back to the placeholder. No card
      claims a screen that is black.
- [ ] **A blip is not an unplug.** If you can, cause a momentary display drop (a
      GPU reset, a dock re-seat, a monitor waking) rather than a real removal —
      the console should survive it. A single absent frame is not believed.

### 6b. Replug

- [ ] **Plug the monitor back in.** Nothing reopens on its own — that is
      deliberate.
- [ ] **F9, then press that screen on the station's row.** A fresh console opens
      on the replugged monitor and can be claimed again. The operator's press is
      what brings it back, never a silent re-home.
- [ ] **The mission never ended.** Same run, same tick sequence, from before the
      unplug to after the replug.

### 6c. A crashed view — **not constructible by hand**

A view crash is an internal fault of the embedded renderer; there is no operator
gesture that causes one (a Station window is borderless-fullscreen and owned by
the host process — there is nothing to close). The rebuild rule is proved instead
against a *real running bridge* by `src/native_host/bridge_display.rs`'s tests
with an injected view failure, on #1125's own fault path.

- [ ] **If one happens spontaneously during any part of this kit**, record it —
      what was on screen, and above all **which monitor the console came back
      on**. It must be its own, never over the viewscreen. Otherwise mark this
      box *not-run (not constructible)* and see §9.

---

## 7. The bridge is remembered, per ship class

Keep the Explorer window on `%APPDATA%\ProjectPhoenix\bridge-layouts\` open. Run
with `--log info` so the store's own lines are visible:

```
run-native.bat lobby --log info
```

> A run given an explicit `--profile` is deliberately **not** remembered: it uses
> that profile verbatim, has nothing pre-applied over it, and files nothing back.
> So do not pass `--profile` in this section (§5f's optional run is the exception,
> and §7f is where you confirm it wrote nothing).

### 7a. The first arrangement is filed

- [ ] **Pick Combat Test + the destroyer.** The log carries `bridge layouts:
      nothing saved for alliance_destroyer yet; the first arrangement made from
      the lobby writes …`, and the folder is **still empty**. Reading creates no
      directory; the first write does.
- [ ] **Make one press** — move the viewscreen, or open a console. The file
      **`alliance_destroyer.toml`** appears immediately. Not on quit: on the press.
- [ ] **Arrange the whole bridge.** Viewscreen on your chosen monitor, Helm and
      Tactical on a second screen, Engineering on a third if you have one.
- [ ] **Open the file.** It is readable TOML naming your monitors by the same ids
      `--setup` printed and your stations by id — a file you could hand straight
      back to `--profile`.
- [ ] **No `.tmp` files are left beside it.**

### 7b. Quit and relaunch restores it

- [ ] **Close the viewscreen window** (the ordinary quit) and relaunch
      `run-native.bat lobby --log info`.
- [ ] **Pick Combat Test + the destroyer again.** The log carries `bridge layouts:
      alliance_destroyer remembers a bridge — the viewscreen on …, N console(s)
      reopening on the screens they were left on`.
- [ ] **The bridge assembles itself.** The viewscreen moves to the monitor you
      chose and **each console reopens on the screen you left it on**, within a
      frame or two of the pick. You set up nothing.
- [ ] **The consoles are ordinary consoles.** Claim a station from one — it joins
      exactly as a freshly-opened one does.

### 7c. A second class keeps its own bridge

- [ ] **Relaunch and pick Combat Test + the cruiser.** The destroyer's layout is
      **not** applied — the cruiser starts unarranged (the log says nothing is
      saved for it yet).
- [ ] **Arrange the cruiser differently** — a different viewscreen monitor, or
      different stations on different screens. `alliance_cruiser.toml` appears
      beside the destroyer's.
- [ ] **Relaunch and pick the destroyer.** You get the **destroyer's** bridge back,
      untouched by anything you did to the cruiser. Two hulls, two bridges, one
      machine.

### 7d. A missing monitor degrades, and does not error

- [ ] **Quit, unplug one of the monitors a console was assigned to, relaunch, and
      pick the destroyer.** The host **boots normally**. It does not refuse, and it
      does not error.
- [ ] **The station whose screen is gone comes back unassigned** — its row shows
      **Off** — and the log names it (`Station <id>'s console cannot open on
      monitor <id>; it is left unassigned`). Everything else applies.
- [ ] **The file is not overwritten with the degraded bridge.** Check
      `alliance_destroyer.toml`'s modified time: unchanged. The log says so too:
      *"the bridge changed under this host, so alliance_destroyer's saved layout is
      left as it is rather than overwritten with the degraded arrangement"*. A
      cable coming out is not you changing your mind.
- [ ] **Quit, plug the monitor back in, relaunch, pick the destroyer.** The full
      arrangement is back, including the console the missing screen had held. The
      trip cost you nothing.
- [ ] **One honest edge, worth seeing.** On the degraded run, make a press — move
      a console to a screen that *is* there. *That* is filed, and the file is now
      the smaller bridge. A press is intent, and nothing can tell "tidying up on
      the road" from "this is my layout now". Undo it by rearranging on the full
      bridge afterwards.

### 7e. A hand-edited file is refused, with the remedy

The saved-layout folder is one an operator browses, so "copy your `--profile` in
here" is an invitation this feature extends. It is refused at the door.

- [ ] **Copy §5f's `bridge.toml` over `alliance_destroyer.toml`** in
      `%APPDATA%\ProjectPhoenix\bridge-layouts\` (keep a copy of the real one
      first). It carries a `[[display.pane]]` with a `label` and no `station`.
- [ ] **Relaunch and pick the destroyer.** The host runs normally on the displays
      as found, and the log carries **one warning** naming the file, what it found,
      and the **remedy** — it reads (path and ids yours):

      ```
      bridge layouts: saved bridge layout …\bridge-layouts\alliance_destroyer.toml
      is not one the lobby wrote — it carries a [[display.pane]] on
      "\\.\DISPLAY2@1920x1080" with no `station =` ("Ada"). Delete the
      [[display.pane]] entries that have no `station =`, or point --profile at this
      file instead: a saved layout records the viewscreen and the seated station
      consoles and nothing else — it is ignored for this run, the bridge keeps the
      arrangement it booted with, and the first change made from the lobby files a
      saved layout over it
      ```

      The remedy is the point: it names the moves that fix it, not only the
      complaint.
- [ ] **No phantom console.** Nothing is reserved and nothing is occupied: every
      station's screen row still **offers** the monitor the copied file named, and
      no screen reads as full at a seat you cannot see.
- [ ] **The file is left where you put it** — an operator who hand-edited a file
      wants to see what they wrote.
- [ ] **The next press saves normally over it.** Move a console; the file is now a
      real saved layout again.
- [ ] **Optional:** add a `[[touch]]` table to a copy and confirm the remedy
      changes to name *that* table (*"move every [[touch]] table into a --profile
      of your own"*) and does **not** tell you to delete pane entries your file
      does not contain.

### 7f. `--profile` wins for its run and writes nothing

- [ ] **Note `alliance_destroyer.toml`'s bytes and modified time.**
- [ ] **Run §5f's command** (`run-native.bat lobby --profile bridge.toml --pane
      Ada`), pick the destroyer, and **rearrange the bridge from the lobby** — move
      the viewscreen, seat a console.
- [ ] **Quit and check the file: byte-for-byte identical, same modified time.** An
      authored run is the operator's own arrangement for that run and is never
      filed over their remembered one.

---

## 8. Keyboard, and the accessibility passes

### 8a. Keyboard-only operation of the rows (this is the closable half)

Run `run-native.bat lobby`, pick a scenario and hull, and put the mouse down.

- [ ] **Ctrl+Tab reaches the lobby surface.** A bright focus reticle — a full
      frame plus four corner brackets — appears around the viewscreen window's
      surface. With consoles open, Ctrl+Tab cycles through them and the surface;
      **Ctrl+Shift+Tab** goes back.
- [ ] **A no-console host boots with no reticle**, and Ctrl+Tab is what brings it
      to the surface. That is deliberate: the reticle is a promise the next
      keystroke lands somewhere, and the surface carries no typing target, so it is
      never the *seeded* focus — only ever a deliberate destination.
- [ ] **Tab walks the controls inside the surface.** With the surface focused,
      press **Tab** repeatedly: focus visits the QR toggle (top right), every
      monitor button, and every station card's **Off** button and screen buttons —
      one at a time, none skipped. They are real `<button>` elements (the toggle
      is a `role="button"` with its own Enter/Space handler); nothing here needs
      a mouse.
- [ ] **Focus is always visible.** At every stop there is an outline on the
      control that has it — plain `:focus` is drawn as well as `:focus-visible`,
      because the embedded WebKit view decides for itself what counts as keyboard
      focus and an operator who cannot see where the next keystroke lands is worse
      served by a clean row.
- [ ] **Enter or Space presses it.** Move the viewscreen with the keyboard alone.
      Open a console with the keyboard alone. Close it with the keyboard alone.
- [ ] **A full screen is skipped, not offered-and-refused.** Fill a screen (§5a)
      and Tab through another station's row: the greyed button is `disabled`, so
      the keyboard steps past it rather than landing on a press that can only come
      back as a refusal.
- [ ] **State is not carried by colour.** Squint or imagine the surface in
      greyscale: which monitor shows the viewscreen is in **words** on the button
      (and in `aria-pressed`), and which screen a console is on is a border **plus**
      `aria-pressed`. You can read both without telling two blues apart.
- [ ] **In play the surface leaves the focus order.** With the mission running and
      the surface hidden, Ctrl+Tab does not reach it — a click over where it was
      goes to the viewscreen. **F9**, and it is reachable again.

### 8b. `prefers-contrast` and reduced motion — **parked, and here is why**

The lobby surface carries a full high-contrast block (`gui/host-lobby.css`:
`@media (prefers-contrast: more), (prefers-contrast: custom)`) that turns the
monitor row and every screen row into full-strength white-on-black edges with a
dashed edge for a greyed button, and a reduced-motion block that stills the ready
badge. The host-drawn focus reticle has the same response in Rust
(`FocusReticleStyle::for_os_prefs`). **None of it can be driven from Windows
today**, for two stacked reasons, both recorded in the code:

* Ultralight ships **no OS-backed `matchMedia`**, so a CSS `prefers-contrast` /
  `prefers-reduced-motion` query inside the embedded view answers "no preference"
  whatever Windows is set to (`src/native_host/panes/os_prefs.rs`).
* The host's own OS read is a **documented stub**:
  `query_os_accessibility_prefs()` returns `OsAccessibilityPrefs::default()` on
  every target, because a live Windows read needs `unsafe` FFI or a Win32 binding
  crate and this crate is `#![forbid(unsafe_code)]`. The whole seam beneath it —
  the injected `window.PhoenixOsAccessibilityDefaults` layer, the page's overlay,
  the precedence and clamping — is complete and CI-tested; only the read is
  deferred, behind that one function.

So there is nothing for an operator to switch on:

- [ ] **Confirm the honest state, once.** Turn Windows' high-contrast theme on and
      relaunch. The lobby's rows look **the same** as with it off. Record that this
      is the expected, documented state — **not a failure of this kit's run** — and
      leave the contrast/reduced-motion criterion unticked in §9.
- [ ] **Confirm nothing in the lobby animates.** Watch the monitor row and the
      screen rows for a few seconds: no pulse, no slide, no fade. Reduced motion
      has nothing to switch off here, which is why the row's answer is
      "static by design" rather than a media query nobody can reach. A viewscreen
      is a television in a room full of people.
- [ ] **The console content is a different question** and is #1127/#1171's, not
      this kit's — a phone or a console page honours its own in-page Accessibility
      settings, which are the player's explicit choices rather than the OS's.

---

## 9. Parked leftovers — explicit, not dropped

Three things this kit cannot settle on this rig. Each is written down here rather
than quietly omitted, following #1124's touch leg. **Do not tick the
corresponding criterion until the condition beside it is met.**

| # | Parked | Why | What unparks it |
|---|---|---|---|
| P1 | **Touch operation of the lobby** — a finger on a monitor button, a screen button, the QR toggle | The dev machine has **no touch hardware**. PRD #1324 puts this out of scope explicitly, parked *with #1124's touch leg*. The routing logic (contact capture, per-screen independence, the physical→pane transform) is already covered by synthetic tests | A real multi-touch Windows display. Then run `docs/acceptance/1124-input.md` Part B **and** repeat §3b, §4a, §5b and §8a's presses with a finger |
| P2 | **`prefers-contrast` / reduced motion reaching the surface** (§8b) | Ultralight has no OS-backed `matchMedia`, and `query_os_accessibility_prefs()` is a documented stub returning "no preference" on every target — a live Windows read needs `unsafe` FFI this crate forbids | A sanctioned live read (a safe binding, or an isolated FFI shim crate). It drops into that one function without touching anything else; then re-run §8b expecting the row to change |
| P3 | **A crashed console's rebuild, observed** (§6c) | Not constructible by hand: a view crash is an internal renderer fault and a Station window cannot be closed by the operator. Proved instead against a real running bridge with an injected failure | A spontaneous crash during any real session — record which monitor the console came back on |

And two **prerequisites**, which are not parks but will read like them if they are
not met on the day. Say which applied:

- **§2b (a phone actually joining)** needs the rendezvous service deployed *and*
  its `ALLOWED_ORIGIN` carrying the bridge machine's own LAN origin
  (`http://192.168.x.y:8080`) as well as the `--origin` you pass. See
  `docs/delivery-checklist.md` §3a. Without both there is no join path at all —
  not a degraded one — and every QR check past §2a fails identically and
  uninformatively.
- **§3d and §4e (one-monitor behaviour)** need you to actually run one-screen. If
  you did not, say so; they are not hardware you lack, they are a configuration
  you skipped.

---

## Where to record results

Add a dated note to the issue #1335 thread (or the batch's acceptance log):

- your monitor set — how many, their ids, sizes, resolutions and scales (paste
  `run-native.bat --setup`'s output), and which one you used as the viewscreen;
- the phone(s) and the network, in your own words, for §2;
- each box: **pass / fail / not-run**, with a line on anything that surprised you;
- the three **numbers** this kit exists to collect: the viewscreen move's
  duration (§3b), the re-tile blink's duration (§5a), and the
  legible-at-bridge-distance verdict with the display size and the distance
  (§5c);
- for any failure, the operator log around it (`--log info`) and a photo of the
  screen where the failure is visual (a console over the viewscreen, a mis-tiled
  pair, a stale marker);
- for §7, the contents of `%APPDATA%\ProjectPhoenix\bridge-layouts\` at the end.

**Say explicitly which of §9's parks applied.** P1 and P2 are expected to be
parked on this rig; that is the correct outcome, not an incomplete run.

---

## What the automated half already covers (no hardware)

Feature-**off**, run by the ordinary `cargo test` in CI:

- **The layout law** (`src/native_host/bridge_layout.rs` + `bridge_layout_tests.rs`) —
  one viewscreen, a console never on the viewscreen's monitor, at most two
  consoles per screen counting authored `--pane` surfaces and seated stations
  together, the deterministic split geometry, the no-op doctrine, every refusal
  and every adoption note by name, and the authored tiling preserved over every
  shape a `--profile` can take.
- **The saved layouts** (`layout_store.rs` / `layout_store_tests.rs` /
  `layout_store_systems_tests.rs`) — the class key, the atomic write, the two
  load doors and their remedies, arrange-quit-relaunch against a *real running
  host* with injected `Monitor` entities, two classes staying independent, the
  missing-monitor degradation, the cable-out re-baseline, and the `--profile`
  data-loss guard from both ends.
- **The lobby surface's contracts** (`src/native_host/host_lobby/*`) — the
  document assembly against this repository's own `server.html`, the bridge's
  latest-wins and deferral rules, the reveal state machine (F9 both ways, the
  latch cleared by every phase change), and all six page→host record tags through
  one drain.
- **The view models and renderers** (`tests/client/host-lobby-view.test.js`,
  `host-lobby-render.test.js`, `host-qr.test.js`, `qr-encoder.test.js`) — the
  monitor row, the per-station screen rows and their greying, the QR visibility
  law, the join URL, and the vendored encoder loaded from disk with no network.
- **The runtime application** (`bridge_display.rs`) — the whole open/close/move/
  re-tile loop on *fake* `Monitor` entities spawned and despawned as `bevy_winit`
  does on hot-plug, including the unplug closing exactly the right console, the
  seat reconciler's bounded rebuild and its surrender, and the placement rule
  (`panes::placement::home_for_pane`) that keeps a rebuilt console off the
  viewscreen.

Run deliberately, on this Windows box, and worth doing **before** the walkthrough:

```
TRUNK_BUILD_RELEASE=true trunk build --release
cargo test --features ultralight --test native_host_lobby_ultralight -- --ignored --nocapture
cargo test --features host --test native_bridge_displays -- --ignored --nocapture
cargo test --features host --test native_bridge_accessibility -- --ignored --nocapture
```

The first loads the real lobby document in a real Ultralight view over this
process's own HTTP and proves the surface rasterises, the QR draws and toggles,
the picker answers a click, and the monitor row and screen rows send their
presses back over the same bridge and the same drain. The second opens real
borderless-fullscreen windows and proves each covers the display it was assigned.
The third runs #1128's reflow and focus-order checks against this machine's real
monitor geometries. What none of them can reach is the whole-feature walkthrough —
press a button on one screen and watch the viewscreen arrive on another — which
is what §§1–8 above are for.

---

## Related

- `docs/acceptance/1124-input.md` — mouse, keyboard and the focus reticle across
  panes; **Part B is the touch leg this kit's P1 parks with**.
- `docs/acceptance/1128-accessibility.md` — text scaling, focus without colour,
  and the same OS-preference limit from the console side.
- `docs/acceptance/1126-media.md` — the media kit, parked on its own missing
  backend in the same way.
- `docs/acceptance/1113-networks.md` — the network scenarios behind §2's
  prerequisites; its §0 is where the rendezvous deploy check lives.
- `wiki/concepts/native-host.md` — what each of these behaviours is and why,
  section by section (#1325 → #1334).
