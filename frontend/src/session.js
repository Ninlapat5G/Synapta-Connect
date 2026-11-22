import { useEffect, useRef, useState } from 'react';
import { desktop, request, routes, serviceControl, serviceStatus, openWeb } from './api';

const offline = 'เชื่อมต่อ Agent ไม่ได้ กำลังลองใหม่อัตโนมัติ';
const locked = 'เปิดหน้าจัดการจากไอคอน Synapta Connect หรือคำสั่งด้านล่าง บนเครื่องที่ติดตั้ง';
const linkExpired = 'ลิงก์เปิดหน้านี้หมดอายุหรือถูกใช้ไปแล้ว เปิดจากไอคอนหรือคำสั่งด้านล่างอีกครั้ง';
const reloaded = 'หน้าถูกโหลดใหม่ จึงต้องยืนยันสิทธิ์อีกครั้ง เปิดจากไอคอนหรือคำสั่งด้านล่างอีกครั้ง';
const expired = 'เซสชันหมดอายุ เปิดจากไอคอนหรือคำสั่งด้านล่างอีกครั้ง';
const openedKey = 'synapta-connect-opened';

// A reload drops the one-time code from the address, so tell that apart from a first visit.
function lockedMessage(code) {
  try {
    if (code) sessionStorage.setItem(openedKey, '1');
    else if (sessionStorage.getItem(openedKey)) return reloaded;
  } catch { /* Browser storage may be disabled. */ }
  return locked;
}

export function useSession(code) {
  const [state, setState] = useState(() => ({ connection: desktop || code ? 'opening' : 'locked', message: desktop ? '' : lockedMessage(code) }));
  const [service, setService] = useState(null);
  const [busy, setBusy] = useState(false);
  const [notice, setNotice] = useState(null);
  const operation = useRef(null);
  const refresh = useRef(null);
  const token = useRef(null);
  const pending = useRef(false);

  useEffect(() => {
    if (!code && !desktop) return;
    const active = new AbortController();
    let timer;
    let failures = 0;

    function schedule() {
      clearTimeout(timer);
      if ((desktop || token.current) && !active.signal.aborted) {
        const delay = failures
          ? Math.min(30000, 1200 * 2 ** Math.min(failures - 1, 5)) + Math.random() * 500
          : document.hidden ? 10000 : 2000;
        timer = setTimeout(poll, delay);
      }
    }

    async function poll() {
      if ((!desktop && !token.current) || pending.current || active.signal.aborted) return;
      if (operation.current) { schedule(); return; }
      clearTimeout(timer);
      pending.current = true;
      const options = { token: token.current, signal: active.signal };
      try {
        if (desktop) {
          const current = await serviceStatus();
          if (active.signal.aborted) return;
          setService(current);
          if (current.agent !== 'Running') {
            setState({ connection: 'offline', message: 'Windows Service ยังไม่พร้อมใช้งาน' });
            return;
          }
        }
        const [status, update] = await Promise.all([
          request(routes.status, options),
          request(routes.updates, options),
        ]);
        failures = 0;
        if (!operation.current && !active.signal.aborted) {
          setState({ connection: 'online', status, update, checkedAt: Date.now() });
        }
      } catch (error) {
        if (active.signal.aborted) return;
        if (error.status === 403 && !desktop) {
          token.current = null;
          setState({ connection: 'locked', message: expired });
        } else {
          failures += 1;
          setState(previous => ({
            ...previous,
            connection: 'offline',
            message: desktop && error.status === 403 ? error.message : previous.update?.state === 'applying'
              ? 'กำลังรอ service เริ่มใหม่ ระบบจะเชื่อมต่ออีกครั้งอัตโนมัติ'
              : offline,
          }));
        }
      } finally {
        pending.current = false;
        schedule();
      }
    }

    refresh.current = poll;
    if (desktop) poll();
    else request(routes.exchange, { method: 'POST', body: { code }, signal: active.signal })
      .then(session => {
        if (active.signal.aborted) return;
        if (!session.token) throw new Error(linkExpired);
        token.current = session.token;
        return poll();
      })
      .catch(error => {
        if (!active.signal.aborted) {
          setState({ connection: 'locked', message: error.status === 403 ? linkExpired : 'เชื่อมต่อเพื่อยืนยันตัวตนไม่ได้ กรุณาขอลิงก์ใหม่' });
        }
      });
    document.addEventListener('visibilitychange', poll);
    window.addEventListener('online', poll);
    return () => {
      active.abort();
      clearTimeout(timer);
      document.removeEventListener('visibilitychange', poll);
      window.removeEventListener('online', poll);
    };
  }, [code]);

  async function run(command, success) {
    if (operation.current) return false;
    operation.current = true;
    setBusy(true);
    try {
      const result = await command();
      setNotice({ text: result?.revoked === false ? 'ไม่พบอุปกรณ์ที่ต้องการยกเลิกสิทธิ์' : success });
      return result;
    } catch (error) {
      if (error.status === 403 && !desktop) {
        token.current = null;
        setState({ connection: 'locked', message: expired });
      } else {
        const message = error instanceof TypeError || error.name === 'TimeoutError' || error.status === 0
          ? 'ยังยืนยันผลไม่ได้ ระบบจะตรวจสถานะล่าสุดอีกครั้ง' : error.message || String(error);
        setNotice({ text: message, error: true });
      }
      return false;
    } finally {
      operation.current = null;
      setBusy(false);
      refresh.current?.();
    }
  }

  function action(path, method = 'POST', body, success = 'ดำเนินการแล้ว') {
    if (!desktop && !token.current) return false;
    return run(() => request(path, { method, body, token: token.current }), success);
  }

  function serviceAction(action) {
    return run(() => serviceControl(action), 'ส่งคำสั่งให้ Windows Service แล้ว');
  }

  return { ...state, service, action, serviceAction, openBrowser: () => run(openWeb, 'เปิด Web Panel แล้ว'),
    refresh: () => refresh.current?.(), busy, notice, setNotice };
}
