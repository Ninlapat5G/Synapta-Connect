# Synapta Connect

<img src="frontend/assets/synapta-connect.png" alt="โลโก้ Synapta Connect" width="128" height="128">

## ดาวน์โหลดและสิทธิ์การใช้งาน

ดาวน์โหลดตัวติดตั้ง `SynaptaConnectSetup-v<รุ่น>.exe` ได้จากหน้า [Releases](https://github.com/Ninlapat5G/Synapta-Connect/releases/latest) เครื่องที่ติดตั้งรุ่น 0.3.5 ขึ้นไปจะอ่านอัปเดตที่เซ็นแล้วจากหน้าเดียวกันนี้เอง

repo นี้เปิดให้ดูโค้ดและดาวน์โหลดรุ่นที่ปล่อยเท่านั้น ลิขสิทธิ์ทั้งหมดเป็นของเจ้าของโครงการ ไม่ใช่ open source ห้ามนำไปใช้ ดัดแปลง หรือแจกจ่ายต่อโดยไม่ได้รับอนุญาตเป็นลายลักษณ์อักษร ดูรายละเอียดใน [LICENSE](LICENSE)

## Panel ปัจจุบัน

เปิดด้วย `synapta-connect.exe panel` หรือเข้า `http://localhost:52402/synaptaconnect` ตามพอร์ตที่ตั้งไว้ หน้า **เครือข่าย** จัดการช่องทาง localhost และ HTTPS บน LAN พร้อมเปลี่ยนพอร์ตเว็บ, Agent API และ discovery ได้

เครื่องปลายทาง Windows x64 ติดตั้งจากไบนารีสำเร็จรูป ส่วนหน้าเว็บและ Agent อัปเดตจากแพ็กเกจที่เซ็นแยกกัน ดูขั้นตอน build, ติดตั้งและเตรียม HTTPS ที่ [คู่มือส่งมอบรุ่น](docs/RELEASE.md)

> ระบบเชื่อมต่ออัจฉริยะระหว่าง **Synapta Code / น้องซิน** และเครื่องปลายทาง (Remote Host) เพื่อวิเคราะห์ทรัพยากร สั่งการเทอร์มินัล ควบคุมหน้าจอ และรับส่งไฟล์ความเร็วสูง ด้วย Rust Core Daemon ประสิทธิภาพสูง รองรับความปลอดภัยระดับ TLS 1.3 / mTLS และ SPAKE2 Zero-Knowledge PAKE

---

## 📌 วิสัยทัศน์โครงการ (Project Vision)
ขยายขีดความสามารถของ Synapta Code ให้สามารถเข้าถึง ควบคุม และบริหารจัดการเครื่องคอมพิวเตอร์ปลายทาง (Windows / Linux / macOS) ได้ทั้งผ่านเครือข่ายภายใน (LAN) และเครือข่ายส่วนตัวเสมือน (Tailscale / VPN) โดย Agent สามารถวิเคราะห์สเปกเครื่อง รันคำสั่ง Shell/Terminal จับภาพหน้าจอสำหรับการวิเคราะห์ด้วย AI Vision ควบคุมเมาส์/คีย์บอร์ด และรับส่งไฟล์ได้อย่างรวดเร็วและปลอดภัย

---

## 🚀 ผลงานการส่งมอบ Phase 0 (Phase 0 Deliverables)

### 1. โปรแกรมปฏิบัติการหลัก (Core Executable Daemon)
- **ไฟล์โปรแกรมคอมไพล์สำเร็จ (Single Binary)**: สร้างด้วย `cargo build --release --locked` จาก `backend/` (build ทดสอบ Windows x64 พร้อม local panel)
- **service เบื้องหลัง + อัปเดตอัตโนมัติ (v0.0.1)**: [docs/SERVICE_AND_UPDATE.md](docs/SERVICE_AND_UPDATE.md) · สัญญา panel: [docs/PANEL_API.md](docs/PANEL_API.md) · การเผยแพร่/เซ็นแพ็กเกจ: [docs/RELEASE.md](docs/RELEASE.md) · ส่วนประกอบภายนอก: [docs/THIRD_PARTY.md](docs/THIRD_PARTY.md)
- **สถานะงานและรายการทดสอบที่เหลือ**: [docs/HANDOFF.md](docs/HANDOFF.md)
- **สัญญาอนุญาต**: [LICENSE](LICENSE) — public source แบบสงวนสิทธิ์ทุกประการ (ไม่ใช่ open source)
- **โค้ดต้นฉบับ**: โฟลเดอร์ `backend/` (พัฒนาด้วยภาษา Rust ทั้งหมด 17+ source files พร้อมชุดทดสอบ)
- **เอกสารการใช้งาน API ฉบับสมบูรณ์**: [docs/API.md](docs/API.md)

### 2. ผลตรวจความปลอดภัยของ v0.2.1
- **รายงานการตรวจสอบอย่างเป็นทางการ**: [docs/SECURITY_AUDIT_v0.2.1.md](docs/SECURITY_AUDIT_v0.2.1.md)
- **ผลการประเมิน**: **ผ่านแบบมีเงื่อนไข (Conditional Pass)**
  - ยืนยันการทดสอบจากภายนอกด้วย Client อิสระ: **ไม่มีช่องโหว่ที่คนแปลกหน้าบน LAN จะสั่งรันคำสั่ง (RCE) ได้**
  - รหัส OTP ใช้ **SPAKE2 PAKE** (OTP ไม่วิ่งบนสายส่งสัญญาณ ป้องกันการดักฟัง Wi-Fi)
  - ทุกการเชื่อมต่อบังคับใช้ **Mutual TLS 1.3 (mTLS)** ยืนยันตัวตนด้วยกุญแจ Ed25519
  - สิทธิ์ของไฟล์ลับ (`local_token`, `host.key`) ล็อกเฉพาะ Owner, SYSTEM และ Administrators
  - ปิดกั้น Path Traversal (`..`) และซ่อนค่า Environment Variables ที่เป็นความลับ (`***`)
  - แก้ไขปัญหา Process ตกค้างหลัง Timeout เรียบร้อย

### 3. หน้าจัดการบนเครื่องและคู่มือติดตั้ง

- เปิดหน้าจัดการด้วย `synapta-connect.exe panel` เมื่อ agent ทำงานอยู่ หน้าเว็บมาจากไฟล์ที่ฝังใน exe และใช้รหัสเปิดหน้าแบบครั้งเดียว
- ดูสถานะ agent บัญชีที่ใช้รัน และเครือข่าย เปิดการจับคู่ด้วย OTP 8 หลัก จัดการสิทธิ์อุปกรณ์ และตัดการเชื่อมต่อ
- เปิด/ปิดการจับภาพและ input ดูสถานะ helper และจัดการแหล่งอัปเดต เวอร์ชัน และประวัติ
- คู่มือติดตั้งสร้างคำสั่งสำหรับ PowerShell แบบ Administrator
- สวิตช์ **ธีมมืด** ใช้ธีมของระบบเป็นค่าเริ่มต้นและจดจำธีมที่เลือกในเบราว์เซอร์
- ภาพจากโปรแกรมจริง [ธีมสว่าง](media/themes/synapta-connect-light.png) · [ธีมมืด](media/themes/synapta-connect-dark.png) · [ธีมมืดบนมือถือ](media/themes/synapta-connect-dark-mobile.png) · [คู่มือติดตั้ง](media/themes/synapta-connect-dark-install-guide.png)

---

## 🛠️ เครื่องมือสำหรับ Agent (Synapta Code Tools Interface)

| Tool Name | Route | คำอธิบาย |
| :--- | :--- | :--- |
| `connect_list_devices` | UDP Probe / mDNS | ตรวจจับและค้นหาเครื่องใน LAN หรือ Tailscale VPN |
| `connect_system_info` | `POST /v1/system.info` | ดึงสเปกเครื่องละเอียด: CPU, RAM, OS, สิทธิ์, และ Env (Masked) |
| `connect_exec_cmd` | `POST /v1/terminal.exec` | สั่งรันคำสั่ง Shell ครั้งเดียว คืนค่า stdout/stderr พร้อม Timeout |
| `connect_terminal_pty` | `WSS /v1/pty.ws` | เปิด Interactive ConPTY Console (รองรับ ANSI และ UTF-8 ภาษาไทย) |
| `connect_capture_screen`| `POST /v1/screen.snapshot` | ถ่ายภาพหน้าจอ (JPEG/WebP) ส่งให้ AI Vision พร้อมปรับ DPI scaling |
| `connect_send_input` | `POST /v1/input.send` | ควบคุมเมาส์ เลื่อนตำแหน่ง และส่งคีย์บอร์ด |
| `connect_file_transfer`| `POST /v1/files.upload` / `download` | รับส่งไฟล์แบบ Chunked พร้อมตรวจสอบ SHA-256 Checksum |

---

## ⚠️ คำแนะนำและเงื่อนไขการใช้งาน Phase 0
1. **การใช้งานบนเครือข่าย**: แนะนำให้รันบน **LAN ที่ไว้ใจได้ (Trusted LAN)** หรือผ่าน **Tailscale / Private VPN**
2. **โหมดสิทธิ์**: โหมด console (Phase 0) รันด้วยสิทธิ์ผู้ใช้ที่เปิด · โหมด **service** (v0.0.1) รันด้วยบัญชีเสมือนสิทธิ์ต่ำ `NT SERVICE\SynaptaConnect` และจับภาพ/ส่ง input ผ่าน helper ใน session ของเจ้าของที่อนุญาตเท่านั้น (ดู docs/SERVICE_AND_UPDATE.md)
