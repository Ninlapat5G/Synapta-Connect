# Synapta Connect — API ของ agent (v1, as-built)

Agent คือโปรแกรม Rust ตัวเดียว (`synapta-connect.exe`) บนเครื่องปลายทาง เปิด **HTTPS + WSS บนพอร์ต 52401 (TLS 1.3 เท่านั้น ไม่มี plaintext)**
ผู้ใช้ API สองกลุ่ม: หน้าบ้าน (Frontend) และ Agent ของ Synapta Code (เรียกเป็น tool) — ใช้ช่องทางเดียวกัน

โค้ดอ้างอิง: `backend/src/api.rs` (ตาราง method ทั้งหมด), `backend/src/pairing.rs` (SPAKE2), `backend/tests/api.rs`
(**ตัวอย่าง client ที่ทำงานจริงครบทุกขั้น** — สร้างกุญแจ/ใบรับรอง, ปักหมุด host key, SPAKE2, mTLS, WSS)

## 1. ภาพรวม

```
ค้นหา (beacon/mDNS/probe) ─▶ จับคู่ครั้งแรก (TLS + SPAKE2 ด้วย OTP) ─▶ ใช้งานด้วย mTLS (ใบรับรอง Ed25519 ของ client)
```

| ช่องทาง | ใช้ทำอะไร | ต้องมีใบรับรอง client |
|---|---|---|
| `GET  https://IP:52401/api/v1/hello` | ตัวตนพื้นฐาน + fingerprint ของ host key | ไม่ |
| `POST https://IP:52401/api/v1/auth/spake2-handshake` | จับคู่ (สองขั้น) — เฉพาะตอนเปิดโหมดจับคู่ | ไม่ |
| `POST /api/v1/rpc` | JSON-RPC 2.0 ครั้งเดียวจบ (เหมาะกับ AI tool) | **ใช่** |
| `GET  /api/v1/ws` (`wss://`) | WebSocket: JSON-RPC + notification (ต้องใช้กับ PTY) | **ใช่** |
| `PUT  /api/v1/transfers/{id}?offset=N` | อัปโหลด chunk (binary) | **ใช่** |
| `GET  /api/v1/files?path=...` | ดาวน์โหลดไฟล์ (รองรับ `Range`) | **ใช่** |
| `GET  /api/v1/screen/stream` | MJPEG ถ่ายทอดหน้าจอสด | **ใช่** |
| `http://127.0.0.1:52402/api/v1/local/*` | หน้าจัดการบนเครื่องปลายทางเอง (§6) | ไม่ใช้ TLS — loopback เท่านั้น |

**ผู้เรียกต้องไม่ใช่เบราว์เซอร์**: ทุก request ที่มี header `Origin` ถูกปฏิเสธ (`403`) และไม่มี CORS —
เบราว์เซอร์แนบใบรับรอง Ed25519 แบบ self-managed ไม่ได้อยู่แล้ว ดังนั้นหน้าเว็บต้องคุยกับ Agent ผ่าน Synapta Code (ฝั่งเซิร์ฟเวอร์/เนทีฟ) ซึ่งเป็นตัวถือกุญแจ

เข้ารหัสข้อความ: **ข้อมูลทั่วไป (ภาพ, output ของ PTY, SPAKE2 message, confirm) = base64 มาตรฐาน**;
**public key = base64url ไม่มี padding**

## 2. ตัวตนและความเชื่อใจ

* **Host (เครื่องปลายทาง)**: คู่กุญแจ Ed25519 + ใบรับรอง self-signed สร้างครั้งเดียว (`host.crt`/`host.key` ใน data dir)
  Client **ปักหมุด public key** (ไม่ใช่ชื่อ/ใบรับรอง/CA): เทียบ public key ดิบ 32 ไบต์ของใบรับรองที่เซิร์ฟเวอร์ส่งมา กับค่าที่ปักหมุด
  `host_pubkey_fingerprint` = `"SHA256:" + hex(sha256(public key ดิบ))`
* **Client (Synapta Code)**: สร้างคู่กุญแจ Ed25519 เอง **เก็บ private key ใน keystore ของ OS** (agent ไม่เคยเห็น)
  ทำใบรับรอง self-signed ที่มี public key นั้นไว้ใช้เป็น client certificate ใน mTLS
