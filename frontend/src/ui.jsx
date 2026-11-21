import { useEffect, useRef, useState } from 'react';
import { Check, Copy, X } from 'lucide-react';

export function Card({ children, className = '' }) {
  return <section className={'surface min-w-0 rounded-2xl p-5 sm:p-6 ' + className}>{children}</section>;
}

export function SectionTitle({ icon: Icon, children, description }) {
  return <div className="mb-5 flex items-start gap-3">
    {Icon && <span className="tile"><Icon size={20} aria-hidden="true" /></span>}
    <div className="min-w-0">
      <h2 className="text-xl font-semibold">{children}</h2>
      {description && <p className="mt-0.5 text-sm text-muted">{description}</p>}
    </div>
  </div>;
}

export function Button({ children, variant = 'secondary', icon: Icon, ...props }) {
  return <button type="button" className={'button ' + variant} {...props}>
    {Icon && <Icon size={17} aria-hidden="true" />}
    {children}
  </button>;
}

export function Status({ tone = 'neutral', children }) {
  return <span className={'status ' + tone}>
    <span className="size-1.5 shrink-0 rounded-full bg-current" aria-hidden="true" />{children}
  </span>;
}

export function Field({ label, children }) {
  return <label className="grid gap-1.5 text-sm font-semibold">{label}{children}</label>;
}

export function Toggle({ children, detail, checked, onChange, disabled }) {
  return <label className="flex min-h-12 cursor-pointer items-center gap-3 rounded-xl border border-line p-3 has-disabled:cursor-default">
    <span className="flex-1">
      <span className="block font-semibold">{children}</span>
      {detail && <span className="block text-sm text-muted">{detail}</span>}
    </span>
    <input type="checkbox" role="switch" className="switch" checked={checked} onChange={onChange} disabled={disabled} />
  </label>;
}

export function Confirm({ value, onClose }) {
  const ref = useRef(null);
  useEffect(() => {
    const dialog = ref.current;
    if (value && !dialog.open) dialog.showModal();
    if (!value && dialog.open) dialog.close();
  }, [value]);

  return <dialog ref={ref} className="dialog surface rounded-2xl p-6 shadow-2xl" aria-labelledby="confirm-title"
    onCancel={onClose} onClose={onClose}>
    {value && <>
      <h2 id="confirm-title" className="text-xl font-semibold">{value.title}</h2>
      <p className="mt-2 text-muted">{value.text}</p>
      <div className="mt-6 flex flex-wrap justify-end gap-2">
        <Button autoFocus onClick={onClose}>ยกเลิก</Button>
        <Button variant={value.danger ? 'danger' : 'primary'} onClick={() => { onClose(); value.run(); }}>
          {value.label || 'ยืนยัน'}
        </Button>
      </div>
    </>}
  </dialog>;
}

export function CopyButton({ text, label = 'คัดลอก', onError, disabled }) {
  const [copied, setCopied] = useState(false);
  useEffect(() => {
    if (!copied) return;
    const timer = setTimeout(() => setCopied(false), 1800);
    return () => clearTimeout(timer);
  }, [copied, text]);
  return <Button icon={copied ? Check : Copy} disabled={disabled || !text} onClick={async () => {
    try {
      await navigator.clipboard.writeText(text);
      setCopied(true);
    } catch {
      onError?.('คัดลอกอัตโนมัติไม่ได้ กรุณาเลือกข้อความแล้วคัดลอก');
    }
  }}>{copied ? 'คัดลอกแล้ว' : label}</Button>;
}

export function Toast({ value, dismiss }) {
  useEffect(() => {
    if (!value || value.error) return;
    const timer = setTimeout(dismiss, 3500);
    return () => clearTimeout(timer);
  }, [value, dismiss]);
  if (!value) return null;
  return <div className="toast surface fixed z-20 flex max-w-sm items-center gap-3 rounded-xl p-3 shadow-xl" role={value.error ? 'alert' : 'status'}>
    <span className={'flex-1 ' + (value.error ? 'text-danger' : '')}>{value.text}</span>
    <button type="button" className="icon-button" aria-label="ปิดข้อความ" onClick={dismiss}><X size={18} aria-hidden="true" /></button>
  </div>;
}
