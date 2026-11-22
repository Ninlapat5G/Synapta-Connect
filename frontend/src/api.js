import { invoke, isTauri } from '@tauri-apps/api/core';

const LOCAL = '/api/v1/local';
export const panelCommand = String.raw`& "C:\Program Files\SynaptaConnect\synapta-connect.exe" panel`;
export const desktop = isTauri();
export const serviceStatus = () => invoke('service_status');
export const serviceControl = action => invoke('service_control', { action });
export const openWeb = () => invoke('open_web');

export const routes = {
  status: LOCAL + '/status',
  pairing: LOCAL + '/pairing/',
  devices: LOCAL + '/devices/',
  disconnect: LOCAL + '/kill-all',
  updates: LOCAL + '/update',
  webUpdates: LOCAL + '/web-update',
  network: LOCAL + '/network',
  reload: LOCAL + '/panel-reload',
  screen: LOCAL + '/screen-input',
  restartWeb: LOCAL + '/restart-web',
  recoverWeb: LOCAL + '/recover-web',
  exchange: '/api/v1/panel/exchange',
};

export class RequestError extends Error {
  constructor(status, message) {
    super(message);
    this.status = status;
  }
}

export async function request(path, { method = 'GET', body, token, signal } = {}) {
  if (desktop) {
    let reply;
    try {
      reply = await invoke('agent_request', { path, method, body });
    } catch (error) {
      throw new RequestError(0, String(error));
    }
    if (reply.status >= 400) {
      throw new RequestError(reply.status, reply.data?.error?.message || 'Agent ปฏิเสธคำขอ (' + reply.status + ')');
    }
    return reply.data ?? {};
  }

  const headers = {};
  if (token) headers['X-Synapta-Local'] = token;
  if (body !== undefined) headers['Content-Type'] = 'application/json';

  const controller = new AbortController();
  const abort = () => controller.abort();
  const timer = setTimeout(abort, 8000);
  signal?.addEventListener('abort', abort, { once: true });
  if (signal?.aborted) abort();
  try {
    const response = await fetch(path, {
      method,
      headers,
      signal: controller.signal,
      body: body === undefined ? undefined : JSON.stringify(body),
      cache: 'no-store',
      redirect: 'error',
    });
    const data = await response.json().catch(() => null);
    if (!response.ok || !data) {
      throw new RequestError(response.status, data?.error?.message ??
        (response.status === 429 ? 'เรียกบ่อยเกินไป กรุณารอสักครู่' : 'คำขอไม่สำเร็จ (' + response.status + ')'));
    }
    return data;
  } finally {
    clearTimeout(timer);
    signal?.removeEventListener('abort', abort);
  }
}

export function formatDate(value) {
  if (!value) return '—';
  const time = new Date(typeof value === 'number' ? value * 1000 : value);
  return Number.isNaN(time.valueOf()) ? '—' : time.toLocaleString('th-TH');
}

export function readOpeningCode() {
  const code = new URLSearchParams(location.hash.slice(1)).get('code');
  if (location.hash) history.replaceState(null, '', location.pathname + location.search);
  return code;
}
