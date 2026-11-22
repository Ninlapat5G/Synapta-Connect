import { useCallback, useEffect, useRef, useState } from 'react';
import { createRoot } from 'react-dom/client';
import { Activity, ArrowRight, Box, CircleHelp, Download, ExternalLink, Globe, LayoutDashboard, Monitor, Moon, Network, Play, Radio, RefreshCw, Server, Settings2, ShieldCheck, Square, Sun, Undo2 } from 'lucide-react';
import { LazyMotion, domAnimation, useReducedMotion } from 'motion/react';
import * as m from 'motion/react-m';
import logo from '../assets/synapta-connect.png';
import { desktop, panelCommand, readOpeningCode, routes } from './api';
import { Devices, NetworkDetails, Pairing } from './devices';
import { InstallGuide, NetworkSettings, Settings } from './settings';
import { useSession } from './session';
import { Button, Card, Confirm, CopyButton, SectionTitle, Status, Toast } from './ui';
import { Updates } from './updates';
import './style.css';

const pages = [
  ['overview', 'ภาพรวม', LayoutDashboard],
  ['devices', 'อุปกรณ์', Monitor],
  ['network', 'เครือข่าย', Network],
  ['updates', 'อัปเดต', Download],
  ['settings', 'ตั้งค่า', Settings2],
  ['install', 'คู่มือติดตั้ง', CircleHelp],
];
const themeKey = 'synapta-connect-theme';

function initialTheme() {
  try {
    const saved = localStorage.getItem(themeKey);
    if (saved === 'light' || saved === 'dark') return saved;
  } catch { /* Browser storage may be disabled. */ }
  return matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light';
}

function currentPage() {
  const page = new URLSearchParams(location.search).get('view');
  return pages.some(([id]) => id === page) ? page : 'overview';
}

function Brand() {
  return <div className="flex items-center gap-3">
    <img className="brand-mark" src={logo} alt="" width="44" height="44" />
    <span className="leading-tight"><span className="block font-display text-lg font-semibold">Synapta</span><span className="block text-sm text-muted">Connect</span></span>
  </div>;
}

function Overview({ data, update, navigate, ...controls }) {
  return <>
    <div className="grid gap-3 sm:grid-cols-3">
      {[
        [Monitor, 'อุปกรณ์ที่ได้รับสิทธิ์', (data.devices || []).length + ' เครื่อง', 'devices'],
        [Box, 'รุ่น Agent', update.current_version || data.agent?.version || '—', 'updates'],
        [ShieldCheck, 'บัญชีที่รัน Agent', data.agent?.account || '—', 'settings'],
      ].map(([Icon, label, value, id]) => <button key={id} type="button" className="surface metric group flex min-h-21 cursor-pointer items-center gap-3 rounded-2xl p-4 text-left"
        onClick={() => navigate(id)}>
        <span className="tile"><Icon size={19} aria-hidden="true" /></span>
        <span className="min-w-0 flex-1"><span className="block text-sm text-muted">{label}</span><span className="block break-all font-semibold">{value}</span></span>
        <ArrowRight size={17} className="shrink-0 text-muted group-hover:text-accent" aria-hidden="true" />
      </button>)}
    </div>
    <div className="grid gap-4 xl:grid-cols-[1.2fr_1fr]">
      <Pairing data={data} {...controls} />
      <NetworkDetails data={data} />
    </div>
  </>;
}

function HostBanner({ data, online, service, checkedAt }) {
  return <section className="panel-banner host-banner overflow-hidden">
    <div className="host-content">
      <span className="host-kicker"><Radio size={16} aria-hidden="true" />เครื่องปลายทาง</span>
      <h2 className="mt-2 break-words font-display text-3xl font-semibold sm:text-5xl">{data?.name || 'เครื่องนี้'}</h2>
      <p className="mt-2 text-sm text-indigo-100">{data?.agent?.mode === 'console' ? 'Agent ในโหมดคอนโซล' : 'Synapta Connect Agent'}</p>
      <div className="mt-5 flex flex-wrap items-center gap-3">
        <Status tone={online ? 'good' : 'warn'}>{online ? 'ออนไลน์' : service?.agent === 'Stopped' ? 'Agent หยุดอยู่' : 'กำลังรอสัญญาณ'}</Status>
        {checkedAt && <time className="text-xs text-indigo-100" dateTime={new Date(checkedAt).toISOString()}>
          ตรวจล่าสุด {new Date(checkedAt).toLocaleTimeString('th-TH')}
        </time>}
      </div>
    </div>
  </section>;
}

