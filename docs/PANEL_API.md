# Synapta Connect — สัญญา API ของ panel (service + auto update)

เว็บเริ่มที่ `/synaptaconnect` พอร์ต panel เริ่มต้นคือ TCP 52402 ใช้ HTTP บน loopback หรือ HTTPS บน LAN เมื่อเจ้าของเปิดช่องทางนั้น API ของ panel อยู่ origin เดียวกับหน้าเว็บ และใช้ session token ที่ผูกกับช่องทางและชื่อ HTTPS ที่ออกให้
เอกสารออกแบบและเหตุผล: `docs/SERVICE_AND_UPDATE.md`

## 1. การเปิด panel และยืนยันตัว

panel **ไม่เปิดเอง** — ผู้ใช้สั่ง `synapta-connect.exe panel` (หรือทางลัดที่เรียกคำสั่งนี้) ซึ่งทำ 3 อย่าง:
อ่าน `local_token` (ไฟล์ที่เจ้าของเครื่องและ Administrators อ่านได้) → ตรวจ health และขอรหัสครั้งเดียวผ่าน HMAC → เปิดเบราว์เซอร์ที่
`http://127.0.0.1:52402/synaptaconnect/#code=<รหัสครั้งเดียว อายุ 60 วินาที>`

`synapta-connect.exe panel --lan HOST` พิมพ์ลิงก์ HTTPS สำหรับชื่อหรือ IPv4 ที่อยู่ใน SAN ของใบรับรองและใช้ได้เมื่อเปิดช่องทาง LAN

หน้าเว็บต้องทำ:
1. อ่าน `code` จาก `location.hash` แล้ว **ลบ hash ทิ้งทันที** (`history.replaceState`)
2. `POST /api/v1/panel/exchange` body `{"code": "..."}` → `{"token": "...", "expires_at": <unix วินาที>}`
   (รหัสใช้ได้ครั้งเดียว; ผิด/หมดอายุ = `403`)
3. ทุก request หลังจากนั้นแนบ header `X-Synapta-Local: <token>` เบราว์เซอร์เก็บ token อายุ 8 ชั่วโมงใน `sessionStorage` ของ origin และแท็บเดิมเพื่อให้รีเฟรชได้ ถ้า server ปฏิเสธสิทธิ์ หน้าเว็บจะลบ token ออก
4. token อายุ 8 ชั่วโมง และ **รอดการรีสตาร์ทของ agent** (agent เก็บเฉพาะ SHA-256 ของ token ไว้ใน `panel_sessions.json` จนถึงเวลาหมดอายุ) —
   ตอนอัปเดตสำเร็จ/กู้คืน service รีสตาร์ท แต่ token เดิมยังใช้ได้ หน้าเว็บ poll ต่อได้เลยและเห็นผลอัปเดต; ช่วงที่ service ยังไม่กลับมา request จะ fail (เชื่อมต่อไม่ได้) ให้ลองใหม่เงียบ ๆ
   `403` = token หมดอายุ/ไม่รู้จัก (หรือ Host/Origin ผิด) → แสดงข้อความ "เปิด panel ใหม่ด้วยคำสั่ง synapta-connect panel"

หากมี `Origin` ต้องตรงกับ scheme และ `Host` ของคำขอ Agent ตรวจชื่อ HTTPS กับ SAN ของใบรับรองและไม่เปิด CORS เส้นทาง health, helper, การออกรหัสด้วย CLI และการกู้สิทธิ์อยู่บน loopback เท่านั้น

## เครือข่ายและเว็บอัปเดต

| Endpoint | ความหมาย |
| --- | --- |
| `GET /api/v1/local/network` | พอร์ตจริงและสถานะ `local_enabled`, `lan_enabled`, `lan_ready`, `lan_hosts` |
| `PUT /api/v1/local/network` | รับ `port`, `discovery_port`, `local_port`, `local_enabled`, `lan_enabled` จองพอร์ตใหม่ก่อนบันทึก ถ้าย้ายพอร์ตเว็บจะส่ง `code` สำหรับเปิดเว็บบนพอร์ตใหม่เมื่อช่องทางปัจจุบันยังเปิดอยู่ |
| `POST /api/v1/local/panel-reload` | ออกรหัสครั้งเดียวสำหรับโหลดเว็บรุ่นปัจจุบันด้วยเซสชันที่ยืนยันสิทธิ์แล้ว |
| `GET /api/v1/local/web-update` | สถานะ ประวัติ และรุ่นเว็บที่แยกจาก Agent |
| `POST /api/v1/local/web-update/check\|apply\|retry\|rollback` | คืน 202 และทำงานเบื้องหลัง `apply` รับ `{"version":"x.y.z"}` |

