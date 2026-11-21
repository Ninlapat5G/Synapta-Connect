# Synapta Connect — เอกสารส่งต่องาน v0.3.5

ภาพรวมการออกแบบ: `docs/SERVICE_AND_UPDATE.md` · สัญญา panel: `docs/PANEL_API.md` · การเผยแพร่: `docs/RELEASE.md` · license ส่วนประกอบ: `docs/THIRD_PARTY.md`

## 1. สถานะ

### รอบ v0.3.5 (2026-11-22) — รุ่นแรกที่อัปเดตตัวเองได้

- repo เปิดเป็น public แบบสงวนสิทธิ์ (LICENSE ใส่ชื่อ Ninlapat5G) เพื่อให้ตัวอัปเดตอ่าน GitHub Releases ได้โดยไม่ล็อกอิน
- ฝัง public key `166f2acdfe579251700c9e291fed6e77f614b2204f8e45adb2c1423ed588389d` และแหล่ง `github:Ninlapat5G/Synapta-Connect` ใน `backend/release/`
  กุญแจเซ็นอยู่ที่ `C:\Users\MSII\.synapta-release\synapta-release.key` บนเครื่อง dev (นอก repo) ต้องมีสำเนาสำรองอีกที่
- เครื่องที่เป็น v0.3.4 อัปเดตเองไม่ได้ (ไม่มีกุญแจ) ต้องเปิด `SynaptaConnectSetup-v0.3.5.exe` แล้วกด **อัปเดตเป็น v0.3.5** หนึ่งครั้ง จากนั้นใช้ **ตรวจอัปเดต** หรือ **อัปเดตอัตโนมัติ** ใน panel
- หน้าขอสิทธิ์ของ panel คัดลอกคำสั่งที่วางใน PowerShell ได้ทันที (`& "$env:ProgramFiles\SynaptaConnect\synapta-connect.exe" panel`) และแยกข้อความลิงก์หมดอายุ โหลดหน้าใหม่ และเซสชันหมดอายุออกจากกัน
- เทสต์สองตัวที่ถือว่า public key ต้องเป็นศูนย์ ปรับให้ไม่ขึ้นกับค่าที่ฝังตอน build
- **ยังไม่ได้ทดสอบ:** อัปเดตจริงจาก v0.3.5 ไปรุ่นถัดไป (ทำได้เมื่อออก v0.3.6 ด้วยกุญแจเดิม) และ `upgrade` ทับ v0.3.4 บน app01

### รอบ v0.3.4 (2026-11-21) — อัปเดตทับจากตัวติดตั้ง + แก้ถอนการติดตั้งรายงานผิด

- บนเครื่องจริง ปุ่มถอนการติดตั้งขึ้น "ถอนการติดตั้งไม่สำเร็จ (รหัส 0)" ทั้งที่ถอนสำเร็จ: ตัวติดตั้งเรียก uninstall จาก exe ใน Program Files
  ซึ่งลบตัวเองไม่ได้ทันทีจึงตั้งเวลาลบตามหลัง แต่หน้าต่างตรวจไฟล์ทันที — แก้ให้ทุกคำสั่งที่ยกสิทธิ์รันจาก agent ที่ฝังในตัวติดตั้ง และรอไฟล์หายก่อนตัดสิน
- เพิ่ม `synapta-connect upgrade` (ผู้ดูแล): หยุด service/helper, สำรอง exe, คัดลอกตัวเองทับ, เริ่มและรอ health 40 วินาที ไม่ขึ้น = ใส่รุ่นเดิมคืน
  ปฏิเสธการลดรุ่นและการรันจากตัวที่ติดตั้งอยู่ คงข้อมูล การจับคู่ ค่าตั้ง และ ACL ทั้งหมด
