# Synapta Connect — service เบื้องหลัง + อัปเดตอัตโนมัติ (v0.3.0)

สถานะ: **เขียนและทดสอบอัตโนมัติแล้ว (73 เทสต์ผ่าน, 2 ข้ามเพราะต้องใช้ฮาร์ดแวร์) · ส่วนที่ต้องสิทธิ์ Administrator (ติดตั้ง/ถอน service จริง) ยังไม่ได้ตรวจกับเครื่องจริง — ดู §9**
สัญญากับหน้าบ้าน: `docs/PANEL_API.md` · ขั้นตอนเผยแพร่: `docs/RELEASE.md` · สัญญาอนุญาตส่วนประกอบ: `docs/THIRD_PARTY.md`

## 1. ภาพรวม

```
 เจ้าของเครื่อง ──synapta-connect panel──▶ เบราว์เซอร์ ─┐ http://127.0.0.1:52402/panel  (รหัสครั้งเดียว → token 8 ชม.)
                                                         │
 Synapta Code ══ TLS1.3 + mTLS (พอร์ต 52401) ═══▶  ┌─────▼──────────────────────────────┐
                                                   │ SynaptaConnect (service, Session 0)│  บัญชีเสมือน NT SERVICE\SynaptaConnect
                                                   │ จับคู่ · exec · ไฟล์ · PTY · panel API│  ไม่ใช่ admin ไม่ใช่ SYSTEM
                                                   └──┬───────────────┬─────────────────┘
        ws loopback + local_token                     │ ตรวจ/ดาวน์โหลด │ StartService (สิทธิ์ START อย่างเดียว)
   ┌──────────────────────────────┐                   │ + วางแพ็กเกจ    ▼
   │ helper (session ของเจ้าของ)   │◀─ screen.*/input.*│ staging   ┌───────────────────────────────┐
   │ รันด้วยสิทธิ์ผู้ใช้ จับภาพ/ส่ง input│                   ▼           │ SynaptaConnectUpdater (SYSTEM) │ ไม่มี listener
   └──────────────────────────────┘          GitHub Releases / https    │ ตรวจซ้ำ → สำรอง → สลับ → ตรวจสุขภาพ │ ตื่นตอนบูต+เมื่อถูกเรียก แล้วจบเอง
                                                                       │ ล้ม/ถูกขัดจังหวะ → กู้รุ่นเดิม       │
                                                                       └───────────────────────────────┘
```

exe ตัวเดียว (`synapta-connect.exe`) ทำหน้าที่ต่างกันตามคำสั่งย่อย: `service run` · `updater run` · `helper` · `panel` · `install` · `uninstall` · `service start|stop|status` · ไม่มีคำสั่ง = โหมด console เดิมของ Phase 0 (ไม่เปลี่ยนพฤติกรรม)

## 2. แบบจำลองสิทธิ์ (ข้อสำคัญที่สุดสำหรับผู้ตรวจ)