`GET /api/v1/local/update` มีฟิลด์ `web` และ `web_source` เพิ่มเติม `PUT /api/v1/local/update/settings` รับ `web_source` แบบไม่บังคับ เว้นว่างเพื่อใช้แหล่งเดียวกับ backend การปิดแต่ละช่องทางบล็อกหน้าเว็บ การแลกรหัส และการใช้ token ของช่องทางนั้น

หน้าเว็บมี CSP แบบ same-origin สำหรับสคริปต์ สไตล์ ฟอนต์และ API พร้อมป้องกันการฝังในเฟรม

ไฟล์สำรองของ panel ฝังมาจาก `frontend/dist/` หลัง Vite build โดยมี `index.html` และ assets ที่เรียกผ่าน `/synaptaconnect/assets/` เมื่อมีอัปเดตเว็บที่ผ่านการตรวจ Agent จะเสิร์ฟแพ็กเกจนั้น ไฟล์เว็บและฟอนต์เสิร์ฟในเครื่อง

## 2. Endpoint เดิม (ไม่เปลี่ยน)

`GET /api/v1/local/status` · `POST …/pairing/open` · `POST …/pairing/close` · `POST …/devices/{id}/revoke` · `POST …/kill-all`

`GET /api/v1/local/status` **เพิ่ม** ฟิลด์:
```json
{"agent": {"version": "0.3.0", "mode": "service|console", "account": "nt service\\synaptaconnect", "privilege": "standard|administrator|system|root",
           "pid": 1234, "uptime_secs": 3600},
 "screen_input": {"enabled": true, "helper_connected": true, "helper_user": "owner", "helper_session": 1}}
```
`helper_connected=false` ขณะ `enabled=true` = ผู้ใช้ยังไม่ล็อกอิน/ helper ไม่ทำงาน → การจับภาพ/ส่ง input จะตอบ error `-32004` (ดู §4)

## 3. การอัปเดต

### `GET /api/v1/local/update`
```json
{
  "current_version": "0.3.0",
  "state": "idle | checking | update_available | downloading | verifying | applying | up_to_date | failed | rolled_back | disabled | not_configured",
  "auto_update": true,
  "source": "github:owner/repo",
  "last_check_at": 1763600000,
  "available": {"version": "0.3.1", "notes": "…", "published_at": "2026-11-20T00:00:00Z"},
  "last_error": {"code": "signature_invalid", "message": "ข้อความภาษาไทยพร้อมแสดง", "stage": "verify", "version": "0.3.1", "at": 1763600100},
  "attempts": 1, "max_attempts": 3, "next_retry_at": 1763603700,
  "can_retry": true,
  "history": [{"at": 1763600100, "from": "0.3.0", "to": "0.3.1", "outcome": "failed", "stage": "verify",
               "code": "signature_invalid", "message": "…", "attempt": 1}]
}
```
`available`, `last_error`, `next_retry_at`, `last_check_at` เป็น `null` เมื่อไม่มี · `history` ใหม่สุดก่อน 20 รายการ
(รายการเต็ม: `GET /api/v1/local/update/history?limit=100`, สูงสุด 200) · `state` อธิบาย:
`idle` ยังไม่เคยตรวจ · `up_to_date` ตรวจแล้วเป็นรุ่นล่าสุด · `update_available` พบรุ่นใหม่ (`available` มีค่า) รอเจ้าของกดติดตั้ง
(หรือรอระบบติดตั้งเองถ้าเปิดอัตโนมัติ) · `checking`/`downloading`/`verifying` กำลังทำงาน (ชั่วครู่) ·
`applying` ส่งให้ตัวติดตั้งแล้ว กำลังเปลี่ยนรุ่น (service จะรีสตาร์ท — panel ต้องรอแล้ว poll ใหม่ ปกติไม่เกิน 1 นาที) ·
`failed` ล้มเหลวโดยรุ่นเดิมยังทำงานปกติ (ยกเว้น `rollback_failed` ซึ่งอันตราย) · `rolled_back` ติดตั้งรุ่นใหม่แล้วใช้ไม่ได้ ระบบกู้รุ่นเดิมกลับแล้ว ·
`disabled` ปิดอัปเดตอัตโนมัติ (ตรวจ/ติดตั้งด้วยมือยังได้) · `not_configured` ยังไม่ได้ตั้งแหล่ง release หรือ build นี้ยังไม่มี public key ของผู้เผยแพร่