- ตัวติดตั้งอ่านรุ่นที่ติดตั้งด้วย `--version` ถ้าเก่ากว่าจะมีปุ่ม "อัปเดตเป็น vX"
- **ยังไม่ได้รัน `upgrade` จริง** (เซสชันนี้ไม่มีสิทธิ์ผู้ดูแล) ตรวจแค่ทางที่ไม่ใช่ผู้ดูแล (exit 3) และหน้าจอของตัวติดตั้ง — ต้องลองบนเครื่องปลายทาง

### รอบ v0.3.3 (2026-11-21) — ผลทดสอบบน Windows Server 2016 จริง + แก้ helper

ติดตั้ง v0.3.2 ด้วยตัวติดตั้งแบบหน้าต่างบนเครื่องปลายทางจริง (app01, Windows Server 2016 Standard build 14393, เข้าผ่าน Radmin VPN)
แล้วทดสอบจากเครื่อง dev ด้วย client อิสระ (SPAKE2 + mTLS):

| ทดสอบ | ผล |
|---|---|
| จับคู่ด้วย OTP, ลายนิ้วมือ host ตรงกับ beacon ของ UDP probe | ผ่าน |
| service รันบน 14393 (ยืนยันว่าแก้ import ของ DPI แล้ว exe เปิดได้) | ผ่าน |
| `system.profile`, `exec.run` (cmd/PowerShell 5.1, ภาษาไทยใน stdout) | ผ่าน |
| `pty.open` ผ่าน WinPTY (cmd และ PowerShell, resize, output, exit code) | ผ่าน |
| service ตั้ง AUTO_START + รีสตาร์ทเองเมื่อพัง (60 วินาที x3) | ยืนยันด้วย `sc qc` / `sc qfailure` |
| `screen.*` / `input.*` | ไม่ผ่าน: ไม่มี helper ใน session ของเจ้าของ |

สาเหตุที่ไม่มี helper: Run key เริ่ม helper ตอนล็อกอินเท่านั้น ติดตั้งขณะเจ้าของล็อกอินค้างอยู่จึงไม่มี helper จนล็อกอินรอบหน้า
v0.3.3 แก้ให้ `synapta-connect panel` (ซึ่งตัวติดตั้งเรียกตอนจบ และรันด้วยบัญชีเจ้าของ) เริ่ม helper ใน session นั้น
และ helper ใช้ mutex `Local\SynaptaConnectHelper` ให้มีได้ตัวเดียวต่อ session (ตัวที่สองถูก service ปฏิเสธและจะวนต่อใหม่ไม่จบ)
ยังไม่ได้ยืนยันการจับภาพ/คลิกบนเครื่องจริง — เครื่องที่ลง 0.3.2 แล้ว ให้ล็อกเอาต์/ล็อกอินใหม่ หรือติดตั้ง 0.3.3 ทับ

ข้อสังเกต: `system.profile` รายงาน `privilege.level = administrator` ทั้งที่ service รันด้วย `NT SERVICE\SynaptaConnect` ยังไม่ได้ตรวจว่าแสดงผิดหรือได้สิทธิ์จริง


### รอบ v0.3.2 (2026-11-21) — รองรับ Windows Server 2016 + ตัวติดตั้งแบบหน้าต่าง

เครื่องปลายทางจริงเป็น Windows Server 2016 (เทียบเท่า Windows 10 1607) รอบนี้แก้สามเรื่องที่ทำให้ใช้บนเครื่องนั้นไม่ได้

1. **exe เปิดไม่ขึ้นบน Server 2016**: `platform.rs` ลิงก์ `SetProcessDpiAwarenessContext` (มีตั้งแต่ 1703) แบบตรง Windows 1607 จึงไม่ยอมโหลด exe เลย
   แก้เป็นหาแบบ dynamic และสำรองด้วย `shcore!SetProcessDpiAwareness` ตรวจ import table ของ exe ใหม่ด้วย `dumpbin /imports` แล้วไม่มี API ที่ใหม่กว่า 1607