* เนื้อหา TLS: **TLS 1.3 เท่านั้น**, ALPN `http/1.1` (WebSocket ต้องใช้ HTTP/1.1), ใบรับรองทั้งสองฝั่งต้องเป็น Ed25519
* **ชุด TLS ของฝั่ง client ต้องรองรับ Ed25519**: Node.js (OpenSSL) ✅ ทดสอบแล้วได้ `TLSv1.3 200`, Rust (rustls) ✅ (ใช้ใน `tests/api.rs`),
  Python/OpenSSL ✅ ส่วน **Windows Schannel ❌** (`curl.exe` ตัวที่มากับ Windows, .NET `HttpClient`, `Invoke-WebRequest`) ต่อไม่ได้ —
  ได้ `SEC_E_ILLEGAL_MESSAGE` เพราะ Schannel ไม่รองรับใบรับรอง Ed25519
* Agent ยอมเฉพาะ client ที่ public key อยู่ใน allowlist `paired_devices.json` — ใบรับรองอื่นถูกตัดตั้งแต่ TLS handshake
  client ที่ **ไม่แสดงใบรับรอง** ต่อเข้ามาได้แต่ใช้ได้เฉพาะ `hello` และ `spake2-handshake`

## 3. จับคู่ (SPAKE2)

OTP **ไม่เคยวิ่งบนสาย** — ใช้เป็นรหัสผ่านของ SPAKE2 (กลุ่ม Ed25519, crate `spake2` ของ RustCrypto)

**เปิดโหมดจับคู่**: ปิดอยู่เป็นค่าเริ่มต้น เปิดได้เมื่อเจ้าของเครื่องสั่งเท่านั้น (เปิดโปรแกรมด้วยมือ หรือ `POST /api/v1/local/pairing/open` §6)
OTP 8 หลัก สุ่มจาก OS (CSPRNG) อายุ **120 วินาที** ใช้ได้ครั้งเดียว — จับคู่สำเร็จแล้วโหมดจับคู่ปิดเอง

**ตัวระบุ SPAKE2** (ตายตัว): ฝั่ง client = `A`, identity `"synapta-connect client"`; ฝั่ง agent = `B`, identity `"synapta-connect host"`;
password = ไบต์ ASCII ของ OTP

### ขั้น 1 — `POST /api/v1/auth/spake2-handshake`
```json
{"step": 1, "client_msg": "<b64 ของข้อความ SPAKE2 ขาออกของ A>",
 "client_pubkey": "<b64url public key Ed25519 ดิบ 32 ไบต์ของ client>", "client_name": "Synapta Code (laptop)"}
```
→ `200 {"handshake_id": "…", "server_msg": "<b64 ข้อความ SPAKE2 ของ B>"}` — client เรียก `finish(server_msg)` ได้กุญแจร่วม `K`
(handshake_id ใช้ได้ครั้งเดียว อายุ 30 วินาที)

### ขั้น 2 — ยืนยันกุญแจ + ผูกกับ host key ที่เห็นบน TLS
```
client_confirm = HMAC-SHA256(key=K, "synapta-connect client" ‖ host_pubkey ‖ client_pubkey)   (ไบต์ดิบทั้งหมด 32+32)
```
```json
{"step": 2, "handshake_id": "…", "client_confirm": "<b64>"}
```
→ `200 {"device_id","host_pubkey","host_pubkey_fingerprint","host_confirm"}` โดย
`host_confirm = HMAC-SHA256(K, "synapta-connect host" ‖ host_pubkey ‖ client_pubkey)` — **client ต้องตรวจ `host_confirm`**
แล้วจึง **ปักหมุด `host_pubkey`** เก็บไว้ใช้ทุกครั้งต่อไป และ `host_pubkey` ในสูตรต้องเป็น key ที่ client เห็นจริงบน TLS ของ connection นี้
(ถ้ามีตัวกลางสวม TLS ด้วย key ของตัวเอง confirm จะไม่ผ่านแม้ OTP ถูก — จึงกัน MITM ได้ในขั้นจับคู่)

| ผล | HTTP | `error.code` |
|---|---|---|
| โหมดจับคู่ปิดอยู่ / OTP หมดอายุ | 403 | `pairing_closed` |
| confirm ไม่ผ่าน (OTP ผิด หรือ host key ที่เห็นไม่ตรง) | 403 | `invalid_otp` |
| ผิดครบ 5 ครั้ง **นับทั้งเครื่อง** → ทำลาย OTP + ปิดโหมดจับคู่ + ล็อก 60 วินาที (เปิดใหม่ในช่วงล็อกก็เริ่มไม่ได้) | 429 | `locked` (มี `retry_in_secs`) |
| handshake ค้างเกินเพดาน: IP เดียวค้างได้ 2 อัน, ทั้งเครื่อง 8 อัน (**ปฏิเสธตัวใหม่ ไม่ทิ้งตัวเก่า**) | 429 | `busy` |
| OTP ถูกเปลี่ยนระหว่างขั้น 1–2 | 409 | `pairing_changed` |
| handshake_id ไม่มี/หมดเวลา | 404 | `unknown_handshake` |
| ข้อมูลผิดรูป | 400 | `bad_key`, `bad_client_msg`, `missing_field`, `bad_step` |