`last_error.code` (ใช้ทำปุ่ม/คำแนะนำ; `message` เป็นไทยพร้อมแสดงเสมอ):

| code | ความหมาย | วิธีลองใหม่ที่ panel ควรแนะนำ |
|---|---|---|
| `source_unreachable` | ต่อแหล่ง release ไม่ได้ / ไม่มี release | ตรวจอินเทอร์เน็ต แล้วกดตรวจใหม่ |
| `manifest_invalid` | ไฟล์ manifest อ่านไม่ได้/ขาดฟิลด์ | รอ release แก้ไข (ลองใหม่ไม่ช่วย) |
| `signature_invalid` | ลายเซ็นไม่ตรง — **ไม่ติดตั้งเด็ดขาด** | แจ้งผู้เผยแพร่; อย่าลองซ้ำ |
| `version_not_newer` | รุ่นที่เสนอไม่ใหม่กว่ารุ่นที่ติดตั้ง (กัน downgrade) | – |
| `download_failed` / `hash_mismatch` | ดาวน์โหลดไม่ครบ/ไฟล์ไม่ตรง hash | กดลองใหม่ |
| `updater_unavailable` | เรียกตัวติดตั้งไม่ได้ (service ตัวติดตั้งหาย/ไม่มีสิทธิ์) | ติดตั้งโปรแกรมใหม่ด้วยสิทธิ์ผู้ดูแล |
| `apply_failed` | ตัวติดตั้งสลับไฟล์ไม่ได้ (ไฟล์ถูกล็อก/ดิสก์เต็ม) — ถ้าล้มหลังเริ่มสลับจะกู้เอง (`rolled_back`) | กดลองใหม่ได้ |
| `service_start_failed` / `health_check_failed` | รุ่นใหม่เริ่มไม่ขึ้น/ตรวจสุขภาพไม่ผ่าน → กู้รุ่นเดิมแล้ว (`state=rolled_back`) | กดลองใหม่ได้ (นับรวมเพดาน) |
| `interrupted_rolled_back` | ถูกขัดจังหวะ (ไฟดับ/ถูกปิดกลางทาง) → กู้รุ่นเดิมแล้ว | กดลองใหม่ |
| `rollback_failed` | **กู้ไม่สำเร็จ** — ต้องแก้ด้วยมือ | แสดงคำเตือนแดง + path ตัวติดตั้งรุ่นเดิมที่สำรองไว้ใน `message` |
| `retry_limit_reached` | ลองครบ `max_attempts` แล้วหยุดอัตโนมัติ | ปุ่ม "ลองอีกครั้ง" (ยืนยันก่อน) |
| `not_configured` / `disabled` | ดู state | – |

### การกระทำ
ทุกตัวทำงานเบื้องหลังและตอบ **`202` พร้อมสถานะตอนนั้น (รูปเดียวกับ `GET /update`)** แล้วให้ poll `GET /update` ทุก 1–2 วินาที ·
ที่ปฏิเสธทันทีตอบ `{"error":{"code","message"}}`: `400 invalid_request` (เช่น ไม่มี release รุ่นนั้น / ไม่มีงานให้ลองใหม่) ·
`409 busy` (มีงานอัปเดตอยู่) · `429 rate_limited` (ตรวจถี่เกิน) · `500 save_failed` (บันทึกค่าตั้งไม่ได้)
| endpoint | body | หมายเหตุ |
|---|---|---|
| `POST /api/v1/local/update/check` | – | ตรวจ release ตอนนี้ (ไม่ติดตั้ง) · เรียกถี่เกิน 1 ครั้ง/15 วินาที → `429` |
| `POST /api/v1/local/update/apply` | `{"version": "0.3.1"}` | ต้องตรงกับ `available.version` · ดาวน์โหลด → ตรวจ → ส่งให้ตัวติดตั้ง · ตอบ `202`; ผลดูจาก poll |
| `POST /api/v1/local/update/retry` | – | ใช้เมื่อ `can_retry=true` · ล้างตัวนับของรุ่นเป้าหมายแล้วทำเหมือน apply |
| `PUT /api/v1/local/update/settings` | `{"auto_update": bool, "source"?: "github:owner/repo" หรือ "https://host/path/" หรือ ""}` | บันทึกถาวร ตอบ `200` + สถานะใหม่ · `source` ว่าง = ใช้ค่าที่ฝังตอน build · ผิดรูป → `400 bad_source` |

