import { expect, Page } from '@playwright/test';
import { createHmac } from 'node:crypto';

export const PASS = 'testpass';

export async function createVault(page: Page, master = 'e2e-master-pass'): Promise<void> {
  await page.getByTestId('master1').fill(master);
  await page.getByTestId('master2').fill(master);
  await page.getByTestId('create').click();
  await expect(page.getByTestId('server-list')).toBeVisible();
}

export async function addServer(page: Page, o: { name: string; host: string; password: string; user?: string }): Promise<void> {
await page.getByTestId('tab-home').click(); // the session list is only shown on the home tab
  await page.getByTestId('add').click();
  await page.getByTestId('f-name').fill(o.name);
  await page.getByTestId('f-host').fill(o.host);
  await page.getByTestId('f-user').fill(o.user ?? 'tester');
  await page.getByTestId('f-password').fill(o.password);
  await page.getByTestId('form-save').click();
  await expect(page.getByTestId('server-item').filter({ hasText: o.name })).toBeVisible();
}

export async function connectFirst(page: Page): Promise<void> {
  await page.getByTestId('server-item').first().dblclick();
}

export async function acceptHostKey(page: Page): Promise<string> {
  await expect(page.getByTestId('hostkey-dialog')).toBeVisible();
  const fp = (await page.getByTestId('fingerprint').textContent()) ?? '';
  expect(fp).toMatch(/^SHA256:/);
  await page.getByTestId('hostkey-accept').click();
  return fp;
}

function base32(s: string): Buffer {
  const A = 'ABCDEFGHIJKLMNOPQRSTUVWXYZ234567';
  let bits = '';
  for (const c of s) bits += A.indexOf(c).toString(2).padStart(5, '0');
  const out: number[] = [];
  for (let i = 0; i + 8 <= bits.length; i += 8) out.push(parseInt(bits.slice(i, i + 8), 2));
  return Buffer.from(out);
}

/** RFC 6238 code for the 2FA test server. */
export function totp(secret = 'JBSWY3DPEHPK3PXP'): string {
  const counter = Buffer.alloc(8);
  counter.writeBigUInt64BE(BigInt(Math.floor(Date.now() / 1000 / 30)));
  const h = createHmac('sha1', base32(secret)).update(counter).digest();
  const o = h[19] & 0xf;
  const code = ((h[o] & 0x7f) << 24) | (h[o + 1] << 16) | (h[o + 2] << 8) | h[o + 3];
  return String(code % 1_000_000).padStart(6, '0');
}

export const row = (page: Page, name: string) => page.locator(`[data-testid="explorer-row"][data-name="${name.replace(/"/g, '\\"')}"]`);

/** Call the bridge's JSON-RPC directly (a second, independent client of the same core). */
// eslint-disable-next-line @typescript-eslint/no-explicit-any
export async function rpc(bridgeUrl: string, method: string, params: Record<string, unknown> = {}): Promise<any> {
  const r = await fetch(`${bridgeUrl}/rpc`, { method: 'POST', body: JSON.stringify({ method, params }) });
  const j = (await r.json()) as { ok: boolean; result?: unknown; error?: { code: string; message: string } };
  if (!j.ok) throw Object.assign(new Error(j.error!.message), { code: j.error!.code });
  return j.result;
}

/** Read / overwrite a file on the test server through a separate SFTP session (not the UI's). */
export async function serverFile(bridgeUrl: string, path: string, write?: string): Promise<{ text: string; eol: string }> {
  const c = await rpc(bridgeUrl, 'connect', { host: 'sshd-password', port: 22, username: 'tester', password: PASS, attempt: `chk${Math.random()}` });
  try {
    if (write !== undefined) {
      const o = await rpc(bridgeUrl, 'edit_open', { session_id: c.session_id, path });
      await rpc(bridgeUrl, 'edit_save', { session_id: c.session_id, path, text: write, bom: o.bom, eol: o.eol === 'crlf' ? 'crlf' : 'lf', base: o.base, force: true });
    }
    const o = await rpc(bridgeUrl, 'edit_open', { session_id: c.session_id, path });
    return { text: o.text as string, eol: o.eol as string };
  } finally {
    await rpc(bridgeUrl, 'disconnect', { session_id: c.session_id });
  }
}

/** Remove a test work folder on the server (tests must not leave junk: it pushes real folders out of view). */
export async function removeOnServer(bridgeUrl: string, path: string): Promise<void> {
  const c = await rpc(bridgeUrl, 'connect', { host: 'sshd-password', port: 22, username: 'tester', password: PASS, attempt: `rm${Math.random()}` });
  try {
    await rpc(bridgeUrl, 'fs', { session_id: c.session_id, op: 'delete', paths: [path] });
  } catch {
    /* already gone */
  } finally {
    await rpc(bridgeUrl, 'disconnect', { session_id: c.session_id });
  }
}

/** Create a text file on the test server in a given encoding (through a separate session). */
export async function serverWriteEnc(bridgeUrl: string, path: string, text: string, encoding: string): Promise<void> {
  const c = await rpc<{ session_id: string }>(bridgeUrl, 'connect', { host: 'sshd-password', port: 22, username: 'tester', password: PASS, attempt: `enc${Math.random()}` });
  try {
    await rpc(bridgeUrl, 'edit_open', { session_id: c.session_id, path, new: true });
    await rpc(bridgeUrl, 'edit_save', { session_id: c.session_id, path, text, bom: false, eol: 'lf', encoding, base: null });
  } finally {
    await rpc(bridgeUrl, 'disconnect', { session_id: c.session_id });
  }
}

/** Read it back in the given encoding. */
export async function serverReadEnc(bridgeUrl: string, path: string, encoding: string): Promise<string> {
  const c = await rpc<{ session_id: string }>(bridgeUrl, 'connect', { host: 'sshd-password', port: 22, username: 'tester', password: PASS, attempt: `rd${Math.random()}` });
  try {
    return (await rpc(bridgeUrl, 'edit_open', { session_id: c.session_id, path, encoding })).text as string;
  } finally {
    await rpc(bridgeUrl, 'disconnect', { session_id: c.session_id });
  }
}
