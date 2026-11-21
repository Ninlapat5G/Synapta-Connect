# Synapta Connect — Security Review (Phase 0)

ผู้ตรวจ: Security & QA · วันที่: 2026-11-20
ขอบเขต: ตรวจ **แบบร่างการออกแบบ** ใน `README.md` และ `docs/API_SCHEMA.md` — ตอนนี้ยังไม่มีโค้ด Agent/Backend ให้ตรวจ
ผลทุกข้อจึงเป็น "สิ่งที่สเปกพาไปสู่ช่องโหว่" ไม่ใช่บั๊กในโค้ด ต้องตรวจซ้ำกับโค้ดจริงเมื่อ Backend ส่งงาน

**ระบบนี้ทำอะไร:** Agent (Rust daemon) บนเครื่องปลายทางเปิด port ให้ Synapta Code สั่งรันคำสั่ง shell, จับภาพหน้าจอ,
ควบคุมเมาส์/คีย์บอร์ด และรับส่งไฟล์ ผ่าน LAN หรือ VPN จับคู่ครั้งแรกด้วย OTP 6 หลัก

**สมมติฐานภัยคุกคาม:** ใครก็ได้ในวง LAN เดียวกัน (Wi-Fi ร้านกาแฟ, หอพัก, ออฟฟิศ) หรือใน tailnet เดียวกัน
ส่ง packet ถึง port 52400/52401 ได้ และดักฟัง/ปลอมตัวบน LAN ได้ (ARP spoofing) — ใครเข้า Agent ได้ = **รันโค้ดบนเครื่องนั้นได้เต็มสิทธิ์ของ Agent**
ฉะนั้นทุกข้อด้านล่างวัดว่า "คนแปลกหน้าในวง LAN ได้ shell หรือไม่"

---

## Must fix — ต้องแก้ในสเปกก่อน Backend เริ่มเขียน

### 1. OTP ถูกส่งตรง ๆ ทาง network → คนกลางขโมยไปจับคู่แทนได้
`API_SCHEMA.md §2` — `POST /api/v1/auth/pair` ส่ง `"otp": "849201"` ไปใน body
- **ปัญหา:** ตอนจับคู่ยังไม่มีกุญแจของอีกฝั่ง TLS จึงยืนยันไม่ได้ว่าคุยกับเครื่องจริง ผู้โจมตีในวง LAN
  ปลอม UDP beacon ด้วย `device_id` เดียวกัน (beacon ไม่มีลายเซ็น) → Synapta Code ต่อมาที่เครื่องผู้โจมตี
  และส่ง OTP ให้ → ผู้โจมตีเอา OTP ไปจับคู่กับเครื่องจริงภายในอายุ OTP → ได้ token = ได้ shell
- **แก้ (แนะนำ):** ห้าม OTP วิ่งบนสาย ใช้ OTP เป็นรหัสผ่านของ PAKE (เช่น SPAKE2 — crate `spake2` ของ RustCrypto)
  ทั้งสองฝั่งได้กุญแจร่วมเฉพาะเมื่อรู้ OTP ตรงกัน แล้วใช้กุญแจนั้นยืนยัน public key ของกันและกัน
  ผู้โจมตีที่อยู่ตรงกลางได้เดาแค่ครั้งเดียวต่อการจับคู่ (ซึ่งข้อ 3 จำกัดไว้แล้ว)
- **ทางเลือกที่ถูกกว่า:** เครื่องปลายทางแสดง OTP **คู่กับ fingerprint ของ host key** (เช่น 4 คำ) ให้ผู้ใช้เทียบ
  ง่ายกว่าแต่พึ่งผู้ใช้ตั้งใจเทียบ ซึ่งในทางปฏิบัติมักไม่ทำ
- **ถ้าข้าม:** ใครในวง Wi-Fi เดียวกันได้ shell ทันทีที่เจ้าของจับคู่เครื่อง

