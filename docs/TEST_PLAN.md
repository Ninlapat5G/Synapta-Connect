# Synapta Connect — Test Specification (Phase 0)

ผู้เขียน: Security & QA · วันที่: 2026-11-20
อ้างอิง: `README.md`, `docs/API_SCHEMA.md`, `docs/SECURITY_REVIEW.md` (เลข `SR-n` = ข้อในเอกสารนั้น)

เอกสารนี้บอก **ต้องทดสอบอะไร และผ่านเมื่อไหร่** ตัวเลขประสิทธิภาพที่ระบุเป็น **เป้าที่เสนอ** — Lead ยืนยันหรือปรับได้
เทสต์ที่ระบุว่า `[auto]` ควรเป็น integration test ในโค้ด Rust (`cargo test`) ที่รันทุก build; `[manual]` ทำก่อนปล่อยแต่ละเวอร์ชัน

---

## 1. Security tests

แต่ละข้อคือการโจมตีจริงที่ต้อง **ล้มเหลว** ถ้าเทสต์ไหนผ่าน (โจมตีสำเร็จ) = ปล่อยไม่ได้

### 1.1 Pairing / OTP
| ID | ทำอะไร | ผ่านเมื่อ | อ้างอิง |
|---|---|---|---|
| SEC-P1 `[auto]` | เรียก `/auth/pair` ตอนผู้ใช้ไม่ได้กดโหมดจับคู่ | ถูกปฏิเสธ, ไม่มี OTP ใดใช้ได้ | SR-3 |
| SEC-P2 `[auto]` | ส่ง OTP ผิด 5 ครั้งจาก **5 IP ต่างกัน** แล้วส่ง OTP ถูก | ครั้งที่ 6 ถูกปฏิเสธ, โหมดจับคู่ปิด | SR-3 |
| SEC-P3 `[auto]` | ใช้ OTP ถูกหลัง 121 วินาที | ถูกปฏิเสธ | SR-3 |
| SEC-P4 `[auto]` | ใช้ OTP เดิมจับคู่ครั้งที่สอง | ถูกปฏิเสธ | SR-3 |
| SEC-P5 `[auto]` | สุ่ม OTP 100,000 ใบ ตรวจการกระจาย | ทุกหลักกระจายเท่ากัน (chi-square p > 0.01), เลขนำหน้า 0 ออกได้ | SR-3 |
| SEC-P6 `[manual]` | ตั้งเครื่องผู้โจมตีปลอม beacon `device_id` เดียวกัน ให้ Synapta Code จับคู่ผ่านเครื่องผู้โจมตี (relay) | จับคู่ล้มเหลว และผู้โจมตีไม่ได้ OTP ในรูปที่นำไปใช้ซ้ำได้ (ตรวจด้วย Wireshark) | SR-1 |
| SEC-P7 `[auto]` | หลังจับคู่ เปลี่ยน host key ของเครื่องปลายทางแล้วต่อใหม่ | client ปฏิเสธ และแจ้งผู้ใช้ว่า key เปลี่ยน | SR-2 |

### 1.2 Auth / Revoke
| ID | ทำอะไร | ผ่านเมื่อ | อ้างอิง |
|---|---|---|---|
| SEC-A1 `[auto]` | เรียกทุก route ใน `API_SCHEMA §3` โดยไม่มี client cert / ด้วย cert ที่ไม่ได้จับคู่ | ทุก route ถูกปฏิเสธก่อนถึง handler | SR-4 |
| SEC-A2 `[auto]` | เปิด PTY ค้างไว้ แล้ว revoke key นั้น | PTY ถูกตัดภายใน 1 วินาที, ต่อใหม่ไม่ได้ | SR-4 |
| SEC-A3 `[auto]` | revoke key A ขณะ key B ใช้งาน | B ไม่สะดุด | SR-4 |
| SEC-A4 `[auto]` | ต่อ port 52401 แบบ plaintext HTTP และ `ws://` | ไม่ได้ข้อมูลใดกลับมาเลย (TLS handshake fail) | SR-5 |
| SEC-A5 `[auto]` | ส่ง request พร้อม header `Origin: https://evil.example` | ถูกปฏิเสธ | SR-14 |

### 1.3 การรั่วของข้อมูล
| ID | ทำอะไร | ผ่านเมื่อ | อ้างอิง |
|---|---|---|---|
| SEC-L1 `[auto]` | ดัก UDP 52400 | payload ไม่มี `privilege`, `hostname`, IP ของ VPN | SR-6 |
| SEC-L2 `[auto]` | ตั้ง env `FAKE_SECRET_TOKEN=abc123` แล้วเรียก `system/info` | ไม่มี `abc123` ใน response | SR-7 |
| SEC-L3 `[manual]` | `nmap -sV` จาก LAN ตอนไม่ได้อยู่ในโหมดจับคู่ | เห็นแค่ TLS port ที่ไม่ตอบอะไรถ้าไม่มี cert, ไม่มี banner บอกเวอร์ชัน | SR-5 |

