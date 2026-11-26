// SPDX-License-Identifier: GPL-3.0-or-later
// Cloudflare Worker for anonymized fefw-recomp crash & performance reports.
// Stores zstd-compressed report bundles in R2 and queryable metadata in D1.
// Client IP addresses are never logged or stored.

const MAX_BODY_BYTES = 1_572_864; // 1.5 MiB compressed
const ALLOWED_TRIGGERS = new Set(['crash', 'freeze', 'fatal', 'exit_code', 'hitch', 'low_fps']);

function json(data, status = 200) {
  return new Response(JSON.stringify(data), {
    status,
    headers: { 'content-type': 'application/json; charset=utf-8' },
  });
}

function cleanStr(val, maxLen = 128) {
  if (typeof val !== 'string') return '';
  return val.replace(/[\r\n\0]/g, ' ').trim().slice(0, maxLen);
}

function cleanNum(val) {
  return typeof val === 'number' && Number.isFinite(val) ? val : null;
}

function cleanInt(val) {
  return typeof val === 'number' && Number.isInteger(val) ? val : null;
}

function safePathSegment(val, fallback = 'unknown') {
  const s = cleanStr(val, 64).replace(/[^A-Za-z0-9._-]/g, '_');
  return s || fallback;
}

function decodeMetaHeader(raw) {
  if (!raw || raw.length > 8192) return null;
  try {
    const bin = atob(raw.trim());
    const bytes = new Uint8Array(bin.length);
    for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
    const text = new TextDecoder().decode(bytes);
    const parsed = JSON.parse(text);
    return parsed && typeof parsed === 'object' ? parsed : null;
  } catch {
    return null;
  }
}

function timingSafeEqual(a, b) {
  if (typeof a !== 'string' || typeof b !== 'string') return false;
  const enc = new TextEncoder();
  const ab = enc.encode(a);
  const bb = enc.encode(b);
  if (ab.length !== bb.length) return false;
  let diff = 0;
  for (let i = 0; i < ab.length; i++) diff |= ab[i] ^ bb[i];
  return diff === 0;
}

function isAdmin(request, env) {
  const secret = env.ADMIN_TOKEN;
  if (!secret || typeof secret !== 'string' || secret.length < 16) return false;
  const auth = request.headers.get('authorization') || '';
  if (!auth.startsWith('Bearer ')) return false;
  return timingSafeEqual(auth.slice(7), secret);
}

async function handleSubmit(request, env) {
  const contentLen = Number(request.headers.get('content-length') || '0');
  if (contentLen > MAX_BODY_BYTES) {
    return json({ error: 'payload too large' }, 413);
  }

  const meta = decodeMetaHeader(request.headers.get('x-report-meta'));
  if (!meta) {
    return json({ error: 'invalid or missing X-Report-Meta header' }, 400);
  }

  const trigger = cleanStr(meta.trigger, 32);
  if (!ALLOWED_TRIGGERS.has(trigger)) {
    return json({ error: 'invalid trigger' }, 400);
  }

  const body = await request.arrayBuffer();
  if (body.byteLength === 0 || body.byteLength > MAX_BODY_BYTES) {
    return json({ error: 'invalid payload size' }, body.byteLength === 0 ? 400 : 413);
  }

  // Zstd magic number: 28 B5 2F FD
  const head = new Uint8Array(body, 0, Math.min(4, body.byteLength));
  if (head.length < 4 || head[0] !== 0x28 || head[1] !== 0xb5 || head[2] !== 0x2f || head[3] !== 0xfd) {
    return json({ error: 'expected zstd-compressed body' }, 400);
  }

  const id = crypto.randomUUID();
  const createdAt = new Date().toISOString();
  const datePrefix = createdAt.slice(0, 10);
  const buildId = cleanStr(meta.build_id, 64) || 'unknown';
  const launcherVersion = cleanStr(meta.launcher_version, 32) || 'unknown';
  const os = cleanStr(meta.os, 32) || 'unknown';
  const gpu = cleanStr(meta.gpu, 128) || 'unknown';
  const exitCode = cleanInt(meta.exit_code);
  const meanFps = cleanNum(meta.mean_fps);
  const low1Fps = cleanNum(meta.low1_fps);
  const worstMs = cleanNum(meta.worst_ms);
  const hitchCount = cleanInt(meta.hitch_count);
  const crashCause = cleanStr(meta.crash_cause, 256) || null;

  const r2Key = `${datePrefix}/${trigger}/${safePathSegment(buildId)}/${id}.zst`;

  await env.REPORTS_BUCKET.put(r2Key, body, {
    httpMetadata: { contentType: 'application/zstd' },
    customMetadata: { id, trigger, build_id: buildId, os },
  });

  await env.REPORTS_DB.prepare(
    `INSERT INTO reports (
       id, created_at, trigger, build_id, launcher_version, os, gpu,
       exit_code, mean_fps, low1_fps, worst_ms, hitch_count, crash_cause,
       r2_key, size_bytes
     ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`
  )
    .bind(
      id,
      createdAt,
      trigger,
      buildId,
      launcherVersion,
      os,
      gpu,
      exitCode,
      meanFps,
      low1Fps,
      worstMs,
      hitchCount,
      crashCause,
      r2Key,
      body.byteLength
    )
    .run();

  return json({ ok: true, id }, 202);
}

