'use strict';
const $ = selector => document.querySelector(selector);
const themeKey = 'synapta-connect-theme';
const themeToggle = $('#themeToggle');
const themeRoot = document.documentElement;
let initialTheme = matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light';
try {
  const savedTheme = localStorage.getItem(themeKey);
  if (savedTheme === 'dark' || savedTheme === 'light') initialTheme = savedTheme;
} catch {
  // The switch works for this page when persistent storage is unavailable.
}
function setTheme(theme) {
  themeRoot.dataset.theme = theme;
  themeToggle.setAttribute('aria-checked', String(theme === 'dark'));
}
setTheme(initialTheme);
themeToggle.onclick = () => {
  setTheme(themeRoot.dataset.theme === 'dark' ? 'light' : 'dark');
  try {
    localStorage.setItem(themeKey, themeRoot.dataset.theme);
  } catch {
    // Keep the theme selected for this page.
  }
};

const code = new URLSearchParams(location.hash.slice(1)).get('code');
history.replaceState(null, '', location.pathname + location.search);
let token = '', status = null, update = null, busy = false, sourceDirty = false;
let renderedDevices;
let toastTimer;
const states = {idle:'ยังไม่เคยตรวจ',checking:'กำลังตรวจ',downloading:'กำลังดาวน์โหลด',verifying:'กำลังตรวจสอบ',update_available:'พบรุ่นใหม่ พร้อมติดตั้ง',applying:'กำลังเปลี่ยนรุ่นและเริ่ม service ใหม่',up_to_date:'เป็นรุ่นล่าสุด',failed:'อัปเดตไม่สำเร็จ',rolled_back:'ย้อนกลับไปใช้รุ่นเดิมแล้ว',disabled:'ปิดอัปเดตอัตโนมัติ',not_configured:'ยังไม่ได้ตั้งค่าแหล่งอัปเดตหรือกุญแจผู้เผยแพร่'};
const advice = {source_unreachable:'ตรวจอินเทอร์เน็ต แล้วกดตรวจใหม่',manifest_invalid:'รอผู้เผยแพร่แก้ไข release',signature_invalid:'แจ้งผู้เผยแพร่ อย่าลองติดตั้งซ้ำ',version_not_newer:'รุ่นที่เสนอไม่ใหม่กว่ารุ่นปัจจุบัน',download_failed:'ตรวจการเชื่อมต่อแล้วลองใหม่',hash_mismatch:'ลองดาวน์โหลดใหม่',updater_unavailable:'ติดตั้งโปรแกรมใหม่ด้วยสิทธิ์ผู้ดูแล',apply_failed:'ตรวจสอบไฟล์ที่ถูกล็อกหรือพื้นที่ว่าง แล้วลองใหม่',service_start_failed:'ระบบกู้รุ่นเดิมแล้ว สามารถลองใหม่',health_check_failed:'ระบบกู้รุ่นเดิมแล้ว สามารถลองใหม่',interrupted_rolled_back:'ระบบกู้รุ่นเดิมแล้ว สามารถลองใหม่',rollback_failed:'กู้คืนไม่สำเร็จ ต้องแก้ไขด้วยมือ โปรดดูตำแหน่งไฟล์สำรองในข้อความข้อผิดพลาด',retry_limit_reached:'ลองครบจำนวนแล้ว ยืนยันก่อนลองอีกครั้ง'};
function message(text, online = false) {
  const notice = $('#connection');
  notice.hidden = online;
  if (notice.textContent !== text) notice.textContent = text;
  const state = $('#linkState');
  state.dataset.online = String(online);
  state.textContent = online ? 'เชื่อมต่อกับ Agent แล้ว' : 'ขาดการเชื่อมต่อกับ Agent';
}
function toast(text) {
  const notice = $('#toast');
  notice.textContent = text;
  notice.classList.add('show');
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => notice.classList.remove('show'), 3000);
}
function expired() { const restarting = update?.state === 'applying'; token = ''; $('#panelContent').hidden = true; message(restarting ? 'service เริ่มใหม่ กรุณาเปิด panel ใหม่ด้วยคำสั่ง synapta-connect.exe panel เพื่อตรวจผลการอัปเดต' : 'เซสชันหมดอายุหรือรหัสไม่ถูกต้อง กรุณาเปิด panel ใหม่ด้วยคำสั่ง synapta-connect.exe panel'); }
async function request(path, method = 'GET', body) {
  const options = {method, cache:'no-store', headers:{'X-Synapta-Local':token}};
  if (body !== undefined) { options.headers['Content-Type'] = 'application/json'; options.body = JSON.stringify(body); }
  const response = await fetch(path, options);
  if (response.status === 403) { expired(); throw new Error('เซสชันหมดอายุ'); }
  const data = await response.json().catch(() => ({}));
  if (!response.ok) throw new Error(data.error?.message || (response.status === 429 ? 'เรียกบ่อยเกินไป กรุณารอสักครู่' : 'คำขอไม่สำเร็จ (' + response.status + ')'));
  return data;
}
function date(value) { if (!value) return '—'; const d = new Date(typeof value === 'number' ? value * 1000 : value); return isNaN(d) ? '—' : d.toLocaleString('th-TH'); }
function renderDevices(devices) {
  const snapshot = JSON.stringify(devices);
  if (snapshot === renderedDevices) return;
  renderedDevices = snapshot;

  const list = $('#devices');
  const count = $('#deviceCount');
  count.hidden = false;
  count.textContent = devices.length;
  list.replaceChildren();

  for (const device of devices) {
    const row = document.createElement('li');
    row.className = 'session';
    const icon = document.createElement('span');
    icon.className = 'device-icon';
    icon.setAttribute('aria-hidden', 'true');

    const label = document.createElement('div');
    const name = document.createElement('strong');
    const deviceName = device.client_name || device.id;
    name.textContent = deviceName;
    name.title = device.id;
    const seen = document.createElement('small');
    seen.textContent = device.last_seen ? 'เชื่อมต่อล่าสุด ' + date(device.last_seen) : 'ยังไม่มีประวัติการเชื่อมต่อ';
    label.append(name, seen);

    const button = document.createElement('button');
    button.type = 'button';
    button.className = 'revoke';
    button.textContent = 'ยกเลิกสิทธิ์';
    button.setAttribute('aria-label', 'ยกเลิกสิทธิ์ ' + deviceName);
    button.onclick = () => {
      if (confirm('ยกเลิกสิทธิ์ ' + deviceName + '? อุปกรณ์นี้ต้องจับคู่ใหม่เพื่อเชื่อมต่อ')) {
        action('/api/v1/local/devices/' + encodeURIComponent(device.id) + '/revoke');
      }
    };
    row.append(icon, label, button);
    list.append(row);
  }
  if (!devices.length) {
    const empty = document.createElement('li');
    empty.className = 'device-empty';
    empty.textContent = 'ยังไม่มีอุปกรณ์ที่จับคู่กับเครื่องนี้';
    list.append(empty);
  }
}
function renderStatus(data) {
  status = data;
  $('#deviceName').textContent = data.name || 'เครื่องนี้';
  const mode = { console: 'โหมดคอนโซล', service: 'Windows Service' }[data.agent?.mode];
  const privilege = { standard: 'สิทธิ์มาตรฐาน', administrator: 'สิทธิ์ผู้ดูแล', system: 'สิทธิ์ระบบ' }[data.agent?.privilege];
  $('#runtime').textContent = [mode, privilege || data.agent?.privilege].filter(Boolean).join(' · ') || 'ไม่มีข้อมูลโหมดการทำงาน';
  $('#account').textContent = data.agent?.account || '—';
  $('#address').textContent = (data.addresses || []).map(a => a.name + ': ' + a.ip).join(' · ') || 'ไม่มีข้อมูลเครือข่าย';
  const screen = data.screen_input;
  $('#screenInput').checked = !!screen?.enabled;
  $('#screenInput').disabled = !screen;
  let helper = 'ไม่มีข้อมูล helper';
  if (data.agent?.mode === 'console') {
    helper = 'สวิตช์นี้ใช้กับ Windows Service';
  } else if (screen) {
    helper = !screen.enabled ? 'การจับภาพและส่งอินพุตผ่านบริการปิดอยู่' :
      screen.helper_connected ? 'Helper เชื่อมต่อแล้ว · ' + (screen.helper_user || 'ผู้ใช้ที่อนุญาต') :
      'เปิดสิทธิ์แล้ว กำลังรอ helper ในเซสชันของเจ้าของ';
  }
  $('#helper').textContent = helper;
  $('#otp').textContent = data.otp || '········';
  const remaining = Math.max(0, Math.ceil((data.otp_expires_at || 0) - Date.now()/1000));
  $('#pairingStatus').textContent = data.pairing_mode ? 'โหมดจับคู่เปิดอยู่ · เหลือ ' + remaining + ' วินาที' : 'ปิดโหมดจับคู่';
  const report = data.pairing;
  const failures = (report?.recent_failures || []).map(f => [date(f.at), f.ip, f.client_name].filter(Boolean).join(' · '));
  $('#pairingWarning').textContent = report?.locked_for_secs ? 'การจับคู่ถูกล็อกอีก ' + report.locked_for_secs + ' วินาที หลังใส่รหัสผิด กรุณาตรวจสอบรายการล่าสุด: ' + (failures.join(' | ') || 'ไม่มีรายละเอียด') : failures.length ? 'ความพยายามจับคู่ที่ไม่สำเร็จล่าสุด: ' + failures.join(' | ') : '';
  $('#pairingToggle').textContent = data.pairing_mode ? 'ปิดโหมดจับคู่' : 'เปิดโหมดจับคู่';
  $('#regen').disabled = !data.pairing_mode;
  renderDevices(data.devices || []);
}
function renderUpdate(data) {
  update = data;
  $('#version').textContent = data.current_version || '—';
  $('#updateState').textContent = states[data.state] || data.state || 'ไม่ทราบสถานะ';
  $('#available').textContent = data.available?.version || 'ไม่มีข้อมูล';
  $('#lastCheck').textContent = date(data.last_check_at);
  if (!sourceDirty) { $('#autoUpdate').checked = !!data.auto_update; $('#source').value = data.source || ''; }
  $('#checkUpdate').disabled = busy || ['checking','downloading','verifying','applying'].includes(data.state);
  $('#applyUpdate').disabled = busy || !data.available?.version || ['checking','downloading','verifying','applying'].includes(data.state);
  $('#retryUpdate').disabled = busy || !data.can_retry;
  const error = $('#updateError'); error.hidden = !data.last_error;
  error.classList.toggle('critical', data.last_error?.code === 'rollback_failed');
  if (data.last_error) {
    const recovery = data.last_error.code === 'apply_failed' && data.state === 'rolled_back' ? 'ระบบกู้รุ่นเดิมแล้ว' : '';
    error.textContent = [data.last_error.message, 'รหัส: ' + data.last_error.code, recovery, advice[data.last_error.code] || 'ตรวจสอบรายละเอียดแล้วติดต่อผู้ดูแล'].filter(Boolean).join(' · ');
  }
  const history = $('#history'); history.replaceChildren();
  for (const entry of data.history || []) { const p = document.createElement('p'); p.textContent = [date(entry.at), (entry.from || '—') + ' → ' + (entry.to || '—'), entry.outcome, entry.stage, entry.message || entry.code].filter(Boolean).join(' · '); history.append(p); }
  if (!history.children.length) history.textContent = 'ยังไม่มีประวัติ';
}
async function refresh() {
  if (!token || busy) return;
  try {
    const [s,u] = await Promise.all([request('/api/v1/local/status'), request('/api/v1/local/update')]);
    renderStatus(s); renderUpdate(u); $('#panelContent').hidden = false; message('เชื่อมต่อกับ Agent บนเครื่องนี้แล้ว', true);
  } catch (e) { if (token) message(update?.state === 'applying' ? 'กำลังรอ service เริ่มใหม่ ระบบจะตรวจอีกครั้งอัตโนมัติ' : 'เชื่อมต่อ Agent ไม่ได้ กำลังลองใหม่อัตโนมัติ'); }
}
async function action(path, method = 'POST', body) {
  if (!token || busy) return;
  busy = true;
  try { const result = await request(path,method,body); toast(result.revoked === false ? 'ไม่พบอุปกรณ์ที่ต้องการยกเลิกสิทธิ์' : 'ดำเนินการแล้ว'); }
  catch (e) { if (token) toast(e.message || 'เชื่อมต่อไม่ได้ กรุณาลองใหม่'); }
  finally { busy = false; refresh(); }
}
$('#pairingToggle').onclick = () => action('/api/v1/local/pairing/' + (status?.pairing_mode ? 'close' : 'open'));
$('#regen').onclick = () => action('/api/v1/local/pairing/open');
$('#killAll').onclick = () => { if (confirm('ตัดทุกการเชื่อมต่อที่เปิดอยู่? สิทธิ์การจับคู่ของอุปกรณ์จะยังอยู่')) action('/api/v1/local/kill-all'); };
$('#screenInput').onchange = e => action('/api/v1/local/screen-input','PUT',{enabled:e.target.checked});
$('#checkUpdate').onclick = () => action('/api/v1/local/update/check');
$('#applyUpdate').onclick = () => { if (update?.available?.version && confirm('ติดตั้งรุ่น ' + update.available.version + '? service จะเริ่มใหม่')) action('/api/v1/local/update/apply','POST',{version:update.available.version}); };
$('#retryUpdate').onclick = () => { if (confirm('ลองอัปเดตอีกครั้ง?')) action('/api/v1/local/update/retry'); };
$('#source').oninput = () => { sourceDirty = true; };
$('#autoUpdate').onchange = () => { sourceDirty = true; };
$('#settingsForm').onsubmit = async e => {
  e.preventDefault();
  const source = $('#source').value.trim();
  if (!token || busy) return;
  busy = true;
  try { await request('/api/v1/local/update/settings','PUT',{auto_update:$('#autoUpdate').checked,source}); sourceDirty = false; toast('บันทึกการตั้งค่าแล้ว'); }
  catch (error) { if (token) toast(error.message || 'บันทึกไม่สำเร็จ'); }
  finally { busy = false; refresh(); }
};
const navigation = document.querySelectorAll('.nav');
const pages = document.querySelectorAll('.page');
for (const button of navigation) {
  button.onclick = () => {
    for (const tab of navigation) {
      tab.classList.toggle('active', tab === button);
      if (tab === button) tab.setAttribute('aria-current', 'page');
      else tab.removeAttribute('aria-current');
    }
    for (const page of pages) page.classList.toggle('active', page.id === button.dataset.page);
  };
}
function installCommand() {
  const source = $('#installSource').value.trim(), owner = $('#owner').value.trim();
  if ((source && !/^(github:[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+|https:\/\/[^\s'";|&`$<>]+\/)$/.test(source)) || (owner && !/^[\p{L}\p{N}_. @\\-]+$/u.test(owner))) { $('#installError').textContent = 'ตรวจสอบแหล่งอัปเดตและชื่อผู้ใช้ ห้ามใช้อักขระคำสั่งพิเศษ'; $('#installCommand').textContent = ''; return ''; }
  $('#installError').textContent = '';
  const command = ".\\synapta-connect.exe install --auto-update " + ($('#installAuto').checked ? 'on' : 'off') + (source ? " --update-source '" + source + "'" : "") + (owner ? " --owner '" + owner + "'" : '') + ($('#installScreen').checked ? ' --allow-screen-input' : '') + ($('#noStart').checked ? ' --no-start' : '');
  $('#installCommand').textContent = command; return command;
}
$('#installForm').oninput = installCommand;
$('#installForm').onsubmit = async e => { e.preventDefault(); const command = installCommand(); if (!command) return; try { await navigator.clipboard.writeText(command); toast('คัดลอกคำสั่งแล้ว'); } catch { toast('คัดลอกไม่ได้ กรุณาเลือกข้อความคำสั่งแล้วคัดลอก'); } };
installCommand();
(async () => {
  if (!code || !['127.0.0.1','localhost'].includes(location.hostname) || location.protocol !== 'http:') { expired(); return; }
  try {
    const response = await fetch('/api/v1/panel/exchange',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({code}),cache:'no-store'});
    if (!response.ok) { expired(); return; }
    const data = await response.json(); token = data.token;
    if (!token) { expired(); return; }
    await refresh(); setInterval(refresh, 2000);
  } catch { message('เชื่อมต่อเพื่อยืนยันตัวตนไม่ได้ กรุณาเปิด panel ใหม่ด้วยคำสั่ง synapta-connect.exe panel'); }
})();