const serviceLabels = {
  Running: 'กำลังทำงาน', Stopped: 'หยุดอยู่', StartPending: 'กำลังเริ่ม',
  StopPending: 'กำลังหยุด', NotInstalled: 'ยังไม่ได้ติดตั้ง', Unavailable: 'อ่านสถานะไม่ได้',
};

function ComponentStatus({ icon: Icon, title, state, description, tone, children }) {
  return <section className="surface component-status">
    <div className="flex items-center gap-3">
      <span className="tile"><Icon size={20} aria-hidden="true" /></span>
      <h2 className="min-w-0 flex-1 font-semibold">{title}</h2>
      <Status tone={tone}>{state}</Status>
    </div>
    {description && <p className="mt-3 break-words text-sm text-muted">{description}</p>}
    {children && <div className="mt-4 flex flex-wrap gap-2">{children}</div>}
  </section>;
}

function SystemStatus({ data, service, online, busy, confirm, action, serviceAction, openBrowser }) {
  const agent = service?.agent;
  const network = data?.network;
  const listener = network?.web_listener;
  const webEnabled = network?.local_enabled || network?.lan_enabled;
  const webReady = online && webEnabled && listener?.ready !== false;
  const helper = data?.screen_input;
  const working = ['StartPending', 'StopPending'].includes(agent) || service?.updater === 'Running';

  function confirmService(command, label) {
    confirm({
      title: label + ' Agent',
      text: 'การเชื่อมต่อที่ใช้งานอยู่จะถูกตัด อุปกรณ์จะเชื่อมใหม่ได้เมื่อ Agent พร้อมใช้งาน',
      label,
      danger: command === 'stop',
      run: () => serviceAction(command),
    });
  }

  return <div className="system-grid">
    <ComponentStatus icon={Server} title="Agent"
      state={agent === 'Running' && !online ? 'Service เปิดอยู่ · รอ Agent' : serviceLabels[agent] || (online ? 'กำลังทำงาน' : 'รอตรวจสอบ')}
      tone={online ? 'good' : agent === 'Running' ? 'warn' : 'neutral'}
      description={service?.account ? 'บัญชี Desktop · ' + service.account : data?.agent?.account}>
      {desktop && <>
        {agent === 'Stopped' && <Button icon={Play} variant="primary" disabled={busy || working || !service.can_start} onClick={() => serviceAction('start')}>เริ่ม</Button>}
        {agent === 'Running' && <>
          <Button icon={RefreshCw} disabled={busy || working || !service.can_start || !service.can_stop} onClick={() => confirmService('restart', 'เริ่มใหม่')}>เริ่มใหม่</Button>
          <Button icon={Square} variant="danger" disabled={busy || working || !service.can_stop} onClick={() => confirmService('stop', 'หยุด')}>หยุด</Button>
        </>}
        {service && ['Running', 'Stopped'].includes(agent) && (!service.can_start || !service.can_stop) &&
          <p className="w-full text-sm text-muted">Windows จำกัดคำสั่งที่บัญชีนี้ใช้ได้กับ Service</p>}
      </>}
    </ComponentStatus>
    <ComponentStatus icon={Globe} title="Web Panel"
      state={!online ? 'รอตรวจสอบ' : !webEnabled ? 'ปิดช่องทางเว็บ' : webReady ? 'พร้อมใช้งาน' : listener?.attempts >= 5 ? 'เปิดเว็บไม่ได้' : 'กำลังฟื้นฟู'}
      tone={webReady ? 'good' : online && webEnabled ? 'warn' : 'neutral'}
      description={online ? listener?.error || (network?.local_port ? 'พอร์ต ' + network.local_port : null) : null}>
      {desktop && <>
        <Button icon={ExternalLink} disabled={busy || !webReady || !network?.local_enabled} onClick={openBrowser}>เปิดเว็บ</Button>
        <Button icon={RefreshCw} disabled={busy || !online} onClick={() => action(routes.restartWeb, 'POST', undefined, 'เริ่ม Web Panel ใหม่แล้ว')}>เริ่มเว็บใหม่</Button>
        <Button icon={Undo2} disabled={busy || !online} onClick={() => confirm({
          title: 'กู้คืนเว็บรุ่นที่ใช้งานได้',
          text: 'ระบบจะสลับกลับไปใช้เว็บรุ่นก่อนหน้าที่ตรวจผ่าน หรือเว็บที่ฝังมากับ Agent',
          label: 'กู้คืนเว็บ',
          run: () => action(routes.recoverWeb, 'POST', undefined, 'กู้คืน Web Panel แล้ว'),
        })}>กู้คืนเว็บ</Button>
      </>}
    </ComponentStatus>
    <ComponentStatus icon={Activity} title="Screen & Input"
      state={!online ? 'รอตรวจสอบ' : !helper?.enabled ? 'ปิดอยู่' : helper.helper_connected || data?.agent?.mode === 'console' ? 'พร้อมใช้งาน' : 'รอ Helper'}
      tone={online && helper?.enabled && (helper.helper_connected || data?.agent?.mode === 'console') ? 'good' : 'neutral'}
      description="จับภาพและควบคุมใน Windows session ที่อนุญาต" />
  </div>;
}