### 1.4 สิทธิ์และไฟล์
| ID | ทำอะไร | ผ่านเมื่อ | อ้างอิง |
|---|---|---|---|
| SEC-F1 `[auto]` | upload ไป `../../x`, `..\\..\\x`, absolute path นอก allowlist, path ที่มี `%2e%2e` | ถูกปฏิเสธทั้งหมด ไม่มีไฟล์เกิดขึ้น | SR-10 |
| SEC-F2 `[auto]` | สร้าง symlink ในโฟลเดอร์ที่อนุญาตชี้ไป `C:\Windows` / `/etc` แล้ว upload ผ่าน symlink | ถูกปฏิเสธ | SR-10 |
| SEC-F3 `[manual]` | โหมด User: `exec` คำสั่งเขียนไฟล์ลง `C:\Windows\System32` / `/etc` | OS ปฏิเสธ (Access denied) — Agent ไม่ได้ยกสิทธิ์ให้ | SR-9 |
| SEC-F4 `[manual]` | โหมด Service (Windows): ผู้ใช้ธรรมดาอีกคนบนเครื่องเปิด named pipe ของ helper | ถูกปฏิเสธโดย ACL | SR-8 |
| SEC-F5 `[auto]` | ทุกคำสั่งใน 1.1–1.4 | มีบรรทัดตรงกันใน audit log (เวลา, key fingerprint, route, ผล) | SR-11 |

---

## 2. Command & Terminal

| ID | ทำอะไร | ผ่านเมื่อ |
|---|---|---|
| TRM-1 `[auto]` | single-shot `echo hello` บน cmd, PowerShell, bash, zsh | stdout = `hello`, exit code = 0 |
| TRM-2 `[auto]` | คำสั่งที่จบด้วย exit code 3 และเขียน stderr | ได้ exit code 3 และ stderr แยกจาก stdout |
| TRM-3 `[auto]` | PTY: รัน `python -c "input('name? ')"` ส่ง `ทดสอบ\r` | เห็น prompt ก่อนส่ง input, ได้ `ทดสอบ` กลับครบ ไม่เป็น `?` หรือ mojibake |
| TRM-4 `[auto]` | output ภาษาไทย+อังกฤษ+emoji บน Windows ที่ code page เป็น 874 (TIS-620) และ 65001 | ได้ UTF-8 ถูกต้องทั้งสองกรณี — **กับดักจริงบน Windows ไทย**: console เริ่มต้นเป็น cp874 |
| TRM-5 `[auto]` | output ที่ตัวอักษรไทย 3 ไบต์ถูกตัดคร่อม chunk ของ stream | ไม่มี `U+FFFD` ปรากฏ (decoder ต้องเก็บไบต์ค้างไว้ต่อ chunk ถัดไป) |
| TRM-6 `[auto]` | `ls --color` / `Get-ChildItem` ที่มีสี | ANSI escape ส่งผ่านครบ ไม่ถูกตัด; ถ้ามีโหมด "plain" สำหรับ LLM ต้องตัด ANSI ออกหมด |
| TRM-7 `[auto]` | คำสั่งค้าง (`sleep 3600` / `ping -t`) กับ timeout 5 วินาที | คืนผลภายใน ~5 วินาที, **process และลูกทั้งหมดถูก kill** (ตรวจด้วย process list) — ไม่ใช่แค่ตัด connection |
| TRM-8 `[auto]` | ปิด WebSocket ของ PTY กะทันหัน | shell process ถูกปิดภายใน 5 วินาที ไม่มี zombie |
| TRM-9 `[auto]` | output 200 MB (`yes` / loop) | Agent RAM ไม่โตเกิน 64 MB (มี backpressure หรือตัด output ตามเพดาน) |
| TRM-10 `[auto]` | เปิด PTY พร้อมกัน 20 session | ทุก session ทำงานถูก ไม่ปน output กัน |
| TRM-11 `[auto]` | resize PTY เป็น 40x10 แล้ว 200x60 | `tput cols` / `$Host.UI.RawUI.WindowSize` ตรงค่าใหม่ |
| TRM-12 `[auto]` | `cwd` ที่ไม่มีอยู่ / มีช่องว่าง / ภาษาไทย (`C:\ทดสอบ เอกสาร`) | ไม่มีอยู่ = error ชัดเจน; มีช่องว่าง/ไทย = รันถูกที่ |

