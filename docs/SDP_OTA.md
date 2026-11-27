# SDP OTA ใน Synapta Connect

> คนที่เพิ่งเริ่มใช้ ESP32 กับ SDP: อ่าน `tools/sdp-ota/GUIDE.md` ใน SDP checkout หรือพิมพ์ `sdpo guide`
> ในนั้นมีขั้นเตรียมบอร์ด เขียน `main/app.c` (เริ่มด้วย `sdpo new FOLDER --example blink`) และ Build แล้วอัปโหลด
> ปุ่ม Build แล้วอัปโหลดแบบกดครั้งเดียวมีใน SynaptaCode ส่วน Connect ทำหน้าที่เป็นเครื่อง build ให้ และรับ RPC ด้านล่างเท่านั้น

รัน Connect (service, console หรือ Portable) บน **เครื่องอัปโหลด** — เครื่องนี้ต้องเข้าถึงบอร์ดผ่านวงแลนได้ และมีไฟล์แอปที่เซ็นแล้ว
ส่วนเครื่อง compile แยกเป็นอีกเครื่องได้ (เช่น `app01`) Connect อ่าน `tools/sdp-ota/module.json` และรัน `tools/sdp-ota/sdp_ota.py`
จาก SDP checkout ที่ตั้งไว้โดยตรง เครื่องอัปโหลดต้องมี Python 3.11 ขึ้นไป

## ตั้งค่าเครื่องอัปโหลด

ตั้ง environment `SYNAPTA_SDP_OTA_CONFIG` ของโปรเซส/service Connect ให้ชี้ไปที่ไฟล์ JSON (UTF-8, path เต็ม) ที่เจ้าของเครื่องดูแล
แล้วรีสตาร์ต Connect หนึ่งครั้ง หลังจากนั้นไฟล์ config และ manifest ของโมดูลถูกอ่านใหม่ทุกครั้งที่มี RPC (แก้แล้วไม่ต้องรีสตาร์ต)

```json
{
  "sdpRoot": "C:/Synapta/SDP",
  "python": "C:/Python312/python.exe",
  "artifactRoot": "C:/Synapta/SignedFirmware",
  "allowFirmwareWrite": false,
  "writeClients": [],
  "boards": {
    "1234567890abcdef": {
      "host": "192.0.2.42",
      "projectId": "project",
      "keyFile": "C:/Synapta/BoardCredentials/1234567890abcdef.key"
    }
  }
}
```

- ให้บัญชีที่รัน Connect อ่าน path เหล่านี้ได้ ส่วนสิทธิ์เขียนให้เฉพาะผู้ดูแลเครื่องที่ไว้ใจ และอย่าวางไว้ใต้โฟลเดอร์ที่รับส่งไฟล์ระยะไกลได้
- จำกัดสิทธิ์อ่านไฟล์กุญแจของแต่ละบอร์ด — ไฟล์นี้ได้จาก `sdpo.cmd provision --port COMx --key-file PATH` ตอนเสียบ USB
  (บอร์ด SDP v0.2.1 ขึ้นไปส่งรหัสลับ 64 ตัวอักษรของตัวเองมาให้ ID บอร์ดไม่เปลี่ยนแม้จับคู่ใหม่)
- `host` ใน config เป็นที่อยู่คงที่ ถ้าบอร์ดได้ IP ใหม่ ดูที่อยู่ล่าสุดด้วย `sdpo.cmd discover --device-id <ID>` แล้วแก้ค่านี้
  (ตั้งแต่ SDP v0.2.2 บอร์ดที่จับคู่แล้วจะไม่ตอบ `discover` เฉย ๆ ต้องใส่ ID ของบอร์ดด้วย)
- จับคู่ Connect เฉพาะกับ client ที่ไว้ใจให้เข้าถึงเครื่องได้อยู่แล้ว (รวมถึง shell) — ด่านสิทธิ์เขียนเฟิร์มแวร์เป็นด่านเพิ่มเฉพาะ RPC ชุด OTA

**ดูสถานะอย่างเดียว:** ไม่ต้องใส่ `artifactRoot` และปล่อย `allowFirmwareWrite` เป็น `false`

**เปิดให้อัปโหลด:** ตั้ง `allowFirmwareWrite` เป็น `true` และใส่ pairing ID ของ client ที่อนุญาตใน `writeClients`
(ใช้ค่า `current` ที่ `session.list` ตอบกลับเมื่อเรียกจาก client นั้น) แล้ววางไฟล์แอปที่เซ็นแล้วไว้ที่
`<artifactRoot>/<projectId>/<buildId>/sdp_ota.bin` บนเครื่องอัปโหลด

## JSON-RPC (ต้องยืนยันตัวตนแล้ว)

เรียกผ่าน JSON-RPC แบบ mTLS ของ Connect ตามปกติ สำหรับ Portable แอปฝั่งควบคุมต้องเปิดเซสชัน Portable ค้างไว้ —
ปิดเซสชันแล้ว CLI ที่กำลังรันจะถูกยกเลิกด้วย ถ้าเซสชันหลุดกลางการอัปโหลด ให้เช็กสถานะจริงของบอร์ดก่อน

| Method | Parameters | Result |
| --- | --- | --- |
| `sdp.ota.module` | `{}` | `id`, `status` (`ready`, `unconfigured`, `unavailable`), `version`, `firmwareWrite` ของ pairing นี้ และ `schemaVersion` เมื่อ ready — ready แปลว่ามี config และโมดูลครบ |
| `sdp.ota.status` | `{deviceId}` | `{type, deviceId, projectId, runningBuildId, bootState}` หลัง CLI ยืนยันคำตอบของบอร์ดแล้ว |
| `sdp.ota.upload` | `{deviceId, projectId, buildId}` | `{progress, result}` — `result` มี `type`, `outcome`, `deviceId`, `projectId`, `buildId`, `runningBuildId` และ `error` (ถ้ามี) ส่วน progress ส่งกลับมาพร้อมผลทีเดียว |

- client ส่งได้แค่ ID เท่านั้น ตัวโปรแกรม สคริปต์ ที่อยู่บอร์ด กุญแจ และ path ไฟล์เฟิร์มแวร์มาจากเครื่องอัปโหลดทั้งหมด
- Connect บันทึก log ว่า pairing ไหนสั่งบอร์ด/project/build อะไร และได้ผลอะไร
- `verified` ต้องมาจาก CLI ที่ยืนยันว่าบอร์ดบูตบิลด์ที่ขอสำเร็จแล้วเท่านั้น · `rolled_back`, `failed`, `unknown` เป็นผลแยกกัน
- ถ้าการเชื่อมต่อหลุดหรือได้ `unknown` ให้เรียก `sdp.ota.status` ก่อนลองใหม่ทุกครั้ง
- อัปโหลดได้ทีละงานต่อเครื่อง งานที่สองระหว่างที่งานแรกยังรันได้ error `-32003`
