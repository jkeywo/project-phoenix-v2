# Native surface language check (#1537)

Build the native host with an Ultralight-enabled client bundle, and start a
local `--lobby` session. Have at least one Station pane and, where available,
a separate GM monitor. The shipped catalogue's German column should be present.

1. In Viewscreen Settings → Display, choose Deutsch. Confirm the lobby's static
   labels and current roster change in place, and the HUD's current readouts
   change in play. The QR and scenario/ship selection stay active.
2. Choose English in a Station pane. Keep a typed Comms draft, caret, focus and
   selected Station; switch that pane to Deutsch. The draft and Station seat
   remain. A second pane and the Viewscreen retain their own choices.
3. Change the GM screen and Workshop independently. Check retained GM activity
   and Workshop findings; keep an unsaved Workshop draft while switching.
4. Recreate each native view by moving its monitor or relaunching the host.
   Confirm each surface restores its own choice. If no choice was saved on a
   fresh profile, confirm the available OS language is used, with English for
   an unsupported language.

This check needs actual Ultralight windows and monitor interaction; the
automated adapter tests cover host profile persistence, the Viewscreen record,
and a retained HUD repaint.
