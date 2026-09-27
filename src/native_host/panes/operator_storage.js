// One native operator-profile storage shim for Station and private GM surfaces.
// The host chooses the scope and filename; no transport identity is stored.
window.PhoenixInstallNativeOperatorStorage = function (send) {
  const profileKey = 'phoenix-operator-profile-v1';
  let profileJson = null, profileLoaded = false;
  let localeChoice = null, pendingLocale = null;
  const validLocale = value => typeof value === 'string' && /^[A-Za-z0-9-]{1,35}$/.test(value);
  const withLocale = json => {
    let profile;
    try { profile = JSON.parse(json || 'null'); } catch (_) { profile = null; }
    if (!profile || typeof profile !== 'object') {
      profile = { kind: 'project-phoenix/operator-profile', version: 1 };
    }
    if (localeChoice) profile.locale = localeChoice;
    return JSON.stringify(profile);
  };
  function record(operation, data) {
    return send(JSON.stringify(Object.assign({type:'NativeOperator',operation},data || {})));
  }
  window.PhoenixOperatorStorage = {
    isReady: () => profileLoaded,
    getItem: key => key === profileKey ? profileJson : null,
    setItem(key,json) {
      if (key !== profileKey || !profileLoaded) throw new Error('Operator profile is still loading');
      const next = withLocale(String(json));
      if (record('save',{profile:next}) === false) throw new Error('Operator profile storage is unavailable');
      profileJson = next;
    },
  };
  window.PhoenixLocaleStorage = {
    getItem: () => localeChoice,
    setItem(_key, value) {
      if (!validLocale(value)) throw new Error('Invalid private language');
      localeChoice = value;
      if (!profileLoaded) { pendingLocale = value; return; }
      const next = withLocale(profileJson);
      if (record('save', {profile:next}) === false) throw new Error('Language storage is unavailable');
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
      try {
        const loaded = JSON.parse(profileJson || 'null')?.locale;
        localeChoice = validLocale(loaded) ? loaded : null;
      } catch (_) { localeChoice = null; }
      if (pendingLocale) {
        localeChoice = pendingLocale;
        pendingLocale = null;
        profileJson = withLocale(profileJson);
        record('save', { profile: profileJson });
      }
      window.dispatchEvent(new Event('phoenix-operator-profile-loaded'));
      window.dispatchEvent(new Event('phoenix-native-locale-loaded'));
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