การเดา OTP หนึ่งครั้ง = หนึ่งครั้งที่ขั้น 2 ไม่ผ่าน (การเริ่มขั้น 1 แล้วทิ้งไม่นับ) · handshake ค้างได้สูงสุด 8 อัน (IP ละ 2) อายุ 30 วินาที ·
การเดาผิดทุกครั้งถูกบันทึกพร้อม IP ต้นทางและชื่อที่ client แจ้ง (20 ครั้งล่าสุด) ให้เจ้าของดูที่ `GET /api/v1/local/status` → `pairing`

**กัน connection เปล่าแย่งช่อง (DoS)**: connection ที่ยังไม่ยืนยันตัว (กำลัง handshake หรือไม่มีใบรับรอง) ใช้ช่องแยกจากอุปกรณ์ที่จับคู่แล้ว —
ทั้งระบบ 64 ช่อง และ **IP ละไม่เกิน 8** (เกินถูกตัดทันที) · TLS handshake ต้องเสร็จใน 5 วินาที · connection ที่ไม่มีใบรับรองไม่ keep-alive
และมีอายุรวมไม่เกิน 10 วินาที (ใช้แค่ `hello` / จับคู่ ซึ่งจบใน request เดียว) · อุปกรณ์ที่จับคู่แล้วมีช่องของตัวเอง 256 connection

## 4. JSON-RPC 2.0 (`POST /api/v1/rpc` หรือเป็นข้อความ text บน `wss://…/api/v1/ws`)

```json
{"jsonrpc": "2.0", "id": 1, "method": "exec.run", "params": {"command": "hostname"}}
```
ตอบ `{"jsonrpc":"2.0","id":1,"result":…}` หรือ `{…,"error":{"code":-32602,"message":"…"}}` (HTTP `/rpc` ตอบ 200 เสมอเมื่อ mTLS ผ่าน)

| `error.code` | ความหมาย |
|---|---|
| -32700 | JSON อ่านไม่ได้ |
| -32601 | ไม่มี method นี้ |
| -32602 | params ผิด (รวมถึง path ที่ถูกปฏิเสธโดยกฎ path) |
| -32000 | ผิดพลาดภายใน (ข้อความบอกสาเหตุ) |
| -32001 | เรียกผิดช่องทาง (เช่น `pty.*` ผ่าน HTTP) |
| -32002 | สถานะไม่ตรง (offset ผิด, checksum ไม่ตรง, ไม่มี pty/transfer นั้น) |
| -32003 | ทรัพยากรเต็ม (stream พร้อมกันครบ 3 อัน) |
| -32004 | (โหมด service) จับภาพ/ส่ง input ไม่ได้: เจ้าของยังไม่อนุญาต / ไม่มี helper / helper ตอบช้า — ข้อความไทยบอกสาเหตุ |

HTTP นอก JSON-RPC (`/transfers`, `/files`, `/screen/stream`) ตอบ `{"error":{"code","message"}}` พร้อม status 400/409/429/500 ตาม code ข้างบน
route ที่ต้องมีใบรับรองแต่ไม่มี → `401 client_certificate_required`

### ความสอดคล้องกับ `docs/API_SCHEMA.md` §4
สเปกเขียนเป็น REST แยกตาม path; backend ทำเป็น RPC method บนช่องทางเดียว (พฤติกรรมเทียบเท่ากัน):

| ใน API_SCHEMA | ใน backend |
|---|---|
| `GET /system/info` | `system.profile` |
| `POST /terminal/exec` | `exec.run` |
| `wss …/terminal/pty` | `wss …/api/v1/ws` + `pty.open/write/resize/close` |
| `GET /screen/snapshot` | `screen.snapshot` (ผลเป็น base64 ใน JSON) |
| `POST /input/send` | `input.mouse_move / click / scroll / type / key` |
| `POST /files/upload` | `transfer.begin` → `PUT /transfers/{id}` → `transfer.finish` |
| `GET /files/download` | `transfer.stat` + `GET /api/v1/files?path=` |
| `POST /auth/spake2-handshake` | ตรงกัน (สองขั้น §3) |