function OpenPanel({ message, notify }) {
  return <Card className="max-w-2xl">
    <SectionTitle icon={ShieldCheck} description={message}>ยืนยันสิทธิ์เข้าหน้าจัดการ</SectionTitle>
    <p className="text-sm">เปิด <strong>Synapta Connect</strong> บนเครื่องปลายทางด้วยบัญชีที่ได้รับสิทธิ์ แล้วกด <strong>เปิดเว็บ</strong> หรือรันคำสั่งนี้ใน PowerShell บนเครื่องนั้นเพื่อเปิดหน้าเว็บพร้อมยืนยันสิทธิ์</p>
    <div className="inset mt-2 flex flex-wrap items-center justify-between gap-3 rounded-xl p-3">
      <code className="break-all text-sm">{panelCommand}</code>
      <CopyButton text={panelCommand} onError={notify} />
    </div>
    <p className="mt-3 text-sm text-muted">ถ้าเบราว์เซอร์ยังไม่เปิดพร้อมสิทธิ์ ให้ต่อท้ายคำสั่งด้วย <code className="text-ink">--link</code> แล้ววางลิงก์ที่ได้ลงใน Chrome หรือ Edge ภายใน 60 วินาที</p>
    <p className="mt-3 text-sm text-muted">ถ้าแจ้งว่าหาไฟล์ไม่พบ ให้รัน <code className="text-ink">sc.exe qc SynaptaConnect</code> เพื่อดูพาธจริงที่ <code className="text-ink">BINARY_PATH_NAME</code></p>
    <p className="mt-3 text-sm text-muted">ถ้าเปิดจากเครื่องอื่น ให้เจ้าของเครื่องปลายทางออกลิงก์ HTTPS ด้วยคำสั่ง <code className="break-all text-ink">{panelCommand} --lan ชื่อเครื่อง</code></p>
  </Card>;
}

