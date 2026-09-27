import { describe, expect, it } from 'vitest';
import { nativeMatrixOutcome } from '../../scripts/fleet-native-matrix.mjs';

function completeEvidence() {
  const ids = ['ship-1','ship-2','ship-3','ship-4','gm-1','gm-2'];
  const events = new Map(ids.map(id => [id,[]]));
  for (const id of ids) {
    const rows = events.get(id), put = (kind,value) => rows.push({kind,value});
    put(id === 'ship-1' ? 'fleet_code' : 'fleet_join_status',{status:'admitted'});
    put('onRoster',{participants:ids,slots:ids.slice(0,4)});
    put('simulation-roster',{generation:1,local:ids.indexOf(id)+1,participants:[1,2,3,4,5,6],shipHosts:[1,2,3,4],gmHosts:[1,5,6]});
    put('state',{roster_result:{generation:1,accepted:true}});
    put('onDiag',{event:'transport',transport:'ws-relay'});
    for (const tick of [30,60]) put('digest',{tick,from:ids.indexOf(id)+1,digest:'0123456789abcdef'});
    if (id.startsWith('ship')) for (const station of ['captain','helm','engineering']) {
      put('station-assigned',{station,assigned:true}); put('station-ready',{station,ready:true}); put('station-started',{station});
      for (const n of [1,2]) {
        put('station-command',{station,correlation:`${station}-${n}`});
        put('station-feedback',{station,correlation:`${station}-${n}`,outcome:'Applied'});
      }
    } else {
      put('gm-metadata',{local_operator_id:id});
      for (const n of [1,2]) {
        put('gm-action-requested',{correlation:`action-${n}`,accepted:true});
        put('gm-activity',{entries:[{detail:{type:'gm_action',data:{operator:{id},correlation:`action-${n}`,outcome:'applied'}}}]});
      }
    }
  }
  return events;
}

describe('native runtime matrix evidence gate', () => {
  it('requires authoritative receipts, distinct GM outcomes, real routes and common six-peer digests', () => {
    expect(nativeMatrixOutcome(completeEvidence()).sixPeerWorkloadPassed).toBe(true);
    expect(nativeMatrixOutcome(completeEvidence(),{digestAfter:Date.now()}).sixPeerWorkloadPassed).toBe(false);
    for (const omitted of ['station-feedback','gm-activity','onDiag','digest','station-started','simulation-roster','state']) {
      const evidence = completeEvidence();
      for (const [id,rows] of evidence) evidence.set(id,rows.filter(row => row.kind !== omitted));
      expect(nativeMatrixOutcome(evidence).sixPeerWorkloadPassed,omitted).toBe(false);
    }
  });
  it('refuses divergent or contradictory digests and pending/refused outcomes', () => {
    for (const mutation of [
      rows => rows.find(row => row.kind === 'digest').value.digest = 'different',
      rows => rows.find(row => row.kind === 'simulation-roster').value.local = 1,
      rows => rows.push({kind:'digest',value:{tick:30,digest:'different'}}),
      rows => rows.find(row => row.kind === 'station-feedback').value.outcome = 'Refused',
      ...['fleet_fault','station-error','gm-control-error','observer-overflow'].map(kind => rows => rows.push({kind,value:{reason:'failed'}})),
    ]) {
      const evidence = completeEvidence(); mutation(evidence.get('ship-2'));
      expect(nativeMatrixOutcome(evidence).sixPeerWorkloadPassed).toBe(false);
    }
  });
  it('does not count replayed receipts or another GMs result twice', () => {
    const evidence = completeEvidence();
    const rows = evidence.get('ship-1');
    const captain = rows.filter(row => row.kind === 'station-feedback' && row.value.station === 'captain');
    captain[1].value.correlation = captain[0].value.correlation;
    expect(nativeMatrixOutcome(evidence).sixPeerWorkloadPassed).toBe(false);
    const other = completeEvidence();
    other.get('gm-2').filter(row => row.kind === 'gm-activity').forEach(row => row.value.entries[0].detail.data.operator.id = 'gm-1');
    expect(nativeMatrixOutcome(other).sixPeerWorkloadPassed).toBe(false);
  });
});
