/**
 * models-rig-view.js
 *
 * THIN Three.js view for Models Mode. A factory `createRigScene(host, deps)`
 * returns a controller that:
 *
 *   - owns a Three scene (grid, lights, OrbitControls)
 *   - loads a GLB from an ArrayBuffer and applies the base rig live
 *   - exposes the post-base-rig bounding box (extents) after each change
 *   - manages marker objects (ArrowHelper + a movable anchor) edited via a
 *     TransformControls gizmo (translate = position, rotate = direction)
 *
 * Three.js + addons are injected via `deps` for testability; in the browser
 * they fall back to the module imports. The pure rig math lives in
 * models-rig.js — this file only bridges that data to the GPU.
 *
 * NOTE: this module is intentionally NOT exercised by the node test suite
 * (it needs WebGL + a real GLB). The heavy, branchy logic was pushed into
 * models-rig.js, which is fully unit-tested.
 */
import * as THREE_IMPORT from 'three';
import { GLTFLoader } from 'three/addons/loaders/GLTFLoader.js';
import { OrbitControls } from 'three/addons/controls/OrbitControls.js';
import { TransformControls } from 'three/addons/controls/TransformControls.js';
import { FORWARD, normalizeDirection } from './models-rig.js';

const MARKER_COLOR = 0x4ad6a6;
const MARKER_SELECTED_COLOR = 0xffd24a;

/**
 * @param {HTMLElement} host  container the canvas mounts into
 * @param {object} [deps]
 *   Override the Three modules for tests:
 *   { THREE, GLTFLoader, OrbitControls, TransformControls }.
 * @returns {object} controller (see method docs below)
 */
