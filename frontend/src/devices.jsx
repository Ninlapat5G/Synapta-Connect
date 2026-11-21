import { useEffect, useState } from 'react';
import { Cable, KeyRound, Laptop, Network, Plus, RefreshCw, ShieldMinus, Wifi } from 'lucide-react';
import { formatDate, routes } from './api';
import { Button, Card, CopyButton, SectionTitle, Status } from './ui';

function Clock({ expiresAt }) {
  const [now, setNow] = useState(Date.now());
  useEffect(() => {
    const timer = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(timer);
  }, []);
  const seconds = Math.max(0, Math.ceil(expiresAt - now / 1000));
  return <span aria-live="off">เหลือ {seconds} วินาที</span>;
}

export function Pairing({ data, action, disabled, notify }) {
  const failures = data.pairing?.recent_failures || [];
  const locked = data.pairing?.locked_for_secs;
  return <Card>
    <SectionTitle icon={KeyRound} description="นำรหัส 8 หลักไปใส่ใน Synapta Code บนอุปกรณ์ที่จะเชื่อมต่อ">จับคู่อุปกรณ์</SectionTitle>
    <div className="flex flex-wrap items-center gap-2">
      <Status tone={data.pairing_mode ? 'good' : 'neutral'}>{data.pairing_mode ? 'เปิดรับการจับคู่' : 'ปิดรับการจับคู่'}</Status>
      {data.otp_expires_at && <span className="text-sm text-muted"><Clock expiresAt={data.otp_expires_at} /></span>}
    </div>
    <div className="inset mt-4 flex min-h-18 flex-wrap items-center justify-between gap-2 rounded-xl p-3 sm:px-4">
      <code className="select-all text-2xl font-semibold tracking-[.13em] sm:text-3xl" aria-label={data.otp ? 'รหัสจับคู่ ' + data.otp : 'ไม่มีรหัสจับคู่'}>
        {data.otp || '•••• ••••'}
      </code>
      {!!data.otp && <CopyButton text={data.otp} onError={notify} />}
    </div>
    {Boolean(locked || failures.length) && <div className="alert mt-4 rounded-xl p-3 text-sm">
      <p>{locked ? 'ล็อกการจับคู่อีก ' + locked + ' วินาที' : 'รายการจับคู่ไม่สำเร็จล่าสุด'}</p>
      <ul className="mt-1 space-y-1 break-all">
        {failures.map((entry, i) => <li key={i}>{[formatDate(entry.at), entry.ip, entry.client_name].filter(Boolean).join(' · ')}</li>)}
      </ul>
    </div>}
    <div className="mt-4 flex flex-wrap gap-2">
      <Button variant={data.pairing_mode ? 'secondary' : 'primary'} icon={!data.pairing_mode ? Plus : undefined} disabled={disabled}
        onClick={() => action(routes.pairing + (data.pairing_mode ? 'close' : 'open'))}>
        {data.pairing_mode ? 'ปิดรับการจับคู่' : 'เปิดรับการจับคู่'}
      </Button>
      {data.pairing_mode && <Button icon={RefreshCw} disabled={disabled} onClick={() => action(routes.pairing + 'open')}>ออกรหัสใหม่</Button>}
    </div>
  </Card>;
}

export function NetworkDetails({ data }) {
  return <Card>
    <SectionTitle icon={Network} description="ที่อยู่ของ Agent บนเครือข่าย">เครือข่าย</SectionTitle>
    {data.addresses?.length ? <ul className="divide-y divide-line">
      {data.addresses.map(address => <li key={address.name + address.ip} className="flex flex-wrap items-center justify-between gap-2 py-2.5 first:pt-0">
        <span className="flex min-w-0 items-center gap-2 break-all"><Wifi size={15} className="shrink-0 text-muted" aria-hidden="true" />{address.name}</span>
        <code className="break-all text-sm text-muted">{address.ip}</code>
      </li>)}
    </ul> : <p className="text-sm text-muted">ไม่มีข้อมูลเครือข่าย</p>}
  </Card>;
}

export function Devices({ data, action, disabled, confirm, notify }) {
  const devices = data.devices || [];
  return <div className="grid gap-4 xl:grid-cols-[1.45fr_1fr]">
    <Card className="xl:row-span-2">
      <SectionTitle icon={Laptop} description={'อุปกรณ์ที่จับคู่แล้ว ' + devices.length + ' เครื่อง'}>สิทธิ์เข้าถึง</SectionTitle>
      {devices.length ? <ul className="divide-y divide-line">
        {devices.map(device => {
          const name = device.client_name || device.id;
          return <li key={device.id} className="flex flex-wrap items-center gap-x-3 gap-y-2 py-3 first:pt-0">
            <span className="tile"><Laptop size={19} aria-hidden="true" /></span>
            <div className="min-w-32 flex-1">
              <p className="break-all font-semibold" title={device.id}>{name}</p>
              <p className="text-sm text-muted">{device.last_seen ? 'ล่าสุด ' + formatDate(device.last_seen) : 'ยังไม่มีประวัติเชื่อมต่อ'}</p>
            </div>
            <Button variant="danger" disabled={disabled} aria-label={'ยกเลิกสิทธิ์ ' + name}
              onClick={() => confirm({
                title: 'ยกเลิกสิทธิ์อุปกรณ์', text: name + ' ต้องจับคู่ใหม่ก่อนเชื่อมต่อ', danger: true, label: 'ยกเลิกสิทธิ์',
                run: () => action(routes.devices + encodeURIComponent(device.id) + '/revoke'),
              })}>ยกเลิกสิทธิ์</Button>
          </li>;
        })}
      </ul> : <div className="inset grid justify-items-center gap-2 rounded-xl p-7 text-center text-muted">
        <ShieldMinus size={25} aria-hidden="true" />
        <p>ยังไม่มีอุปกรณ์ที่ได้รับสิทธิ์</p>
      </div>}
    </Card>
    <Pairing data={data} action={action} disabled={disabled} notify={notify} />
    <Card>
      <SectionTitle icon={Cable} description="ตัดการเชื่อมต่อที่เปิดอยู่ อุปกรณ์ที่จับคู่แล้วสามารถเชื่อมต่อใหม่ได้">การเชื่อมต่อ</SectionTitle>
      <Button variant="danger" disabled={disabled} onClick={() => confirm({
        title: 'ตัดทุกการเชื่อมต่อ', text: 'งานที่กำลังทำบนอุปกรณ์ที่เชื่อมต่ออาจหยุดทันที', danger: true, label: 'ตัดการเชื่อมต่อ',
        run: () => action(routes.disconnect),
      })}>ตัดทุกการเชื่อมต่อ</Button>
    </Card>
  </div>;
}
