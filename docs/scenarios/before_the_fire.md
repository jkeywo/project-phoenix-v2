# Before the Fire — Design Notes

**Setting:** Axiom System, T−35 (35 years before The Neutral Zone)  
**Players:** 2–6, all playing Alliance crew aboard A.E.V. Ardent  
**Duration:** 30–60 minutes  
**Tone:** Political thriller becoming crisis response

---

## Premise

The last Alliance–Imperium peace talks are collapsing at Axiom Station. House Harrow has
sent a warhawk — I.V. Ashrender — to fire a stellar resonance weapon at the station and
at Kaleth Prime (40 million inhabitants). The weapon will create a persistent quantum
interference zone: a spatial scar across the system that will make navigation impossible
for decades. This is not collateral damage. This is the point. House Harrow intends to
redraw the map.

In 35 years, this scar will be called the Neutral Zone.

The crew cannot prevent the war. They can only choose how it starts.

---

## Files

| File | Purpose |
|------|---------|
| `assets/maps/axiom_system.toml` | Star system map with patrol anchors |
| `assets/scenarios/before_the_fire.toml` | Main scenario (always loaded) |
| `assets/scenarios/btf_path_a.toml` | Diplomat path — loaded on Axiom Station hail |
| `assets/scenarios/btf_path_b.toml` | Scholar path — loaded on Research Outpost hail |
| `assets/scenarios/btf_path_c.toml` | Soldier path — loaded when Ironveil attacks |
| `assets/scenarios/btf_aphelion_protocol.toml` | Finale — loaded when weapon arms |
| `assets/entities/station_axiom.toml` | Axiom Station entity (torus, 200 hull) |
| `assets/entities/station_research_outpost.toml` | Research Outpost entity (sphere, 60 hull) |
| `assets/entities/ship_harrow_patrol.toml` | I.V. Ironveil — patrol AI, Harrow faction |
| `assets/entities/ship_harrow_warhawk.toml` | I.V. Ashrender — idle then attack, no flee |
| `assets/entities/ship_requiem_courier.toml` | Requiem courier — idle, flees if attacked |
| `assets/entities/region_kaleth_nebula.toml` | Nebula: radar_dampening + damage_zone |
| `assets/entities/region_radiation_zone.toml` | Weapon heat: heavy damage_zone (dynamic) |
| `assets/factions/harrow.toml` | Harrow faction (enemy of Alliance/Federation) |
| `assets/factions/requiem.toml` | Requiem faction (neutral) |

---

## System Layout

```
                           [Research Outpost]
                           (-100, 0, 450)

[Requiem Courier]                              [Axiom Station]
(-230, 0, -80)                                 (350, 0, 50)
                   ASTEROID BELT
  [Ship Starts]    (90–260 radius)             [Kaleth Prime]
  (0, 0, 0)                                    planet

          [I.V. Ironveil patrol]
          (180,0,-160) ↔ (320,0,-280)

                        [I.V. Ashrender]    [Kaleth Nebula]
                        (620, 0, -380)      (680, 0, -440)
                                            radius 220
```

The crew starts near the system entry point. Axiom Station is the obvious first
destination (350 units, ~7 s at max speed). The Research Outpost requires deliberate
navigation away from the station. Ironveil patrols the mid-system between the crew and
the outer system. Ashrender is at the nebula's edge; reaching it requires passing through
the nebula (damage + radar dampening).

---

## Narrative Flow

### Opening (on map load)

Two comms arrive simultaneously:
- Starcorp Command deployment orders: investigate, Harrow ships are acting strangely
- Axiom Station distress hail: Administrator Chen, missing negotiator, Harrow patrol

The crew chooses who to approach first.

### Branch Phase (T+0 to T+10 min)

Each path loads a sub-scenario with its own dialogue and objectives. Paths are not
mutually exclusive — the crew can trigger all three before the weapon arms.