function App({ code }) {
  const session = useSession(code);
  const { status, update, connection, message, action, busy, notice, setNotice } = session;
  const [theme, setTheme] = useState(initialTheme);
  const [page, setPage] = useState(currentPage);
  const [confirmation, setConfirmation] = useState(null);
  const reducedMotion = useReducedMotion();
  const main = useRef(null);
  const dismiss = useCallback(() => setNotice(null), [setNotice]);
  const notify = text => setNotice({ text, error: true });
  const label = pages.find(([id]) => id === page)[1];
  const online = connection === 'online';
  const controls = { action, disabled: busy || !online, confirm: setConfirmation, notify };

  useEffect(() => {
    document.documentElement.dataset.theme = theme;
    try { localStorage.setItem(themeKey, theme); } catch { /* Use the selected theme for this page. */ }
  }, [theme]);
  useEffect(() => {
    const onPop = () => setPage(currentPage());
    window.addEventListener('popstate', onPop);
    return () => window.removeEventListener('popstate', onPop);
  }, []);

  function navigate(id) {
    const url = new URL(location.href);
    if (id === 'overview') url.searchParams.delete('view');
    else url.searchParams.set('view', id);
    if (id !== page) history.pushState(null, '', url);
    setPage(id);
    main.current?.scrollIntoView({ block: 'start' });
    requestAnimationFrame(() => main.current?.focus({ preventScroll: true }));
  }

  const connectionLabel = online ? 'เชื่อมต่อแล้ว' : connection === 'opening' ? 'กำลังเชื่อมต่อ' : connection === 'offline' ? 'ขาดการเชื่อมต่อ' : 'ต้องยืนยันสิทธิ์';
  const connectionTone = online ? 'good' : connection === 'offline' ? 'warn' : 'neutral';
  return <div className="layout">
    <a href="#main" className="sr-only focus:not-sr-only focus:fixed focus:top-3 focus:left-3 focus:z-50 focus:rounded-lg focus:bg-panel focus:p-3">ข้ามไปเนื้อหาหลัก</a>
    <aside className="sidebar">
      <div className="sidebar-brand hidden lg:block"><Brand /></div>
      <nav className="menu" aria-label="เมนูหลัก">
        {pages.filter(([id]) => id !== 'install').map(([id, title, Icon]) =>
          <button type="button" className="nav-item" key={id} aria-current={page === id ? 'page' : undefined} onClick={() => navigate(id)}>
            <Icon size={20} aria-hidden="true" /><span>{title}</span>
          </button>)}
      </nav>
      <button type="button" className="nav-item install-sidebar mt-auto" aria-current={page === 'install' ? 'page' : undefined} onClick={() => navigate('install')}>
        <CircleHelp size={20} aria-hidden="true" />คู่มือติดตั้ง
      </button>
    </aside>
    <div className="min-w-0">
      <header className="toolbar flex flex-wrap items-center justify-between gap-2 px-4 py-2 sm:px-6">
        <div className="lg:hidden"><Brand /></div>
        <span className="toolbar-context hidden lg:flex"><span className="toolbar-led" />{desktop ? 'Desktop' : 'Web Panel'}</span>
        <div className="ml-auto flex items-center gap-1.5 sm:gap-2">
          <Status tone={connectionTone}>{connectionLabel}</Status>
          <button type="button" className="icon-button install-toolbar" aria-label="คู่มือติดตั้ง" onClick={() => navigate('install')}>
            <CircleHelp size={21} aria-hidden="true" />
          </button>
          <button type="button" className="icon-button" aria-label={theme === 'dark' ? 'ใช้ธีมสว่าง' : 'ใช้ธีมมืด'}
            onClick={() => setTheme(theme === 'dark' ? 'light' : 'dark')}>
            {theme === 'dark' ? <Sun size={21} aria-hidden="true" /> : <Moon size={21} aria-hidden="true" />}
          </button>
        </div>
      </header>
      <main id="main" ref={main} tabIndex={-1} className="page-frame mx-auto max-w-345 px-4 pt-5 pb-28 sm:px-6 sm:pt-7 lg:px-9 lg:pb-8">
        <m.div key={page} className="grid content-start gap-4 sm:gap-5"
          initial={reducedMotion ? false : { opacity: 0, y: 8 }} animate={{ opacity: 1, y: 0 }}
          transition={{ duration: reducedMotion ? 0 : 0.24, ease: [0.2, 0.7, 0.2, 1] }}>
        <h1 className="font-display text-3xl font-semibold sm:text-4xl">{label}</h1>
        {page === 'install' ? <InstallGuide notify={notify} /> : <>
          {page === 'overview' && (desktop || status) && <>
            <HostBanner data={status} online={online} service={session.service} checkedAt={session.checkedAt} />
            <SystemStatus data={status} online={online} service={session.service} busy={busy} confirm={setConfirmation}
              action={action} serviceAction={session.serviceAction} openBrowser={session.openBrowser} />
          </>}
          {connection === 'opening' && <p role="status" className="text-muted">กำลังเชื่อมต่อกับ Agent</p>}
          {connection === 'locked' && <OpenPanel message={message} notify={notify} />}
          {connection === 'offline' && <div className="connection-alert alert flex flex-wrap items-center justify-between gap-3 rounded-2xl border p-3">
            <span role="status">{message}</span><Button icon={RefreshCw} disabled={busy} onClick={session.refresh}>ตรวจอีกครั้ง</Button>
          </div>}
          {desktop && online && status?.agent?.version && session.service?.desktop_version !== status.agent.version &&
            <p className="alert rounded-2xl border p-3 text-sm" role="status">Agent ใช้รุ่น {status.agent.version} แล้ว เปิด Desktop ใหม่เพื่อใช้หน้าจอรุ่นล่าสุด</p>}
          {status && update && <>
            {page === 'overview' && <Overview data={status} update={update} navigate={navigate} {...controls} />}
            {page === 'devices' && <Devices data={status} {...controls} />}
            {page === 'network' && <NetworkSettings network={status.network} {...controls} />}
            {page === 'updates' && <Updates update={update} web={update.web} {...controls} />}
            {page === 'settings' && <Settings data={status} update={update} {...controls} />}
          </>}
        </>}
        </m.div>
      </main>
    </div>
    <Confirm value={confirmation} onClose={() => setConfirmation(null)} />
    <Toast value={notice} dismiss={dismiss} />
  </div>;
}

function PanelEntry() {
  const [code, setCode] = useState(readOpeningCode);
  useEffect(() => {
    function openLink() {
      const next = readOpeningCode();
      if (next) setCode(next);
    }
    window.addEventListener('hashchange', openLink);
    return () => window.removeEventListener('hashchange', openLink);
  }, []);
  return <App key={code} code={code} />;
}

createRoot(document.getElementById('root')).render(
  <LazyMotion features={domAnimation} strict><PanelEntry /></LazyMotion>,
);
