#[cfg(target_arch = "wasm32")]
use std::cell::RefCell;
use std::collections::HashMap;

/// One installed pack in the ordered overlay stack (issue #987).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ActivePack {
    /// Stable pack id from the manifest `[pack] id`.
    pub id: String,
    /// Display name from `[pack] name`.
    pub name: String,
    /// Version string from `[pack] version`.
    pub version: String,
    /// Exact authored path -> TOML for every supported file the pack carries.
    pub files: HashMap<String, String>,
    /// Immutable render/audio members. Every runtime reader uses this same
    /// ordered overlay; archive bytes never need a temporary public URL.
    pub assets: std::collections::BTreeMap<String, std::sync::Arc<[u8]>>,
    /// The exact accepted source bundle, used when cloning into offline
    /// Workshop. Runtime interventions never reconstruct authored source.
    pub source_archive: Option<std::sync::Arc<[u8]>>,
    /// The pack's raw `scenarios.toml` manifest.
    pub manifest_toml: String,
}

/// A single authored path carried by more than one active pack (issue #987).
///
/// `winner` is the pack id that wins the path under the precedence policy (the
/// latest-loaded pack carrying it); `losers` are the shadowed pack ids, in load
/// order (earliest first).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PathConflict {
    pub path: String,
    pub winner: String,
    pub losers: Vec<String>,
}

/// Resolve `path` against an ordered pack stack (oldest → newest); later wins.
///
/// Pure: the returned content is a function only of the stack contents and their
/// order, so precedence is deterministic and testable without session state.
pub fn overlay_lookup<'a>(packs: &'a [ActivePack], path: &str) -> Option<&'a str> {
    packs
        .iter()
        .rev()
        .find_map(|p| p.files.get(path).map(String::as_str))
}

/// The id of the pack that wins `path` under the precedence policy, if any
/// (issue #987 provenance). The pure core behind [`overlay_source`].
pub fn overlay_source_in<'a>(packs: &'a [ActivePack], path: &str) -> Option<&'a str> {
    packs
        .iter()
        .rev()
        .find(|p| p.files.contains_key(path) || p.assets.contains_key(path))
        .map(|p| p.id.as_str())
}

/// Every authored path carried by two or more packs in the stack, with its
/// winner + shadowed losers (issue #987). Deterministic: paths are reported in
/// sorted order and each conflict's pack ids follow load order.
pub fn overlay_conflicts(packs: &[ActivePack]) -> Vec<PathConflict> {
    use std::collections::BTreeMap;
    let mut by_path: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for pack in packs {
        for path in pack.files.keys().chain(pack.assets.keys()) {
            by_path
                .entry(path.as_str())
                .or_default()
                .push(pack.id.as_str());
        }
    }
    let mut conflicts = Vec::new();
    for (path, ids) in by_path {
        if ids.len() < 2 {
            continue;
        }
        // Load order is preserved because the outer loop walks `packs` in order;
        // the last id is the newest carrier (the winner).
        let (winner, losers) = ids.split_last().expect("len >= 2");
        conflicts.push(PathConflict {
            path: path.to_string(),
            winner: winner.to_string(),
            losers: losers.iter().map(|s| s.to_string()).collect(),
        });
    }
    conflicts
}

