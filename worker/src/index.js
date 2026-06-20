// Cloudflare Worker — proxies Metered.ca TURN credential requests so the
// API key never appears in the publicly-visible client source.
//
// Required secrets (set with `wrangler secret put`):
//   METERED_KEY   — your Metered.ca API key
//
// Required vars (set in wrangler.toml [vars]):
//   METERED_APP   — your Metered.ca app subdomain (e.g. "myapp")
//   ALLOWED_ORIGIN — your deployed site origin (e.g. "https://you.github.io")

export default {
  async fetch(request, env) {
    // CORS preflight
    if (request.method === 'OPTIONS') {
      return new Response(null, { status: 204, headers: corsHeaders(env) });
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
        ...corsHeaders(env),
      },
    });
  },
};

function corsHeaders(env) {
  return {
    'Access-Control-Allow-Origin': env.ALLOWED_ORIGIN || '*',
    'Access-Control-Allow-Methods': 'GET, OPTIONS',
  };
}