`auto_update=false` = ไม่ตรวจและไม่ติดตั้งเอง; ปุ่ม check/apply ด้วยมือยังใช้ได้ ·
`source` ที่รับ: `github:OWNER/REPO` หรือ `https://…/` (โฟลเดอร์ที่มี `manifest.json`) เท่านั้น — แหล่งแบบโฟลเดอร์ในเครื่อง (`dir:`) ตั้งได้จาก CLI ของผู้ดูแลเท่านั้น (ใช้ทดสอบ)

## 4. จับภาพ/ส่ง input ใต้ session ที่ผู้ใช้อนุญาต

| endpoint | body/ผล |
|---|---|
| `GET /api/v1/local/screen-input` | `{"enabled", "helper_connected", "helper_user", "helper_session"}` |
| `PUT /api/v1/local/screen-input` | `{"enabled": bool}` → ปิด = ตัดการจับภาพ/input ทั้งหมดทันที (คำสั่งที่มาจากอุปกรณ์ได้ error `-32004`) |
| `GET /api/v1/local/commands` | `{"run_as_owner", "administrator", "helper_connected", "helper_user", "helper_elevated"}` (มีใน `status.commands` ด้วย ตั้งแต่ 0.3.14) |
| `PUT /api/v1/local/commands` | `{"run_as_owner"?: bool, "administrator"?: bool}` → เปิด administrator ขณะ helper ยังไม่ elevated = ขอ UAC บนหน้าจอทันที |

เมื่อใช้ผ่าน RPC แล้วใช้ไม่ได้ในโหมด service: error `-32004` `screen_input_unavailable` พร้อมข้อความไทยบอกสาเหตุ
(ปิดอยู่ / ไม่มี helper ใน session ที่อนุญาต / helper ตอบช้า) — ผู้ใช้ปลายทางของ API (Synapta Code) ควรแสดงข้อความนี้ตรง ๆ
`GET /api/v1/screen/stream` (MJPEG) **ไม่รองรับในโหมด service** → `400` `NOT_AVAILABLE` ให้ใช้ `screen.snapshot` ซ้ำแทน

## 5. สุขภาพ (สำหรับตัวติดตั้ง/CLI ไม่ใช่ panel)

`GET /api/v1/local/health[?nonce=<hex ≤64>]` (ไม่ต้องมี token แต่ต้องเป็น loopback + Host ถูกต้อง) → `{"ok": true, "version", "pid", "uptime_secs", "proof": "<hex>"|null}`
`proof` (มีเมื่อส่ง `nonce`) = HMAC-SHA256(local_token, "health" ‖ 0x00 ‖ `nonce|version|pid`) — ผู้ที่อ่าน local_token ได้ใช้ตรวจว่าคนตอบคือ agent จริง (ตัวติดตั้งและคำสั่ง `panel` ทำทุกครั้ง)

## 6. คำสั่งติดตั้ง (flag ที่ยืนยันแล้ว — ตรงกับ `backend/src/cli.rs`)

`synapta-connect.exe install [--plan] [--json] [--auto-update on|off] [--update-source SRC] [--web-update-source SRC] [--allow-screen-input] [--owner ชื่อ] [--no-start] [--port N] [--discovery-port N] [--local-port N] [--panel-cert PEM --panel-key PEM] [--allow-lan-panel]`

CLI **ตรวจค่าเองทุกครั้ง ไม่เชื่อสิ่งที่หน้าเว็บสร้างให้**: `--update-source` ใช้กฎเดียวกับ `PUT /update/settings` (`github:OWNER/REPO` หรือ `https://…/` ที่เป็น ASCII ที่พิมพ์ได้ ไม่มีช่องว่าง/อัญประกาศ/`<>\^`{|}`;
CLI ของผู้ดูแลรับ `dir:PATH` เพิ่ม ซึ่ง panel ตั้งไม่ได้) · `--owner` รับเฉพาะตัวอักษร/ตัวเลข `_ . - @ $` ช่องว่าง `\` ไม่เกิน 128 ตัว (ห้าม `: ( ) , ; ' " |` เพราะถูกต่อเป็น `ชื่อ:R` ให้ icacls) ·
`--auto-update` รับ `on`/`off` เท่านั้น · ตัวเลือกอื่น/ค่าที่ขาด → exit 2 พร้อมข้อความ
