import { useState } from 'react';
import { Globe, Monitor, Network, Radio, Settings2, Terminal } from 'lucide-react';
import { routes } from './api';
import { Button, Card, CopyButton, Field, SectionTitle, Toggle } from './ui';

const portFields = [
  ['local_port', 'Web panel · TCP', 52402],
  ['port', 'Agent API · TCP', 52401],
  ['discovery_port', 'Discovery · UDP', 52400],
];

function PortFields({ values, change, disabled }) {
  return <div className="grid gap-3 md:grid-cols-3">
    {portFields.map(([key, label]) => <Field key={key} label={label}>
      <input className="input" type="number" inputMode="numeric" min="1" max="65535" required
        value={values[key]} onChange={event => change(key, event.target.value)} disabled={disabled} />
    </Field>)}
  </div>;
}

function portsValid(values) {
  return portFields.every(([key]) => /^\d+$/.test(values[key]) && Number(values[key]) >= 1 && Number(values[key]) <= 65535)
    && Number(values.port) !== Number(values.local_port);
}

export function NetworkSettings({ network, disabled, action, confirm }) {
  const [draft, setDraft] = useState(null);
  if (!network) return null;
  const next = draft || network;
  const change = (key, value) => setDraft({ ...next, [key]: value });
  const valid = portsValid(next);
  const changedPort = portFields.some(([key]) => Number(next[key]) !== network[key]);
  const currentChannelEnabled = location.protocol === 'https:' ? next.lan_enabled : next.local_enabled;

  function save() {
    const body = Object.fromEntries(portFields.map(([key]) => [key, Number(next[key])]));
    body.local_enabled = next.local_enabled;
    body.lan_enabled = next.lan_enabled;
    const run = async () => {
      const result = await action(routes.network, 'PUT', body, 'บันทึกการตั้งค่าเครือข่ายแล้ว');
      if (!result) return;
      if (result.code) {
        const target = new URL('/synaptaconnect/', location.href);
        target.port = body.local_port;
        target.hash = 'code=' + result.code;
        location.assign(target);
      }
      if (!currentChannelEnabled) location.reload();
      setDraft(null);
    };
    if (!currentChannelEnabled || changedPort) {
      confirm({
        title: 'ใช้การตั้งค่าเครือข่ายใหม่',
        text: !currentChannelEnabled
          ? 'ช่องทางที่กำลังใช้อยู่จะถูกปิด หากต้องการเปิด localhost กลับ ให้รัน & "$env:ProgramFiles\\SynaptaConnect\\synapta-connect.exe" panel --enable-local ใน PowerShell บนเครื่องที่ติดตั้ง'
          : 'เครื่องที่กำลังเชื่อมต่ออาจต้องใช้พอร์ตใหม่ และหน้าจัดการจะย้ายไปพอร์ตใหม่เมื่อบันทึกสำเร็จ',
        label: 'บันทึกและใช้ค่าใหม่',
        run,
      });
    } else {
      run();
    }
  }

  return <form className="grid gap-4" onSubmit={event => { event.preventDefault(); save(); }}>
    <Card>
      <SectionTitle icon={Globe} description="กำหนดว่าเบราว์เซอร์เปิดหน้าจัดการจากที่ใดได้บ้าง">ช่องทางเข้า panel</SectionTitle>
      <div className="grid gap-3">
        <Toggle checked={next.local_enabled} disabled={disabled} onChange={event => change('local_enabled', event.target.checked)}
          detail={'http://localhost:' + next.local_port + '/synaptaconnect'}>Localhost · เครื่องนี้</Toggle>
        <Toggle checked={next.lan_enabled} disabled={disabled || (!network.lan_ready && !next.lan_enabled)}
          onChange={event => change('lan_enabled', event.target.checked)}
          detail={network.lan_ready ? 'HTTPS จาก IP หรือชื่อเครื่องที่อยู่ในใบรับรอง' : 'ต้องตั้งใบรับรอง HTTPS ก่อนเปิดช่องทางนี้'}>LAN · เครื่องอื่น</Toggle>
      </div>
      {!!network.lan_hosts?.length && <div className="mt-4 flex flex-wrap gap-2">
        {network.lan_hosts.map(host => <code className="inset break-all rounded-md px-2 py-1 text-sm" key={host}>
          https://{host}:{next.local_port}/synaptaconnect
        </code>)}
      </div>}
    </Card>
    <Card>
      <SectionTitle icon={Radio} description="พอร์ต TCP ของ Web panel และ Agent API ต้องต่างกัน">พอร์ตเครือข่าย</SectionTitle>
      <PortFields values={next} change={change} disabled={disabled} />
      {!valid && <p className="mt-3 text-sm text-danger" role="alert">ใช้พอร์ต 1–65535 และกำหนดพอร์ต TCP ให้ต่างกัน</p>}
      <div className="mt-5 flex flex-wrap gap-2">
        <Button type="submit" variant="primary" disabled={disabled || !draft || !valid}>บันทึกและใช้ค่าใหม่</Button>
        {draft && <Button disabled={disabled} onClick={() => setDraft(null)}>คืนค่าที่ใช้อยู่</Button>}
      </div>
    </Card>
  </form>;
}