### รายการ method

#### ระบบ
| method | params | result |
|---|---|---|
| `rpc.ping` | – | `{"pong": true}` |
| `system.profile` | `{"include_env"?: bool}` — env ไม่ส่งโดยปริยาย; ถ้าขอ จะได้ค่าจริงเฉพาะ `PATH USERPROFILE HOME SHELL LANG OS` ที่เหลือส่งชื่อพร้อม `"***"` | ดูล่าง |
| `system.network` | – | `[{"name","ip","kind"}]` — `kind` = `physical` \| `vpn` \| `virtual` (เดาจากชื่อ adapter ใช้แสดงผลเท่านั้น ไม่ใช้ตัดสินความเชื่อใจ) |

`system.profile` →
```json
{"hostname": "MSI",
 "os": {"name": "Windows", "version": "…", "kernel": "…", "arch": "x86_64"},
 "cpu": {"model": "…", "physical_cores": 8, "logical_cores": 16, "usage_percent": 7.5},
 "memory": {"total_bytes": 0, "available_bytes": 0},
 "disks": [{"name","mount","file_system","total_bytes","available_bytes","removable"}],
 "network": [ … ],
 "privilege": {"level": "standard|administrator|system|root", "user": "…"},
 "agent": {"version": "0.3.0", "pid": 1234},
 "env": {"PATH": "…", "OPENAI_API_KEY": "***"}}
```
`privilege.level` คือสิทธิ์ของโปรเซส agent — Windows: `standard` / `administrator` (elevated) / `system` (service); Unix: `standard` / `root`
**คำสั่งที่รันผ่าน agent ได้สิทธิ์เท่านี้** (OS เป็นตัวกั้น ไม่ใช่ agent กรองคำสั่ง)

#### รันคำสั่ง
`exec.run` — params: `command` (จำเป็น), `shell?` (`powershell` โดยปริยายบน Windows | `cmd` | `sh` โดยปริยายบน Unix | `bash`),
`cwd?`, `timeout_ms?` (โดยปริยาย 60000, สูงสุด 600000), `env?` (object), `stdin?` (ข้อความ)

result: `{"stdout","stderr","exit_code","timed_out","truncated","duration_ms"}` · **timeout ฆ่าทั้งต้นไม้โปรเซส** (Windows: Job Object, Unix: process group — ยังไม่ได้ทดสอบบน Unix) แต่คำสั่งที่จบเองและตั้งใจเปิดโปรแกรมทิ้งไว้เบื้องหลังจะไม่ถูกฆ่า · `exit_code` เป็น `null` เมื่อโดนฆ่าเพราะ timeout ·
stdout/stderr เก็บ 1 MiB แรกต่อ stream · ภาษาไทย/UTF-8 ผ่านได้ถูกต้อง

#### Terminal โต้ตอบ (PTY) — **เฉพาะ WebSocket**
| method | params | result |
|---|---|---|
| `pty.open` | `{"cols"?: 120, "rows"?: 30, "shell"?: "powershell.exe", "cwd"?}` | `{"pty_id"}` |
| `pty.write` | `{"pty_id", "data": "ข้อความที่พิมพ์ (ใช้ \r เป็น Enter)"}` | `{"ok":true}` |
| `pty.resize` | `{"pty_id", "cols", "rows"}` | `{"ok":true}` |
| `pty.close` | `{"pty_id"}` (ฆ่าโปรเซส) | `{"ok":true}` |

Notification จาก agent (ไม่มี `id`): `pty.output` `{"pty_id","data_b64"}` (ไบต์ดิบรวม ANSI) · `pty.exit` `{"pty_id","exit_code"}` มาหลัง output สุดท้ายเสมอ

PTY เป็นของ WebSocket ที่เปิด — **ปิด connection, revoke หรือ kill-all = ฆ่า PTY ทั้งหมด** · ต่อ connection เปิดได้ 8 อัน ·
**Windows (ConPTY) ส่ง `ESC[6n` (ขอตำแหน่งเคอร์เซอร์) มาตอนเริ่มแล้วรอคำตอบ** — ใช้ **xterm.js** (ส่ง `onData` เข้า `pty.write`) ได้เลยเพราะมันตอบให้เอง
ถ้าเขียน terminal เองต้องตอบ `ESC[<row>;<col>R` ไม่งั้น shell จะค้าง

