/**
 * Parallel-stream raw throughput.
 *
 * The measurement engine issues one request at a time and reads every response
 * through `r.text()`, decoding the whole payload into a JavaScript string. That
 * is a sound way to measure what a single connection experiences, but it means
 * its download figure is bounded by single-stream TCP plus main-thread
 * decoding, not by the link.
 *
 * This measures the other thing: how much the link carries when several
 * connections pull at once and nothing decodes the bytes. It is a **different
 * number**, always reported as such — never merged with the engine's result,
 * and never presented as "the" speed.
 *
 * Bodies are drained through the streaming reader and discarded, so the
 * browser never materialises the payload and the client is not the bottleneck.
 */

export interface ParallelResult {
  /** Aggregate bits per second across all streams. */
  bps: number;
  streams: number;
  bytesPerStream: number;
  totalBytes: number;
  elapsedMs: number;
}

export interface ParallelOptions {
  streams: number;
  bytesPerStream: number;
  /** Abort if the whole run exceeds this, so a stalled link cannot hang the page. */
  timeoutMs?: number;
  signal?: AbortSignal;
}

/** Pulls one stream, discarding bytes as they arrive. */
async function drain(url: string, signal: AbortSignal): Promise<number> {
  const res = await fetch(url, { cache: 'no-store', signal });
  if (!res.ok) throw new Error(`${url} responded ${res.status}`);
  if (!res.body) {
    // No streaming reader available: fall back to a blob, which at least
    // avoids the string decode that `text()` would do.
    const blob = await res.blob();
    return blob.size;
  }

  const reader = res.body.getReader();
  let total = 0;
  for (;;) {
    const { done, value } = await reader.read();
    if (done) break;
    total += value?.byteLength ?? 0;
  }
  return total;
}

/**
 * Runs the harness. Resolves with the aggregate rate, or throws if a stream
 * fails or the run is aborted.
 */
export async function measureParallelThroughput(
  opts: ParallelOptions,
): Promise<ParallelResult> {
  const { streams, bytesPerStream } = opts;
  if (streams < 1 || bytesPerStream < 1) {
    throw new Error('parallel harness needs at least one stream of at least one byte');
  }

  const controller = new AbortController();
  const onAbort = () => controller.abort();
  opts.signal?.addEventListener('abort', onAbort, { once: true });
  const timeout = opts.timeoutMs
    ? setTimeout(() => controller.abort(), opts.timeoutMs)
    : undefined;

  try {
    // A short warm-up so TCP has ramped and the connections exist before the
    // clock starts; otherwise the measurement is mostly connection setup.
    await drain(`/__down?bytes=1000000&warmup=1`, controller.signal);

    const started = performance.now();
    const sizes = await Promise.all(
      Array.from({ length: streams }, (_, i) =>
        // The distinct query parameter keeps the browser from coalescing or
        // caching these as one request.
        drain(`/__down?bytes=${bytesPerStream}&stream=${i}`, controller.signal),
      ),
    );
    const elapsedMs = performance.now() - started;

    const totalBytes = sizes.reduce((a, b) => a + b, 0);
    const expected = streams * bytesPerStream;
    if (totalBytes < expected) {
      throw new Error(`short read: expected ${expected} bytes, received ${totalBytes}`);
    }

    return {
      bps: elapsedMs > 0 ? (totalBytes * 8) / (elapsedMs / 1000) : 0,
      streams,
      bytesPerStream,
      totalBytes,
      elapsedMs,
    };
  } finally {
    if (timeout !== undefined) clearTimeout(timeout);
    opts.signal?.removeEventListener('abort', onAbort);
  }
}

/**
 * Stream count and size for a link of roughly `expectedBps`.
 *
 * The transfer wants to last long enough to be a measurement rather than a
 * burst, and short enough not to be tedious. Around a second in total is the
 * balance; the bounds stop a very slow or very fast link from producing
 * something absurd.
 */
export function suggestedProfile(expectedBps: number): {
  streams: number;
  bytesPerStream: number;
} {
  const streams = 4;
  const targetSeconds = 1;
  const perStream = (expectedBps * targetSeconds) / 8 / streams;

  const bytesPerStream = Math.round(
    Math.min(256 * 1024 * 1024, Math.max(4 * 1024 * 1024, perStream)),
  );
  return { streams, bytesPerStream };
}