export function Settings({ data, update, disabled, action }) {
  const [draft, setDraft] = useState(null);
  const settings = draft || { source: update.source || '', web_source: update.web_source || '', auto_update: Boolean(update.auto_update) };
  const screen = data.screen_input;
  const helper = data.agent?.mode === 'console'
    ? 'ใช้สำหรับ Windows Service'
    : screen?.helper_connected ? 'Helper เชื่อมต่อแล้ว · ' + (screen.helper_user || 'เจ้าของเครื่อง')
      : screen?.enabled ? 'กำลังรอ helper ในเซสชันของเจ้าของเครื่อง' : 'ปิดการจับภาพและส่งอินพุตผ่านบริการ';

  return <div className="grid gap-4">
    <Card>
      <SectionTitle icon={Monitor}>หน้าจอและอินพุต</SectionTitle>
      <Toggle checked={Boolean(screen?.enabled)} disabled={disabled || !screen}
        detail={helper} onChange={event => action(routes.screen, 'PUT', { enabled: event.target.checked })}>
        อนุญาตการจับภาพและส่งอินพุต
      </Toggle>
    </Card>
    <Card>
      <SectionTitle icon={Settings2}>การตั้งค่าอัปเดต</SectionTitle>
      <form className="grid gap-4" onSubmit={async event => {
        event.preventDefault();
        if (await action(routes.updates + '/settings', 'PUT', {
          ...settings, source: settings.source.trim(), web_source: settings.web_source.trim(),
        }, 'บันทึกการตั้งค่าแล้ว')) {
          setDraft(null);
        }
      }}>
        <Toggle checked={settings.auto_update} disabled={disabled} onChange={event => setDraft({ ...settings, auto_update: event.target.checked })}>
          ตรวจและติดตั้งอัปเดตอัตโนมัติ
        </Toggle>
        <Field label="แหล่งอัปเดต Agent">
          <input className="input" value={settings.source} spellCheck="false" autoCapitalize="none" placeholder="github:OWNER/REPO หรือ https://…/"
            onChange={event => setDraft({ ...settings, source: event.target.value })} disabled={disabled} />
        </Field>
        <Field label="แหล่งอัปเดตเว็บ panel">
          <input className="input" value={settings.web_source} spellCheck="false" autoCapitalize="none" placeholder="เว้นว่างเพื่อใช้แหล่งเดียวกับ Agent"
            onChange={event => setDraft({ ...settings, web_source: event.target.value })} disabled={disabled} />
        </Field>
        <div><Button type="submit" variant="primary" disabled={disabled || !draft}>บันทึกการตั้งค่า</Button></div>
      </form>
    </Card>
  </div>;
}

const initialInstall = {
  ...Object.fromEntries(portFields.map(([key, , fallback]) => [key, fallback])),
  source: '', webSource: '', owner: '', cert: '', key: '', auto: true, screen: false, noStart: false, lan: false,
};

function quote(value) {
  return "'" + value.replaceAll("'", "''") + "'";
}