#### ส่งไฟล์ (SHA-256, ต่อจากที่ค้างได้)
**อัปโหลด**
1. `transfer.begin` `{"path": "<absolute path>", "size": N, "sha256": "<hex>", "overwrite"?: false}`
   → `{"transfer_id", "offset", "chunk_size": 524288, "max_chunk_size": 1048576, "resumed": bool}`
2. `PUT /api/v1/transfers/{transfer_id}?offset=<offset>` body = ไบต์ดิบของ chunk (64 KiB–1 MiB) → `{"offset": <ไบต์ที่ได้รับรวมแล้ว>}`
   `offset` ต้อง **เท่ากับ** ที่ agent มีอยู่พอดี ไม่งั้น `409`
3. `transfer.finish` `{"transfer_id"}` → ตรวจ SHA-256 ทั้งไฟล์ แล้ว rename เป็นไฟล์จริงแบบ atomic → `{"path","size","sha256"}`
   checksum ไม่ตรง = error -32002 + ลบไฟล์ค้าง **และไม่แตะไฟล์เดิม** (ต้อง begin ใหม่ส่งทั้งไฟล์)
4. `transfer.abort` `{"transfer_id"}` ทิ้งงานที่ค้าง

**ต่อจากที่ค้าง (สัญญาณหลุด / agent restart)**: เรียก `transfer.begin` ด้วยพารามิเตอร์ชุดเดิม → ได้ `offset` ที่ค้างอยู่ (`resumed: true`) และ
`transfer_id` ใหม่ แล้วส่งต่อ — ไฟล์ที่ยังไม่ครบอยู่ที่ `<path>.synapta-part` (agent ใช้ขนาดไฟล์นี้เป็นความจริง ไม่เชื่อ offset จาก client)

**ดาวน์โหลด**: `transfer.stat` `{"path"}` → `{"size","sha256","modified"}` แล้ว `GET /api/v1/files?path=<URL-encoded>`
ต่อจากที่ค้างด้วย `Range: bytes=N-` (ตอบ `206`) แล้วเทียบ SHA-256 เอง

**กฎ path** (`backend/src/file_access.rs`): ทุก path ถูก `canonicalize` ก่อนใช้ (ตาม symlink, `.`, `..` แล้ว) ·
path เขียนต้องเป็น absolute, **ห้ามมี `..`**, ห้ามเป็น symlink · ถ้าเปิด agent ด้วย `--file-root <โฟลเดอร์>` (ซ้ำได้)
ทุก path ที่ canonical แล้วต้องอยู่ใต้ root นั้น ไม่งั้นถูกปฏิเสธ (`-32602` / `400`) — **ไม่ตั้ง `--file-root` = ไม่จำกัดโฟลเดอร์**
(เหมือน `exec` ที่เข้าถึงได้ทั้งเครื่องอยู่แล้ว; ขอบเขตจริงคือสิทธิ์ของ OS account ที่รัน agent)

#### หน้าจอ
| method | params | result |
|---|---|---|
| `screen.list` | – | `{"monitors": [{"id","name","x","y","width","height","scale_factor","is_primary"}], "windows": [{"id","pid","app","title","x","y","width","height","minimized","focused"}]}` |
| `screen.snapshot` | ดูล่าง | ดูล่าง |

`screen.snapshot` params (ไม่ใส่เลย = จอหลักทั้งจอ): `monitor?` (id จาก `screen.list`) · เลือกหน้าต่างด้วย `window_id?` / `window_pid?` /
`window_title?` (บางส่วน ไม่สนตัวพิมพ์; หน้าต่างที่ย่ออยู่ใช้ไม่ได้) · `format?` = `webp` (โดยปริยาย) | `png` | `jpeg` · `max_width?` · `quality?` 1-100 (เฉพาะ jpeg)

result: `{"format","mime","width","height","origin_x","origin_y","scale","data_b64"}`

**แปลงพิกัดภาพ → เมาส์** (สำคัญสำหรับ AI ที่ดูภาพแล้วสั่งคลิก): `screen_x = origin_x + pixel_x * scale`, `screen_y = origin_y + pixel_y * scale`
แล้วส่งให้ `input.*` — ภาพ, `screen.list` และ `input.*` ใช้ระบบพิกเซลจริงของ virtual desktop เหมือนกันหมด (แม้ Windows ตั้ง scale 125%)

