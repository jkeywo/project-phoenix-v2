// Cloudflare Worker — mints TURN relay credentials for the browser clients,
// so no API key ever appears in the publicly-visible client source.
//
// Two independent credential sources, tried concurrently; the response is the
// flat iceServers array the clients already consume ([{urls, username,
// credential}, ...]). Either source alone is enough; configuring both gives
// relay redundancy for free.
//
//   Metered.ca   — GET  https://<METERED_APP>.metered.live/api/v1/turn/credentials
//                  secret: METERED_KEY, var: METERED_APP
//   Cloudflare   — POST https://rtc.live.cloudflare.com/v1/turn/keys/<id>/credentials/generate-ice-servers
//   Realtime TURN  secrets: CF_TURN_KEY_ID, CF_TURN_API_TOKEN
//
// Required vars (set in wrangler.toml [vars]):
//   ALLOWED_ORIGIN — comma-separated list of allowed site origins
//                    (e.g. "https://pp-dev.example.com,https://you.github.io").
//                    The response echoes the request origin when it matches;
//                    a stale single-origin value here CORS-blocks every client
//                    fetch, which silently strips TURN from the ICE list and
//                    breaks all CGNAT/hotspot players (2026-08 field failure).

// Credentials outlive a game session comfortably but still expire. Clients
// fetch fresh ones on every page load, so a short TTL costs nothing.
const CF_TURN_TTL_SECONDS = 6 * 60 * 60;

async function meteredServers(env) {
  if (!env.METERED_APP || !env.METERED_KEY) return [];
  const r = await fetch(
    `https://${env.METERED_APP}.metered.live/api/v1/turn/credentials?apiKey=${env.METERED_KEY}`
  );
  if (!r.ok) throw new Error(`Metered.ca returned ${r.status}`);
  const body = await r.json();
  return Array.isArray(body) ? body : [];
}

async function cloudflareServers(env) {
  if (!env.CF_TURN_KEY_ID || !env.CF_TURN_API_TOKEN) return [];
  const r = await fetch(
    `https://rtc.live.cloudflare.com/v1/turn/keys/${env.CF_TURN_KEY_ID}/credentials/generate-ice-servers`,
    {
      method: 'POST',
      headers: {
        'Authorization': `Bearer ${env.CF_TURN_API_TOKEN}`,
        'Content-Type': 'application/json',
      },
      body: JSON.stringify({ ttl: CF_TURN_TTL_SECONDS }),
    }
  );
  if (!r.ok) throw new Error(`Cloudflare TURN returned ${r.status}`);
  const body = await r.json();
  // Documented shape is { iceServers: [...] }; older examples show a single
  // object — normalise both to a flat array.
  const ice = body.iceServers;
  return Array.isArray(ice) ? ice : ice ? [ice] : [];
}

export default {
  async fetch(request, env) {
    // CORS preflight
    if (request.method === 'OPTIONS') {
      return new Response(null, { status: 204, headers: corsHeaders(request, env) });
    }

    if (request.method !== 'GET') {
      return new Response('Method Not Allowed', { status: 405 });
    }

    const results = await Promise.allSettled([
      meteredServers(env),
      cloudflareServers(env),
    ]);
    const servers = results.flatMap(r => (r.status === 'fulfilled' ? r.value : []));
    const errors = results
      .filter(r => r.status === 'rejected')
      .map(r => String(r.reason && r.reason.message || r.reason));

    // Only a total failure is an error status; one working source is a normal
    // response. Failures ride along in a header so `curl -i` shows the story
    // without breaking the JSON body shape the clients parse.
    const status = servers.length > 0 ? 200 : 502;
    return new Response(JSON.stringify(servers), {
      status,
      headers: {
        'Content-Type': 'application/json',
        'Cache-Control': 'no-store', // credentials are time-limited; never cache
        ...(errors.length ? { 'X-Turn-Source-Errors': errors.join('; ') } : {}),
        ...corsHeaders(request, env),
      },
    });
  },
};

function corsHeaders(request, env) {
  const allowed = (env.ALLOWED_ORIGIN || '*').split(',').map(s => s.trim()).filter(Boolean);
  const origin = request.headers.get('Origin');
  // Echo the request origin when it's on the list; otherwise fall back to the
  // first entry so the browser still sees a definite (deny-by-mismatch) value.
  const allow = allowed.includes('*') ? '*'
    : (origin && allowed.includes(origin)) ? origin
    : allowed[0];
  return {
    'Access-Control-Allow-Origin': allow,
    'Access-Control-Allow-Methods': 'GET, OPTIONS',
    // Response varies by request origin, so caches must key on it.
    'Vary': 'Origin',
  };
}
