import { describe, it, expect } from 'vitest';
import { worldCommsToEditor, editorCommsToWorld } from '../comms-adapter.js';

// Fixture mirroring the comms section of assets/worlds/default.toml.
function defaultCommsFixture() {
  return [
    {
      from: 'raider_alpha',
      trigger: 'on_attacked',
      entity: 'raider_alpha',
      message: 'MAYDAY MAYDAY — Pirate raider calling for backup.',
    },
    {
      from: 'Starbase Alpha',
      trigger: 'on_attacked',
      entity: 'Starbase Alpha',
      message: 'Starbase Alpha to all vessels — we are under attack!',
    },
    {
      from: 'Starbase Alpha',
      trigger: 'on_hailed',
      entity: 'Starbase Alpha',
      message: 'USS Phoenix, this is Starbase Alpha. Please state your business.',
      response: [
        {
          text: 'We are on a survey mission.',
          action: [
            {
              type: 'add_objective',
              id: 'obj-survey',
              text: 'Complete the survey in this sector.',
            },
          ],
        },
        {
          text: 'We require docking clearance.',
          action: [
            {
              type: 'add_objective',
              id: 'obj-dock',
              text: 'Dock at Starbase Alpha.',
              mandatory: true,
            },
          ],
        },
      ],
    },
  ];
}

describe('comms-adapter', () => {
  it('worldCommsToEditor on empty / missing input returns empty array', () => {
    expect(worldCommsToEditor(undefined)).toEqual([]);
    expect(worldCommsToEditor(null)).toEqual([]);
    expect(worldCommsToEditor([])).toEqual([]);
  });

  it('editorCommsToWorld on empty input returns empty array', () => {
    expect(editorCommsToWorld(undefined)).toEqual([]);
    expect(editorCommsToWorld([])).toEqual([]);
  });

  it('worldCommsToEditor maps message→node.body and trigger→{kind, entity}', () => {
    const editor = worldCommsToEditor(defaultCommsFixture());

    expect(editor).toHaveLength(3);
    expect(editor[0].from).toBe('raider_alpha');
    expect(editor[0].trigger.kind).toBe('on_attacked');
    expect(editor[0].trigger.entity).toBe('raider_alpha');
    expect(editor[0].node.body).toBe(
      'MAYDAY MAYDAY — Pirate raider calling for backup.',
    );
    expect(editor[0].node.responses).toEqual([]);
  });

  it('worldCommsToEditor maps response[].action → responses[].actions', () => {
    const editor = worldCommsToEditor(defaultCommsFixture());
    const hail = editor[2];

    expect(hail.trigger.kind).toBe('on_hailed');
    expect(hail.node.responses).toHaveLength(2);
    expect(hail.node.responses[0].text).toBe('We are on a survey mission.');
    expect(hail.node.responses[0].actions).toEqual([
      {
        type: 'add_objective',
        id: 'obj-survey',
        text: 'Complete the survey in this sector.',
      },
    ]);
    expect(hail.node.responses[0].follow_up).toBeUndefined();
  });

  it('round-trips default.toml comms: editorCommsToWorld(worldCommsToEditor(x)) ≈ x', () => {
    const original = defaultCommsFixture();
    const round = editorCommsToWorld(worldCommsToEditor(original));
    expect(round).toEqual(original);
  });

  it('round-trips a follow_up nested node', () => {
    const original = [
      {
        from: 'Starbase Alpha',
        trigger: 'on_hailed',
        entity: 'Starbase Alpha',
        message: 'State your business.',
        response: [
          {
            text: 'Trade.',
            action: [{ type: 'add_objective', id: 'obj-trade', text: 'Trade.' }],
            follow_up: {
              message: 'Proceed to docking bay 3.',
              response: [
                {
                  text: 'Acknowledged.',
                  action: [
                    { type: 'apply_flag', kind: 'CommsJammed' },
                  ],
                },
              ],
            },
          },
        ],
      },
    ];
    const round = editorCommsToWorld(worldCommsToEditor(original));
    expect(round).toEqual(original);
  });

  it('preserves comms templates with no entity field (drops `entity` cleanly)', () => {
    const noEntity = [
      {
        from: 'Narrator',
        trigger: 'on_hailed',
        message: 'A voice from the dark.',
      },
    ];
    const editor = worldCommsToEditor(noEntity);
    expect(editor[0].trigger.entity).toBeUndefined();

    const round = editorCommsToWorld(editor);
    expect(round[0]).not.toHaveProperty('entity');
    expect(round).toEqual(noEntity);
  });

  it('round-trips a patrol.toml-style empty comms array', () => {
    // patrol.toml has no [[comms]] blocks at all.
    expect(editorCommsToWorld(worldCommsToEditor([]))).toEqual([]);
  });

  it('strips empty responses array from the round-tripped output', () => {
    const original = [
      {
        from: 'Drone',
        trigger: 'on_attacked',
        entity: 'drone',
        message: 'Threat detected.',
      },
    ];
    const round = editorCommsToWorld(worldCommsToEditor(original));
    expect(round[0]).not.toHaveProperty('response');
  });

  it('strips empty actions array from a response in the round-tripped output', () => {
    const original = [
      {
        from: 'Starbase Alpha',
        trigger: 'on_hailed',
        entity: 'Starbase Alpha',
        message: 'Hello.',
        response: [{ text: 'Ack.' }],
      },
    ];
    const round = editorCommsToWorld(worldCommsToEditor(original));
    expect(round[0].response[0]).not.toHaveProperty('action');
    expect(round).toEqual(original);
  });
});