เคล็ดลับ Vision Model: `webp`/`png` เป็น lossless (จอ 1080p ประมาณ 2–4 MB) — ใช้ `{"format":"jpeg","max_width":1280}` (ประมาณ 100 KB)

**ถ่ายทอดสด (คนดู)**: `GET /api/v1/screen/stream?fps=5&max_width=1280&quality=60[&monitor=…|&window_title=…]` →
`multipart/x-mixed-replace; boundary=frame` · fps สูงสุด 15 · พร้อมกันได้ 3 stream (เกิน → `429`) · หยุดเมื่อปิด connection / revoke / kill-all

#### จำลองเมาส์/คีย์บอร์ด
| method | params |
|---|---|
| `input.mouse_move` | `{"x", "y"}` (พิกัด virtual desktop) |
| `input.click` | `{"x"?, "y"?, "button"?: "left"\|"right"\|"middle", "double"?: bool}` (ไม่ใส่ x,y = ตำแหน่งเมาส์ปัจจุบัน) |
| `input.scroll` | `{"dy"?, "dx"?}` (บวก = ลง/ขวา) |
| `input.type` | `{"text"}` (รองรับ Unicode/ไทย) |
| `input.key` | `{"key", "modifiers"?: ["ctrl","alt","shift","meta"]}` — `key` = `enter tab escape backspace delete space up down left right home end pageup pagedown f1…f12` หรืออักขระตัวเดียว |

result ทุกตัว `{"ok": true}` · คำสั่ง input ทำทีละคำสั่งทั้งเครื่อง · ถ้าหน้าต่างที่โฟกัสรันด้วยสิทธิ์ admin แต่ agent เป็น standard user
Windows จะไม่ยอมรับ input (UIPI) · **ในโหมด Windows Service** (Session 0) การจับภาพ/ส่ง input ทำผ่าน helper ในเซสชันของเจ้าของเครื่องที่อนุญาตไว้เท่านั้น — ใช้ไม่ได้ตอบ error `-32004` พร้อมสาเหตุ (ดู `docs/SERVICE_AND_UPDATE.md` §4); MJPEG stream ไม่รองรับในโหมดนี้

#### อุปกรณ์ที่จับคู่ (ใช้ชื่อ `session` ตามเดิม)
| method | params | result |
|---|---|---|
| `session.list` | – | `{"current": "<device id ของผู้เรียก>", "sessions": [{"id","client_name","public_key","created_at","last_seen"}]}` |
| `session.revoke` | `{"session_id"?}` — ไม่ใส่ = ตัวเอง; **ใส่ได้เฉพาะ id ของตัวเอง** (id อื่น → error -32602) | `{"revoked": bool}` — ลบออกจาก allowlist และ **ตัด connection ของอุปกรณ์นั้นทันที** |

อุปกรณ์หนึ่งเพิกถอนอุปกรณ์อื่นไม่ได้ (กันอุปกรณ์ที่ถูกขโมยไปตัดเจ้าของตัวจริงออก) — เพิกถอนตัวอื่นทำที่หน้าจัดการบนเครื่องปลายทาง §6 เท่านั้น

`id` ของอุปกรณ์ = `hex(sha256(public key ดิบ))` ตัด 32 ตัวอักษรแรก ·
การเปิดโหมดจับคู่ (ออก OTP) ทำได้เฉพาะจากเครื่องปลายทางเอง (§6) ไม่มี RPC ให้สั่งจากระยะไกล

## 5. ค้นหาเครื่อง (Discovery)

| วิธี | รายละเอียด | ข้าม VPN? |
|---|---|---|
| **mDNS** | `_synapta-connect._tcp.local.` พอร์ต 52401, TXT: `device_id`, `fp`, `v` | ไม่ (multicast) |
| **UDP broadcast beacon** | ทุก 3 วินาที ไป broadcast ของทุก interface IPv4 แบบ physical, UDP พอร์ต **52400** | ไม่ (L2 เท่านั้น) |
| **UDP probe** | ส่ง `{"probe":"synapta-connect"}` แบบ unicast ไป `IP:52400` → ได้ beacon ตอบทันที | **ได้** (ทดสอบผ่าน Tailscale แล้ว) |
| **Direct IP** | ต่อ `https://IP:52401/api/v1/hello` ตรง ๆ | ได้ |