---

## 3. File Transfer

| ID | ทำอะไร | ผ่านเมื่อ |
|---|---|---|
| FT-1 `[auto]` | upload/download ขนาด 0 B, 1 B, ขนาดพอดี chunk, chunk+1 B, 5 GB | SHA-256 ปลายทางตรงต้นทางทุกไฟล์ |
| FT-2 `[auto]` | แก้ 1 ไบต์ใน chunk กลางทาง (proxy ทดสอบ) | การโอนล้มเหลว, ไม่มีไฟล์ที่ `dest`, ไฟล์เดิมที่ `dest` (ถ้ามี) ไม่ถูกแตะ |
| FT-3 `[auto]` | ตัด connection ที่ 37% แล้ว resume | ส่งต่อจาก ~37% (ไม่เริ่มใหม่), hash สุดท้ายตรง |
| FT-4 `[auto]` | ตัด connection แล้วไฟล์ต้นทางถูกแก้ก่อน resume | ตรวจพบและเริ่มใหม่ ไม่ต่อไฟล์ผสมสองเวอร์ชัน |
| FT-5 `[auto]` | resume ด้วย offset เกินขนาดไฟล์ / ติดลบ | ถูกปฏิเสธ |
| FT-6 `[auto]` | ดิสก์ปลายทางเต็มระหว่างโอน | error ชัดเจน, `.partial` ถูกลบ |
| FT-7 `[auto]` | ชื่อไฟล์ไทย, emoji, ช่องว่าง, ยาว 255 ตัว; ชื่อต้องห้าม Windows (`CON`, `aux.txt`, ลงท้ายจุด) | ไทย/emoji ผ่าน; ชื่อต้องห้ามได้ error ไม่ใช่ crash |
| FT-8 `[auto]` | โอน 10 ไฟล์พร้อมกันไป `dest` เดียวกัน | ไฟล์สุดท้ายสมบูรณ์ตัวใดตัวหนึ่ง ไม่ใช่ไฟล์ปน |
| FT-9 `[manual]` | วัด throughput ไฟล์ 2 GB | เป้าที่เสนอ: LAN 1 Gbps ≥ 80 MB/s, Wi-Fi 5 ≥ 70% ของ `iperf3` บนเส้นเดียวกัน, VPN ≥ 70% ของ `iperf3` ผ่าน VPN |
| FT-10 `[manual]` | วัด latency เริ่มโอน (กดสั่ง → ไบต์แรกถึง) | เป้า: LAN < 100 ms, VPN < 300 ms |

วิธีวัด FT-9/10: รัน `iperf3` บนเส้นเดียวกันก่อนทุกครั้งเป็นเพดาน รายงานเป็น % ของเพดาน — ตัวเลข MB/s ลอย ๆ เทียบข้ามเครือข่ายไม่ได้

---

## 4. Screen Capture & Vision

| ID | ทำอะไร | ผ่านเมื่อ (เป้าที่เสนอ) |
|---|---|---|
| SCR-1 `[manual]` | snapshot เต็มจอ 1080p, 1440p, 4K × 50 ครั้ง | p95 latency (สั่ง → ไฟล์ถึง client บน LAN): 1080p < 300 ms, 4K < 800 ms |
| SCR-2 `[manual]` | วัด RAM/CPU ของ Agent ขณะ idle และขณะ snapshot ต่อเนื่อง 1 ภาพ/วินาที 10 นาที | idle: RAM < 50 MB, CPU < 1%; ขณะทำงาน: RAM ไม่โตต่อเนื่อง (ไม่มี leak), CPU เฉลี่ย < 15% ของหนึ่ง core |
| SCR-3 `[manual]` | ขนาดไฟล์ WebP q80 vs PNG บนหน้าจอ 3 แบบ (IDE, เว็บ, รูปภาพเต็มจอ) | บันทึกอัตราบีบอัดเป็นตาราง; WebP q80 ตัวหนังสือขนาด 12px ยังอ่านออก (ทดสอบด้วย Vision model ถามข้อความบนจอ) |
| SCR-4 `[manual]` | หลายจอ / จอ DPI ต่างกัน (100% + 150%) | เลือกจอได้, พิกัดใน metadata ตรงกับที่ `send_input` ใช้ (คลิกตรงจุดที่เห็นในภาพ) |
| SCR-5 `[manual]` | `window_title` ที่ไม่มีอยู่ / ซ้ำกันสองหน้าต่าง / ถูกย่อ | ไม่มี = error; ซ้ำ = กฎเลือกชัดเจนและบอกใน response; ถูกย่อ = error หรือภาพที่ถูกต้อง ไม่ใช่ภาพดำโดยไม่แจ้ง |
| SCR-6 `[manual]` | หน้าจอล็อก / UAC prompt / Wayland (Linux) / macOS ยังไม่ให้สิทธิ์ Screen Recording | error ที่บอกสาเหตุ ไม่ใช่ภาพดำเงียบ ๆ |
| SCR-7 `[manual]` | โหมด Service บน Windows จับจอผู้ใช้ที่ login อยู่ | ได้ภาพ desktop จริง (ยืนยัน SR-8) |
| SCR-8 `[manual]` | ระหว่างจับจอหรือส่ง input | ไอคอนแจ้งผู้ใช้แสดง และปุ่มหยุดตัดได้ทันที (SR-12) |