### 2. Response การจับคู่ไม่มี public key ของเครื่องปลายทาง → ปักหมุด (pin) ไม่ได้
`API_SCHEMA.md §2 Response` มีแค่ `session_token`, `device_name`
- **ปัญหา:** README บอกว่าใช้ Mutual TLS แต่ client ไม่เคยได้ host key ไปเก็บ ทุกการเชื่อมต่อครั้งต่อไปจึงยืนยันไม่ได้ว่า
  เป็นเครื่องเดิม — beacon ปลอม (ข้อ 1) ใช้ได้ตลอด ไม่ใช่แค่ตอนจับคู่
- **แก้:** เครื่องปลายทางสร้าง Ed25519 host key ตอนติดตั้ง ส่ง `host_pubkey` กลับในขั้นจับคู่ (ยืนยันผ่าน PAKE ข้อ 1)
  client เก็บไว้คู่ `device_id` แล้วทุกการเชื่อมต่อ TLS ต้องตรงกับ key นี้ ไม่ตรง = ตัดทันที

### 3. ไม่มีกฎกัน brute-force OTP
สเปกไม่พูดถึง TTL, จำนวนครั้ง, หรือช่วงเวลาที่รับการจับคู่
- **ปัญหา:** 6 หลัก = 1,000,000 แบบ ถ้า endpoint เปิดตลอดและไม่จำกัด สคริปต์ที่ยิง 1,000 ครั้ง/วินาที เจอภายใน ~17 นาที
  การจำกัดราย IP ไม่พอ เพราะใน LAN เปลี่ยน IP ได้เรื่อย ๆ
- **แก้ (กฎทั้งหมดอยู่ที่ Pairing Module ที่เดียว):**
  - endpoint จับคู่ **ปิด** โดยค่าเริ่มต้น เปิดเมื่อผู้ใช้กด "จับคู่อุปกรณ์ใหม่" บนเครื่องปลายทางเท่านั้น
  - OTP สุ่มจาก CSPRNG (`rand::rngs::OsRng`) ไม่ใช่ `thread_rng` ที่ seed เดาได้, อายุ 120 วินาที, ใช้ได้ครั้งเดียว
  - ผิด **รวม 5 ครั้ง (นับทั้งเครื่อง ไม่ใช่ราย IP)** → OTP ใบนั้นตาย ปิดโหมดจับคู่ ผู้ใช้ต้องกดใหม่
  - เทียบ OTP แบบ constant-time (`subtle::ConstantTimeEq`) — ถ้าใช้ PAKE ข้อนี้หายไปเอง
- โอกาสเดาถูกใน 5 ครั้ง = 1 ใน 200,000 ต่อการกดจับคู่หนึ่งครั้ง ยอมรับได้

### 4. Session token แบบ bearer อายุ 30 วัน และอาจเป็น JWT → ขโมยได้ ยกเลิกไม่ได้
`"session_token": "jwt_or_encrypted_token..."`, `"expires_in": 2592000`
- **ปัญหา:** (ก) ใครได้ token (log, clipboard, ไฟล์ config รั่ว) ได้ shell 30 วัน
  (ข) JWT แบบ stateless ยกเลิกก่อนหมดอายุไม่ได้ถ้าไม่มีรายการดำ — "Revoke" ที่โจทย์ต้องการจึงไม่ทำงานจริง
  (ค) มีทั้ง `client_pubkey` และ token = กลไกยืนยันตัวสองอันที่ต้องดูแลให้ตรงกัน
- **แก้ (ตัดของออก):** ไม่ต้องมี token ใช้ `client_pubkey` ที่ได้จากการจับคู่เป็น client certificate ของ mTLS เลย
  เครื่องปลายทางเก็บ allowlist ของ public key ที่จับคู่แล้ว (ไฟล์เดียว) → **Revoke = ลบ key ออกจากไฟล์ แล้วตัด connection ที่เปิดอยู่ของ key นั้น**
  มีผลทันที ไม่มีอะไรให้ขโมยนอกจาก private key ซึ่งเก็บใน keystore ของ OS ฝั่ง Synapta Code (DPAPI / Keychain / libsecret)

