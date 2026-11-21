import { useCallback, useEffect, useRef, useState } from 'react';
import { createRoot } from 'react-dom/client';
import { ArrowRight, Box, CircleHelp, Download, LayoutDashboard, Monitor, Moon, Network, Settings2, ShieldCheck, Sun } from 'lucide-react';
import logo from '../assets/synapta-connect.png';
import { readOpeningCode } from './api';
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
// The installer puts the agent in Program Files, which is not on PATH; PowerShell needs `&` for a quoted path.
const installedAgent = String.raw`& "$env:ProgramFiles\SynaptaConnect\synapta-connect.exe"`;
const openCommand = installedAgent + ' panel';
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
  const mode = data.agent?.mode === 'service' ? 'Windows Service' : 'โหมดคอนโซล';
  const privilege = { standard: 'สิทธิ์มาตรฐาน', administrator: 'สิทธิ์ผู้ดูแล', system: 'สิทธิ์ระบบ' }[data.agent?.privilege] || data.agent?.privilege;
  return <>
    <section className="panel-banner host-banner overflow-hidden rounded-2xl p-5 sm:px-7">
      <div className="relative z-1 min-w-0 sm:max-w-[55%]">
        <span className="flex items-center gap-2 text-sm font-semibold text-indigo-100"><Monitor size={17} aria-hidden="true" />เครื่องปลายทาง</span>
        <h2 className="mt-1 break-all font-display text-3xl font-semibold sm:text-4xl">{data.name || 'เครื่องนี้'}</h2>
        <p className="mt-2 break-all text-sm text-indigo-100">{mode}{privilege ? ' · ' + privilege : ''}</p>
      </div>
    </section>
    <div className="grid gap-3 sm:grid-cols-3">
      {[
        [Monitor, 'อุปกรณ์ที่ได้รับสิทธิ์', (data.devices || []).length + ' เครื่อง', 'devices'],
        [Box, 'รุ่น Agent', update.current_version || data.agent?.version || '—', 'updates'],
        [ShieldCheck, 'บัญชีที่รัน Agent', data.agent?.account || '—', 'settings'],
      ].map(([Icon, label, value, id]) => <button key={id} type="button" className="surface group flex min-h-21 cursor-pointer items-center gap-3 rounded-xl p-3 text-left"
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

function OpenPanel({ message, notify }) {
  return <Card className="max-w-2xl">
    <SectionTitle icon={ShieldCheck} description={message}>ยืนยันสิทธิ์เข้าหน้าจัดการ</SectionTitle>
    <div className="inset flex flex-wrap items-center justify-between gap-3 rounded-xl p-3">
      <code className="break-all text-sm">{openCommand}</code>
      <CopyButton text={openCommand} onError={notify} />
    </div>
    <p className="mt-3 text-sm text-muted">วางคำสั่งใน PowerShell บนเครื่องปลายทาง ถ้าเปิดจากเครื่องอื่น ให้เจ้าของเครื่องปลายทางออกลิงก์ HTTPS ด้วยคำสั่ง <code className="break-all text-ink">{openCommand} --lan ชื่อเครื่อง</code></p>
  </Card>;
}

function App({ code }) {
  const { status, update, connection, message, action, busy, notice, setNotice } = useSession(code);
  const [theme, setTheme] = useState(initialTheme);
  const [page, setPage] = useState(currentPage);
  const [confirmation, setConfirmation] = useState(null);
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
      <div className="hidden px-3 pt-1 lg:block"><Brand /></div>
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
        <span className="hidden text-sm text-muted lg:block">หน้าจัดการเครื่องปลายทาง</span>
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
      <main id="main" ref={main} tabIndex={-1} className="mx-auto grid max-w-320 content-start gap-4 px-4 pt-5 pb-28 sm:px-6 sm:pt-7 lg:px-8 lg:pb-8">
        <h1 className="font-display text-3xl font-semibold sm:text-4xl">{label}</h1>
        {page === 'install' ? <InstallGuide notify={notify} /> : <>
          {connection === 'opening' && <p role="status" className="text-muted">กำลังยืนยันสิทธิ์เข้าหน้าจัดการ</p>}
          {connection === 'locked' && <OpenPanel message={message} notify={notify} />}
          {connection === 'offline' && <div className="alert rounded-xl border p-3" role="status">{message}</div>}
          {status && update && <>
            {page === 'overview' && <Overview data={status} update={update} navigate={navigate} {...controls} />}
            {page === 'devices' && <Devices data={status} {...controls} />}
            {page === 'network' && <NetworkSettings network={status.network} {...controls} />}
            {page === 'updates' && <Updates update={update} web={update.web} {...controls} />}
            {page === 'settings' && <Settings data={status} update={update} {...controls} />}
          </>}
        </>}
      </main>
    </div>
    <Confirm value={confirmation} onClose={() => setConfirmation(null)} />
    <Toast value={notice} dismiss={dismiss} />
  </div>;
}

createRoot(document.getElementById('root')).render(<App code={readOpeningCode()} />);
