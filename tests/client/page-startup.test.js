import { expect, it, vi } from 'vitest';
import '../../gui/page-startup.js';

it('holds early Host Channel delivery until audio is installed and preserves order', () => {
  const win = new EventTarget(), delivered = [];
  const startup = globalThis.PhoenixPageStartup.install(win, { host: true });
  win.__hostChannel('audio_config', 1); win.__hostChannel('hud', 2);
  startup.callAudio('startGameAudio');
  win.__hostChannelReady((...args) => delivered.push(args));
  expect(delivered).toEqual([]);
  const audio = { startGameAudio: vi.fn(), setPageActive: vi.fn() };
  win.__hostAudioReady(audio);
  expect(audio.startGameAudio).toHaveBeenCalledOnce();
  expect(delivered).toEqual([['audio_config', 1], ['hud', 2]]);
  win.__hostChannel('hud', 3);
  expect(delivered.at(-1)).toEqual(['hud', 3]);
  expect(globalThis.PhoenixPageStartup.install(win)).toBe(startup);
  startup.dispose(); startup.dispose();
  win.__hostChannel('hud', 4);
  expect(delivered).toHaveLength(3);
  expect(audio.setPageActive).toHaveBeenCalledExactlyOnceWith(false);
});

it('queues chrome calls independently, tears down leases, and can restart', () => {
  const win = new EventTarget();
  const startup = globalThis.PhoenixPageStartup.install(win);
  startup.callChrome('setConnectionStatus', 'connecting');
  const chrome = { setConnectionStatus: vi.fn(), releaseWakeLock: vi.fn() };
  win.__pageChromeReady(chrome);
  expect(chrome.setConnectionStatus).toHaveBeenCalledWith('connecting');
  startup.dispose();
  expect(chrome.releaseWakeLock).toHaveBeenCalledOnce();
  const replacement = globalThis.PhoenixPageStartup.install(win);
  expect(replacement).not.toBe(startup); replacement.dispose();
});