### 5. `ws://` และ HTTP ในสเปก → ข้อมูลและคำสั่งวิ่งเป็น plaintext
`API_SCHEMA.md §3.2` — `ws://{ip}:{port}/api/v1/terminal/pty`
- **ปัญหา:** ถ้า Backend ทำตามตัวอักษร คำสั่ง shell, output, ภาพหน้าจอ และไฟล์ ถูกอ่านและแก้กลางทางได้
- **แก้:** port 52401 พูด TLS 1.3 อย่างเดียว ไม่มี fallback เป็น plaintext, ทุก route (รวม pairing) อยู่หลัง TLS,
  WebSocket ใช้ `wss://` บน connection เดียวกัน, mTLS บังคับทุก route ยกเว้น pairing (ซึ่งเปิดเฉพาะตอนอยู่ในโหมดจับคู่ ข้อ 3)
  แก้สเปกให้เขียน `wss://` เพื่อไม่ให้ใครทำตามผิด

### 6. Beacon บอกคนแปลกหน้าว่าเครื่องนี้เป็น Administrator และ IP ของ VPN
`API_SCHEMA.md §1` — UDP broadcast ส่ง `"privilege": "administrator"`, `hostname`, IP ทั้ง LAN และ Tailscale โดยไม่ต้องยืนยันตัว
- **ปัญหา:** ใครฟัง port 52400 ก็ได้รายชื่อเป้าหมายที่คุ้มที่สุด (รัน admin) และ IP ภายใน tailnet ไปใช้ต่อ
- **แก้:** beacon ใส่แค่ `protocol`, `device_id`, fingerprint ของ host key, `port`, `pairing_required` ส่วน `privilege`, OS version,
  endpoints อื่น ให้ดึงผ่าน `system/info` หลังยืนยันตัวแล้ว

### 7. `system/info` คืน Environment Variables ทั้งหมด → secret รั่วไปถึง LLM และ log
`API_SCHEMA.md §3.1`
- **ปัญหา:** env มักมี `OPENAI_API_KEY`, `AWS_SECRET_ACCESS_KEY`, `GITHUB_TOKEN` ค่าเหล่านี้จะเข้า context ของโมเดล
  ถูกเก็บใน transcript และอาจถูกส่งต่อ
- **แก้:** คืนเฉพาะ **ชื่อ** ตัวแปร หรือคืนค่าเฉพาะ allowlist สั้น ๆ (`PATH`, `HOME`/`USERPROFILE`, `SHELL`, `LANG`, `OS`)
  ที่เหลือแสดงเป็น `"***"` (ถ้า agent ต้องการค่าจริงก็สั่ง `exec` ซึ่งผ่านเส้นที่บันทึก audit อยู่แล้ว)

### 8. โหมด Service บน Windows จะจับหน้าจอและส่ง input ไม่ได้ (และถ้าแก้ผิดจะกลายเป็นช่องยกระดับสิทธิ์)
README: "Rust Daemon / Service" + "Screen & Input Engine"
- **ปัญหา:** Windows Service รันใน Session 0 แยกจาก desktop ของผู้ใช้ — `connect_capture_screen` จะได้ภาพดำ, `send_input` ไม่ไปถึงหน้าต่างไหน
  ทางที่มักทำกันคือ service spawn helper เข้า session ผู้ใช้ด้วย token ของ SYSTEM — ถ้า helper นั้นเปิดช่อง IPC ให้ใครก็ต่อได้
  ผู้ใช้ธรรมดาบนเครื่องจะสั่งงาน SYSTEM ได้ (local privilege escalation)
- **แก้:** แยกให้ชัดในสเปก: helper ในเซสชันผู้ใช้รันด้วย **token ของผู้ใช้คนนั้น** (`WTSQueryUserToken` + `CreateProcessAsUser`)
  คุยกับ service ผ่าน named pipe ที่ ACL อนุญาตแค่ SYSTEM และ SID ของผู้ใช้คนนั้น — macOS/Linux มีเรื่องเดียวกัน (launchd agent vs daemon, Wayland ไม่ให้จับจอจากนอกเซสชัน)

---

## Should fix — ควรเขียนลงสเปกตอนนี้ เพราะแก้ทีหลังแพง