| ส่วน | รันเป็น | เขียนได้ที่ไหน | เครือข่าย |
|---|---|---|---|
| agent (`SynaptaConnect`) | `NT SERVICE\SynaptaConnect` (บัญชีเสมือน ไม่มีรหัสผ่าน ไม่ใช่ admin) | `%ProgramData%\SynaptaConnect\` (ยกเว้น `updater\` กับ `updater-status\`) เท่านั้น | TCP 52401 (TLS 1.3+mTLS), UDP 52400, loopback 52402, ขาออกไป GitHub/https ที่ตั้งไว้ |
| ตัวติดตั้ง (`SynaptaConnectUpdater`) | `LocalSystem` | `Program Files\SynaptaConnect\`, `updater\`, `updater-status\` เท่านั้น | **ไม่มี** (ไม่ listen ไม่ติดต่อออก; ตรวจสุขภาพผ่าน loopback) |
| helper | ผู้ใช้ที่ล็อกอิน (เจ้าของ) | – | loopback → agent |
| panel/CLI | ผู้ใช้ที่เรียก | – | loopback |

- **คำสั่ง `exec`/PTY/ไฟล์ ของ agent ทำงานด้วยสิทธิ์ต่ำของบัญชีเสมือน** (OS เป็นตัวกั้น) — ถ้าเจ้าของต้องการให้เข้าถึงโฟลเดอร์ผู้ใช้ ต้องให้สิทธิ์บัญชี `NT SERVICE\SynaptaConnect` ที่โฟลเดอร์นั้นเอง หรือใช้ `--file-root` ไม่มีทาง "ยกสิทธิ์เงียบ ๆ"
- agent ที่ถูกยึดทำได้อย่างมาก: ลงอัปเดตที่ **ผู้เผยแพร่เซ็นและใหม่กว่า** เท่านั้น (ตัวติดตั้งตรวจซ้ำเอง) · ตัวติดตั้งเชื่อ agent แค่ "เลขรุ่นที่ขอ + ไฟล์ใน staging" และ **อ่านอย่างเดียว** จากที่ที่ agent คุม (ไม่ลบ ไม่เขียน — กัน junction/symlink หลอกให้ SYSTEM แตะไฟล์อื่น) ผลย้อนกลับผ่านโฟลเดอร์ที่ agent อ่านได้อย่างเดียว
- ตัวติดตั้งตรวจลายเซ็นของ manifest ก่อนแตะแพ็กเกจ แล้วคัดลอกแพ็กเกจเข้าที่ของตัวเองแบบมีเพดาน (ขนาดต้องเท่าที่ manifest เซ็นไว้พอดี ≤ 150 MB — agent ที่ถูกยึดทำให้ไดรฟ์ระบบเต็มไม่ได้) แล้วตรวจจากสำเนา (กัน agent เปลี่ยนไฟล์หลังตรวจ) · อ่านรุ่นที่ติดตั้งจาก `install.json` ไม่ได้ = ปฏิเสธ (ไม่ข้ามการตรวจ downgrade)
- agent "เริ่ม" ตัวติดตั้งได้อย่างเดียว (SDDL ของ service ให้เฉพาะ `RP LC RC` กับ SID ของ agent) — หยุด/แก้ config ไม่ได้

### โครงไฟล์และ ACL

| path | เนื้อหา | ACL |
|---|---|---|
| `%ProgramFiles%\SynaptaConnect\` | `synapta-connect.exe`, `install.json` | ค่าเริ่มต้นของ Program Files (Users อ่าน/รันได้ เขียนไม่ได้) |
| `%ProgramData%\SynaptaConnect\` | `host.key/host.crt`, `paired_devices.json`, `settings.json`, `update-state.json`, `update-history.jsonl`, `panel_sessions.json` (SHA-256 ของ token ของ panel), `staging\`, `logs\` | service:Modify · SYSTEM · Administrators (ไม่มี Users) |
| `…\local_token` | token ของหน้าจัดการ (สุ่มใหม่ทุกครั้งที่ service เริ่ม) | service:F · SYSTEM · Administrators · **เจ้าของเครื่อง:อ่าน** |
| `…\updater-status\` | `result.json`, `history.jsonl` | SYSTEM · Administrators:F · service:**อ่านอย่างเดียว** |
| `…\updater\` | journal, สำรองรุ่นเดิม (`backup\<รุ่น>\`), สำเนาที่ตรวจแล้ว | SYSTEM · Administrators เท่านั้น |

## 3. Service: ชื่อ สถานะ การจัดการ

| ข้อกำหนด | ทำอย่างไร |
|---|---|
| ชื่อและสถานะจัดการได้ | ชื่อ `SynaptaConnect` (แสดง "Synapta Connect Agent") + `SynaptaConnectUpdater`; ดูผ่าน `services.msc` หรือ `synapta-connect service status [--json]` (รวมรุ่นที่ติดตั้ง, บัญชี, ผลตรวจสุขภาพ) |
| เริ่มอัตโนมัติ | `start=auto` (เริ่มตอนบูต ก่อนล็อกอิน) · ล้มกลางคัน → SCM รีสตาร์ท 3 ครั้ง ห่าง 60 วินาที นับใหม่เมื่อเสถียร 1 วัน |
| เริ่มตามการติดตั้งที่เจ้าของอนุมัติ | ติดตั้งต้องใช้ UAC/Administrator เสมอ (agent ไม่ขอ UAC เอง); ไม่มี service = ไม่เริ่มเอง |
| ปิด | `synapta-connect service stop` หรือ `sc stop SynaptaConnect` · ปิดชั่วคราวทุกการเชื่อมต่อ: ปุ่ม "หยุดทันที" (kill-all) ใน panel |
| ถอนการติดตั้ง | `synapta-connect uninstall [--keep-data]` — หยุด+ลบ service, ลบกฎ firewall, ลบ Run key, หยุด helper, ลบข้อมูลและโปรแกรม (`--keep-data` เก็บอุปกรณ์ที่จับคู่/กุญแจ host) ทำได้ซ้ำถ้าไม่ครบ |
| panel เปิดเมื่อผู้ใช้เรียก | ไม่มีอะไรเปิดเอง — `synapta-connect panel` (§ PANEL_API) |
| ติดตั้งล้มกลางทาง | ถอนสิ่งที่ลงไปแล้วทั้งหมด (exit 4) ไม่ทิ้งเครื่องครึ่ง ๆ กลาง ๆ |
| firewall | เปิด TCP 52401 / UDP 52400 **เฉพาะโปรไฟล์ private+domain** ผูกกับ exe — เครือข่าย public (Wi-Fi ร้านกาแฟ) ถูกบล็อกโดยปริยาย |
| คงการจับคู่/สิทธิ์เดิม | ไม่แตะ `pairing.rs`/`auth.rs`/`tls.rs` — เทสต์ SPAKE2/mTLS/revoke เดิมทั้งหมดยังผ่าน; service ไม่เปิดโหมดจับคู่เอง (เจ้าของสั่งที่ panel เท่านั้น ต่างจาก console ที่เปิดให้ 120 วินาทีแรก) |

exit code ของ `install`/`uninstall`: 0 สำเร็จ · 3 ไม่ได้รันด้วย Administrator · 4 ล้มเหลว · `--json` พิมพ์เหตุการณ์ทีละบรรทัด `{"step","status","detail"}` + บรรทัดสุดท้าย `{"result"}`

## 4. จับภาพ / ส่ง input ใต้ session ที่ผู้ใช้อนุญาต

- service อยู่ Session 0 จับภาพ/ส่ง input ไม่ได้ และ **ไม่ควรได้** (ถ้าทำด้วยสิทธิ์ SYSTEM ผู้ใช้ธรรมดาจะสั่งงาน SYSTEM ได้ — ข้อ 8 ของ SECURITY_REVIEW)
- `screen.*` / `input.*` ถูกส่งต่อให้ helper ที่รันใน session ของเจ้าของด้วยสิทธิ์ของเจ้าของ ผ่าน WebSocket loopback ที่ agent เปิดรับ (`/api/v1/local/helper`)
- **ตัวตนของ helper = การรู้ `local_token`** (ไฟล์ที่เฉพาะเจ้าของอ่านได้) ไม่ใช่สิ่งที่ helper อ้างเอง: ผู้ใช้อื่นอ่านไฟล์ไม่ได้ → helper ของเขาเลิกทำงานเงียบ ๆ · token ของ panel ใช้เป็น helper ไม่ได้ · ต่อได้ทีละ helper
- **พิสูจน์ตัวสองทาง โดยไม่ส่ง token ทางสาย** (`proof.rs`): helper ส่ง nonce สุ่ม + HMAC-SHA256(token, "helper-hello", nonce) → agent ตอบ HMAC(token, "helper-server", nonce) ในหัวข้อการอัปเกรด → helper **ตรวจก่อน** ส่ง hello หรือรับคำสั่งใด ๆ ไม่ผ่าน = ตัดทันที. เหตุผล: พอร์ต 127.0.0.1:52402 ใครบนเครื่องก็ bind ได้ตอน service หยุด/crash/กำลังสลับไฟล์/ก่อนบูตเสร็จ — ถ้า helper ไม่ตรวจ ผู้ใช้อื่นจะสั่งจับภาพ/คลิก/พิมพ์ใน session ของเจ้าของได้ (เช่น Win+R) · nonce ใช้ได้ครั้งเดียว (จำ 256 ตัวล่าสุด) กันเล่นซ้ำจากที่ดักเห็นบน loopback
- ตัวติดตั้งตรวจสุขภาพ agent ด้วย HMAC แบบเดียวกัน (`/local/health?nonce=` → `proof`) และคำสั่ง `panel` ตรวจก่อนส่ง token ให้ผู้ฟัง — โปรเซสอื่นที่ยึดพอร์ตหลอกว่ารุ่นใหม่ขึ้นแล้ว (ทำให้ไม่ rollback) หรือขโมย token ไม่ได้
- helper ทำเฉพาะ 7 method (`screen.list/snapshot`, `input.mouse_move/click/scroll/type/key`) — ที่เหลือถูกปฏิเสธฝั่ง helper เอง
- สวิตช์ `screen_input.enabled` (ปิดเป็นค่าเริ่มต้น; ตั้งตอนติดตั้งด้วย `--allow-screen-input` หรือที่ panel) ปิด = ทุกคำสั่งได้ `-32004` ทันที
- MJPEG stream **ไม่รองรับ**ในโหมด service → ใช้ `screen.snapshot` ซ้ำ
- ผู้ใช้อื่นที่ล็อกอินพร้อมกัน (Fast User Switching): helper ของเขาอ่าน token ไม่ได้ จึงไม่ถูกใช้ — การจับภาพ/ส่ง input จึงอยู่ใต้ session ของเจ้าของเท่านั้น ถ้าเจ้าของไม่ได้ล็อกอิน = `-32004` "ไม่มี helper"

## 5. อัปเดตอัตโนมัติ

### 5.1 รูปแบบ release (ต้องมี 3 ไฟล์ใน GitHub Release ล่าสุด หรือโฟลเดอร์ https/dir ที่ตั้งไว้)
`SynaptaConnect-vX.Y.Z.exe` · `manifest.json` (`version, asset, sha256, size, platform, notes, published_at`) · `manifest.json.sig` (Ed25519 hex ของ **ไบต์ดิบ** manifest.json)

### 5.2 แหล่ง release ตั้งค่าได้ + ทดสอบโดยไม่ต้องมี repo จริง
`github:OWNER/REPO` (ใช้ `releases/latest` — ไม่รับ draft/prerelease และรับเฉพาะ asset URL ใต้ repo นั้น) · `https://host/path/` · `dir:C:\path` (เฉพาะผู้ดูแลตั้งผ่าน CLI/`settings.json`; panel ตั้งไม่ได้) — ค่าเริ่มต้นฝังตอน build (`release/source.txt`, ว่างตอนนี้เพราะ repo ยังไม่ถูกสร้าง)

