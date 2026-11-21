<#
  เปิด agent จริงในโหมด console สำหรับทดสอบ panel (ไม่ใช่ service, ไม่ต้อง Administrator, ไม่แตะของจริง):
  พอร์ตแยก 52701 / 52702 / UDP 52700 (ของจริงคือ 52401/52402/52400), ข้อมูลอยู่ในโฟลเดอร์ชั่วคราว, ฟังเฉพาะเครื่องนี้
  มีอุปกรณ์ที่จับคู่ตัวอย่าง 2 เครื่องให้ล่วงหน้า, ออก URL ของ panel (ลิงก์ใช้ได้ครั้งเดียว 60 วินาที)

    powershell -ExecutionPolicy Bypass -File backend\tests\fixture\panel-fixture.ps1 [-Open] [-Exe path\to\SynaptaConnect.exe]
  เรียกซ้ำเมื่อลิงก์หมดอายุ (จะออกลิงก์ใหม่ให้ agent ตัวเดิม) · หยุดและล้าง:  ... -Stop

  สถานะที่ทดสอบได้กับ fixture นี้: runtime (mode=console), อุปกรณ์/OTP/revoke/kill-all, สวิตช์จับภาพ/input (ปิด, helper ไม่ต่อ),
  อัปเดต = not_configured หรือ failed (source_unreachable ถ้าตั้ง github: ที่ไม่มีอยู่)
  ที่ fixture นี้ทำให้เกิดไม่ได้ (ต้องมี service จริง): update_available/applying/rolled_back — ใช้ข้อมูลจำลอง (mock) ฝั่งหน้าเว็บ
#>
param([string]$Exe = (Join-Path $PSScriptRoot '..\..\target\release\synapta-connect.exe'), [switch]$Open, [switch]$Stop)
$ErrorActionPreference = 'Stop'
$dir = Join-Path $env:TEMP 'synapta-panel-fixture'
$pidFile = Join-Path $dir 'agent.pid'
$port = 52702

if ($Stop) {
    if (Test-Path $pidFile) {
        $id = [int](Get-Content $pidFile)
        $p = Get-Process -Id $id -ErrorAction SilentlyContinue
        # หยุดเฉพาะโปรเซสที่ fixture เริ่มเอง (ตรวจ path ว่าเป็น exe เดิม) — ไม่แตะ agent ตัวอื่นของเจ้าของ
        if ($p -and $p.Path -eq (Resolve-Path $Exe).Path) { Stop-Process -Id $id -Force }
    }
    Start-Sleep 1
    Remove-Item -Recurse -Force $dir -ErrorAction SilentlyContinue
    'หยุดและล้าง fixture แล้ว'; return
}

function Health { try { Invoke-RestMethod "http://127.0.0.1:$port/api/v1/local/health" -TimeoutSec 2 } catch { $null } }

if (-not (Health)) {
    if (Get-NetTCPConnection -LocalPort 52701, $port -State Listen -ErrorAction SilentlyContinue) { throw "พอร์ต 52701/$port ถูกโปรแกรมอื่นใช้อยู่" }
    New-Item -ItemType Directory -Force $dir | Out-Null
    # รูปแบบเดียวกับที่ agent เขียนเองหลังจับคู่ (public_key = base64url ของ Ed25519 ดิบ 32 ไบต์ — ตัวอย่างนี้ไม่ใช่กุญแจจริงของใคร)
    $now = [int][DateTimeOffset]::UtcNow.ToUnixTimeSeconds()
    $devices = @(
        @{ id = '3f2a9c4e1b7d5a60c8e2f1a4b9d3c7e5'; client_name = 'Synapta Code (laptop)'; public_key = 'AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8'; created_at = $now - 86400; last_seen = $now - 300 },
        @{ id = '9b8c7d6e5f4a3b2c1d0e9f8a7b6c5d4e'; client_name = 'Synapta Code (phone)'; public_key = 'gIGCg4SFhoeIiYqLjI2Oj5CRkpOUlZaXmJmam5ydnp8'; created_at = $now - 3600; last_seen = $now - 3600 }
    ) | ConvertTo-Json
    # ไม่มี BOM: agent อ่าน JSON ที่มี BOM ไม่ได้ (จะถือว่าไม่มีอุปกรณ์เลย)
    [IO.File]::WriteAllText((Join-Path $dir 'paired_devices.json'), $devices, (New-Object Text.UTF8Encoding $false))
    $proc = Start-Process -FilePath $Exe -PassThru -WindowStyle Hidden -ArgumentList @('--port', '52701', '--discovery-port', '52700', '--local-port', "$port", '--data-dir', $dir, '--name', 'panel-fixture')
    Set-Content $pidFile $proc.Id
    for ($i = 0; $i -lt 30 -and -not (Health); $i++) { Start-Sleep -Milliseconds 500 }
    if (-not (Health)) { throw 'agent ไม่ตอบ' }
}

$token = (Get-Content -Raw (Join-Path $dir 'local_token')).Trim()
$code = (Invoke-RestMethod -Method Post "http://127.0.0.1:$port/api/v1/local/panel-session" -Headers @{ 'X-Synapta-Local' = $token }).code
$url = "http://127.0.0.1:$port/synaptaconnect/#code=$code"
"panel: $url"
'(ลิงก์ใช้ได้ครั้งเดียว 60 วินาที; OTP จับคู่ที่ agent เปิดไว้ให้ตอนเริ่มอายุ 120 วินาที — กด "เปิดโหมดจับคู่" ใหม่ได้)'
if ($Open) { Start-Process $url }