### 9. "โหมด User ป้องกันการเข้าโฟลเดอร์ระบบ" — ทำไม่ได้กับ shell และไม่ควรสัญญา
โจทย์ขอให้ตรวจว่าโหมด User กัน Agent เข้าโฟลเดอร์ระบบอย่างไร
- **ข้อเท็จจริง:** `exec` = รันอะไรก็ได้ในสิทธิ์ของผู้ใช้ การกรองคำสั่ง/path บน shell โดนเลี่ยงได้เสมอ (`cd`, alias, base64, สคริปต์)
  ขอบเขตจริงมีอย่างเดียวคือ **สิทธิ์ของ OS account ที่ Agent รันอยู่** — โหมด User ปลอดภัยกว่าเพราะ OS ห้ามเขียน `C:\Windows`, `/etc` เอง ไม่ใช่เพราะ Agent กรอง
- **แก้:** ค่าเริ่มต้นติดตั้งเป็นโหมด User; โหมด Admin/Service ต้องเลือกเองใน Installer พร้อมคำเตือนชัด และแสดงป้าย "Admin" ใน Device Manager
  path allowlist ใช้ได้กับ `files/*` เท่านั้น (ข้อ 10) อย่าเขียนในเอกสารว่ามันป้องกัน `exec` — ผู้ใช้จะเชื่อผิด
- Agent ต้องไม่มีคำสั่งใดที่ขอ UAC/sudo เอง (กัน prompt ปลอมให้ผู้ใช้กดอนุญาต)

### 10. File transfer รับ path จาก client ตรง ๆ → path traversal / เขียนทับไฟล์ระหว่างทาง
`connect_file_transfer(device_id, dir, src, dest)`, `POST /files/upload`
- **กรณี:** อัปโหลดไป `dest = "..\..\AppData\Roaming\Microsoft\Windows\Start Menu\Programs\Startup\x.bat"` หรือ symlink ที่ชี้ออกนอกโฟลเดอร์
  ถ้ามี allowlist โฟลเดอร์ ก็หลุดได้ถ้าเทียบ string แทน path ที่ canonicalize แล้ว
  และถ้าเขียนลง `dest` ตรง ๆ ระหว่างโอน ไฟล์จริงจะพังครึ่งไฟล์เมื่อสายหลุด
- **แก้:** (ก) canonicalize แล้วค่อยเทียบ allowlist (ถ้ามี) และเปิดไฟล์โดยไม่ตาม symlink
  (ข) เขียนลง `dest.partial` → ตรวจ SHA-256 ทั้งไฟล์ → `rename` ทับแบบ atomic; hash ไม่ตรง = ลบ `.partial` และไม่แตะไฟล์เดิม
  (ค) resume อิงจากขนาด `.partial` + hash ราย chunk ไม่ใช่เชื่อ offset จาก client อย่างเดียว

### 11. ไม่มีบันทึกว่า agent ทำอะไรไปบ้าง
- **ปัญหา:** เมื่อมีคน revoke เพราะสงสัย หรือมีไฟล์หายบนเครื่องปลายทาง ไม่มีทางรู้ว่า key ไหนสั่งอะไร เมื่อไหร่
- **แก้:** Agent เขียน audit log ฝั่งเครื่องปลายทาง (append-only, หมุนไฟล์ตามขนาด): เวลา, fingerprint ของ client key, route,
  คำสั่ง/ path (ไม่เก็บเนื้อไฟล์หรือ output), ผลลัพธ์ — จับคู่สำเร็จ/ล้มเหลวและ revoke ต้องอยู่ในนี้ด้วย

### 12. ผู้ใช้หน้าเครื่องไม่รู้ว่ากำลังถูกควบคุม และหยุดไม่ได้
- **กรณี:** agent สั่ง `send_input` ขณะผู้ใช้กำลังพิมพ์ → คลิกผิดหน้าต่าง, หรือ key ถูกขโมยไปแล้วถูกใช้ตอนกลางคืน
- **แก้:** ไอคอน tray เปลี่ยนสี/แสดงข้อความขณะมี session จับจอหรือส่ง input, และปุ่ม/คีย์ลัด "หยุดทันที" ที่ตัดทุก connection
  (เป็น UI งานของ Frontend; ฝั่ง Backend ต้องมีคำสั่ง kill-all ให้เรียก)