function createCommand(values) {
  const { owner, source, webSource, cert, key } = Object.fromEntries(['owner', 'source', 'webSource', 'cert', 'key'].map(field => [field, values[field].trim()]));
  if (!portsValid(values)) return { error: 'ตรวจสอบหมายเลขพอร์ต' };
  if (owner && !/^[\p{L}\p{N}_. @$\\-]{1,128}$/u.test(owner)) return { error: 'ตรวจสอบชื่อบัญชี Windows' };
  if ([source, webSource].some(value => value && !/^(github:[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+|https:\/\/[^\s'"|<>]+)$/.test(value))) return { error: 'ตรวจสอบรูปแบบแหล่งอัปเดต' };
  if (Boolean(cert) !== Boolean(key)) return { error: 'กรอกพาธใบรับรองและคีย์ให้ครบทั้งคู่' };
  if (values.lan && !cert) return { error: 'ระบุใบรับรองและคีย์เพื่อเปิด LAN' };
  return {
    command: [
      '.\\synapta-connect.exe install',
      '--port ' + values.port, '--discovery-port ' + values.discovery_port, '--local-port ' + values.local_port,
      '--auto-update ' + (values.auto ? 'on' : 'off'),
      source && '--update-source ' + quote(source), owner && '--owner ' + quote(owner),
      webSource && '--web-update-source ' + quote(webSource),
      cert && '--panel-cert ' + quote(cert) + ' --panel-key ' + quote(key),
      values.lan && '--allow-lan-panel',
      values.screen && '--allow-screen-input', values.noStart && '--no-start',
    ].filter(Boolean).join(' '),
  };
}

export function InstallGuide({ notify }) {
  const [values, setValues] = useState(initialInstall);
  const change = (field, value) => setValues(current => ({ ...current, [field]: value }));
  const { command, error } = createCommand(values);
  return <div className="grid gap-4">
    <section className="panel-banner install-banner overflow-hidden rounded-2xl p-5 sm:px-7">
      <div className="relative z-1 max-w-lg sm:max-w-[55%]">
        <h2 className="flex items-center gap-2 text-xl font-semibold sm:text-2xl">
          <Terminal size={20} className="shrink-0" aria-hidden="true" />ติดตั้ง Windows Service
        </h2>
        <p className="mt-1.5 text-sm text-indigo-100">รันคำสั่งจากโฟลเดอร์ที่มี synapta-connect.exe ใน PowerShell แบบ Administrator</p>
      </div>
    </section>
    <Card>
      <div className="grid gap-4">
        <PortFields values={values} change={change} />
        <div className="grid gap-3 md:grid-cols-3">
          {[['owner', 'บัญชีเจ้าของเครื่อง', 'PC\\User'], ['source', 'แหล่งอัปเดต Agent', 'github:OWNER/REPO'],
            ['webSource', 'แหล่งอัปเดตเว็บ', 'ใช้แหล่งเดียวกับ Agent']].map(([key, label, hint]) =>
            <Field label={label} key={key}><input className="input" spellCheck="false" placeholder={hint}
              value={values[key]} onChange={event => change(key, event.target.value)} /></Field>)}
        </div>
        <details className="rounded-xl border border-line p-3">
          <summary className="flex min-h-11 cursor-pointer items-center gap-2 font-semibold"><Network size={17} aria-hidden="true" />ใบรับรอง HTTPS สำหรับ LAN</summary>
          <div className="grid gap-3 md:grid-cols-2">
            {[['cert', 'ไฟล์ใบรับรอง PEM'], ['key', 'ไฟล์คีย์ PEM']].map(([key, label]) => <Field label={label} key={key}>
              <input className="input" spellCheck="false" value={values[key]} onChange={event => change(key, event.target.value)} />
            </Field>)}
          </div>
          <div className="mt-3"><Toggle checked={values.lan} onChange={event => change('lan', event.target.checked)}>
            เปิดช่องทาง HTTPS บน LAN ตอนติดตั้ง
          </Toggle></div>
        </details>
        <div className="grid gap-2 xl:grid-cols-3">
          {[['auto', 'อัปเดตอัตโนมัติ'], ['screen', 'อนุญาตหน้าจอและอินพุต'], ['noStart', 'ติดตั้งโดยยังไม่เริ่ม Service']].map(([key, label]) =>
            <Toggle key={key} checked={values[key]} onChange={event => change(key, event.target.checked)}>{label}</Toggle>)}
        </div>
      </div>
    </Card>
    <Card>
      <div className="flex flex-wrap items-center justify-between gap-2">
        <h2 className="text-xl font-semibold">คำสั่งติดตั้ง</h2>
        <CopyButton text={command} onError={notify} />
      </div>
      {error ? <p role="alert" className="mt-3 text-danger">{error}</p> : <pre className="inset mt-3 whitespace-pre-wrap break-all rounded-xl p-3 text-sm">{command}</pre>}
    </Card>
  </div>;
}
