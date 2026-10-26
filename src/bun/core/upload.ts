// Uploads to a soju.im/FILEHOST endpoint, authenticated with the same
// credentials as the IRC session.

import type { ServerConfig } from './config';

export async function upload(server: ServerConfig, url: string, filename: string, contentType: string, body: Uint8Array): Promise<string> {
  let user: string;
  let pass: string;
  if (server.saslPassword) {
    user = server.saslUsername || server.nick;
    pass = server.saslPassword;
  } else {
    user = server.username;
    pass = server.password;
  }
  // Header values travel as Latin-1, so UTF-8 names are passed as their bytes.
  const safeName = Buffer.from([...filename].filter((c) => !/[\x00-\x1f\x7f-\x9f"]/.test(c)).join(''), 'utf8').toString('latin1');
  const headers: Record<string, string> = {
    'Content-Type': contentType || 'application/octet-stream',
    'Content-Disposition': `attachment; filename="${safeName}"`,
  };
  if (pass) headers.Authorization = `Basic ${Buffer.from(`${user}:${pass}`).toString('base64')}`;

  let resp: Response;
  try {
    resp = await fetch(url, {
      method: 'POST',
      headers,
      body,
      // A redirect would resend the credentials to wherever it points.
      redirect: 'manual',
      tls: { rejectUnauthorized: !server.acceptInvalidCerts },
    } as RequestInit);
  } catch (e) {
    throw new Error(`upload failed: ${e instanceof Error ? e.message : e}`);
  }
  if (!resp.ok) {
    const text = await resp.text().catch(() => '');
    const status = resp.statusText ? `${resp.status} ${resp.statusText}` : String(resp.status);
    throw new Error(`upload rejected (${status}): ${text.trim()}`);
  }
  const location = resp.headers.get('Location');
  if (!location) throw new Error('server did not return a file URL');
  return new URL(location, url).toString();
}

export function guessType(name: string): string {
  const ext = (name.split('.').pop() ?? '').toLowerCase();
  switch (ext) {
    case 'png':
      return 'image/png';
    case 'jpg':
    case 'jpeg':
      return 'image/jpeg';
    case 'gif':
      return 'image/gif';
    case 'webp':
      return 'image/webp';
    case 'svg':
      return 'image/svg+xml';
    case 'txt':
    case 'log':
      return 'text/plain; charset=utf-8';
    case 'pdf':
      return 'application/pdf';
    case 'mp4':
      return 'video/mp4';
    case 'webm':
      return 'video/webm';
    case 'mp3':
      return 'audio/mpeg';
    case 'json':
      return 'application/json';
    default:
      return 'application/octet-stream';
  }
}
