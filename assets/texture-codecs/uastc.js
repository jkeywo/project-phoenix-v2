// One worker at a time bounds transcoder memory. Bevy owns caching and fallback;
// no fetch/context interception and no retained decoded texture in JavaScript.
let queue = Promise.resolve();
export function transcode(bytes, target) {
  const task = queue.then(() => runWorker(bytes, target));
  // Do not retain the last decoded buffer in this module's resolved queue.
  queue = task.then(() => {}, () => {});
  return task;
}

function runWorker(bytes, target) {
  return new Promise((resolve, reject) => {
    const worker = new Worker(new URL('./uastc-worker.js', import.meta.url));
    const timer = setTimeout(() => finish(new Error('UASTC worker timed out')), 30000);
    function finish(error, result) {
      clearTimeout(timer);
      worker.terminate();
      if (error) reject(error); else resolve(new Uint8Array(result));
    }
    worker.onerror = event => finish(new Error(event.message || 'UASTC worker failed'));
    worker.onmessageerror = () => finish(new Error('Invalid UASTC worker message'));
    worker.onmessage = ({ data }) => {
      if (data.error) finish(new Error(data.error));
      else if (!(data.buffer instanceof ArrayBuffer)) finish(new Error('Missing UASTC output'));
      else finish(null, data.buffer);
    };
    try {
      worker.postMessage({ buffer: bytes.buffer, target }, [bytes.buffer]);
    } catch (error) { finish(error); }
  });
}
