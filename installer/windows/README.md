# ตัวติดตั้งแบบหน้าต่าง (Windows x64)

`SynaptaConnectSetup-v<รุ่น>.exe` ห่อ `synapta-connect.exe` ไว้ข้างใน ผู้ใช้ดับเบิลคลิก ตั้งพอร์ต แล้วกดติดตั้ง
ตัวติดตั้งขอสิทธิ์ผู้ดูแล (UAC) แล้วเรียก `synapta-connect.exe install` ต่อให้ ตรรกะติดตั้งจริงทั้งหมดอยู่ใน agent
ไฟล์ในโฟลเดอร์นี้ทำแค่หน้าจอ ตรวจไฟล์ที่ฝัง และส่งต่อคำสั่ง

- **build**: `scripts/build-release.ps1` สร้างให้เองคู่กับ backend ทุกครั้ง หรือเรียก `build-installer.ps1 -Agent -Notices -Version -Output` ตรง ๆ
  ใช้ `csc.exe` ของ .NET Framework 4.x ที่มากับ Windows ไม่ต้องลง SDK
- **รันบน**: Windows 10/11 และ Windows Server 2016 ขึ้นไปที่มีหน้า Desktop (.NET Framework 4.6.2 มากับ Server 2016)
  Server Core ไม่มี WinForms ให้ใช้ `SynaptaConnect-v<รุ่น>.exe install` จาก PowerShell แทน
- **หน้าจอ**: ตั้งพอร์ต → กำลังติดตั้ง → เสร็จ (เปิด panel ให้อัตโนมัติ) และถ้าเครื่องติดตั้งอยู่แล้ว จะมีปุ่มเปิดหน้าจัดการกับถอนการติดตั้ง
  (ติ๊ก "ลบข้อมูลการจับคู่และค่าตั้งด้วย" = `uninstall` เต็ม ไม่ติ๊ก = `uninstall --keep-data`)
- **เจ้าของเครื่อง** คือบัญชีที่ดับเบิลคลิกตัวติดตั้ง (อ่านก่อนยกสิทธิ์) แม้ UAC จะใช้รหัสของผู้ดูแลอีกคน บัญชีนี้จะเปิด panel ได้
- **ความปลอดภัย**: agent ที่ฝังถูกตรวจ SHA-256 ก่อนติดตั้ง (`--verify-payload`) และแตกลงโฟลเดอร์ใน ProgramData ที่มีแค่ SYSTEM/Administrators เขียนได้
- **ฟอนต์**: Anuphan (Regular/SemiBold) และ Chakra Petch SemiBold จาก Google Fonts (OFL 1.1) แบบ TTF ใน `fonts/`
  Anuphan ทำเป็นน้ำหนักคงที่จากไฟล์ variable ด้วย fontTools เพราะ GDI+ อ่าน WOFF2/variable font ไม่ได้
- **ตรวจดีไซน์**: `SynaptaConnectSetup.exe --preview <setup|progress|done|installed> out.png` เก็บภาพแต่ละหน้าโดยไม่ติดตั้งอะไร

ตัวติดตั้งใช้สำหรับติดตั้งครั้งแรกเท่านั้น การอัปเดตยังใช้ `SynaptaConnect-v<รุ่น>.exe` ตัวเปล่าที่เซ็นด้วย manifest ตามเดิม