---

## 5. Integration Matrix

สัญลักษณ์: **P0** = ต้องผ่านก่อนปล่อย Phase 0 · **P1** = ทดสอบก่อนปล่อยถ้ามีเครื่อง · — = ไม่รองรับ/ไม่ทดสอบ

### 5.1 OS × ความสามารถ

| เครื่องปลายทาง | Pair | Exec/PTY | File | Screen | Input | Service mode |
|---|---|---|---|---|---|---|
| Windows 11 (ไทย, cp874) | P0 | P0 | P0 | P0 | P0 | P0 |
| Windows 10 22H2 | P0 | P0 | P0 | P0 | P0 | P1 |
| Ubuntu 24.04 (X11) | P0 | P0 | P0 | P1 | P1 | P1 (systemd) |
| Ubuntu 24.04 (Wayland) | P1 | P1 | P1 | P1 (คาดว่าต้องผ่าน portal) | P1 | — |
| macOS 14+ (Apple Silicon) | P1 | P1 | P1 | P1 (ต้องขอสิทธิ์ Screen Recording/Accessibility) | P1 | — |

เหตุที่ Windows 11 ไทยเป็น P0 ทุกช่อง: เครื่องของเจ้าของงานเป็น Windows ไทย และ cp874 คือจุดที่ TRM-4 พังบ่อยที่สุด

### 5.2 เครือข่าย × ความสามารถ

| เส้นทาง | Discovery | Pair | Exec | File (throughput) | Screen |
|---|---|---|---|---|---|
| LAN เดียวกัน (สาย) | P0 | P0 | P0 | P0 | P0 |
| LAN เดียวกัน (Wi-Fi) | P0 | P0 | P0 | P0 | P0 |
| ข้าม router / คนละ subnet | P0 (คาดว่า mDNS/broadcast **ไม่ข้าม** — ต้องมีช่องใส่ IP เอง) | P0 | P0 | P1 | P1 |
| Tailscale (direct) | P0 | P0 | P0 | P0 | P0 |
| Tailscale (relay/DERP) | P1 | P1 | P1 | P1 (บันทึกตัวเลข ไม่ตั้งเป้า) | P1 |
| Wi-Fi ที่เปิด client isolation (ร้านกาแฟ) | P1 — ต้องแสดง error ที่เข้าใจได้ | — | — | — | — |

### 5.3 เหตุการณ์ระหว่างใช้งาน (ทำบนทุกแถว P0 ของ 5.1)
- เครื่องปลายทาง sleep แล้วตื่น → ต่อกลับได้เองภายใน 30 วินาที
- เปลี่ยน IP (DHCP ต่ออายุ / ย้าย Wi-Fi) → Discovery พบใหม่, ไม่ต้องจับคู่ซ้ำ (pin ด้วย host key ไม่ใช่ IP)
- Agent crash ระหว่าง PTY + file transfer → service manager restart, `.partial` resume ได้, audit log ไม่เสีย
- อัปเดต Agent เวอร์ชันใหม่ → key ที่จับคู่ไว้ยังใช้ได้

---

## 6. เกณฑ์ปล่อย Phase 0
1. Security tests หมวด 1 ทุกข้อ "ผ่าน" (โจมตีไม่สำเร็จ)
2. ทุกช่อง P0 ใน 5.1 และ 5.2 ผ่าน
3. TRM-4, TRM-5, TRM-7, FT-2, FT-3 เป็น `[auto]` และอยู่ใน CI
4. ตัวเลขประสิทธิภาพ (FT-9/10, SCR-1/2/3) บันทึกจริงแล้ว — ถ้าต่ำกว่าเป้า ให้ Lead ตัดสินว่ายอมรับหรือไม่ ไม่บล็อกอัตโนมัติ
