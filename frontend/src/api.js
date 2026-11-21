const LOCAL = '/api/v1/local';

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
  exchange: '/api/v1/panel/exchange',
};

export class RequestError extends Error {
  constructor(status, message) {
    super(message);
    this.status = status;
  }
}

export async function request(path, { method = 'GET', body, token, signal } = {}) {
  const headers = {};
  if (token) headers['X-Synapta-Local'] = token;
  if (body !== undefined) headers['Content-Type'] = 'application/json';

  const response = await fetch(path, {
    method,
    headers,
    signal,
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
