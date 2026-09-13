// Installed before the shared private owner. This document has no audio device:
// one current settings/control envelope and one expiring cue cross its native
// bridge. Rust binds the surface identity; the page cannot name another output.
(function (root) {
  const buses = ['master', 'music', 'ambience', 'effects', 'alerts', 'interface'];
  const ids = ['clicks', 'pending', 'applied', 'refused', 'timedOut', 'actionable', 'test'];
  const registered = new Map(), listeners = new Set();
  let state = {status:'loading',test:'idle',generation:null,categories:['alerts','interface'],native:true};
  let mix = {}, mono = false, reducedRange = false, dirtyMix = false, pending = null, stop = false, retry = false, disposed = false;
  let preview=null,previewReply=null,previewSerial=0,previewTimer=null,stopPreview=false;
  const notify = () => { for (const fn of listeners) fn(); };
  const profileReady = () => root.PhoenixOperatorStorage?.isReady?.() === true;
  function discard() {
    if (pending?.test || state.test === 'loading') { state = {...state,test:'idle'}; notify(); }
    pending = null;
  }
  const silent = category => !mix.master || !mix[category] || mix.master.muted || mix[category].muted
    || mix.master.level === 0 || mix[category].level === 0;
  function finishPreview(result) { if(previewTimer!==null) root.clearTimeout(previewTimer);previewTimer=null;
    const reply=previewReply;previewReply=null;reply?.resolve(result); }
  function cancelPreview() { if(preview || previewReply || ['loading','playing'].includes(state.preview)) stopPreview=true;
    preview=null;finishPreview(false); }
  function cue(id, test = false) {
    const sound = registered.get(id);
    if (disposed || !profileReady() || state.status !== 'playing' || !Number.isSafeInteger(state.generation)
        || !sound || silent(sound.category)) return false;
    pending = {id,at_ms:Date.now(),test};
    if (test) { state = {...state,test:'loading'}; notify(); }
    return true;
  }
  root.__phoenixPrivateAudioApply = function (json) {
    let value; try { value = JSON.parse(json); } catch (_) { return; }
    if (!value || !Number.isSafeInteger(value.generation) || value.generation < 0) return;
    if (Number.isSafeInteger(state.generation) && value.generation < state.generation) return;
    if (state.generation !== value.generation) dirtyMix = true;
    if (state.generation !== value.generation || value.status !== 'playing') { pending = null;cancelPreview(); }
    state = {...value,native:true}; notify();
    if(previewReply && value.preview_id===previewReply.id && value.preview!=='loading') finishPreview(value.preview==='playing');
  };
  root.__phoenixPrivateAudioDrain = function () {
    if (!Number.isSafeInteger(state.generation)) return '';
    const request = {type:'NativePrivateAudio',generation:state.generation};
    if (dirtyMix && profileReady()) { request.mix = mix; request.mono = mono; request.reducedRange = reducedRange; dirtyMix = false; }
    if (stop) { request.stop = true; stop = false; }
    if (retry && profileReady()) { request.retry = true; retry = false; }
    if(stopPreview) {request.stop_preview=true;stopPreview=false;}
    if(preview && profileReady() && !disposed) {
      const elapsed=Date.now()-preview.at_ms;
      if(elapsed>=0 && elapsed<=250) request.preview=preview;
      else finishPreview(false);
      preview=null;
    }
    if (pending && profileReady() && !disposed) {
      const elapsed = Date.now() - pending.at_ms;
      if (elapsed >= 0 && elapsed <= 250) request.cue = pending;
    }
    if (pending && !request.cue) discard();
    else pending = null;
    return Object.keys(request).length > 2 ? JSON.stringify(request) : '';
  };
  root.PhoenixPrivateAudioProvider = function ({onChange} = {}) {
    if (typeof onChange === 'function') listeners.add(onChange);
    return {
      register(id, sound) {
        if (ids.includes(id) && ['alerts','interface'].includes(sound?.category)) registered.set(id,sound);
      },
      setMix(value) {
        mix = Object.fromEntries(buses.map(id => [id, {
          level:Number.isFinite(value?.[id]?.level) ? Math.max(0,Math.min(1,value[id].level)) : 1,
          muted:value?.[id]?.muted === true,
        }]));
        dirtyMix = true;
        if (pending && silent(registered.get(pending.id)?.category)) discard();
        if(previewReply && silent(previewReply.category)) cancelPreview();
      },
      cue,
      setMono(value) { mono = value === true; dirtyMix = true; },
      setReducedRange(value) { reducedRange = value === true; dirtyMix = true; },
      async enable() {
        if (disposed || !profileReady()) return false;
        if (state.status !== 'playing') retry = true;
        return state.status === 'playing';
      },
      async testOutput(id) { return cue(id, true); },
      audition(definition,asset) {
        cancelPreview();
        if(disposed || !profileReady() || !state.audition || state.status!=='playing'
          || !Number.isSafeInteger(state.generation) || silent(definition.category)) return Promise.resolve(false);
        const id=++previewSerial;preview={id,at_ms:Date.now(),definition,asset};
        return new Promise(resolve=>{previewReply={id,category:definition.category,resolve};
          previewTimer=root.setTimeout(()=>{cancelPreview();notify();},6000);});
      },
      stopAudition() {cancelPreview();notify();},
      stopAll() { discard(); cancelPreview(); stop = true; },
      snapshot() { return {...state,reducedRange,reducedRangeAvailable:true}; },
      dispose() { disposed = true; discard(); cancelPreview(); stop = true; listeners.clear(); },
    };
  };
})(window);