**Path A — The Diplomat** (hail Axiom Station)
> Administrator Chen reveals Dr. Sol Varen (lead negotiator) is missing, last seen near
> Ironveil's position. The crew learns the political context: Harrow is not negotiating,
> they are waiting. If Ironveil is destroyed, Dr. Varen escapes via pod and confirms the
> weapon order.

**Path B — The Scholar** (hail Research Outpost)
> Dr. Myst has detected anomalous resonance readings from the outer system. The crew
> gets the technical picture: stellar resonance class weapon, 20-minute charge time,
> sub-harmonic that will create a persistent quantum interference zone. The Scholar path
> is also where the first hint of the Singularity signal appears — a sub-harmonic Myst
> cannot explain.

**Path C — The Soldier** (attack or be attacked by Ironveil)
> Combat with Ironveil. On destruction, Science pulls the decrypted Harrow operational
> orders from the wreckage: targets, authorisation chain, confirmation code. The crew
> learns 40 million people are listed as a secondary target as a deliberate demonstration.

### Weapon Arming (Kobayashi Maru trigger)

The Aphelion Protocol arms when **either**:
- Ironveil is destroyed (Ashrender executes contingency immediately)
- 600 seconds elapse without Ironveil being destroyed (Ashrender proceeds on schedule)

The `btf_aphelion_protocol.toml` sub-scenario loads, spawning a radiation zone centred
on Ashrender. Three comms arrive simultaneously: the warning from Ashrender, an urgent
order from Starcorp, and a secret encrypted offer from Requiem House.

### Finale — Four Responses

All four responses are available simultaneously. The crew may attempt any combination.
None prevent the war.

| Response | Consoles | Outcome |
|----------|----------|---------|
| **Fight** — navigate nebula, destroy Ashrender | Helm (damage zone), Tactical (reduced radar combat), Engineering (constant repair) | Partial detonation on destruction damages Axiom Station. War starts: Harrow claims unprovoked attack on Imperium territory. |
| **Evacuate** — Comms coordinates civilian evacuation of Axiom Station and Kaleth Prime | Comms (multi-round dialogue), Science (escape vector plotting) | Station and planet are hit but casualties are reduced. War starts. |
| **Shield containment** — Engineering routes all power to shields; Helm positions Ardent between Ashrender and the station | Engineering (max power allocation drains Tactical + Helm), Helm (intercept position) | Ardent absorbs the discharge. Heavy hull damage; possible ship destruction. Station saved. War starts. |
| **Requiem override** — Comms accepts the encrypted override code | Comms (encrypted channel) | Weapon neutralised. Requiem is immediately denounced by Harrow as traitors and destroyed. War starts via different Casus Belli. |

**Regardless of outcome:** Starcorp Command confirms a state of conflict with the
Imperium. The quantum resonance from the weapon — fired, partially detonated, or
contained — leaves a spatial scar. This scar, 35 years from now, will be the Neutral Zone.

---

## Feature Coverage