export function createRigScene(host, deps = {}) {
  const THREE = deps.THREE || THREE_IMPORT;
  const Loader = deps.GLTFLoader || GLTFLoader;
  const Orbit = deps.OrbitControls || OrbitControls;
  const Transform = deps.TransformControls || TransformControls;

  const width = host.clientWidth || 480;
  const height = host.clientHeight || 360;

  const scene = new THREE.Scene();
  scene.background = new THREE.Color(0x0c0d10);

  const camera = new THREE.PerspectiveCamera(50, width / height, 0.1, 100000);
  camera.position.set(30, 20, 30);

  const renderer = new THREE.WebGLRenderer({ antialias: true });
  renderer.setSize(width, height);
  renderer.setPixelRatio(Math.min(2, (typeof window !== 'undefined' && window.devicePixelRatio) || 1));
  host.appendChild(renderer.domElement);

  // ── Lighting ────────────────────────────────────────────────────────
  scene.add(new THREE.AmbientLight(0xffffff, 0.6));
  const key = new THREE.DirectionalLight(0xffffff, 1.0);
  key.position.set(1, 2, 1);
  scene.add(key);
  const fill = new THREE.DirectionalLight(0x88aaff, 0.4);
  fill.position.set(-1, 0.5, -1);
  scene.add(fill);

  // ── Grid (XZ plane at Y=0; 1-unit cells, heavier major every 10) ─────
  let grid = makeGrid(THREE, 100);
  scene.add(grid);

  // ── Controls ────────────────────────────────────────────────────────
  const controls = new Orbit(camera, renderer.domElement);
  controls.enableDamping = true;

  const gizmo = new Transform(camera, renderer.domElement);
  // TransformControls is a controls object; its visual lives in a child.
  scene.add(getGizmoHelper(gizmo));
  gizmo.addEventListener('dragging-changed', (e) => {
    controls.enabled = !e.value;
  });

  // ── Model + rig state ───────────────────────────────────────────────
  // `baseGroup` holds the loaded GLB AND the marker anchors, and carries the
  // base transform, so the post-base-rig bounding box is just baseGroup's
  // world box. Markers live in baseGroup so their position/direction are
  // expressed in POST-base-rig space (the same frame the schema serializes),
  // composing correctly with a non-identity base transform.
  const baseGroup = new THREE.Group();
  // Engine applies base rotation as Quat::from_euler(EulerRot::XYZ, x, y, z)
  // (src/world/server.rs). Three's default Euler order is also 'XYZ'; set it
  // explicitly so the match is documented and can't drift.
  baseGroup.rotation.order = 'XYZ';
  scene.add(baseGroup);
  let modelRoot = null; // the loaded gltf.scene

  // ── Forward-direction arrow ─────────────────────────────────────────
  // World-space arrow at the origin pointing -Z (game forward = [0,0,-1]).
  // Stays fixed in world space so it always shows the canonical forward
  // direction regardless of base-rig rotation.
  const forwardArrow = new THREE.ArrowHelper(
    new THREE.Vector3(0, 0, -1),
    new THREE.Vector3(0, 0, 0),
    3,
    0xffffff,
    0.8,
    0.4,
  );
  scene.add(forwardArrow);

  const markers = new Map(); // name -> { anchor, arrow }
  let selectedName = null;
  let onChangeCb = null;

  // ── Render loop ─────────────────────────────────────────────────────
  let disposed = false;
  let rafId = null;
  function animate() {
    if (disposed) return;
    rafId = requestAnimationFrame(animate);
    controls.update();
    renderer.render(scene, camera);
  }
  if (typeof requestAnimationFrame === 'function') animate();

  // ── Gizmo → marker change wiring ────────────────────────────────────
  gizmo.addEventListener('objectChange', () => {
    if (!selectedName) return;
    const entry = markers.get(selectedName);
    if (!entry) return;
    syncArrowToAnchor(entry);
    emitMarkerChange(selectedName);
  });

  function emitMarkerChange(name) {
    if (typeof onChangeCb !== 'function') return;
    const entry = markers.get(name);
    if (!entry) return;
    onChangeCb(name, {
      position: [entry.anchor.position.x, entry.anchor.position.y, entry.anchor.position.z],
      direction: anchorForward(THREE, entry.anchor),
    });
  }

  // ── Public API ──────────────────────────────────────────────────────

  /**
   * Replace the loaded model from a GLB ArrayBuffer. Resolves with the
   * post-base-rig extents once parsed. Re-frames the camera to fit.
   */
  async function loadModel(arrayBuffer) {
    const loader = new Loader();
    const gltf = await new Promise((resolve, reject) => {
      loader.parse(arrayBuffer, '', resolve, reject);
    });
    if (modelRoot) {
      baseGroup.remove(modelRoot);
      disposeObject(THREE, modelRoot);
    }
    modelRoot = gltf.scene || gltf.scenes?.[0];
    baseGroup.add(modelRoot);
    frameCamera();
    return getExtents();
  }

  /**
   * Apply the base rig (offset/rotation/scale) to the loaded model. Returns
   * the recomputed post-base-rig extents.
   */
  function setBase(base) {
    const offset = base?.offset || [0, 0, 0];
    const rotation = base?.rotation || [0, 0, 0];
    const scale = base?.scale || [1, 1, 1];
    baseGroup.position.set(offset[0], offset[1], offset[2]);
    baseGroup.rotation.set(rotation[0], rotation[1], rotation[2]);
    baseGroup.scale.set(scale[0], scale[1], scale[2]);
    baseGroup.updateMatrixWorld(true);
    return getExtents();
  }

  /**
   * Post-base-rig bounding box as `{ min, max, size }` (vec3 arrays). Empty
   * box when nothing is loaded.
   */
  function getExtents() {
    if (!modelRoot) return { min: [0, 0, 0], max: [0, 0, 0], size: [0, 0, 0] };
    const box = new THREE.Box3().setFromObject(baseGroup);
    if (!isFinite(box.min.x)) return { min: [0, 0, 0], max: [0, 0, 0], size: [0, 0, 0] };
    const size = new THREE.Vector3();
    box.getSize(size);
    return {
      min: [box.min.x, box.min.y, box.min.z],
      max: [box.max.x, box.max.y, box.max.z],
      size: [size.x, size.y, size.z],
    };
  }

  /** Create (or replace) a marker's visuals at position with direction. */
  function addMarker(name, { position = [0, 0, 0], direction = FORWARD } = {}) {
    removeMarker(name);
    const dir = normalizeDirection(direction);

    const anchor = new THREE.Group();
    anchor.position.set(position[0], position[1], position[2]);
    orientAnchor(THREE, anchor, dir);

    const dot = new THREE.Mesh(
      new THREE.SphereGeometry(0.1, 16, 12),
      new THREE.MeshBasicMaterial({ color: MARKER_COLOR }),
    );
    anchor.add(dot);

    // Arrow points along the anchor's local -Z (game forward).
    const arrow = new THREE.ArrowHelper(
      new THREE.Vector3(0, 0, -1),
      new THREE.Vector3(0, 0, 0),
      3,
      MARKER_COLOR,
    );
    anchor.add(arrow);

    // Parent to baseGroup so the anchor's local transform IS its post-base-rig
    // position/direction — i.e. the marker rides the rigged model and the
    // gizmo-derived values are already in the serialized frame.
    baseGroup.add(anchor);
    markers.set(name, { anchor, arrow, dot });
  }

  /** Remove a marker's visuals and detach the gizmo if it was selected. */
  function removeMarker(name) {
    const entry = markers.get(name);
    if (!entry) return;
    if (selectedName === name) {
      detachGizmo();
      selectedName = null;
    }
    baseGroup.remove(entry.anchor);
    disposeObject(THREE, entry.anchor);
    markers.delete(name);
  }

  /**
   * Remove ALL marker visuals (anchors/arrows/dots) and detach the gizmo.
   * Used when switching variants so the previous variant's markers don't
   * linger as ghosts (addMarker only replaces same-named markers).
   */
  function clearMarkers() {
    for (const name of [...markers.keys()]) removeMarker(name);
  }

  /** Select a marker for editing; attaches the gizmo to its anchor. */
  function select(name) {
    if (selectedName && markers.has(selectedName)) {
      tintMarker(markers.get(selectedName), MARKER_COLOR);
    }
    selectedName = name || null;
    if (!selectedName) {
      detachGizmo();
      return;
    }
    const entry = markers.get(selectedName);
    if (!entry) {
      detachGizmo();
      selectedName = null;
      return;
    }
    tintMarker(entry, MARKER_SELECTED_COLOR);
    gizmo.attach(entry.anchor);
  }

  /** 'translate' edits position; 'rotate' edits direction. */
  function setGizmoMode(mode) {
    if (mode === 'translate' || mode === 'rotate') {
      gizmo.setMode(mode);
    }
  }

  /** Register a callback fired when a marker is moved/rotated via gizmo. */
  function onChange(cb) {
    onChangeCb = typeof cb === 'function' ? cb : null;
  }

  /** Re-frame the camera to fit the current post-base-rig extents. */
  function frame() {
    frameCamera();
  }

  /** Resize the renderer/camera to the host's current size. */
  function resize() {
    const w = host.clientWidth || width;
    const h = host.clientHeight || height;
    camera.aspect = w / h;
    camera.updateProjectionMatrix();
    renderer.setSize(w, h);
  }

  /** Tear down GPU resources and the render loop. */
  function dispose() {
    disposed = true;
    // Stop the render loop. cancelAnimationFrame on the last-scheduled id
    // guarantees no further frame fires even before the `disposed` guard.
    if (rafId !== null && typeof cancelAnimationFrame === 'function') {
      cancelAnimationFrame(rafId);
      rafId = null;
    }
    detachGizmo();
    clearMarkers();
    if (modelRoot) disposeObject(THREE, modelRoot);
    disposeGizmo(gizmo, scene);
    scene.remove(forwardArrow);
    disposeObject(THREE, forwardArrow);
    disposeObject(THREE, grid);
    controls.dispose?.();
    renderer.dispose?.();
    if (renderer.domElement?.parentNode === host) host.removeChild(renderer.domElement);
  }

  // ── helpers ─────────────────────────────────────────────────────────

  function detachGizmo() {
    gizmo.detach?.();
  }

  function frameCamera() {
    const ext = getExtents();
    const sx = ext.size[0] || 1;
    const sy = ext.size[1] || 1;
    const sz = ext.size[2] || 1;
    const radius = Math.max(sx, sy, sz, 1) * 1.6 + 1;
    const cx = (ext.min[0] + ext.max[0]) / 2;
    const cy = (ext.min[1] + ext.max[1]) / 2;
    const cz = (ext.min[2] + ext.max[2]) / 2;
    controls.target.set(cx, cy, cz);
    camera.position.set(cx + radius, cy + radius * 0.7, cz + radius);
    camera.near = Math.max(0.01, radius / 1000);
    camera.far = radius * 100;
    camera.updateProjectionMatrix();
    controls.update?.();
    resizeGridToExtents(ext);
    resizeForwardArrow(ext);
  }

  function resizeForwardArrow(ext) {
    const span = Math.max(ext.size[0], ext.size[1], ext.size[2], 1);
    const len = span * 0.8;
    const headLen = len * 0.25;
    const headWidth = headLen * 0.5;
    forwardArrow.setLength(len, headLen, headWidth);
    forwardArrow.position.set(
      (ext.min[0] + ext.max[0]) / 2,
      ext.min[1],
      (ext.min[2] + ext.max[2]) / 2,
    );
  }

  function resizeGridToExtents(ext) {
    const span = Math.max(ext.size[0], ext.size[2], 10);
    const units = Math.ceil(span * 1.5);
    scene.remove(grid);
    disposeObject(THREE, grid);
    grid = makeGrid(THREE, units);
    scene.add(grid);
  }

  return {
    loadModel,
    setBase,
    getExtents,
    addMarker,
    removeMarker,
    clearMarkers,
    select,
    setGizmoMode,
    onChange,
    frame,
    resize,
    dispose,
    // exposed for debugging/tests
    _scene: scene,
    _markers: markers,
  };
}

