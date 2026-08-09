// Unit tests for scripts/strings-rust.mjs — the `src/` half of the rule-11 gate
// (issue #975). The point of a green gate is that it looked, so these pin both
// the shapes the rule must CATCH and the tokens it must NOT, and assert the one
// module it guards is actually clean.

import { describe, it, expect } from 'vitest';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import {
  readsAsProse,
  stringLiterals,
  productionRegion,
  proseLiterals,
} from '../../scripts/strings-rust.mjs';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');

describe('readsAsProse — the prose signal', () => {
  it('flags player-visible English', () => {
    expect(readsAsProse('ALERT')).toBe(true); // single uppercase word
    expect(readsAsProse('AI')).toBe(true);
    expect(readsAsProse('Designating target: {label}')).toBe(true);
    expect(readsAsProse('{label} Offline')).toBe(true); // space + letter after strip
    expect(readsAsProse('Belay that — {weapon} no longer able to bear')).toBe(true);
    expect(readsAsProse('Repair requested for {station_label} ({tier:?})')).toBe(true);
  });

  it('does NOT flag ids, machine tokens, or format glue', () => {
    expect(readsAsProse('server.hud_alert')).toBe(false); // string id
    expect(readsAsProse('chatter.sender.sensors')).toBe(false);
    expect(readsAsProse('tactical')).toBe(false); // lowercase machine token
    expect(readsAsProse('station.{}.name')).toBe(false); // id built by format!
    expect(readsAsProse('{title}: {body}')).toBe(false); // no letters after strip
    expect(readsAsProse('')).toBe(false);
    expect(readsAsProse('{deg}')).toBe(false);
  });
});

describe('stringLiterals — the lexer', () => {
  it('extracts plain and raw strings with line numbers', () => {
    const src = 'let a = "Hello";\nlet b = r#"World"#;\n';
    expect(stringLiterals(src)).toEqual([
      { line: 1, text: 'Hello' },
      { line: 2, text: 'World' },
    ]);
  });

  it('skips line and block comments', () => {
    const src = '// "Not a literal"\nlet a = "real"; /* "also not" */\n';
    expect(stringLiterals(src)).toEqual([{ line: 2, text: 'real' }]);
  });

  it('does not mistake a double-quote char literal for a string opener', () => {
    // `'"'` must not swallow the following real string.
    const src = "let q = '\"'; let s = \"kept\";";
    expect(stringLiterals(src).map((l) => l.text)).toEqual(['kept']);
  });

  it('handles escaped quotes inside a string', () => {
    const src = 'let a = "say \\"hi\\" now";';
    expect(stringLiterals(src)[0].text).toBe('say \\"hi\\" now');
  });
});

describe('productionRegion — tests are not player-facing', () => {
  it('drops everything from the first #[cfg(test)]', () => {
    const src = 'fn prod() {}\n#[cfg(test)]\nmod tests { let x = "Fixture English"; }';
    expect(productionRegion(src)).toBe('fn prod() {}\n');
    expect(proseLiterals(src)).toEqual([]); // the fixture English is invisible
  });
});

describe('proseLiterals — the failing case #975 was filed about', () => {
  // The pre-#975 body of `format_coordination_chatter`, verbatim enough to prove
  // the rule would have gone red on it. If this ever stops flagging, the rule
  // has quietly stopped protecting the class.
  const PRE_FIX = `
    fn format_coordination_chatter(payload: &CoordinationPayload) -> String {
        match payload {
            CoordinationPayload::FrequencyHint { frequency } => {
                format!("Frequency hint: {frequency:.1}")
            }
            CoordinationPayload::TargetDesignation { label, .. } => {
                format!("Designating target: {label}")
            }
            _ => "AI".to_string(),
        }
    }
  `;

  it('goes red on composed English', () => {
    const found = proseLiterals(PRE_FIX).map((l) => l.text);
    expect(found).toContain('Frequency hint: {frequency:.1}');
    expect(found).toContain('Designating target: {label}');
    expect(found).toContain('AI');
    expect(found.length).toBeGreaterThanOrEqual(3);
  });

  it('is green once the sentences are ids/consts', () => {
    // The shape #975 left behind: no format!, a const reference (not a literal)
    // for the fallback, an id for the condition.
    const POST_FIX = `
      fn emit(msg: &Msg) {
          let from_label = if msg.sender_label.is_empty() {
              coordination::CHATTER_SENDER_AI.to_string()
          } else {
              msg.sender_label.clone()
          };
          let target = SystemId("tactical".into());
          chatter_writer.write(AiChatterEvent { from_label, to_label: msg.target.0.clone(), payload: msg.payload.clone() });
      }
    `;
    expect(proseLiterals(POST_FIX)).toEqual([]);
  });
});

describe('proseLiterals — the guarded module is actually clean', () => {
  it('src/ship/coordination_systems.rs composes no player-visible English', () => {
    const src = fs.readFileSync(
      path.join(root, 'src', 'ship', 'coordination_systems.rs'),
      'utf8',
    );
    expect(proseLiterals(src)).toEqual([]);
  });
});
