# release/ — ค่าที่เจ้าของโปรเจกต์ต้องกำหนดก่อนปล่อยรุ่นจริง

| ไฟล์ | ใส่อะไร | ถ้ายังไม่ใส่ |
|---|---|---|
| `pubkey.hex` | public key Ed25519 ของผู้เผยแพร่ (hex 64 ตัว) — สร้างด้วย `synapta-release keygen` | ศูนย์ล้วน = ปิดการติดตั้งอัปเดตทุกกรณี (`not_configured`) |
| `source.txt` | แหล่ง release เริ่มต้น เช่น `github:OWNER/Synapta-Connect` (บรรทัดเดียว) | ว่าง = ผู้ติดตั้งต้องระบุ `--update-source` เอง |

ทั้งสองไฟล์ถูกฝังเข้า exe ตอน build (`include_str!`) — **เปลี่ยนแล้วต้อง build ใหม่และออกรุ่นใหม่**
private key ห้ามอยู่ในโฟลเดอร์นี้หรือใน repo; ขั้นตอนเต็มดู `docs/RELEASE.md`
