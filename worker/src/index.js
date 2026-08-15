// Cloudflare Worker — proxies Metered.ca TURN credential requests so the
// API key never appears in the publicly-visible client source.
//
// Required secrets (set with `wrangler secret put`):
//   METERED_KEY   — your Metered.ca API key
//
// Required vars (set in wrangler.toml [vars]):
//   METERED_APP   — your Metered.ca app subdomain (e.g. "myapp")
//   ALLOWED_ORIGIN — comma-separated list of allowed site origins
//                    (e.g. "https://pp-dev.example.com,https://you.github.io").
//                    The response echoes the request origin when it matches;
//                    a stale single-origin value here CORS-blocks every client
//                    fetch, which silently strips TURN from the ICE list and
//                    breaks all CGNAT/hotspot players (2026-08 field failure).

export default {
  async fetch(request, env) {
    // CORS preflight
    if (request.method === 'OPTIONS') {
      return new Response(null, { status: 204, headers: corsHeaders(request, env) });
    }

    if (request.method !== 'GET') {
      return new Response('Method Not Allowed', { status: 405 });
    }

    let response;
    try {
      response = await fetch(
        `https://${env.METERED_APP}.metered.live/api/v1/turn/credentials?apiKey=${env.METERED_KEY}`
      );
    } catch (e) {
      return new Response('Failed to reach Metered.ca: ' + e.message, { status: 502 });
    }

    const body = await response.text();
    return new Response(body, {
      status: response.status,
      headers: {
        'Content-Type': 'application/json',
        'Cache-Control': 'no-store', // credentials are time-limited; never cache
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
