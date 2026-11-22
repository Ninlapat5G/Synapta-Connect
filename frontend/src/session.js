import { useEffect, useRef, useState } from 'react';
import { request, routes } from './api';

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
  const [state, setState] = useState(() => ({ connection: code ? 'opening' : 'locked', message: lockedMessage(code) }));
  const [busy, setBusy] = useState(false);
  const [notice, setNotice] = useState(null);
  const operation = useRef(null);
  const refresh = useRef(null);
  const token = useRef(null);
  const pending = useRef(false);

  useEffect(() => {
    if (!code) return;
    const active = new AbortController();
    let timer;

    function schedule() {
      clearTimeout(timer);
      if (token.current && !active.signal.aborted) {
        timer = setTimeout(poll, document.hidden ? 10000 : 2000);
      }
    }

    async function poll() {
      if (!token.current || pending.current || active.signal.aborted) return;
      if (operation.current) { schedule(); return; }
      pending.current = true;
      const options = { token: token.current, signal: active.signal };
      try {
        const [status, update] = await Promise.all([
          request(routes.status, options),
          request(routes.updates, options),
        ]);
        if (!operation.current) setState({ connection: 'online', status, update });
      } catch (error) {
        if (active.signal.aborted) return;
        if (error.status === 403) {
          token.current = null;
          setState({ connection: 'locked', message: expired });
        } else {
          setState(previous => ({
            ...previous,
            connection: 'offline',
            message: previous.update?.state === 'applying'
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
    request(routes.exchange, { method: 'POST', body: { code }, signal: active.signal })
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
    return () => {
      active.abort();
      clearTimeout(timer);
      document.removeEventListener('visibilitychange', poll);
    };
  }, [code]);

  async function action(path, method = 'POST', body, success = 'ดำเนินการแล้ว') {
    if (!token.current || operation.current) return false;
    operation.current = true;
    setBusy(true);
    try {
      const result = await request(path, { method, body, token: token.current });
      setNotice({ text: result.revoked === false ? 'ไม่พบอุปกรณ์ที่ต้องการยกเลิกสิทธิ์' : success });
      return result;
    } catch (error) {
      if (error.status === 403) {
        token.current = null;
        setState({ connection: 'locked', message: expired });
      } else {
        setNotice({ text: error instanceof TypeError ? 'เชื่อมต่อ Agent ไม่ได้ กรุณาลองใหม่' : error.message, error: true });
      }
      return false;
    } finally {
      operation.current = null;
      setBusy(false);
      refresh.current?.();
    }
  }

  return { ...state, action, busy, notice, setNotice };
}
