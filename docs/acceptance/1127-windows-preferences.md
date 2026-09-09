# Windows preference adoption (#1127 / #1449 C2)

The host-only Windows adapter uses `windows 0.62.2` safe WinRT bindings for
`UISettings::AnimationsEnabled`, `UISettings::TextScaleFactor` and
`AccessibilitySettings::HighContrast`. No unsafe-code rule was relaxed.
Each read has independent availability. The native setup report names failed
reads; a Station's private Accessibility panel shows a fallback notice. Successful
neutral values remain distinguishable from failure. Other builds report the
adapter unavailable rather than claiming a successful OS read.

The adapter feeds the existing injected default layer. Explicit profile choices
still override it, reset follows the injected defaults, and only derived
eligibility/assistance crosses transport boundaries. New documents read the OS;
a crash recreation retains its original document and defaults. Change OS
preferences before launching the host for this kit. Live preference-change
subscriptions are not implemented.

## Reproducible rig pass

Use a Windows host build with `--features ultralight`, the matching built client
bundle, and one- and two-Station-pane monitor profiles. Record build SHA, Windows
version, physical monitor sizes/resolutions and the profile for each result.

1. With Windows text size 100%, contrast theme off and animation effects on,
   launch the host. Record `--setup` OS defaults and confirm each Station's
   Accessibility controls start at the system values.
2. Quit. Set Windows text size to 150%, enable a contrast theme and disable
   animation effects. Relaunch. Confirm actual Station text, contrast and motion
   effects adopt those values; compare the setup report with Windows Settings.
3. In one Station choose explicit standard contrast/full motion and 100% text.
   Confirm that pane overrides the OS while another pane continues following it.
   Reset the first pane's controls to System/default and confirm adoption returns.
4. Reload/reconnect the first pane and restart the host. Confirm its explicit
   choices persist privately and no other pane inherits them. Crash recreation,
   if available on the rig, must retain its original injected defaults.
5. At both 100% and 150%, operate every available setup action by keyboard and
   mouse in one- and two-pane layouts. Record clipped information, unreachable
   controls, focus loss or overlap as failures. Record touch separately if the
   rig has touch hardware; mouse is not touch evidence.

The shared 200% Settings/profile tracer #1422 is blocked by #1421. **200% and
completed T3 support remain BLOCKED**, not passed by the adapter's absolute 2.0
clamp. This kit does not broaden today's declared 1.0–1.5 setup support range.
Actual Windows adoption, override and multi-monitor inspection are **NOT RUN**
until an operator records them. Automated injected values are supporting evidence.

## Implementation verification

`cargo test --lib --features host -- native_host::panes::os_prefs
native_host::panes::document native_host::setup_accessibility` compiles the real
Windows getters and tests mapping, partial failure, document injection,
recreation, privacy and setup reports. To print this machine's actual read:

```powershell
cargo test --lib --features host a_query_never_panics_and_stays_in_range -- --nocapture
```

The `OS_ACCESSIBILITY_READ` diagnostic reports values and per-property success;
it is not an assertion that the visible native pane adopted them. Browser tests
cover profile precedence/persistence and the localized unavailable hint.

On 8 September 2026, the final working tree over `815574f2` passed **54 native
tests, zero failed/ignored**, with `--features host --nocapture` and the three
filters above. `OS_ACCESSIBILITY_READ` reported reduced_motion=false,
high_contrast=false, text_scale=1.0 and all three availability fields=true on
this Windows machine. This is a live read, not a pane adoption result.
Focused JavaScript tests passed 151 tests before the final visible-hint case;
the affected settings-panel file then passed all 100 tests including that case.
The strict String Table check passed with 2,841 strings, zero errors/warnings.
Independent source review passed. Final integration and rig acceptance remain.

On 8-9 September, the current client bundle passed four existing real-browser
checks: contrast round trip, reduced-motion round trip, and Station Bar keyboard
roving at 390x844 and 844x390 (port 3164, exit 0, 4.4 seconds). A new explicit
native-default-layer browser check passed on port 3165 (one passed, exit 0,
2.8 seconds): two isolated players receive 150%/contrast/reduced-motion defaults,
one player's overrides persist after reload, reset restores follow-system, and
the other player's profile is unchanged. This tests the ordinary rendered client
consumer and persistence seam, not Ultralight or live OS preference changes.

`native_bridge_accessibility` was explicitly run with `--features host --ignored
--nocapture`: process exit 0, one test reported passed, but its actual verdict
was **SKIPPED: needs two displays**. This machine has one 1920x1080 display.
No lawful two-pane Station layout was exercised and no multi-monitor PASS is claimed.

The separate real Ultralight console test was explicitly run on the `5df5a85c`
source tree: **1 passed, 0 failed, 0 ignored**, 13.56 seconds. Two offscreen panes
loaded served console pages, operated a Station, retained independent stores,
and recovered a crashed pane on the same identity. This supplies native engine
storage/reconnect evidence; it does not change the physical layout skip above
or prove live OS preference adoption. The required local integration gates pass.
