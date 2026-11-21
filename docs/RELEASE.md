# Synapta Connect — การเผยแพร่ (public source แบบสงวนสิทธิ์) และการเซ็นแพ็กเกจ

ผู้ดูแลความต้องการเป็นผู้สร้าง repo `Synapta-Connect` หลังงานพร้อม — เอกสารนี้คือสิ่งที่ต้องเตรียมก่อน/หลังสร้าง repo

## 1. สัญญาอนุญาตของตัวเอง: public source แบบ "สงวนสิทธิ์ทุกประการ"

- ไฟล์ `LICENSE` ที่ root ระบุ **All Rights Reserved** (ดูได้ ใช้/แก้/แจกจ่ายไม่ได้ถ้าไม่ได้รับอนุญาตเป็นลายลักษณ์อักษร) — ต้องใส่ **ชื่อเจ้าของลิขสิทธิ์ที่ถูกต้องตามกฎหมาย** แทน `[ชื่อเจ้าของลิขสิทธิ์]` และปีให้ตรง ก่อน push (ผมไม่รู้ชื่อ และไม่ควรเดา)
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
1. แก้เลขรุ่นใน backend/Cargo.toml (x.y.z ตัวเลขล้วน — ต้องสูงกว่ารุ่นก่อนเสมอ ไม่งั้นทุกเครื่องปฏิเสธ)
2. cargo test                      # ต้องเขียวทั้งหมด
3. cargo build --release           # ได้ target\release\synapta-connect.exe (ใช้ pubkey.hex ที่ฝังไว้)
4. (ถ้ามีใบรับรอง) signtool sign … synapta-connect.exe
5. synapta-release package --exe target\release\synapta-connect.exe --version X.Y.Z --key <private key> --out dist\X.Y.Z --notes "สิ่งที่เปลี่ยน"
       ↳ เครื่องมือจะปฏิเสธถ้า exe รายงานเลขรุ่นไม่ตรงกับที่ประกาศ (ตัวติดตั้งใช้เลขนี้ตัดสินว่ารุ่นใหม่เริ่มขึ้นจริง)
6. synapta-release verify --dir dist\X.Y.Z --pubkey <public key>   # ตรวจกลับ
7. สร้าง GitHub Release (tag vX.Y.Z) — ไม่ใช่ draft ไม่ใช่ pre-release (ตัวอัปเดตใช้ releases/latest ซึ่งไม่รวมสองแบบนั้น)
   แนบ SynaptaConnect-vX.Y.Z.exe, manifest.json และ manifest.json.sig สำหรับตัวอัปเดต พร้อม THIRD_PARTY_NOTICES.txt สำหรับผู้ใช้
8. ทดสอบอัปเดตจริงจากรุ่นก่อนบนเครื่องทดสอบ 1 เครื่อง ก่อนประกาศ
9. เก็บสำเนาไฟล์ใน D:\claude_code\exe\ ตามชื่อ SynaptaConnect-vX.Y.Z.exe
```

ผู้ใช้ที่ติดตั้งครั้งแรกดาวน์โหลด `SynaptaConnect-vX.Y.Z.exe` จากหน้า Release แล้วรัน `SynaptaConnect-vX.Y.Z.exe install` (Administrator)

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