async function handleList(request, env) {
  const url = new URL(request.url);
  const clauses = [];
  const binds = [];

  const trigger = url.searchParams.get('trigger');
  if (trigger) {
    clauses.push('trigger = ?');
    binds.push(trigger);
  }
  const buildId = url.searchParams.get('build_id');
  if (buildId) {
    clauses.push('build_id LIKE ?');
    binds.push(`%${buildId}%`);
  }
  const gpu = url.searchParams.get('gpu');
  if (gpu) {
    clauses.push('gpu LIKE ?');
    binds.push(`%${gpu}%`);
  }
  const os = url.searchParams.get('os');
  if (os) {
    clauses.push('os = ?');
    binds.push(os);
  }
  const since = url.searchParams.get('since');
  if (since) {
    clauses.push('created_at >= ?');
    binds.push(since);
  }

  const limit = Math.min(Math.max(parseInt(url.searchParams.get('limit') || '50', 10) || 50, 1), 200);
  const offset = Math.max(parseInt(url.searchParams.get('offset') || '0', 10) || 0, 0);
  const where = clauses.length ? `WHERE ${clauses.join(' AND ')}` : '';
  const sql = `SELECT * FROM reports ${where} ORDER BY created_at DESC LIMIT ? OFFSET ?`;
  binds.push(limit, offset);

  const res = await env.REPORTS_DB.prepare(sql).bind(...binds).all();
  return json({ reports: res.results || [] });
}

async function handleGetOne(id, env) {
  const row = await env.REPORTS_DB.prepare('SELECT r2_key FROM reports WHERE id = ?').bind(id).first();
  if (!row) return json({ error: 'not found' }, 404);
  const obj = await env.REPORTS_BUCKET.get(row.r2_key);
  if (!obj) return json({ error: 'blob not found' }, 404);
  return new Response(obj.body, {
    status: 200,
    headers: {
      'content-type': 'application/zstd',
      'content-disposition': `attachment; filename="${id}.json.zst"`,
    },
  });
}

async function handleDeleteOne(id, env) {
  const row = await env.REPORTS_DB.prepare('SELECT r2_key FROM reports WHERE id = ?').bind(id).first();
  if (!row) return json({ error: 'not found' }, 404);
  await env.REPORTS_BUCKET.delete(row.r2_key);
  await env.REPORTS_DB.prepare('DELETE FROM reports WHERE id = ?').bind(id).run();
  return json({ ok: true, id });
}

export default {
  async fetch(request, env) {
    const url = new URL(request.url);
    const path = url.pathname;

    if (request.method === 'GET' && path === '/health') {
      return json({ ok: true });
    }

    if (request.method === 'POST' && path === '/v1/report') {
      return handleSubmit(request, env);
    }

    if (path === '/v1/reports' && request.method === 'GET') {
      if (!isAdmin(request, env)) return json({ error: 'unauthorized' }, 401);
      return handleList(request, env);
    }

    const match = path.match(/^\/v1\/reports\/([0-9a-f-]{36})$/);
    if (match) {
      if (!isAdmin(request, env)) return json({ error: 'unauthorized' }, 401);
      if (request.method === 'GET') return handleGetOne(match[1], env);
      if (request.method === 'DELETE') return handleDeleteOne(match[1], env);
    }

    return json({ error: 'not found' }, 404);
  },
};