### 13. การแยก LAN vs VPN ต้องไม่ใช้เป็นเกณฑ์ความไว้ใจ
- **กรณี:** ถ้าโค้ดถือว่า IP ในช่วง Tailscale `100.64.0.0/10` "ปลอดภัยกว่า" ใครก็ตั้ง IP นั้นบน LAN ได้
- **แก้:** ใช้ชนิดเครือข่ายแค่เลือก endpoint ที่เร็วที่สุด การยืนยันตัวเหมือนกันทุกเส้นทาง (mTLS ข้อ 4–5)
  Agent bind เฉพาะ interface ที่ผู้ใช้เลือกใน Installer (ค่าเริ่มต้น: LAN ส่วนตัว + VPN) ไม่ใช่ `0.0.0.0` บน interface สาธารณะ

### 14. ช่องจากเบราว์เซอร์บนเครื่องเดียวกัน (DNS rebinding / CSRF)
- **กรณี:** หน้าเว็บที่ผู้ใช้เปิดยิง request ไป `https://192.168.1.120:52401/api/v1/auth/pair` ได้ — ถ้า pairing เปิดตลอด เว็บนั้นช่วย brute-force ได้
- **แก้:** ข้อ 3 (ปิด pairing โดยค่าเริ่มต้น) ปิดช่องนี้เกือบหมด เพิ่ม: ปฏิเสธ request ที่มี header `Origin` ทุกตัว (client ของเราไม่ใช่เบราว์เซอร์),
  ไม่ใช้ cookie, และ mTLS ทำให้เบราว์เซอร์ไม่มีใบรับรองไปยืนยันอยู่แล้ว

---

## Nice to have

### 15. โลโก้ PNG 10.8 MB ใน `frontend/assets/synapta-tech.png`
หน้า Installer/Dashboard จะโหลดช้าและตัวติดตั้งใหญ่โดยไม่จำเป็น — export เป็น WebP/SVG ขนาดใช้จริง (มักต่ำกว่า 100 KB)
และ README อ้าง path `media/Synapta Tech.png` ซึ่งไม่มีอยู่ ไฟล์จริงอยู่ที่ `frontend/assets/synapta-tech.png`

---

## สรุปการตัดสินใจที่ผมเสนอให้ Lead/เจ้าของงานยืนยัน

| เรื่อง | ทางที่แนะนำ | ทางเลือก | trade-off |
|---|---|---|---|
| การจับคู่ | **SPAKE2 ด้วย OTP** | แสดง OTP + fingerprint ให้ผู้ใช้เทียบ | SPAKE2 เพิ่ม crate หนึ่งตัวและโค้ด ~100 บรรทัด แต่กัน MITM โดยไม่พึ่งผู้ใช้; แบบเทียบ fingerprint ง่ายกว่าแต่ผู้ใช้มักข้าม |
| การยืนยันตัวหลังจับคู่ | **mTLS ด้วย Ed25519 key ที่จับคู่ไว้, ไม่มี token** | bearer token + revocation list | mTLS: revoke ทันที ไม่มี token ให้รั่ว; token: debug ด้วย curl ง่ายกว่า แต่ต้องทำรายการดำและเก็บ token ให้ปลอดภัย |
| สิทธิ์ค่าเริ่มต้น | **User mode** | Admin/Service | User mode จับจอได้ตรง ๆ และจำกัดความเสียหาย; Service รันก่อน login ได้แต่ต้องมี helper (ข้อ 8) |

Verdict: สเปกตอนนี้ปล่อยให้คนในวง LAN เดียวกันได้ shell ได้ (ข้อ 1–5) — แก้ส่วน pairing และ auth ในสเปกก่อน Backend เริ่มเขียนโค้ดส่วนนี้
Not checked: โค้ด Agent/Backend/Frontend (ยังไม่มี), การตั้งค่า build/installer, dependency ของ Rust — ต้องตรวจซ้ำเมื่อมีโค้ด โดยใช้ test case ใน `docs/TEST_PLAN.md §1`