### 5.3 ปิดอัปเดตอัตโนมัติ
`settings.json` `update.auto_update=false` (panel สวิตช์ / `install --auto-update off`) — ไม่ตรวจ/ไม่ติดตั้งเอง; เจ้าของยังกดตรวจ/ติดตั้งเองได้

### 5.4 ขั้นตอนและการตรวจ
```
agent: ตรวจ manifest (ลายเซ็น→รูปแบบ→รุ่นใหม่กว่า) → ดาวน์โหลด (เพดาน = size ใน manifest) → SHA-256 → วางใน staging → เรียกตัวติดตั้ง
ตัวติดตั้ง: คัดลอกเข้าที่ตัวเอง → ตรวจซ้ำทั้งหมดจากสำเนา (ลายเซ็น, รุ่นต้อง > ที่ติดตั้ง, SHA-256, ขนาด)
          → สำรอง exe+install.json → จด journal → หยุด agent → rename สลับไฟล์ → เริ่ม agent
          → ตรวจสุขภาพ (รุ่นต้องตรง + โปรเซสเดิมอยู่ครบ 10 วินาที ไม่ crash-loop) → สำเร็จ/กู้คืน
```
- **ลายเซ็น**: public key ฝังตอน build (`release/pubkey.hex` หรือตัวแปร `SYNAPTA_RELEASE_PUBKEY`); ศูนย์ล้วน = ปิดการติดตั้งอัปเดตทั้งหมด (`not_configured`) — ปลอดภัยเป็นค่าเริ่มต้น
- **เวอร์ชัน**: `x.y.z` ตัวเลขล้วน ต้อง **สูงกว่า** ที่ติดตั้ง (กัน downgrade/replay ของ release เก่าที่เซ็นถูก) · ชื่อไฟล์ต้องตรงรุ่น (กัน path)
- **เก็บรุ่นเดิม**: `updater\backup\<รุ่น>\` เก็บ exe+install.json ของ 3 รุ่นล่าสุด · **ค่าตั้ง**: agent สำรอง `settings.json` ของตัวเองเป็น `settings.pre-update.json` ก่อนส่งคำขอ และใส่คืนเองเมื่อผลเป็น rolled_back/failed — ตัวติดตั้ง (SYSTEM) ไม่อ่าน/ไม่เขียน settings เลย

### 5.5 ตารางความล้มเหลว (ทดสอบทุกแถวด้วยเทสต์อัตโนมัติ ยกเว้นที่ระบุ)

| เหตุการณ์ | ผล | รุ่นที่ทำงาน | panel เห็น |
|---|---|---|---|
| แหล่งต่อไม่ได้ / ไม่มี release | ไม่ทำอะไร ตรวจใหม่ตามรอบ | เดิม | `failed` `source_unreachable` |
| ลายเซ็นไม่ตรง / manifest เสีย | ไม่ดาวน์โหลด/ไม่ติดตั้ง ใช้โควตาลองใหม่หมดทันที | เดิม | `signature_invalid` / `manifest_invalid` |
| รุ่นไม่ใหม่กว่า | ปฏิเสธ | เดิม | `version_not_newer` |
| ไฟล์ไม่ตรง hash / ใหญ่เกิน manifest | ลบ staging ไม่เรียกตัวติดตั้ง | เดิม | `hash_mismatch` / `download_failed` |
| ตัวติดตั้งเรียกไม่ได้ | ล้างงานค้าง | เดิม | `updater_unavailable` |
| รุ่นใหม่เริ่มไม่ขึ้น / ไม่ตอบ / ไม่เสถียร | **กู้รุ่นเดิม + ค่าตั้งเดิม** | เดิม | `rolled_back` `service_start_failed`/`health_check_failed` |
| ไฟดับ/ถูกปิดกลางการสลับไฟล์ | ตัวติดตั้งตื่นตอนบูต อ่าน journal แล้วกู้ | เดิม | `rolled_back` `interrupted_rolled_back` |
| กู้ไม่สำเร็จ | แจ้งดัง ๆ พร้อม path ตัวติดตั้งรุ่นเดิมที่สำรองไว้ | – | `failed` `rollback_failed` |
| ตัวติดตั้งเงียบเกิน 15 นาที | agent เลิกรอ ไม่เดาว่าสำเร็จ | – | `updater_unavailable` |

**จำกัดการลองใหม่** (กฎอยู่ที่เดียว `update/state.rs`): ต่อรุ่นเป้าหมายลองอัตโนมัติ **3 ครั้ง** ห่าง 1 ชม. แล้ว 6 ชม. · ผิดแบบถาวร (ลายเซ็น/manifest/เวอร์ชัน) นับครบทันที · เจ้าของกด "ลองอีกครั้ง" ได้เสมอ (เริ่มนับใหม่) · ตรวจด้วยมือจำกัด 1 ครั้ง/15 วินาที
**บันทึกผล**: `update-history.jsonl` (agent) + `updater-status\history.jsonl` (ตัวติดตั้ง) รวมแสดงใน panel (ใหม่สุดก่อน เก็บ ~200 รายการ) · log ของ service ใน `logs\agent.log`, `updater\logs\updater.log` (หมุนที่ 5 MB)

## 6. การตัดสินใจที่ทำแทน (เจ้าของไม่ได้ระบุ) และสมมติฐาน

| เรื่อง | ตัดสินใจ | ทางเลือกที่ไม่เอา / เหตุผล |
|---|---|---|
| บัญชีของ service | บัญชีเสมือน `NT SERVICE\SynaptaConnect` | LocalSystem (exec = SYSTEM ขัดกับ "จำกัดสิทธิ์"); บัญชีผู้ใช้ (ต้องเก็บรหัสผ่าน) |
| ผลต่อ exec/ไฟล์ | ใช้สิทธิ์ต่ำของบัญชีเสมือน — **เข้าโฟลเดอร์ผู้ใช้ไม่ได้ถ้าไม่ให้สิทธิ์** | ต้องแจ้งเจ้าของ: Phase 0 (console) เข้าได้ทั้งโปรไฟล์ผู้ใช้; โหมด service แลกด้วยความปลอดภัย |
| ตัวติดตั้ง | service แยก (SYSTEM) | ให้ agent ลงเองไม่ได้ (ไม่มีสิทธิ์ Program Files); เรียก UAC เองไม่ได้ (ห้ามตามสเปก) |
| ลายเซ็นแพ็กเกจ | Ed25519 บน manifest (กุญแจของผู้เผยแพร่ ฝังตอน build) | Authenticode อย่างเดียว: ต้องซื้อใบรับรอง และตรวจใน service ซับซ้อนกว่า — ยังทำเสริมได้ภายหลัง (ดู RELEASE.md) |
| installer UI | ไม่มีตัวติดตั้ง GUI แยก (ไม่มี Inno/WiX ในเครื่อง) — exe ติดตั้งตัวเองด้วยคำสั่ง; wizard ของหน้าบ้านสร้างคำสั่งให้ | MSI/NSIS: เพิ่ม toolchain + การเซ็นอีกชั้น ค่อยทำเมื่อมีผู้ใช้ทั่วไป |
| พอร์ต | service ใช้พอร์ตค่าเริ่มต้นเสมอ (52401/52400/52402) | ตั้งพอร์ตเองได้เฉพาะโหมด console |
| เจ้าของเครื่อง | = บัญชีที่รัน `install` (หรือ `--owner`) — ถ้า UAC ยกสิทธิ์ด้วย admin คนละบัญชีกับผู้ใช้ประจำ ต้องระบุ `--owner` | – |
| ผู้ใช้ Windows หลายคน | เฉพาะเจ้าของเปิด panel/helper ได้ | – |

## 7. ข้อจำกัดที่ทราบ (ตรงไปตรงมา)

1. **ยังไม่เคยลงตัวติดตั้งจริงบนเครื่องใด ๆ ในรอบนี้ถ้า §9 ไม่ระบุว่าตรวจแล้ว** — ตรรกะอัปเดต/กู้คืนทดสอบด้วย Host ปลอม; การทำงานของ SCM/ACL/firewall จริงตรวจได้ด้วย `backend/tests/admin/service-lifecycle.ps1` (ต้อง Administrator)
2. กุญแจเซ็นจริงยังไม่มี → build ปัจจุบันติดตั้งอัปเดตไม่ได้ (ตั้งใจ) จนกว่าผู้เผยแพร่ทำตาม RELEASE.md
3. repo ยังไม่ถูกสร้าง → ทดสอบ GitHub Releases จริงไม่ได้; ทดสอบแล้ว: การแยกวิเคราะห์ JSON ของ `releases/latest`, การกรอง URL, การปฏิเสธ draft/prerelease, และการเรียกจริงเมื่อ repo ไม่มี (ผล `source_unreachable`); **ไม่ได้ทดสอบ**: ดาวน์โหลด asset ผ่าน redirect ของ GitHub จริง
4. ตรวจเฉพาะ Windows x86_64 · ไม่มี Authenticode · SmartScreen อาจเตือนไฟล์ที่ดาวน์โหลดมาใหม่ (ไม่เซ็น)
5. ไม่มี MJPEG ในโหมด service · helper ต่อทีละตัว (เจ้าของคนเดียว)
6. กุญแจ host (`host.key`) ยังเป็นไฟล์ (ACL จำกัด) ไม่ใช่ DPAPI — เหมือน Phase 0 แต่ตอนนี้อยู่ใต้ ACL ของ service
7. IPv4 เท่านั้น · bind `0.0.0.0` (กันด้วย firewall profile + mTLS) — เหมือน Phase 0
8. audit log ยังเป็น log ไฟล์ ไม่ใช่ append-only หมุนตามสเปก (SECURITY_REVIEW ข้อ 11) — ดีขึ้นเล็กน้อยเพราะ service เขียนลง `logs\agent.log`
9. ตัวติดตั้งตรวจสุขภาพผ่านพอร์ต 52402 ที่ตายตัว — ถ้ามีโปรแกรมอื่นจองพอร์ตนี้ agent เริ่มไม่ขึ้น → รุ่นใหม่จะถูกกู้ (และรุ่นเดิมก็เริ่มไม่ขึ้นเช่นกัน แจ้ง `rollback_failed`)

## 8. การทดสอบ

| ไฟล์ | ครอบคลุม |
|---|---|
| `src/update/*` (หน่วย) | เวอร์ชัน, ลายเซ็น/manifest ปลอม, แหล่ง release (3 แบบ), กฎลองใหม่, ประวัติ |
| `src/update/installer/tests.rs` | สลับสำเร็จ · ตรวจไม่ผ่านไม่แตะอะไร · กู้เมื่อไม่เริ่ม/ไม่ตอบ/crash-loop · ขัดจังหวะหลังสลับ · ขัดจังหวะจน exe หาย · กู้ไม่ได้ · ไม่เขียน/ลบในที่ที่ agent คุม · ไม่ทำคำขอซ้ำ |
| `src/update/tests.rs` | ตัวประสานฝั่ง agent ทั้งวงจร, ของปลอม, ลองใหม่/เพดาน, ตัวติดตั้งตาย, ใส่ค่าตั้งเดิมคืน |
| `tests/service.rs` | panel auth (รหัสครั้งเดียว, Host/Origin), API อัปเดตผ่าน HTTP จริง, ส่งงานจับภาพ/input ผ่าน helper (ปิด/เปิด/หลุด/ตัวที่สอง/token ผิดชนิด) |
| `tests/release_tool.rs` | keygen → package → verify ด้วย exe จริง + ปฏิเสธรุ่นไม่ตรง/คีย์อื่น/ไฟล์ถูกแก้ |
| `tests/api.rs` (เดิม) | จับคู่ SPAKE2, mTLS, revoke, exec, ไฟล์, PTY, หน้าจัดการ — ยังผ่านทั้งหมดหลังย้าย `local.rs` |
| `tests/admin/service-lifecycle.ps1` | ติดตั้ง/สิทธิ์/ACL/firewall/อัปเดตสำเร็จ/อัปเดตล้ม→กู้/ขัดจังหวะ→กู้/ถอน — **ต้อง Administrator** |

## 9. หลักฐานการตรวจ (ณ 2026-11-21) — ตรวจแล้วอะไร ยังไม่ได้ตรวจอะไร

**ตรวจแล้ว (รันจริง):**
- `cargo test`: 73 ผ่าน / 0 ล้ม / 2 ข้าม (เทสต์เดิมที่ต้องใช้จอ/multicast) — รวมเทสต์ Phase 0 เดิมทั้งหมดที่ยังผ่านหลังย้าย local router · `cargo clippy --all-targets` สะอาด · `cargo fmt --check` สะอาด · build แบบ release สำเร็จ (exe ไม่พึ่ง VCRUNTIME140.dll)
- เปิด agent จริง (โหมด console) แล้วเรียก HTTP จริง: panel รหัสครั้งเดียว→token, ใช้รหัสซ้ำ = 403, Origin ต่าง origin = 403, หน้า `/panel/` พร้อม CSP, API อัปเดต/ค่าตั้ง, ปฏิเสธ `dir:`
- เรียก **GitHub จริงผ่าน TLS** (rustls + platform verifier): repo จริงที่ไม่มี manifest → `source_unreachable "release ล่าสุดไม่มีไฟล์ manifest.json"`; repo ที่ไม่มี → `404` ตามที่ออกแบบ
- คำสั่งที่ไม่ต้อง admin ของ exe จริง: `install --plan` (ไม่แก้เครื่อง), `install`/`uninstall` ที่ไม่ใช่ admin → exit 3, `service status` ตอนยังไม่ติดตั้ง, `service run` จากมือ → ปฏิเสธ, `helper` ที่อ่าน token ไม่ได้ → จบเงียบ exit 0
- ขั้น build ของสคริปต์ admin (สร้างกุญแจทดสอบ, build 2 รุ่นฝังกุญแจทดสอบ, สร้าง release ดี/ใช้ไม่ได้/ลายเซ็นปลอม) รันจริงผ่านจนถึงก่อนติดตั้ง

**ยังไม่ได้ตรวจ (ต้อง Administrator — ผมไม่มีสิทธิ์ และเจ้าของยกเลิกหน้าต่าง UAC ที่ผมเรียกขึ้นมาหนึ่งครั้ง):**
- ส่วนหลังจากขั้นติดตั้งใน `backend/tests/admin/service-lifecycle.ps1` ทั้งหมด: SCM สร้าง/เริ่ม/หยุด/ลบ service จริง, บัญชีเสมือน `NT SERVICE\SynaptaConnect` เขียน `local_token` ในโฟลเดอร์ ProgramData และตั้ง ACL ให้เจ้าของอ่านได้จริงหรือไม่ (**จุดเสี่ยงสูงสุด**: ตัวติดตั้งสร้าง `local_token` พร้อม ACL ให้เจ้าของอ่านไว้ก่อน แล้ว service เติมเนื้อหาทับ (ไม่แก้ ACL เพราะบัญชีสิทธิ์ต่ำทำไม่ได้) — ถ้า service เขียนทับไม่ได้หรือ ACL ไม่ตรง panel จะเปิดไม่ได้; ดูข้อ "local_token: เจ้าของอ่านได้" ในรายงาน), ACL ของ `updater\` และ `updater-status\`, กฎ firewall (การส่ง `program=` ที่มีช่องว่างให้ netsh), Run key, SDDL ของตัวติดตั้ง, อัปเดตสำเร็จ/ล้ม→กู้/ขัดจังหวะ→กู้ กับ service จริง, ถอนการติดตั้งรวมโฟลเดอร์โปรแกรม
- helper กับ session ผู้ใช้จริง (จับภาพ/คลิกจริงผ่าน helper) — ทดสอบแล้วเฉพาะตัว routing ด้วย helper ปลอม
- สคริปต์ admin ตัวเองไม่เคยรันถึงตอนท้าย: เคยพบและแก้บั๊กของสคริปต์ 1 จุด (PowerShell 5.1 ตีความ stderr ของ cargo เป็น error ร้ายแรง) — อาจเหลือบั๊กในขั้นหลังติดตั้งที่ยังไม่เคยรัน ผู้รันควรอ่านบรรทัด `[FAIL]` แล้วแยกว่าเป็นบั๊กของระบบหรือของสคริปต์

เปิด PowerShell แบบ Administrator จากโฟลเดอร์โปรเจกต์แล้วรัน `powershell -ExecutionPolicy Bypass -File backend\tests\admin\service-lifecycle.ps1` บนเครื่องทดสอบสะอาด สคริปต์เก็บ `report.txt` ในโฟลเดอร์ใหม่ `%TEMP%\synapta-admin-test-<id>` และแสดงตำแหน่งเมื่อจบ