Beacon / probe reply — **มีแค่ 5 ฟิลด์นี้** (ไม่มี IP, hostname, OS, ระดับสิทธิ์):
```json
{"protocol":"synapta-connect/v1","device_id":"sc-node-7b2a9c4e1f03","host_pubkey_fingerprint":"SHA256:4a7e…","port":52401,"pairing_mode":true}
```
broadcast และ mDNS ออกเฉพาะ interface แบบ `physical` — IP ของ VPN/virtual adapter ไม่ถูกประกาศ (ปิดตามรายชื่อ adapter ตอนเริ่ม agent;
VPN ที่เพิ่งต่อทีหลังต้อง restart agent) · ใช้ `host_pubkey_fingerprint` เทียบกับค่าที่ปักหมุดไว้เพื่อรู้ว่าเป็นเครื่องเดิม

ข้อจำกัด: VPN แบบ L3 (Tailscale, WireGuard, Radmin) **ไม่ส่งต่อ multicast/broadcast** — ข้าม VPN ต้องรู้ IP ของ VPN ก่อน
(เจ้าของดูได้จาก console ของ agent หรือ `/api/v1/local/status`) แล้วใช้ probe/Direct IP · ใช้ชนิดเครือข่ายเลือกเส้นทางที่เร็ว ไม่ใช่เกณฑ์ความไว้ใจ (ยืนยันตัวด้วย mTLS เหมือนกันทุกเส้น)

## 6. หน้าจัดการบนเครื่องปลายทาง (`http://127.0.0.1:52402`, พอร์ตเปลี่ยนด้วย `--local-port`)

สำหรับ UI/tray บนเครื่องนั้น (ปุ่ม "Pairing Mode", ดู OTP, Revoke, "หยุดทันที") — **เป็น HTTP ธรรมดาบน loopback เท่านั้น ไม่เปิดออกเครือข่าย**
(จึงไม่ต้องเจอคำเตือนใบรับรองของเบราว์เซอร์) เรียกได้เมื่อ: ต่อจาก `127.0.0.1`, **แนบ header `X-Synapta-Local` = ข้อความใน `<data-dir>/local_token`** (สุ่มใหม่ทุกครั้งที่เริ่ม agent; ไฟล์จำกัดสิทธิ์ให้ผู้ใช้ที่รัน agent + SYSTEM + Administrators เท่านั้น — Windows ตั้ง ACL ให้เอง, Unix 0600)
ทำให้ process ของผู้ใช้อื่นบนเครื่องเดียวกันอ่าน OTP / เปิดโหมดจับคู่ไม่ได้ (ไม่งั้นผู้ใช้ธรรมดาจะจับคู่แล้ว exec ด้วยสิทธิ์ของ agent), `Host` เป็น
`127.0.0.1`/`localhost`/`[::1]`, และ **ไม่มี** `X-Forwarded-For`/`Forwarded`/`Via`/`X-Real-IP` — ไม่ครบ = `403` · ไม่มี CORS โดยตั้งใจ (กันเว็บอื่นอ่าน OTP)

| endpoint | ผล |
|---|---|
| `GET  /api/v1/local/status` | `{"device_id","name","host_pubkey_fingerprint","pairing_mode","otp","otp_expires_at","devices":[…],"addresses":[{"name","ip","kind"}],"pairing":{"wrong_guesses","max_wrong_guesses","locked_for_secs","recent_failures":[{"at","ip","client_name"}]}}` — `otp` เป็น `null` เมื่อปิดโหมดจับคู่; `pairing` บอกว่าใครกำลังเดา OTP |
| `POST /api/v1/local/pairing/open` | เปิดโหมดจับคู่ด้วย OTP ใหม่ (เรียกซ้ำ = ออก OTP ใหม่): `{"otp","expires_at"}` |
| `POST /api/v1/local/pairing/close` | ปิดโหมดจับคู่ทันที |
| `POST /api/v1/local/devices/{id}/revoke` | `{"revoked": bool}` ลบอุปกรณ์ + ตัด connection |
| `POST /api/v1/local/kill-all` | **"หยุดทันที"**: ตัดทุก connection ที่เปิดอยู่ (อุปกรณ์ยังจับคู่อยู่ ต่อใหม่ได้) |

### เพิ่มใน v0.3.0 (service + อัปเดต)
`GET /api/v1/local/status` มีฟิลด์ `agent` และ `screen_input` เพิ่ม · endpoint ใหม่ทั้งหมด (อัปเดต, สวิตช์จับภาพ/input, เปิด panel, health) และการยืนยันตัวของ panel
อยู่ที่ **`docs/PANEL_API.md`** · การเข้าถึงที่ loopback ใช้ token ของ panel (แลกจากรหัสครั้งเดียว) ได้ด้วย นอกจาก `local_token`