| Feature | How it appears in this scenario |
|---------|--------------------------------|
| **CaptainChair — Red Alert** | Mandatory when Aphelion arms; alerts the crew visually |
| **CaptainChair — View Selector** | Survey nebula edge, Ironveil patrol route, Axiom Station |
| **Helm — thrust / steering** | All navigation; nebula approach; shield containment intercept |
| **Impulse Drive** | Sprint to intercept Ashrender before discharge timer |
| **Science — impulse cancel** | Abort impulse if entering nebula at wrong angle |
| **Science — long-range radar** | Detecting Ashrender and nebula from safe distance (Path B) |
| **Tactical — phasers** | Ironveil combat (Path C); Ashrender engagement |
| **Tactical — torpedoes** | Ashrender finale; heavier hull than Ironveil requires torpedoes |
| **Tactical — shield frequency** | Harrow ships use gamma frequency; Science detects, Tactical tunes |
| **Engineering — repair** | Nebula damage; Ironveil combat damage creates breakdown queue |
| **Engineering — power allocation** | Shield containment option drains all power to shields |
| **Console complexity** | Low Tactical: auto-fires torpedoes when Ashrender shields drop; Low Engineering: no battery for containment option |
| **AI patrol state** | Ironveil patrols between anchors in the mid-system |
| **AI pursuing / attacking** | Ironveil and Ashrender transition to attack on contact |
| **AI fleeing / warping out** | Ironveil flees at 30% hull; warps out at 15% |
| **Region: radar_dampening** | Kaleth Nebula (0.4× Science radar range) |
| **Region: damage_zone** | Kaleth Nebula (3 DPS); Radiation Zone (8 DPS) |
| **Region: sensor_blind** | Kaleth Nebula (Ashrender is not visible until close approach) |
| **on_entered_region trigger** | Warning comms when ship enters nebula |
| **Station entities** | Axiom Station (torus), Research Outpost (sphere) |
| **Faction system** | Harrow faction (enemy of Alliance); Requiem (neutral) |
| **Scenario branching** | Three path sub-scenarios; finale sub-scenario |
| **Comms console** | All NPC dialogue; evacuation coordination; Requiem channel |
| **Save / Load** | Natural save point after exploration before Protocol arms |
| **Station-based lobby** | Crew assigned to Ardent consoles before scenario start |

---

## Lore Notes

- **Axiom Station** — In 35 years this will be the commercial heart of the Neutral Zone,
  run by the Axiom Corporate Combine. Today it is an independent trading hub and the
  site of the last genuine peace talks. Administrator Chen will not appear in the TNZ era
  — but the station will.

- **Kaleth Prime** — The contested world. Its agricultural colonies will become the
  civilian population centre of the TNZ Kaleth system. The atmospheric disturbance from
  the Aphelion discharge explains the unusual weather patterns noted in TNZ-era records.

- **The Kaleth Nebula** — The quantum interference zone created or amplified by the
  Aphelion weapon's discharge (full, partial, or contained) will persist for decades.
  By the TNZ era it will be called the Neutral Zone — not a political agreement but a
  physical barrier that neither side can cross in force.

- **House Harrow** — At T−35, Harrow is the most aggressive expansionist house in the
  Imperium. Their warhawks are heavier and better shielded than Alliance vessels of
  the same era. Ironveil and Ashrender are both Harrow-faction vessels. In the TNZ era,
  Harrow holds several key system nodes inside the Zone.

- **House Requiem** — A more moderate Imperium house. At T−35, they have the override
  capability because they helped design the weapon and disagreed with its use. In the
  TNZ era, Requiem's influence within the Imperium is greatly diminished — a consequence
  of the Ardent scenario playing out across hundreds of game sessions.

- **Dr. Sol Varen** (Path A only) — The Alliance's lead negotiator. If rescued, they
  eventually write the definitive account of the talks. Their testimony is the primary
  historical source for the TNZ-era scholars who study this period.

- **Dr. Myst** (Path B only) — The anomalous sub-harmonic they detected in the weapon's
  signature is the first recorded observation of what TNZ-era scientists call the
  Singularity signal. Myst never understood what they found.

---

## Running Notes

**Difficulty:** The scenario is designed to be completable by a 3-player crew (Captain,
Helm, Tactical) without the planned consoles (Comms, Power). The Comms dialogue
advances automatically on single-response branches; multi-response branches pause
until a player acts. The Requiem override is always available; it doesn't require
prior contact with the courier.

**Timing:** The 600 s fallback timer gives the crew approximately 10 minutes of
exploration before the Protocol arms. A crew that destroys Ironveil quickly will arm
the Protocol faster — there is a cost to being aggressive. A cautious crew that avoids
combat gets more time but arrives at the finale with less intelligence.

**Ship destruction:** If hull reaches 0 during the shield containment attempt, the
scenario does not end gracefully in the current engine — it is a standard game-over.
Narratively this is the Kobayashi Maru outcome: the ship is lost. Future engine work
may add a "ship destroyed" scenario end state with appropriate final comms.
