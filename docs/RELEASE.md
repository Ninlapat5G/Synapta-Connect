# Synapta Connect — การเผยแพร่ (public source แบบสงวนสิทธิ์) และการเซ็นแพ็กเกจ

## ติดตั้งบนเครื่องปลายทาง Windows x64

รองรับ Windows 10/11 และ Windows Server 2016 ขึ้นไป (เครื่องที่ไม่มี ConPTY เช่น Server 2016 ใช้ Terminal ผ่าน WinPTY ที่ฝังมาใน exe)

**ทางหลัก:** ส่ง `SynaptaConnectSetup-v<backend>.exe` ไฟล์เดียว ดับเบิลคลิกด้วยบัญชีที่จะเป็นเจ้าของเครื่อง ตั้งพอร์ตแล้วกดติดตั้ง (ดู [installer/windows/README.md](../installer/windows/README.md))
เครื่องที่ลงรุ่นเก่าไว้แล้ว เปิดตัวติดตั้งรุ่นใหม่แล้วกด **อัปเดตเป็น vX** ได้เลย การจับคู่และค่าตั้งอยู่ครบ (ทางบรรทัดคำสั่ง: `SynaptaConnect-v<ใหม่>.exe upgrade` ด้วยสิทธิ์ผู้ดูแล)

**ทางสำรอง (Server Core หรือสคริปต์):** ส่ง `SynaptaConnect-v<backend>.exe` และ `THIRD_PARTY_NOTICES.txt` เปิด PowerShell แบบ Administrator ที่โฟลเดอร์ดาวน์โหลด แล้วรัน

```powershell
.\SynaptaConnect-v0.3.4.exe install --port 52401 --discovery-port 52400 --local-port 52402
.\SynaptaConnect-v0.3.4.exe panel
```

หน้า panel ต้องใช้ Chrome, Edge หรือ Firefox — Server 2016 มีแค่ Internet Explorer มาให้ คำสั่ง `panel` จะข้าม IE ไปใช้เบราว์เซอร์รุ่นใหม่ที่ติดตั้งไว้ ถ้าไม่มีจะบอกให้ติดตั้งก่อน

Windows Service เสิร์ฟเว็บสำรองที่ฝังมากับ Agent จึงเปิดใช้งานครั้งแรกได้ทันทีที่ `http://localhost:52402/synaptaconnect` เครื่องปลายทางไม่ต้องมี Node.js, npm หรือ Rust

| ค่า | เริ่มต้น | ใช้กับ |
| --- | --- | --- |
| `--local-port` | 52402 TCP | หน้า panel, health และ helper บน loopback รวมถึง HTTPS ของ panel เมื่อเปิด LAN |
| `--port` | 52401 TCP | Agent API ผ่าน TLS 1.3 และ mTLS |
| `--discovery-port` | 52400 UDP | UDP probe และ beacon |

หน้า **เครือข่าย** ใช้พอร์ตใหม่ทันทีเมื่อจองพอร์ตและบันทึกสำเร็จ การเปลี่ยนพอร์ต Agent API ตัดการเชื่อมต่อเดิม ให้ client ใช้พอร์ตใหม่ พอร์ตที่ถูกจองอยู่ทำให้การเปลี่ยนถูกยกเลิก ถ้าจะสลับเลขพอร์ต TCP สองตัวที่กำลังใช้อยู่ ให้ย้ายตัวหนึ่งไปพอร์ตว่างก่อน

### HTTPS บน LAN

localhost เปิดไว้ตามค่าเริ่มต้น ส่วน LAN ปิดไว้ เจ้าของเปิดหรือปิดแต่ละช่องทางได้ในหน้า **เครือข่าย**

1. เตรียมใบรับรอง PEM และคีย์ PEM ที่ตรงกัน ใบรับรองต้องมี SAN ของชื่อเครื่องและ/หรือ IPv4 ที่จะใช้จริง เครื่องที่เปิดเว็บต้องเชื่อถือ CA ที่ออกใบรับรอง ใช้ชื่อ SAN แบบเจาะจง
2. เพิ่ม `--panel-cert C:\certs\panel-chain.pem --panel-key C:\certs\panel-key.pem` ตอนติดตั้ง ใช้ `--allow-lan-panel` หากต้องการเปิด LAN ตั้งแต่ติดตั้ง หรือเปิดจากหน้าเครือข่ายภายหลัง สำหรับเครื่องที่ติดตั้งแล้ว ผู้ดูแลวาง PEM ในโฟลเดอร์ข้อมูลของ service เป็น `panel-tls.pem` และ `panel-tls-key.pem` ที่สืบทอด ACL ของโฟลเดอร์ แล้วเริ่ม service ใหม่
3. ที่เครื่องปลายทาง ให้เจ้าของออกลิงก์ด้วย `synapta-connect.exe panel --lan <ชื่อเครื่องหรือ-IP-ใน-SAN>` แล้วเปิดลิงก์จากเครื่องอื่นภายใน 60 วินาที URL ใช้รูป `https://<ชื่อหรือ-IP>:<พอร์ต>/synaptaconnect` ลิงก์มีรหัสครั้งเดียวใน fragment

