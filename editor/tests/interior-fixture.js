import { stringify } from 'smol-toml';
export const interiorProfile = () => ({ team_homes: ['engineering'],
  deck: [{ id: 'upper', label: 'Upper', damage_weight: 1 }, { id: 'lower', label: 'Lower', damage_weight: 2 }],
  compartment: [
    { id: 'bridge', deck: 'upper', label: 'Bridge', x: -10, z: 0, width: 10, depth: 10, material: 'light', max_hp: 50, recovery_fraction: 0.2, structural_remedy: 'structural' },
    { id: 'engineering', deck: 'lower', label: 'Engineering', x: 0, z: 0, width: 10, depth: 10, material: 'armour', max_hp: 50, recovery_fraction: 0.2, structural_remedy: 'structural' }],
  connection: [{ id: 'stairs', from: 'bridge', to: 'engineering', travel_secs: 5 }],
  placements: { helm: 'bridge', 'shield-arc-fore': 'engineering' } });
export const interiorSource = () => '# unrelated prefix\nname="Test"\n[[system]]\nid="helm"\nkind="helm_thrust"\n'
  + '[repair]\nrepair_team_count=1\n[[shield_arc]]\nid="fore"\n'
  + stringify({ interior: interiorProfile() }) + '\n[health]\nmax=42\n# unrelated footer\n';
