// One native operator-profile storage shim for Station and private GM surfaces.
// The host chooses the scope and filename; no transport identity is stored.
window.PhoenixInstallNativeOperatorStorage = function (send) {
  const profileKey = 'phoenix-operator-profile-v1';
  let profileJson = null, profileLoaded = false;
  function record(operation, data) {
    return send(JSON.stringify(Object.assign({type:'NativeOperator',operation},data || {})));
  }
  window.PhoenixOperatorStorage = {
    isReady: () => profileLoaded,
    getItem: key => key === profileKey ? profileJson : null,
    setItem(key,json) {
      if (key !== profileKey || !profileLoaded) throw new Error('Operator profile is still loading');
      const next = String(json);
      if (record('save',{profile:next}) === false) throw new Error('Operator profile storage is unavailable');
      profileJson = next;
    },
  };
  window.__phoenixOperatorReply = function (reply) {
    if (reply.operation === 'load') {
      // An unselected hull has no filing scope. Never save its defaults over
      // the eventual selected hull's profile; retry this established state.
      if (reply.error === 'Profile storage is unavailable') return;
      profileLoaded = true;
      profileJson = typeof reply.profile === 'string' ? reply.profile : null;
      window.dispatchEvent(new Event('phoenix-operator-profile-loaded'));
    }
    if (reply.operation === 'load' || reply.operation === 'save') {
      window.PhoenixOperatorStorageStatus = reply.status === 'error' ? reply : null;
      window.dispatchEvent(new Event('phoenix-operator-storage-status'));
    }
  };
  function load() {
    if (profileLoaded) return;
    record('load'); setTimeout(load,1000);
  }
  window.__phoenixOperatorReload = function () {
    profileLoaded = false; profileJson = null; load();
  };
  load(); return record;
};