// ── module-scope pure-ish helpers ──────────────────────────────────────

function makeGrid(THREE, units) {
  // GridHelper divisions = units gives 1-unit cells; major lines are the
  // built-in centre lines. We layer a coarser grid for the "every 10".
  const size = Math.max(10, units);
  const grid = new THREE.GridHelper(size, size, 0x444c55, 0x23272d);
  const major = new THREE.GridHelper(size, Math.max(1, Math.round(size / 10)), 0x5a6470, 0x3a4048);
  major.position.y = 0.001;
  grid.add(major);
  return grid;
}

function orientAnchor(THREE, anchor, dir) {
  // Rotate the anchor so its local -Z aligns with `dir`.
  const forward = new THREE.Vector3(0, 0, -1);
  const target = new THREE.Vector3(dir[0], dir[1], dir[2]).normalize();
  const quat = new THREE.Quaternion().setFromUnitVectors(forward, target);
  anchor.quaternion.copy(quat);
}

function anchorForward(THREE, anchor) {
  const v = new THREE.Vector3(0, 0, -1).applyQuaternion(anchor.quaternion).normalize();
  return [v.x, v.y, v.z];
}

function syncArrowToAnchor() {
  // Arrow is a child of the anchor pointing local -Z, so it follows the
  // anchor's rotation automatically. Hook kept for symmetry/extension.
}

function tintMarker(entry, color) {
  if (entry?.dot?.material) entry.dot.material.color.setHex(color);
  entry?.arrow?.setColor?.(color);
}

function getGizmoHelper(gizmo) {
  // r160 TransformControls: getHelper() returns the visual Object3D. Older
  // builds ARE the Object3D. Support both.
  if (typeof gizmo.getHelper === 'function') return gizmo.getHelper();
  return gizmo;
}

function disposeGizmo(gizmo, scene) {
  const helper = getGizmoHelper(gizmo);
  if (helper && helper.parent === scene) scene.remove(helper);
  gizmo.dispose?.();
}

function disposeObject(THREE, obj) {
  obj.traverse?.((child) => {
    if (child.geometry) child.geometry.dispose?.();
    const mat = child.material;
    if (Array.isArray(mat)) mat.forEach((m) => m.dispose?.());
    else mat?.dispose?.();
  });
}
