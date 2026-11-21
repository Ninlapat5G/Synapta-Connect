import { Download, History, RefreshCw } from 'lucide-react';
import { formatDate, routes } from './api';
import { Button, Card, SectionTitle, Status } from './ui';

const states = {
  idle: 'ยังไม่เคยตรวจ', checking: 'กำลังตรวจ', downloading: 'กำลังดาวน์โหลด',
  verifying: 'กำลังตรวจสอบ', update_available: 'มีอัปเดตพร้อมติดตั้ง',
  applying: 'กำลังติดตั้ง', up_to_date: 'เป็นรุ่นล่าสุด', failed: 'อัปเดตไม่สำเร็จ',
  rolled_back: 'กู้รุ่นเดิมแล้ว', disabled: 'ปิดอัปเดตอัตโนมัติ',
  not_configured: 'ยังไม่ได้ตั้งแหล่งอัปเดตหรือกุญแจผู้เผยแพร่',
  incompatible: 'รุ่นนี้ยังใช้กับ backend ปัจจุบันไม่ได้',
};
const working = ['checking', 'downloading', 'verifying', 'applying'];
const outcomes = { installed: 'ติดตั้งแล้ว', rolled_back: 'กู้รุ่นเดิม', failed: 'ล้มเหลว' };

export function UpdateTrack({ title, description, data, base, action, confirm, disabled, backend }) {
  const active = working.includes(data.state);
  const available = data.available;
  const tone = data.state === 'failed' ? 'bad' : data.state === 'rolled_back' ? 'warn' : data.state === 'up_to_date' ? 'good' : 'neutral';
  return <Card>
    <SectionTitle icon={Download} description={description}>{title}</SectionTitle>
    <Status tone={tone}>{states[data.state] || data.state || 'ไม่ทราบสถานะ'}</Status>
    <dl className="mt-4 grid grid-cols-2 gap-3 rounded-xl border border-line p-3 sm:grid-cols-3">
      <div><dt className="text-sm text-muted">รุ่นปัจจุบัน</dt><dd className="break-all font-semibold">{data.current_version || '—'}</dd></div>
      <div><dt className="text-sm text-muted">รุ่นที่พบ</dt><dd className="break-all font-semibold">{available?.version || '—'}</dd></div>
      <div><dt className="text-sm text-muted">ตรวจล่าสุด</dt><dd className="font-semibold">{formatDate(data.last_check_at)}</dd></div>
    </dl>
    {available?.notes && <p className="mt-3 whitespace-pre-wrap break-words text-sm text-muted">{available.notes}</p>}
    {data.last_error && <div role="status" className={'mt-4 rounded-xl border p-3 text-sm ' + (data.last_error.code === 'rollback_failed' ? 'critical' : 'alert')}>
      <p className="break-words">{data.last_error.message}</p>
      <code className="mt-1 block break-all">{data.last_error.code}</code>
    </div>}
    <div className="mt-4 flex flex-wrap gap-2">
      <Button icon={RefreshCw} disabled={disabled || active} onClick={() => action(base + '/check')}>ตรวจอัปเดต</Button>
      <Button icon={Download} variant="primary" disabled={disabled || active || !available?.version}
        onClick={() => confirm({
          title: 'ติดตั้ง ' + title + ' รุ่น ' + available.version,
          text: backend ? 'Agent จะเริ่มใหม่ระหว่างการติดตั้ง' : 'ระบบจะตรวจแพ็กเกจเว็บก่อนเปิดใช้รุ่นใหม่',
          label: 'ติดตั้ง',
          run: () => action(base + '/apply', 'POST', { version: available.version }),
        })}>ติดตั้งอัปเดต</Button>
      {data.can_retry && <Button disabled={disabled || active} onClick={() => confirm({
        title: 'ลองติดตั้งอีกครั้ง', text: 'ระบบจะตรวจแพ็กเกจและลองอัปเดตอีกครั้ง',
        run: () => action(base + '/retry'),
      })}>ลองอีกครั้ง</Button>}
      {!backend && data.can_rollback && <Button variant="danger" disabled={disabled || active} onClick={() => confirm({
        title: 'กู้คืนเว็บรุ่นก่อน', text: 'ระบบจะสลับกลับไปใช้เว็บที่ผ่านการตรวจครั้งก่อน หรือเว็บที่ติดมากับ Agent',
        label: 'กู้คืนเว็บ', danger: true,
        run: () => action(base + '/rollback'),
      })}>กู้คืนเว็บ</Button>}
      {!backend && <Button disabled={disabled || active} onClick={async () => {
        const result = await action(routes.reload);
        if (result?.code) {
          const url = new URL(location.href);
          url.hash = 'code=' + result.code;
          history.replaceState(null, '', url);
          location.reload();
        }
      }}>โหลดหน้าเว็บใหม่</Button>}
    </div>
    {!!data.history?.length && <details className="mt-4 border-t border-line pt-3">
      <summary className="flex min-h-11 cursor-pointer items-center gap-2 font-semibold"><History size={17} aria-hidden="true" />ประวัติการอัปเดต</summary>
      <ul className="divide-y divide-line">
        {data.history.map((entry, index) => <li className="py-2 text-sm" key={entry.at + '-' + index}>
          <p>{outcomes[entry.outcome] || entry.outcome} · {entry.from || '—'} → {entry.to || '—'}</p>
          <p className="break-words text-muted">{[formatDate(entry.at), entry.stage, entry.message || entry.code].filter(Boolean).join(' · ')}</p>
        </li>)}
      </ul>
    </details>}
  </Card>;
}

export function Updates({ update, web, ...controls }) {
  return <div className="grid gap-4">
    <UpdateTrack title="Agent backend" description="โปรแกรม Rust ที่ทำงานบนเครื่องปลายทาง" data={update} base={routes.updates} backend {...controls} />
    {web && <UpdateTrack title="เว็บ panel" description="หน้าจัดการที่เปิดในเบราว์เซอร์" data={web} base={routes.webUpdates} {...controls} />}
  </div>;
}