// ## Where the stack LIVES, and why that differs per host
//
// The browser is one thread, so a `thread_local!` IS the browser's session
// scoping: every reader is the page, and a reload drops the stack.
//
// A native host is not one thread and never was. `apply_mod_pack_choice`
// (`native_host::host_lobby`) is an ordinary Bevy `Update` system, so it installs
// on whichever compute-pool worker took that frame; `feed_mod_pack_shelf` is a
// SEPARATE system that may run on another; `FsFragmentSource::read` runs on
// whichever worker spawns; and the delivery thread is a different thread
// outright. A per-thread stack there would install a pack on one worker and
// leave every other reader looking at an empty overlay — silently, and
// intermittently, which is the worst shape that bug could take. So native stores
// the stack in a process-global `RwLock`, for exactly the reason
// [`NATIVE_CONFIG_CACHE`] below is one.
//
// The public functions over it keep one signature across both targets, so no
// caller knows (or can come to depend on) which storage it is talking to. The
// price is that native tests no longer get libtest's thread-per-test isolation
// for free — see [`overlay_test_guard`], which is how they ask for it.
#[cfg(target_arch = "wasm32")]
thread_local! {
    /// The ordered mod-pack overlay stack for the current host session
    /// (oldest → newest). See the precedence policy above.
    static ACTIVE_PACKS: RefCell<Vec<ActivePack>> = const { RefCell::new(Vec::new()) };
    static PACK_REVISION: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// The ordered mod-pack overlay stack for the current host PROCESS
/// (oldest → newest). See the precedence policy above, and the note on why this
/// is shared rather than per-thread off the browser.
#[cfg(not(target_arch = "wasm32"))]
static ACTIVE_PACKS: std::sync::RwLock<Vec<ActivePack>> = std::sync::RwLock::new(Vec::new());

#[cfg(not(target_arch = "wasm32"))]
static PACK_REVISION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Presentation cache generation, independent of simulation ticks and digests.
pub fn mod_pack_revision() -> u64 {
    #[cfg(target_arch = "wasm32")]
    {
        PACK_REVISION.with(std::cell::Cell::get)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        PACK_REVISION.load(std::sync::atomic::Ordering::Acquire)
    }
}

/// Read the stack. Every public lookup goes through here, so the two storages
/// are described once and the readers stay identical.
#[cfg(target_arch = "wasm32")]
fn with_active_packs<R>(f: impl FnOnce(&[ActivePack]) -> R) -> R {
    ACTIVE_PACKS.with(|s| f(&s.borrow()))
}

#[cfg(not(target_arch = "wasm32"))]
fn with_active_packs<R>(f: impl FnOnce(&[ActivePack]) -> R) -> R {
    f(&ACTIVE_PACKS.read().expect("mod-pack overlay poisoned"))
}

/// Mutate the stack. The writer twin of [`with_active_packs`], for the same
/// reason.
#[cfg(target_arch = "wasm32")]
fn with_active_packs_mut<R>(f: impl FnOnce(&mut Vec<ActivePack>) -> R) -> R {
    ACTIVE_PACKS.with(|s| {
        let result = f(&mut s.borrow_mut());
        PACK_REVISION.with(|revision| revision.set(revision.get().wrapping_add(1)));
        result
    })
}

#[cfg(not(target_arch = "wasm32"))]
fn with_active_packs_mut<R>(f: impl FnOnce(&mut Vec<ActivePack>) -> R) -> R {
    let mut packs = ACTIVE_PACKS.write().expect("mod-pack overlay poisoned");
    let result = f(&mut packs);
    PACK_REVISION.fetch_add(1, std::sync::atomic::Ordering::Release);
    result
}

/// A snapshot of the active pack stack, oldest → newest (issue #987).
pub fn active_packs() -> Vec<ActivePack> {
    with_active_packs(<[ActivePack]>::to_vec)
}

/// Push a validated pack onto the top (newest end) of the stack (issue #987).
///
/// Called only after atomic validation accepts the pack, so nothing partial is
/// ever installed. Does NOT evict earlier packs — a later pack merely shadows an
/// earlier one for the paths they share (that is the whole point of the stack).
pub fn push_mod_pack(pack: ActivePack) {
    with_active_packs_mut(|v| v.push(pack));
}

/// Remove the pack with `id` from the stack (issue #987). Precedence for every
/// path it owned re-resolves on the next lookup — the next pack down that carries
/// the path becomes the winner. Returns whether a pack was removed.
pub fn remove_mod_pack(id: &str) -> bool {
    with_active_packs_mut(|v| {
        let before = v.len();
        v.retain(|p| p.id != id);
        v.len() != before
    })
}

/// Reorder the stack so it matches `ids` (oldest → newest). Packs whose id is not
/// named keep their relative order after the named ones; unknown ids are ignored.
/// Precedence re-resolves on the next lookup (issue #987).
pub fn reorder_mod_packs(ids: &[String]) {
    with_active_packs_mut(|v| {
        let mut reordered: Vec<ActivePack> = Vec::with_capacity(v.len());
        for id in ids {
            if let Some(pos) = v.iter().position(|p| &p.id == id) {
                reordered.push(v.remove(pos));
            }
        }
        // Anything not named by `ids` keeps its (now-compacted) relative order.
        reordered.append(v);
        *v = reordered;
    });
}

/// Look up an overridden authored path in the mod-pack overlay stack, if any.
///
/// Both content channels consult this before falling back to the normal fetch,
/// so the WINNING pack's file (the latest loaded carrying the path) is used for
/// any exact authored path it carries.
pub fn mod_pack_overlay_get(path: &str) -> Option<String> {
    with_active_packs(|packs| overlay_lookup(packs, path).map(str::to_string))
}

/// Exact accepted binary bytes for native and browser render/audio adapters.
pub fn mod_pack_asset(path: &str) -> Option<std::sync::Arc<[u8]>> {
    with_active_packs(|packs| {
        packs
            .iter()
            .rev()
            .find_map(|pack| pack.assets.get(path).cloned())
    })
}

/// One immutable winning map for presentation cache refresh. Arc clones do not
/// copy models or decoded input. Called only when the overlay revision changes.
pub fn mod_pack_assets() -> std::collections::BTreeMap<String, std::sync::Arc<[u8]>> {
    with_active_packs(|packs| {
        packs
            .iter()
            .flat_map(|pack| {
                pack.assets
                    .iter()
                    .map(|(path, data)| (path.clone(), data.clone()))
            })
            .collect()
    })
}

pub fn mod_pack_source_archive(id: &str) -> Option<std::sync::Arc<[u8]>> {
    with_active_packs(|packs| {
        packs
            .iter()
            .find(|pack| pack.id == id)
            .and_then(|pack| pack.source_archive.clone())
    })
}

/// The id of the pack that currently owns `path` in the overlay stack, if any
/// (issue #987 provenance). Lets any consumer name the owning pack — the host
/// conflict summary, a diagnostic log — without duplicating the walk.
///
/// Returns an owned `String` rather than the `&str` the pure [`overlay_source_in`]
/// yields, because the stack lives behind a `RefCell`/`RwLock` that cannot hand
/// out a borrow past the accessor closure.
pub fn overlay_source(path: &str) -> Option<String> {
    with_active_packs(|packs| overlay_source_in(packs, path).map(str::to_string))
}

/// Discard the WHOLE mod-pack overlay stack for the current session (issue #760
/// AC4, #987). Called before return-to-lobby, so uploaded state never leaks into
/// a fresh selection stage or a same-page next round. A page reload clears the
/// browser's thread-local anyway; this covers the same-page seams, and is the
/// only thing that empties a native host's process-global stack short of exit.
pub fn clear_mod_pack_overlay() {
    with_active_packs_mut(Vec::clear);
}

/// Serialise the tests that drive the native overlay stack, and hand each of
/// them an empty one.
///
/// On the browser the stack is a `thread_local!`, so libtest's thread-per-test
/// gave every test its own for free — the same reasoning the sibling preload
/// maps above rely on. Native's stack is process-global (it has to be: Bevy
/// systems run on worker threads), so that isolation now has to be ASKED FOR.
/// Taking the guard clears the stack and blocks any other overlay test; dropping
/// it clears the stack again. A test therefore can neither inherit another
/// test's packs nor leak its own into an unrelated one running beside it.
#[cfg(all(feature = "test-support", not(target_arch = "wasm32")))]
#[allow(dead_code)]
pub struct OverlayTestGuard(std::sync::MutexGuard<'static, ()>);

#[cfg(all(feature = "test-support", not(target_arch = "wasm32")))]
impl Drop for OverlayTestGuard {
    fn drop(&mut self) {
        clear_mod_pack_overlay();
    }
}

#[cfg(all(feature = "test-support", not(target_arch = "wasm32")))]
pub fn overlay_test_guard() -> OverlayTestGuard {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    // A test that panicked holding the guard poisons the mutex. The next test
    // still wants a clean overlay, not a cascade of failures about the first one.
    let guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    clear_mod_pack_overlay();
    OverlayTestGuard(guard)
}

impl From<&ActivePack> for phoenix_model::messages::ActivePackWire {
    fn from(pack: &ActivePack) -> Self {
        Self {
            id: pack.id.clone(),
            name: pack.name.clone(),
            version: pack.version.clone(),
        }
    }
}
