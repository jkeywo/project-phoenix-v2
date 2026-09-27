# Cruiser Elimination reference scenario

This kit records the crew-facing checks for #1549. Automated evidence lives in
`tests/cruiser_elimination.rs`. These human checks are **not yet observed**.
Balance evaluation and tuning are recorded separately under #1547/#1550.

## Setup and play

1. Select **Cruiser Elimination** in the scenario picker. Check that its copy
   explains two Alliance cruisers versus two Dynasty strike cruisers, Backfill
   for unclaimed berths, and spectating after individual destruction.
2. Claim one fixed berth and launch without a GM. Confirm that all four named
   cruisers launch, with two hulls per team. Repeat with a Dynasty berth.
3. Join a Station and leave other Stations and ships on Backfill. Check that
   Alliance receives its Dynasty-elimination Objective and Dynasty receives its
   Alliance-elimination Objective. The opposing team's Objective is private.
4. On Dynasty, use the ordinary Power reserve allocation and Weapons boost
   controls. Check that Helm's approach/strike/recovery, reserve charging, and
   boost feedback remain understandable during the team fight.
5. When your cruiser is destroyed while another teammate survives, check that
   the match continues, gameplay controls are closed, and the shared cinematic
   camera can follow a surviving cruiser using the keyboard-accessible picker.
   The crew must retain its original ship identity.
6. Let the fight resolve. Check both named team report rows: the surviving team
   wins and the eliminated team loses. Simultaneous team elimination is a draw
   for both teams. No crew receives a misleading global Victory banner.
7. Repeat the picker, Objective, spectator, and report reading in another
   supported locale after #1554 translations are integrated.

Record revision, seed, claimed berth, occupied Stations, input device, locale,
observed result, and any confusing control or feedback. A passing numerical
balance batch does not replace this crew usability evidence.
