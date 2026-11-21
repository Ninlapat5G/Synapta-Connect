# Synapta Connect: Hardened API & Protocol Specification (v1.1)

> เอกสารกำหนดโปรโตคอลฉบับปรับปรุงตาม Security Review: ป้องกัน MITM, ขจัด Plaintext OTP, บังคับ mTLS และจำกัดการรั่วไหลของข้อมูล

---

## 1. Network Discovery Protocol (UDP Beacon)
- **mDNS Service**: `_synapta-connect._tcp.local`
- **UDP Broadcast Port**: `52400`
- **ช่วงเวลาส่งสัญญาณ**: ทุก 3 วินาที (เฉพาะเมื่อเปิดใช้งาน)
- **ข้อกำหนดความปลอดภัย**:
  - **ห้าม** ประกาศข้อมูลสิทธิ์ (`privilege`) หรือ IP ภายใน VPN ออกทาง Broadcast เด็ดขาด
  - ส่งเฉพาะข้อมูลระบุตัวตนพื้นฐานและสถานะ Pairing Mode
- **Payload Format**:
```json
{
  "protocol": "synapta-connect/v1",
  "device_id": "sc-node-7b2a9c",
  "host_pubkey_fingerprint": "SHA256:4a7e9b...",
  "port": 52401,
  "pairing_mode": true
}
```

---

## 2. Pairing & Key Exchange (SPAKE2 PAKE over TLS 1.3)

เพื่อป้องกันการดักฟังและ Man-In-The-Middle (MITM) บนวง Wi-Fi หรือ LAN:
1. **Endpoint ปิดเป็นค่าเริ่มต้น**: จะเปิดรับคำขอจับคู่เมื่อผู้ใช้กดเปิด "Pairing Mode" บน Agent Panel เท่านั้น
2. **รหัส OTP เป็น Zero-Knowledge Password Proof**:
   - ใช้เป็นความลับร่วมในอัลกอริทึม **SPAKE2** (crate `spake2` ของ RustCrypto)
   - **ห้ามส่ง OTP เป็น Plaintext บนสายส่งสัญญาณเด็ดขาด** ทั้งสองฝั่งจะแลกเปลี่ยน PAKE commitment เพื่อพิสูจน์ว่ารู้ OTP เดียวกัน
3. **อายุและการจำกัดการเดา (Rate Limiting & Lockout)**:
   - OTP มีอายุ **120 วินาที** และสุ่มจาก CSPRNG (`rand::rngs::OsRng`)
   - เดาผิดรวม **5 ครั้ง (นับทั้งเครื่อง)** ระบบจะทำลาย OTP ทันที และปิด Pairing Mode
4. **การแลกเปลี่ยน Public Key**:
   - เมื่อ SPAKE2 สำเร็จ เครื่องปลายทางจะส่ง `host_pubkey` (Ed25519) ให้ client ปักหมุด (Pin)
   - เครื่องปลายทางจะบันทึก `client_pubkey` (Ed25519) ลงในรายการ `paired_devices.json`

### Endpoint:
`POST https://{ip}:52401/api/v1/auth/spake2-handshake`

---

## 3. Post-Pairing Authentication (mTLS 1.3)

- **ไม่ใช้ Bearer Token หรือ JWT**: เพื่อแก้ปัญหา Token รั่วไหล หรือ Token ไม่สามารถยกเลิกได้ทันที
- **ยืนยันตัวตนด้วย mTLS 1.3**:
  - การเชื่อมต่อทั้งหมดหลังจากการจับคู่ ต้องผ่าน **Mutual TLS 1.3** เท่านั้น
  - Client ใช้ Ed25519 Private Key ทำ Handshake
  - Server (เครื่องปลายทาง) ตรวจสอบว่า Client Public Key อยู่ใน `paired_devices.json` หรือไม่ หากไม่อยู่ ตัดการเชื่อมต่อทันที
- **การตัดสิทธิ์ (Revoke)**:
  - ผู้ใช้กด "Revoke" บน Agent Panel -> ระบบลบ Public Key นั้นออกจากไฟล์ `paired_devices.json` และตัด TCP Socket ที่ค้างอยู่ทันที มีผลแบบ Real-time

---

## 4. Core Secured Endpoints (TLS 1.3 & WSS)

ทุก Endpoint รันบน `https://` หรือ `wss://` (Port 52401) ไม่อนุญาต Fallback เป็น Plaintext

### 4.1 System Profiler (`connect_system_info`)
`GET https://{ip}:52401/api/v1/system/info` (ต้องผ่าน mTLS)
- ส่งคืน: OS, Hostname, CPU Model/Cores/Usage, RAM Total/Free, Disks, Privilege Level (User vs Admin)
- **มาตรการป้องกัน Secret รั่วไหล**:
  - **Environment Variables**: คืนค่าเฉพาะตัวแปรทั่วไปที่ปลอดภัย (`PATH`, `USERPROFILE`, `HOME`, `SHELL`, `LANG`, `OS`)
  - ตัวแปรอื่นๆ ส่งคืนเฉพาะ **ชื่อตัวแปร (Key)** และปิดบังค่าด้วย `"***"` เพื่อป้องกัน API Keys หลุดเข้าโมเดล LLM หรือ Transcript

### 4.2 Terminal Engine (`connect_exec_cmd`)
- **Single-shot Execution**: `POST https://{ip}:52401/api/v1/terminal/exec`
- **Interactive PTY**: `wss://{ip}:52401/api/v1/terminal/pty`
- ขอบเขตความปลอดภัย:
  - คำสั่งจะรันด้วยสิทธิ์ของ User Account ที่ Agent รันอยู่
  - ในโหมด Windows Service ต้องมี Interactive Session Helper ที่รันด้วย Token ของผู้ใช้ (`WTSQueryUserToken`) เพื่อจัดการ Input/Desktop แยกจาก Session 0

### 4.3 Screen & GUI Automation (`connect_capture_screen` & `connect_send_input`)
- `GET https://{ip}:52401/api/v1/screen/snapshot?format=webp&quality=80`
- `POST https://{ip}:52401/api/v1/input/send`

### 4.4 Fast File Transfer (`connect_file_transfer`)
- `POST https://{ip}:52401/api/v1/files/upload`
- `GET https://{ip}:52401/api/v1/files/download`
- **มาตรการความปลอดภัย**:
  - ตรวจสอบ Path Traversal: ทำ Canonicalization (`std::fs::canonicalize`) ก่อนเข้าถึงไฟล์ ป้องกันการใช้ `..\` เพื่อเข้าถึงโฟลเดอร์ที่ไม่ได้รับอนุญาต
  - ตรวจสอบความถูกต้องของไฟล์ด้วย SHA-256 Checksum ทุก Chunk