LAN ใช้ IPv4 ช่วง private บน subnet เดียวกันผ่านอะแดปเตอร์เครือข่ายของเครื่อง ตัวติดตั้งตั้งกฎ Windows Firewall สำหรับ service เฉพาะโปรไฟล์ Private และ Domain ที่อยู่ localhost ยังใช้ HTTP บน loopback หากปิดทั้งสองช่องทาง เจ้าของหรือ Administrator รัน `synapta-connect.exe panel --enable-local` บนเครื่องปลายทางเพื่อเปิดกลับ

## Build และไฟล์เผยแพร่

ใช้ Node.js และ Rust MSVC บนเครื่อง build รันจาก root ของโปรเจกต์

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\build-release.ps1
```

ใช้ `-Component Backend` หรือ `-Component Panel` เพื่อทำเฉพาะส่วนที่ต้องการ เวอร์ชันของแต่ละส่วนอยู่ใน `backend/Cargo.toml` และ `frontend/package.json` ตามลำดับ เว็บสำรองของ backend ใช้เวอร์ชันที่ build มาด้วย

เมื่อมีคีย์ของผู้เผยแพร่ ให้เพิ่ม `-SigningKey <ไฟล์คีย์นอก repo> -PublicKey <Ed25519 hex>` สคริปต์ฝัง public key ใน backend เซ็นแต่ละ manifest และตรวจไฟล์ที่สร้างซ้ำ ใช้ `-ReleaseTool <synapta-release.exe>` กับ `-Component Panel` เพื่อเซ็นด้วยเครื่องมือที่ build มาแล้ว

| ส่วน | ไฟล์บนแหล่งดาวน์โหลด |
| --- | --- |
| Backend | `SynaptaConnect-v<backend>.exe`, `manifest.json`, `manifest.json.sig`, `THIRD_PARTY_NOTICES.txt` และ `SynaptaConnectSetup-v<backend>.exe` สำหรับติดตั้งครั้งแรก |
| เว็บ panel | `SynaptaConnect-Panel-v<web>.json`, `panel-manifest.json`, `panel-manifest.json.sig`, `THIRD_PARTY_NOTICES.txt` |

เผยแพร่แต่ละชุดที่ HTTPS directory ของตัวเองหรือ GitHub repo คนละตัว แล้วกำหนดแหล่งเว็บและ Agent จากหน้า **ตั้งค่า** หรือใช้ `--update-source` กับ `--web-update-source` ตอนติดตั้ง ถ้าใช้ GitHub repo เดียวกัน ให้ stable release ล่าสุดมีทั้งสองชุด เพราะตัวดาวน์โหลดอ่านจาก `releases/latest` การปล่อยเว็บอย่างเดียวสามารถใช้ HTTPS แหล่งเดียวกันกับ Agent ได้โดยเปลี่ยนเฉพาะไฟล์ที่ขึ้นต้นด้วย `panel-` และแพ็กเกจเว็บ

ผล build ที่ไม่มีคีย์จะเป็นไบนารีและไฟล์เว็บสำหรับทดสอบใน `dist/release` เมื่อ build ด้วย public key ศูนย์ เครื่องจะรายงานว่ายังไม่ได้ตั้งค่าอัปเดต

### การตรวจและกู้คืน

- Backend ตรวจ Ed25519 ของ manifest, แพลตฟอร์ม Windows x64, ขนาด, SHA-256 และเลขรุ่น ตัวติดตั้งตรวจซ้ำก่อนสลับ แล้วใช้ health check ที่พิสูจน์ด้วย HMAC เพื่อยืนยันรุ่นใหม่ หากไม่ผ่านจะกู้ไบนารีรุ่นเดิม
- เว็บตรวจ Ed25519, ขนาด, SHA-256, เลขรุ่น และ Panel API รุ่นเดียวกับที่ backend รองรับ ระบบตรวจไฟล์ที่ `index.html` อ้างถึงก่อนเปิดใช้และรักษาเว็บเดิมหากดาวน์โหลดหรือตรวจสอบไม่สำเร็จ
- ตอนเริ่ม Agent ถ้าแพ็กเกจเว็บบนดิสก์เสียหรือใช้กับ backend รุ่นนั้นไม่ได้ ระบบเลือกเว็บก่อนหน้าที่ตรวจผ่านหรือเว็บฝัง สามารถกด **กู้คืนเว็บ** จากหน้าอัปเดต หรือเรียก `synapta-connect.exe panel --recover-web` จากเครื่องปลายทางหากหน้าเว็บเปิดไม่ขึ้น

---

## 1. สัญญาอนุญาตของตัวเอง: public source แบบ "สงวนสิทธิ์ทุกประการ"

- ไฟล์ `LICENSE` ที่ root ระบุ **All Rights Reserved** (ดูได้ ใช้/แก้/แจกจ่ายไม่ได้ถ้าไม่ได้รับอนุญาตเป็นลายลักษณ์อักษร) เจ้าของลิขสิทธิ์ตอนนี้ใส่เป็นบัญชี GitHub `Ninlapat5G` ถ้าต้องการชื่อตามกฎหมายให้แก้บรรทัด Copyright
- **ห้ามเรียกว่า "open source"** ใน README/หน้า release — ตามนิยามแล้วไม่ใช่ (ใช้คำว่า "source-available" หรือ "public source, all rights reserved")
- ข้อสังเกตที่เจ้าของต้องรู้: repo สาธารณะบน GitHub — ข้อกำหนดการใช้งานของ GitHub ให้ผู้ใช้ **ดูและ fork ภายในแพลตฟอร์มได้** แม้ license จะสงวนสิทธิ์ นอกนั้น (เอาไปรันเอง/ดัดแปลง/แจกจ่ายต่อ) ไม่ได้รับอนุญาต; ถ้าต้องการกันการ fork ต้องใช้ repo private (ซึ่งทำให้อัปเดตอัตโนมัติแบบ unauthenticated ของ `github:` ใช้ไม่ได้ — ดู §6)
- ไฟล์ประกอบ: `docs/THIRD_PARTY.md` (ส่วนประกอบภายนอกและเงื่อนไข) อยู่ใน repo พร้อม [THIRD_PARTY_NOTICES.txt](THIRD_PARTY_NOTICES.txt) ซึ่งแนบไปกับไบนารีที่แจกจ่าย

## 2. ก่อนปล่อยรุ่นแรก — ต้องมีสิ่งเหล่านี้ (เจ้าของ/ผู้เผยแพร่ทำ)

| # | ต้องเตรียม | ทำอย่างไร | ถ้าไม่ทำ |
|---|---|---|---|
| 1 | **ชื่อ repo เต็ม** `OWNER/Synapta-Connect` (public) | ผู้ดูแลความต้องการสร้าง | ผู้ติดตั้งต้องใส่ `--update-source` เอง |
| 2 | **กุญแจเซ็นอัปเดต** | `synapta-release keygen --out <โฟลเดอร์นอก repo>` → ได้ไฟล์ `synapta-release.key` (private) + public key hex เก็บ private **สำรอง 2 ที่ (offline) ห้ามอยู่ใน repo/CI ที่คนอื่นเข้าถึง** | ไม่มีทางออกอัปเดตที่เครื่องผู้ใช้ยอมรับ |
| 3 | ฝัง public key + แหล่งเริ่มต้น | ใส่ hex ใน `backend/release/pubkey.hex` และ `github:OWNER/Synapta-Connect` ใน `backend/release/source.txt` (หรือใช้ตัวแปร build `SYNAPTA_RELEASE_PUBKEY`) | build ที่ pubkey เป็นศูนย์ = ติดตั้งอัปเดตไม่ได้ (ตั้งใจ) |
| 4 | **ใบรับรอง Authenticode** (ไม่บังคับ แต่แนะนำสำหรับแจกผู้ใช้ทั่วไป) | ซื้อใบรับรอง code-signing (OV หรือ EV) หรือใช้บริการเซ็นของ Microsoft/Azure Trusted Signing → เซ็น `synapta-connect.exe` ด้วย `signtool sign /fd SHA256 /tr <timestamp-url> /td SHA256 …` **ก่อน** รัน `synapta-release package` (manifest ผูกกับ hash ของไฟล์ที่เซ็นแล้ว) | Windows SmartScreen เตือน "ผู้เผยแพร่ที่ไม่รู้จัก" ตอนผู้ใช้ดาวน์โหลดตัวติดตั้งครั้งแรก; อัปเดตอัตโนมัติไม่กระทบ (ไม่ผ่าน SmartScreen เพราะดาวน์โหลดโดยโปรแกรม) |
| 5 | ชื่อเจ้าของลิขสิทธิ์ใน `LICENSE` | เจ้าของยืนยัน | ห้าม push |
| 6 | ที่มาของรูปใน `frontend/assets/` (`hero.jpg`, `control.jpg`, `synapta-tech.webp`) | เจ้าของยืนยันว่าเป็นงานของตนเอง/ได้สิทธิ์ใช้แจกจ่าย (ผมตรวจที่มาจากไฟล์ไม่ได้) — ถ้าสร้างด้วย AI/ซื้อสต็อก ต้องดูเงื่อนไขของแหล่งนั้นว่าอนุญาตแจกจ่ายใน public repo หรือไม่ | เสี่ยงละเมิดลิขสิทธิ์เมื่อ repo เป็นสาธารณะ |
| 7 | ทดสอบบนเครื่องสะอาดด้วย `backend/tests/admin/service-lifecycle.ps1` | ดู `docs/SERVICE_AND_UPDATE.md` §8 | – |

## 3. ขั้นตอนออกรุ่น

```
1. เพิ่มเลขรุ่นใน backend/Cargo.toml หรือ frontend/package.json ตามส่วนที่อัปเดต
2. Build และเซ็นด้วย scripts/build-release.ps1 ตามหัวข้อด้านบน
3. cargo test --locked จาก backend/
4. จัดเตรียมชุดทดสอบ แล้วทดสอบอัปเดตจากรุ่นก่อนบนเครื่องทดสอบ
5. เผยแพร่ชุดไฟล์ของแต่ละส่วนตามตารางด้านบน
```

ผู้ใช้ที่ติดตั้งครั้งแรกดาวน์โหลด `SynaptaConnect-vX.Y.Z.exe` จากหน้า Release แล้วรัน `SynaptaConnect-vX.Y.Z.exe install` (Administrator)

หากใช้ Authenticode ให้เซ็น exe ก่อนเรียก `synapta-release package` และ `verify` อีกครั้งเพื่อให้ manifest ครอบไบต์ของ exe หลังเซ็น

## 4. กุญแจเซ็นหาย / รั่ว

- **รั่ว**: ใครถือ private key ออกอัปเดตให้ทุกเครื่องได้ → ทำทันที: สร้างกุญแจใหม่, ออกรุ่นใหม่ที่ฝัง public key ใหม่ **เซ็นด้วยกุญแจเดิม** (ผู้ใช้อัปเดตเข้ารุ่นนี้ได้ตามปกติ), หยุดใช้กุญแจเก่า, ลบ release ที่ผู้โจมตีอาจอัปโหลดบน GitHub, แจ้งผู้ใช้ — เครื่องที่ยังไม่อัปเดตเป็นรุ่นใหม่ก่อนผู้โจมตีออกของปลอมจะรับของปลอมได้ ช่วงเวลานี้คือความเสี่ยงของการเก็บกุญแจเดียว
- **หาย**: เครื่องเก่าอัปเดตเองไม่ได้อีก (ต้องติดตั้งรุ่นใหม่ทับด้วยมือ: `uninstall --keep-data` แล้ว `install`) — จึงต้องสำรอง 2 ที่
- ยังไม่มีการหมุนกุญแจอัตโนมัติหลายกุญแจ (key set) — ผมไม่ทำเพราะยังไม่มีผู้ใช้จริง ชี้ได้ว่ากันอะไร: กุญแจรั่วแล้วต้องออกรุ่นแก้ได้โดยไม่ต้องให้ผู้ใช้ลงมือเอง (ทำได้อยู่แล้วตามด้านบน)

## 5. GitHub: ข้อควรระวัง

- การตั้ง repo เป็น public + แนบ release assets: ใครก็ดาวน์โหลดได้ (ไม่มีความลับในไบนารี; ลายเซ็นกันการปลอม ไม่ได้กันการอ่าน)
- API ของ GitHub แบบไม่ยืนยันตัวตนจำกัด 60 ครั้ง/ชั่วโมง/IP — ตัวอัปเดตตรวจทุก 6 ชั่วโมง จึงพอสำหรับบ้าน/สำนักงานเล็ก; หลายร้อยเครื่องหลัง NAT เดียวกันอาจชนเพดาน (แก้โดยตั้ง `source` เป็น URL https ของ mirror ของเราเอง)
- ห้ามเก็บ token/ความลับใน repo (มีไฟล์ `release/` เฉพาะ public key และชื่อแหล่ง)

## 6. ถ้าจะใช้ repo แบบ private

ตัวอัปเดตเรียก GitHub แบบไม่ยืนยันตัวตน — private repo ใช้ไม่ได้โดยตรง ทางเลือก: โฮสต์ 3 ไฟล์ release ที่ URL https ของเราเอง (ตั้ง `https://…/` เป็น source) ซึ่งไม่ต้องเปลี่ยนโค้ด