## 7. ความปลอดภัย — สถานะเทียบ `docs/SECURITY_REVIEW.md`

| ข้อ | สถานะ |
|---|---|
| 1 OTP ไม่วิ่งบนสาย (SPAKE2) | ✅ |
| 2 ส่ง host public key ให้ client ปักหมุด | ✅ (`host_pubkey` ใน step 2, ผูกกับ TLS ด้วย confirm) |
| 3 กัน brute-force (ปิดเป็นค่าเริ่มต้น, 120 วินาที, 5 ครั้งทั้งเครื่อง) | ✅ + ล็อก 60 วินาทีหลังทำลาย OTP |
| 4 ไม่มี bearer/JWT — mTLS, allowlist, revoke ตัด connection | ✅ |
| 5 TLS 1.3 / WSS เท่านั้น ไม่มี plaintext | ✅ (พอร์ต loopback ของหน้าจัดการเป็น HTTP — ไม่ออกเครือข่าย) |
| 6 beacon สะอาด | ✅ |
| 7 ปิดบัง env | ✅ |
| 8 ตรวจ path ด้วย canonicalize | ✅ (`--file-root` ถ้าต้องการจำกัดโฟลเดอร์) |
| 8 (review) helper ของ Windows Service | ✅ เขียนและทดสอบอัตโนมัติแล้ว (helper รันด้วยสิทธิ์เจ้าของ, ตัวตนผูกกับสิทธิ์อ่าน `local_token`); การตรวจกับ service จริงดู `docs/SERVICE_AND_UPDATE.md` §9 |
| 10 เขียนลง `.synapta-part` แล้ว rename, hash ผิดไม่แตะไฟล์เดิม | ✅ (ไม่ได้ทำ hash รายก้อน — ตรวจทั้งไฟล์ตอน finish) |
| 11 audit log | ⚠️ บันทึกลง log (stderr) ทุก method + pair/revoke/kill-all พร้อม device id; **ยังไม่ใช่ไฟล์ append-only หมุนตามขนาด** |
| 12 kill-all สำหรับปุ่มหยุดทันที | ✅ (UI เป็นงานหน้าบ้าน) |
| 13 bind เฉพาะ interface ที่เลือก | ❌ ยัง bind `0.0.0.0` (ความเชื่อใจไม่ขึ้นกับ IP อยู่แล้ว) |
| 14 ปฏิเสธ request ที่มี `Origin` | ✅ |

กุญแจ host (`host.key`) เก็บเป็นไฟล์ใน data dir (Unix สิทธิ์ 0600; Windows พึ่ง ACL ของโฟลเดอร์ผู้ใช้) — ยังไม่ได้ใส่ DPAPI/Keychain

## 8. รัน agent

**ติดตั้งเป็น service เบื้องหลัง (แนะนำ):** `synapta-connect.exe install` (Administrator) → ดู `docs/SERVICE_AND_UPDATE.md` · เปิดหน้าจัดการ: `synapta-connect.exe panel`
คำสั่งย่อย: `install` `uninstall` `service start|stop|status` `panel` — โหมดด้านล่างคือโหมด console เดิม (ไม่ใช่ service):

```
synapta-connect.exe [--port 52401] [--discovery-port 52400] [--local-port 52402]
                    [--data-dir PATH] [--name ชื่อที่แสดง] [--file-root PATH ...]
```
`--data-dir` เก็บ `host.crt`/`host.key`, `device_id`, `paired_devices.json` (โดยปริยาย `%LOCALAPPDATA%\SynaptaConnect`) ·
ระดับ log ด้วย `RUST_LOG` (โดยปริยาย `synapta_connect=info`) · ติดตั้งแบบ Standard User หรือ Administrator/SYSTEM คือเปิดโปรแกรมด้วยสิทธิ์นั้น · **ฟังเฉพาะ IPv4** (`0.0.0.0`) ยังไม่รองรับ IPv6 จึงไม่แสดง/ประกาศที่อยู่ IPv6 ·
`--port` ไม่เปลี่ยนพอร์ต discovery (UDP 52400) ต้องตั้ง `--discovery-port` เอง ·
เปิดโปรแกรมแล้วจะพิมพ์ host fingerprint, ที่อยู่ และ OTP แรก (เปิดโหมดจับคู่ให้ 120 วินาที) ที่ console