2. **Terminal (PTY)**: ConPTY มีตั้งแต่ 1809 จึงเพิ่ม `src/legacy_pty.rs` ใช้ WinPTY เมื่อไม่มี ConPTY ไฟล์ winpty ฝังใน exe แตกลง `<data-dir>\winpty\<hash>\`
   และเทียบไบต์กับของที่ฝังก่อนโหลดทุกครั้ง เทสต์เรียก WinPTY จริงบนเครื่อง dev (เปิด shell, ปรับขนาด, output, exit code, kill)
3. **หน้า panel เปิดใน IE แล้วว่าง**: Server 2016 ตั้ง IE เป็นเบราว์เซอร์เริ่มต้น `synapta-connect panel` จึงข้าม IE ไปใช้ Chrome/Edge/Firefox ที่ติดตั้งไว้ ถ้าไม่มีจะบอกให้ติดตั้ง

และย้ายตัวติดตั้งแบบหน้าต่างจากโฟลเดอร์ทดลองของ Codex (`SynaptaCode\lab_connect_installer`) มาไว้ที่ `installer/windows/` ออกแบบใหม่ด้วยโลโก้
ฟอนต์ Anuphan/Chakra Petch ที่ฝังมา และสีของ panel มีหน้าถอนการติดตั้งเมื่อเครื่องลงไว้แล้ว `scripts/build-release.ps1` สร้างตัวติดตั้งคู่กับ backend ทุกครั้ง

ผลตรวจ: `cargo test` 81 ผ่าน / 2 ข้าม (2 ตัวเดิมที่ต้องมีจอ/multicast) · clippy และ fmt สะอาด · ตัวติดตั้งผ่าน `--verify-payload` และตรวจภาพทั้งสี่หน้าแล้ว
**ยังไม่ได้รันบน Server 2016 จริง** และยังไม่ได้กดติดตั้งผ่าน UAC (เซสชันนี้ไม่มีสิทธิ์ผู้ดูแล) — ดูรายการทดสอบในข้อ 4.0

### รอบก่อนหน้า

รอบล่าสุดรวมงานหน้า panel และ backend จากทีมเดิม ตรวจโค้ดและทดสอบซ้ำในโหมด console บน Windows แล้ว แก้การติดตั้งซ้ำที่อาจถอนของเดิม การถอนจาก exe ที่ติดตั้ง การตรวจ token ช่วงเริ่มระบบ และสิทธิ์ของโฟลเดอร์ข้อมูลกับ log ของ updater หน้า panel บันทึกค่าตั้งได้เมื่อเว้นแหล่งอัปเดตว่าง

หน้า panel รอบล่าสุดใช้ Google Fonts แบบฝังในโปรแกรม (Anuphan และ Chakra Petch) ใช้โทนฟ้าและม่วงตามโลโก้ จัดหน้าสถานะเครื่องใหม่และอธิบายความต่างระหว่างยกเลิกสิทธิ์กับตัดการเชื่อมต่อ ตรวจการแสดงฟอนต์ใน Chrome และหน้ากว้าง 320–1440 พิกเซลแล้ว

หน้า panel และคู่มือติดตั้งมีสวิตช์ **ธีมมืด** ใช้ค่าระบบเมื่อยังไม่ได้เลือก และจดจำค่าที่เลือกในเบราว์เซอร์ ภาพทั้งสองธีมอยู่ใน [media/themes](../media/themes)

ขั้นที่ยังต้องตรวจบนเครื่องทดสอบสะอาดคือ Windows Service จริง รวมถึงสิทธิ์ของบัญชีเสมือน, การเปิด panel ด้วยบัญชีเจ้าของแบบปกติและการปฏิเสธบัญชีอื่น, helper ใน session เจ้าของ, การอัปเดตและกู้คืนกับ SCM และการตรวจความปลอดภัยซ้ำโดยผู้ตรวจอิสระ เซสชันนี้ไม่มีสิทธิ์ Administrator

ชุดทดสอบหลักผ่าน 73 รายการ ส่วนเทสต์ที่ต้องใช้เดสก์ท็อปและ multicast สั่งรันแยกแล้วพบว่า Windows ส่ง `0x80070006` ตอนจับภาพ และ mDNS ไม่พบ service ใน 15 วินาที ต้องยืนยันสองรายการนี้อีกครั้งบนเครื่องใช้งานจริง

Chrome เปิด panel จากไบนารีบน loopback และทดสอบเปิด/ปิดรหัสจับคู่ สวิตช์การจับภาพ การบันทึกค่าตั้งโดยเว้นแหล่งอัปเดต ระบบปฏิเสธรหัสเปิด panel ที่ใช้ซ้ำ และการแสดงสถานะ `applying`, `rolled_back` และ `rollback_failed` ด้วยข้อมูลจำลอง ภาพอยู่ใน [media/panel-redesign](../media/panel-redesign) และใช้ชื่ออุปกรณ์ บัญชี และ IP ตัวอย่าง

## 2. ไฟล์ build ล่าสุด และที่อยู่ของงาน

| อะไร | ที่ไหน |
|---|---|
| **ตัวติดตั้งสำหรับเครื่องปลายทาง (v0.3.4)** | `D:\claude_code\exe\SynaptaConnectSetup-v0.3.4.exe` (สำเนาใน `dist\release\backend\v0.3.4\`) |
| agent ตัวเปล่า v0.3.4 (ทางสำรอง/ใช้อัปเดต) | `D:\claude_code\exe\SynaptaConnect-v0.3.4.exe` |
| ซอร์สตัวติดตั้ง | `installer\windows\` |
| เครื่องมือผู้เผยแพร่ (keygen/package/verify) | `D:\claude_code\exe\SynaptaRelease-v0.3.4.exe` |
| เว็บ panel v0.1.2 ที่ build แล้ว | `dist\release\panel\v0.1.2\` |
| ซอร์ส backend | `D:\claude_code\Synapta Connect\backend\` (`src\winsvc\`, `src\update\`, `src\local.rs`, `src\panel.rs`, `src\display.rs`, `src\helper.rs`, `src\proof.rs`, `src\cli.rs`) |
| หน้าบ้าน (ฝังเข้า exe ตอน build จาก `frontend\`) | `D:\claude_code\Synapta Connect\frontend\` |
| สคริปต์ทดสอบที่ต้อง Administrator | `backend\tests\admin\service-lifecycle.ps1` |
| fixture เปิด panel โดยไม่ต้อง admin | `backend\tests\fixture\panel-fixture.ps1` (`-Open`, `-Stop`) |
| ค่าที่ผู้เผยแพร่ต้องตั้ง | `backend\release\pubkey.hex`, `backend\release\source.txt` (ตอนนี้ว่าง/ศูนย์) |

## 3. ผลทดสอบที่ยืนยันแล้ว

- `cargo test` (ใน `backend\`): **73 ผ่าน / 0 ล้ม / 2 ข้าม** (2 ตัวคือเทสต์เดิมที่ต้องมีจอ/multicast) · `cargo clippy --all-targets` และ `cargo fmt --check` สะอาด · release build สำเร็จ (CRT static ไม่พึ่ง VCRUNTIME140.dll)
- เทสต์ Phase 0 เดิมทั้งหมด (SPAKE2, mTLS, revoke, exec, ไฟล์, PTY, หน้าจัดการ) ยังผ่านหลังย้าย local router ไป `local.rs`
- ตรรกะอัปเดต/กู้คืนกับ Host ปลอม: สลับสำเร็จ, ลายเซ็น/hash/รุ่นผิดไม่แตะอะไร, รุ่นใหม่ไม่ขึ้น/ไม่ตอบ/crash-loop → กู้, ขัดจังหวะหลังสลับ/จน exe หาย → กู้, กู้ไม่ได้ → แจ้ง `rollback_failed`, ตัวติดตั้งไม่เขียน/ลบในที่ที่ agent คุม, ไม่ทำคำขอซ้ำ, ไฟล์ใหญ่เกิน manifest ไม่ถูกคัดลอก, ไม่รู้รุ่นที่ติดตั้ง = ปฏิเสธ
- ผ่าน HTTP จริงบน loopback (`tests\service.rs`): panel รหัสครั้งเดียว→token (token รอดรีสตาร์ทเป็น hash), Host/Origin, API อัปเดตครบวงจร, ส่งงานจับภาพ/input ผ่าน helper ปลอม (ปิด/เปิด/หลุด/ตัวที่สอง), พิสูจน์ตัวแบบ HMAC (helper ปฏิเสธผู้ฟังปลอมโดยไม่ส่งอะไร, replay ถูกปฏิเสธ, health proof)
- เครื่องมือผู้เผยแพร่: keygen → package → verify ด้วย exe จริง (`tests\release_tool.rs`)
- รัน agent จริง (โหมด console) + เรียก GitHub จริงผ่าน TLS (repo จริงที่ไม่มี manifest, repo ที่ไม่มี → `source_unreachable`)
- คำสั่งที่ไม่ต้อง admin ของ exe จริง: `install --plan`, `install`/`uninstall` ที่ไม่ใช่ admin → exit 3, `service status`, `helper` ที่อ่าน token ไม่ได้ → จบเงียบ
- ขั้น build ของสคริปต์ admin (กุญแจทดสอบ, build 0.3.0/0.3.1, release ดี/ใช้ไม่ได้/ลายเซ็นปลอม) รันผ่านจนถึงก่อนติดตั้ง

## 4. สิ่งที่ยังต้องตรวจ / ทำต่อ (เรียงตามลำดับ)

0. **ทดสอบบน Windows Server 2016 จริง** (ทำแล้วบางส่วน ดูผลในข้อ 1 รอบ v0.3.3 — ที่เหลือคือจับภาพ/input ผ่าน helper)
   - ติดตั้ง Chrome หรือ Edge บนเครื่องก่อน (หน้า panel ใช้ IE ไม่ได้)
   - ดับเบิลคลิก `SynaptaConnectSetup-v0.3.4.exe` ด้วยบัญชีเจ้าของ → UAC → หน้าเสร็จ → panel เปิดเองในเบราว์เซอร์
   - จับคู่กับ Synapta Code แล้วลอง `terminal.exec` และ **`pty.ws`** (เส้นทาง WinPTY ใน service จริง — เทสต์บนเครื่อง dev รันแบบ console เท่านั้น)
   - ถ้า PTY ล้ม ดู log ของ service และโฟลเดอร์ `C:\ProgramData\SynaptaConnect\winpty\` ว่ามี `winpty.dll` + `winpty-agent.exe`
   - ลองจับภาพ/ส่ง input ผ่าน helper, ถอนการติดตั้งจากตัวติดตั้งแล้วลงใหม่
   - ถ้าเป็น Server Core (ไม่มีหน้า Desktop) ตัวติดตั้งแบบหน้าต่างใช้ไม่ได้ ให้ใช้ `SynaptaConnect-v0.3.4.exe install` แทน

1. **ติดตั้งและถอน Windows Service จริง** — เจ้าของเปิด PowerShell แบบ Administrator แล้วรัน
   `powershell -ExecutionPolicy Bypass -File "D:\claude_code\Synapta Connect\backend\tests\admin\service-lifecycle.ps1"` (~10 นาที; ใช้กุญแจทดสอบ; ปฏิเสธรันถ้ามี service อยู่แล้วหรือพอร์ต 52401/52402 ถูกใช้; ถอนเองตอนจบแม้ล้ม; รายงาน `%TEMP%\synapta-admin-test-<id>\report.txt`). **ส่วนหลังขั้นติดตั้งของสคริปต์ยังไม่เคยรันจริง** — เจอ `[FAIL]` ให้แยกว่าบั๊กระบบหรือบั๊กสคริปต์ (บั๊กสคริปต์เคยเจอแล้ว 1 จุด: PowerShell 5.1 ถือ stderr ของ cargo เป็น error; สคริปต์ต้องเป็น UTF-8 **มี BOM**)
2. **สถานะ `applying` / `rolled_back` กับ service จริง** — ครอบคลุมในสคริปต์ข้อ 1 (อัปเดต 0.3.0→0.3.1 สำเร็จ, 0.3.2 ที่ไม่ใช่ service → กู้, ลายเซ็นปลอม, จำลองไฟดับแล้วกู้) fixture ไม่สร้างสถานะเหล่านี้ได้ — หน้าบ้านต้องใช้ข้อมูลจำลองไปก่อน
3. **ACL ของ `local_token`** (จุดเสี่ยงสูงสุด) — ตัวติดตั้งสร้างไฟล์เปล่าพร้อม ACL (service:F, SYSTEM, Administrators, เจ้าของ:อ่าน) ใน step `token_file` แล้ว service เติมเนื้อหาทับ (ไม่แก้ ACL); ต้องยืนยันว่า (ก) service เขียนทับได้ (ข) เจ้าของอ่านได้จริงและเปิด `synapta-connect panel` ได้ (ค) ผู้ใช้ทั่วไปอ่านไม่ได้ — ดูบรรทัด "local_token: ACL ให้เจ้าของอ่านและจำกัดบัญชี" ในรายงาน และลองเปิด panel ด้วยบัญชีเจ้าของแบบไม่ยกสิทธิ์. ถ้าล้ม panel จะเปิดไม่ได้. ตรวจ ACL อื่นด้วย: `updater\` (agent เข้าไม่ได้), `updater-status\` (agent อ่านอย่างเดียว), Program Files (agent เขียนไม่ได้), firewall 2 กฎ (private/domain), SDDL ของตัวติดตั้ง
4. **ตรวจ helper กับ session ผู้ใช้จริง** (จับภาพ/คลิกจริง) — ทดสอบแล้วเฉพาะการส่งงานด้วย helper ปลอม
5. **ส่งผู้ตรวจความปลอดภัยตรวจซ้ำ** ข้อ 1, 3, 4, 5 ที่แก้แล้ว (สรุปอยู่ใน §6) และให้ลองโจมตีจริง (hard link/ยึดพอร์ต 52402) หลังมี service จริง
6. **build ทดสอบ** — รอบล่าสุด build จาก `backend\` ด้วย `cargo build --release --locked --offline` และเก็บไบนารีลง `dist\` ภายใน workspace พร้อมเลขรุ่น 0.3.0 สำหรับทดสอบ สคริปต์ Administrator ยังใช้ 0.3.0 เป็นรุ่นตั้งต้นและ 0.3.1 เป็นรุ่นอัปเดต เมื่อแก้หน้า panel ต้อง build ใหม่เพื่อฝังไฟล์จาก `frontend\` และเก็บเป็นชื่อใหม่
7. **ก่อนสร้าง repo/เผยแพร่** (ผู้ดูแลความต้องการสร้าง repo หลังงานพร้อม): ใส่ชื่อเจ้าของลิขสิทธิ์ใน `LICENSE`; เจ้าของยืนยันที่มาของรูปใน `frontend\assets\` (hero.jpg, control.jpg, synapta-tech.webp); `synapta-release keygen` → ใส่ public key ใน `backend\release\pubkey.hex` และ `github:OWNER/Synapta-Connect` ใน `backend\release\source.txt`; แนบ [THIRD_PARTY_NOTICES.txt](THIRD_PARTY_NOTICES.txt) ที่รวมข้อความประกาศของ 271 crate และ Rust standard library; ทางเลือก Authenticode (ต้องซื้อใบรับรอง) — ขั้นตอนเต็มใน `docs\RELEASE.md`. จนกว่าจะตั้ง public key รุ่นนี้ **ติดตั้งอัปเดตไม่ได้โดยตั้งใจ** (`not_configured`)
8. **ทดสอบดาวน์โหลด asset ผ่าน redirect ของ GitHub จริง** — ทำได้เมื่อมี repo และ release จริง (ยังไม่เคยทดสอบ)

## 5. การตัดสินใจที่ทำแทนเจ้าของ (ควรให้เจ้าของยืนยัน)

- service รันด้วยบัญชีเสมือนสิทธิ์ต่ำ `NT SERVICE\SynaptaConnect` → **exec/PTY/ไฟล์ของ agent เข้าโฟลเดอร์ผู้ใช้ไม่ได้ถ้าไม่ให้สิทธิ์** (Phase 0 console เข้าได้ทั้งโปรไฟล์) — ถ้าเจ้าของต้องการระดับเดิม ต้องออกแบบใหม่ (เช่น ให้ service รันเป็นบัญชีผู้ใช้ — ต้องเก็บรหัสผ่าน)
- ลายเซ็นอัปเดตใช้ Ed25519 บน manifest (กุญแจของผู้เผยแพร่ฝังตอน build) ไม่ใช้ Authenticode; ไม่มีตัวติดตั้ง GUI/MSI (exe ติดตั้งตัวเองด้วยคำสั่ง; wizard ของหน้าบ้านสร้างคำสั่งให้)
- ไม่รองรับ MJPEG stream ในโหมด service; service ใช้พอร์ตตายตัว 52401/52400/52402; เจ้าของ = บัญชีที่รัน `install` หรือ `--owner`

## 6. สรุปการแก้ตามผลตรวจรอบแรก (รอผู้ตรวจยืนยัน)

1) helper และการตรวจ health ใช้ HMAC (`proof.rs`) โดยไม่ส่ง token ส่วน CLI `panel` ตรวจ proof ของ health ก่อนขอรหัสเปิดหน้า panel ด้วย token · 2) panel token รอดรีสตาร์ท (เก็บ SHA-256 ใน `panel_sessions.json`) · 3) ตัวติดตั้ง SYSTEM ไม่แตะ settings (agent สำรอง/ใส่คืนเอง `settings.pre-update.json`) · 4) ตรวจขนาดแพ็กเกจก่อนคัดลอก คัดลอกแบบมีเพดาน · 5) ไม่รู้รุ่นที่ติดตั้ง = ปฏิเสธ

## 7. กับดักสำหรับคนทำต่อ

- ห้ามรัน `cargo build/test` สองคำสั่งซ้อนกัน (exe ถูกล็อก, LNK1104) · ห้าม `taskkill /IM synapta-connect.exe` (อาจฆ่า agent ของเจ้าของ) — หยุดเฉพาะโปรเซสที่ตัวเองเริ่ม
- ตัวติดตั้ง (SYSTEM) ต้องไม่เขียน/ลบในโฟลเดอร์ที่ agent เขียนได้ (junction/symlink → ยกสิทธิ์) — เทสต์ `agent_area_files` คุมไว้ อย่าลบ
- สคริปต์ `.ps1` ที่มีภาษาไทยต้องมี BOM; `paired_devices.json`/`settings.json` ต้องไม่มี BOM (serde อ่านไม่ได้)
- Bash tool ของผู้ช่วยพังกับ heredoc ที่มีอัญประกาศไม่สมดุลหรือแบ็กสแลช — เขียนสคริปต์แก้ไฟล์ด้วย Write tool แล้วค่อยรัน
- บันทึกกับดักเพิ่มเติมอยู่ใน memory ของโปรเจกต์ (`synapta-connect-build-gotchas.md`)
