// Cloudflare Worker for Kiku to Yomu:
// - Persistent Cloud Profile Sync (GET /profile, PUT /profile) stored in R2
// - Private Anime MP3 Streaming with HTTP 206 Range support (GET /media/<series>/<file>)
// Set secret once via: `wrangler secret put KIKU_SYNC_SECRET`

export default {
  async fetch(request, env) {
    const url = new URL(request.url);
    const cors = {
      'Access-Control-Allow-Origin': '*',
      'Access-Control-Allow-Methods': 'GET, PUT, HEAD, OPTIONS',
      'Access-Control-Allow-Headers': 'Authorization, Content-Type, Range',
      'Access-Control-Expose-Headers': 'Content-Length, Content-Range, Accept-Ranges',
    };
    if (request.method === 'OPTIONS') return new Response(null, { status: 204, headers: cors });

    // Bearer secret check
    const auth = request.headers.get('Authorization') || '';
    const token = auth.startsWith('Bearer ') ? auth.slice(7).trim() : (url.searchParams.get('k') || '');
    if (!env.KIKU_SYNC_SECRET || token !== env.KIKU_SYNC_SECRET) {
      return new Response(JSON.stringify({ error: 'Unauthorized' }), {
        status: 401,
        headers: { ...cors, 'Content-Type': 'application/json' },
      });
    }

    // 1. Persistent Cloud Profile (known words, listening history, progress, playlists)
    if (url.pathname === '/profile') {
      if (request.method === 'GET') {
        const obj = await env.KIKU_BUCKET.get('profile/profile.json');
        if (!obj) return new Response('null', { status: 404, headers: { ...cors, 'Content-Type': 'application/json' } });
        return new Response(obj.body, {
          status: 200,
          headers: { ...cors, 'Content-Type': 'application/json', 'Cache-Control': 'no-store' },
        });
      }
      if (request.method === 'PUT') {
        const body = await request.text();
        await env.KIKU_BUCKET.put('profile/profile.json', body, {
          httpMetadata: { contentType: 'application/json' },
        });
        return new Response(JSON.stringify({ ok: true }), {
          status: 200,
          headers: { ...cors, 'Content-Type': 'application/json' },
        });
      }
    }

    // 2. Remote Media Streaming (HTTP 206 Range support for anime MP3s)
    if (url.pathname === '/manifest.json' || url.pathname.startsWith('/media/')) {
      const key = decodeURIComponent(url.pathname.slice(1));
      const rangeHeader = request.headers.get('Range');
      const obj = await env.KIKU_BUCKET.get(key, rangeHeader ? { range: request.headers } : undefined);
      if (!obj) return new Response('Not found', { status: 404, headers: cors });

      const headers = new Headers(cors);
      obj.writeHttpMetadata(headers);
      headers.set('Accept-Ranges', 'bytes');
      if (obj.range && rangeHeader) {
        const start = obj.range.offset ?? 0;
        const length = obj.range.length ?? (obj.size - start);
        const end = start + length - 1;
        headers.set('Content-Range', `bytes ${start}-${end}/${obj.size}`);
        headers.set('Content-Length', String(length));
        return new Response(obj.body, { status: 206, headers });
      }
      headers.set('Content-Length', String(obj.size));
      return new Response(obj.body, { status: 200, headers });
    }

    return new Response('Not found', { status: 404, headers: cors });
  },
};
