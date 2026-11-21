#Requires -RunAsAdministrator
<#
  ทดสอบวงจรชีวิตของ service จริงบนเครื่องนี้ (ต้อง Administrator): ติดตั้ง → ตรวจสิทธิ์/ACL/firewall → อัปเดตสำเร็จ →
  อัปเดตล้ม (กู้รุ่นเดิม) → ลายเซ็นปลอม → ขัดจังหวะแล้วกู้ → ถอนการติดตั้ง

  ใช้ build ทดสอบที่ฝังกุญแจทดสอบ (ไม่ใช่กุญแจจริง) ใน $Work และ "ปฏิเสธรันถ้ามี service SynaptaConnect อยู่แล้ว"
  หรือพอร์ต 52401/52402 ถูกใช้ — ไม่ทับของจริงของเจ้าของ
  ผ่านทุกข้อ = exit 0; มีข้อล้ม = exit 1; รายงานเต็มที่ $Work\report.txt
  รัน:  powershell -ExecutionPolicy Bypass -File backend\tests\admin\service-lifecycle.ps1
#>
param(
    [string]$Work = (Join-Path $env:TEMP ('synapta-admin-test-' + [guid]::NewGuid().ToString('N'))),
    # ใช้ target ของ repo (dependency คอมไพล์แล้ว) — ไม่ต้อง build ทุกอย่างใหม่; ใส่ path อื่นได้ถ้าไม่อยากแตะ
    [string]$TargetDir = '',
    [switch]$KeepWork
)
# Continue (ไม่ใช่ Stop): PowerShell 5.1 ถือว่า stderr ของโปรแกรมภายนอก (cargo พิมพ์ความคืบหน้าลง stderr) เป็นข้อผิดพลาดร้ายแรงเมื่อเป็น Stop
$ErrorActionPreference = 'Continue'
$repo = (Resolve-Path (Join-Path $PSScriptRoot '..\..\..')).Path
$results = New-Object System.Collections.ArrayList
$installDir = Join-Path $env:ProgramFiles 'SynaptaConnect'
$dataDir = Join-Path $env:ProgramData 'SynaptaConnect'
$installedExe = Join-Path $installDir 'synapta-connect.exe'
$owner = "$env:USERDOMAIN\$env:USERNAME"

function Check($ok, $name, $detail = '') {
    $status = if ($ok) { 'PASS' } else { 'FAIL' }
    [void]$results.Add("[$status] $name $detail")
    Write-Host "[$status] $name $detail"
}
function Run($exe, [string[]]$arguments) {
    $out = & $exe @arguments 2>&1 | Out-String
    return [pscustomobject]@{ Code = $LASTEXITCODE; Out = $out }
}
function Sha($path) { (Get-FileHash -Algorithm SHA256 -LiteralPath $path).Hash }
function Api($method, $path, $body = $null) {
    # token สุ่มใหม่ทุกครั้งที่ service เริ่ม — อ่านใหม่ทุกครั้ง (Administrators อ่านไฟล์ได้)
    $token = (Get-Content -Raw (Join-Path $dataDir 'local_token')).Trim()
    $p = @{ Uri = "http://127.0.0.1:52402$path"; Method = $method; Headers = @{ 'X-Synapta-Local' = $token }; UseBasicParsing = $true; TimeoutSec = 10 }
    if ($body) { $p.Body = ($body | ConvertTo-Json); $p.ContentType = 'application/json' }
    $r = Invoke-WebRequest @p
    return ($r.Content | ConvertFrom-Json)
}
function WaitFor($what, [scriptblock]$cond, $seconds = 180) {
    $deadline = (Get-Date).AddSeconds($seconds)
    while ((Get-Date) -lt $deadline) {
        try { $v = & $cond; if ($v) { return $v } } catch { }
        Start-Sleep -Milliseconds 700
    }
    Check $false "หมดเวลารอ: $what"
    return $null
}
function SvcState($name) { $o = (& sc.exe query $name 2>&1 | Out-String); if ($o -match 'STATE\s*:\s*\d+\s+(\w+)') { $Matches[1] } else { 'ABSENT' } }
function Acl($path) { (& icacls.exe $path 2>&1 | Out-String) }
function Publish($releaseDir) {
    $cur = Join-Path $Work 'rel\current'
    if (Test-Path $cur) { Remove-Item -Recurse -Force $cur }
    New-Item -ItemType Directory -Force $cur | Out-Null
    Copy-Item (Join-Path $releaseDir '*') $cur
}

# ---------- เตรียม ----------
if ((SvcState 'SynaptaConnect') -ne 'ABSENT' -or (SvcState 'SynaptaConnectUpdater') -ne 'ABSENT') {
    throw 'มี service Synapta Connect อยู่แล้วบนเครื่องนี้ — ถอนก่อนทดสอบ (สคริปต์นี้ไม่ทับของจริง)'
}
if (Test-Path -LiteralPath $installDir) { throw 'มีโฟลเดอร์โปรแกรมเดิมอยู่ ใช้เครื่องทดสอบสะอาด' }
if (Test-Path -LiteralPath $dataDir) { throw 'มีข้อมูลเดิมอยู่ ใช้เครื่องทดสอบสะอาด' }
if (Get-NetTCPConnection -LocalPort 52401, 52402 -State Listen -ErrorAction SilentlyContinue) {
    throw 'พอร์ต 52401/52402 ถูกใช้อยู่ (agent โหมด console?) — ปิดก่อนทดสอบ'
}
if (Test-Path -LiteralPath $Work) { throw 'ใช้โฟลเดอร์งานใหม่เพื่อเก็บรายงานและผลทดสอบ' }
$Work = (New-Item -ItemType Directory -Path $Work -ErrorAction Stop).FullName
$src = Join-Path $Work 'src'
New-Item -ItemType Directory -Force "$src\backend", "$src\frontend" | Out-Null
robocopy (Join-Path $repo 'backend') "$src\backend" /E /XD target .git /NFL /NDL /NJH /NJS /NP | Out-Null
robocopy (Join-Path $repo 'frontend') "$src\frontend" /E /NFL /NDL /NJH /NJS /NP | Out-Null
$target = if ($TargetDir) { $TargetDir } else { Join-Path $repo 'backend\target' }
$installed = $false

try {
    Write-Host '== build เครื่องมือ + กุญแจทดสอบ =='
    Push-Location "$src\backend"
    & cargo build --target-dir $target --bin synapta-release 2>&1 | Select-Object -Last 2
    if ($LASTEXITCODE -ne 0) { throw 'build synapta-release ไม่ผ่าน' }
    $tool = Join-Path $target 'debug\synapta-release.exe'
    $keysA = Join-Path $Work 'keysA'; $keysB = Join-Path $Work 'keysB'
    $kA = & $tool keygen --out $keysA; $pubA = ($kA | Select-Object -Last 1).Trim()
    & $tool keygen --out $keysB | Out-Null
    $env:SYNAPTA_RELEASE_PUBKEY = $pubA

    Write-Host '== build v0.0.1 (รุ่นที่จะติดตั้ง) =='
    & cargo build --target-dir $target --bin synapta-connect 2>&1 | Select-Object -Last 1
    if ($LASTEXITCODE -ne 0) { throw 'build v0.0.1 ไม่ผ่าน' }
    $exe030 = Join-Path $Work 'SynaptaConnect-v0.0.1.exe'
    Copy-Item -ErrorAction Stop (Join-Path $target 'debug\synapta-connect.exe') $exe030

    Write-Host '== build v0.0.2 (รุ่นอัปเดต) =='
    $cargoToml = Get-Content -Raw 'Cargo.toml'
    Set-Content -NoNewline -Encoding UTF8 'Cargo.toml' ($cargoToml -replace '(?m)^version = "0\.0\.1"', 'version = "0.0.2"')
    & cargo build --target-dir $target --bin synapta-connect 2>&1 | Select-Object -Last 1
    if ($LASTEXITCODE -ne 0) { throw 'build v0.0.2 ไม่ผ่าน' }
    $exe031 = Join-Path $Work 'SynaptaConnect-v0.0.2.exe'
    Copy-Item -ErrorAction Stop (Join-Path $target 'debug\synapta-connect.exe') $exe031
    Pop-Location
    Check ((& $exe030 --version) -eq 'synapta-connect 0.0.1' -and (& $exe031 --version) -eq 'synapta-connect 0.0.2') 'build สองรุ่นรายงานเลขรุ่นถูกต้อง'

    # release: ดี (0.0.2) / ใช้ไม่ได้ (0.0.3 = exe ที่ไม่ใช่ service) / เซ็นด้วยกุญแจอื่น (0.0.4)
    $keyA = Join-Path $keysA 'synapta-release.key'; $keyB = Join-Path $keysB 'synapta-release.key'
    & $tool package --exe $exe031 --version 0.0.2 --key $keyA --out (Join-Path $Work 'rel\good') | Out-Null
    & $tool package --exe "$env:SystemRoot\System32\whoami.exe" --version 0.0.3 --key $keyA --out (Join-Path $Work 'rel\bad') --skip-version-check-for-tests | Out-Null
    & $tool package --exe $exe031 --version 0.0.4 --key $keyB --out (Join-Path $Work 'rel\forged') --skip-version-check-for-tests | Out-Null
    New-Item -ItemType Directory -Force (Join-Path $Work 'rel\current') | Out-Null

    # ---------- ติดตั้ง ----------
    Write-Host '== ติดตั้ง =='
    $plan = Run $exe030 @('install', '--plan', '--owner', $owner)
    Check ($plan.Code -eq 0 -and $plan.Out -match 'NT SERVICE') 'install --plan แสดงแผนโดยไม่แก้เครื่อง'
    Check ((SvcState 'SynaptaConnect') -eq 'ABSENT') 'plan ไม่สร้าง service'
    $inst = Run $exe030 @('install', '--owner', $owner, '--update-source', "dir:$(Join-Path $Work 'rel\current')", '--json')
    $installed = $true
    Check ($inst.Code -eq 0) 'install สำเร็จ (exit 0)' $inst.Out
    Check ((SvcState 'SynaptaConnect') -eq 'RUNNING') 'service SynaptaConnect ทำงานอยู่'
    $qc = & sc.exe qc SynaptaConnect | Out-String
    Check ($qc -match 'AUTO_START') 'เริ่มอัตโนมัติตอนบูต'
    Check ($qc -match 'NT SERVICE\\SynaptaConnect') 'รันด้วยบัญชีเสมือน NT SERVICE\SynaptaConnect'
    $proc = Get-CimInstance Win32_Process -Filter "Name='synapta-connect.exe'" | Where-Object { $_.ExecutablePath -eq $installedExe -and $_.CommandLine -match 'service run' }
    $ownerInfo = if ($proc) { Invoke-CimMethod -InputObject $proc -MethodName GetOwner } else { $null }
    Check ($ownerInfo -and $ownerInfo.User -eq 'SynaptaConnect' -and $ownerInfo.Domain -eq 'NT SERVICE') 'โปรเซส agent ไม่ใช่ SYSTEM/admin' "($($ownerInfo.Domain)\$($ownerInfo.User))"
    $health = WaitFor 'health' { Invoke-RestMethod 'http://127.0.0.1:52402/api/v1/local/health' -TimeoutSec 3 } 30
    Check ($health.version -eq '0.0.1') 'health ตอบรุ่น 0.0.1'
    $beforeDuplicateInstall = Sha $installedExe
    $duplicate = Run $exe030 @('install', '--owner', $owner)
    Check ($duplicate.Code -eq 4) 'ติดตั้งซ้ำถูกปฏิเสธ'
    if ((SvcState 'SynaptaConnect') -ne 'RUNNING' -or (Sha $installedExe) -ne $beforeDuplicateInstall) {
        throw 'ติดตั้งซ้ำเปลี่ยน service หรือไฟล์โปรแกรมเดิม'
    }
    Check $true 'service และไฟล์เดิมคงอยู่หลังสั่งติดตั้งซ้ำ'

    # ---------- ACL / firewall ----------
    Write-Host '== สิทธิ์/ACL/firewall =='
    $aclData = Acl $dataDir
    Check ($aclData -match 'NT SERVICE\\SynaptaConnect' -and $aclData -notmatch 'BUILTIN\\Users' -and $aclData -notmatch 'Everyone') 'ข้อมูลของ agent: service มีสิทธิ์ ไม่มี Users/Everyone'
    $tokenPath = Join-Path $dataDir 'local_token'
    $tokenAcl = Get-Acl -LiteralPath $tokenPath -ErrorAction Stop
    $ownerSid = ([Security.Principal.NTAccount]::new($owner)).Translate([Security.Principal.SecurityIdentifier])
    $agentSid = ([Security.Principal.NTAccount]::new('NT SERVICE\SynaptaConnect')).Translate([Security.Principal.SecurityIdentifier])
    $tokenRules = $tokenAcl.GetAccessRules($true, $true, [Security.Principal.SecurityIdentifier])
    $read = [Security.AccessControl.FileSystemRights]::Read
    $ownerRead = @($tokenRules | Where-Object {
        $_.IdentityReference -eq $ownerSid -and $_.AccessControlType -eq 'Allow' -and ($_.FileSystemRights -band $read) -eq $read
    })
    $unexpectedReaders = @($tokenRules | Where-Object {
        $_.AccessControlType -eq 'Allow' -and $_.IdentityReference.Value -notin @($ownerSid.Value, $agentSid.Value, 'S-1-5-18', 'S-1-5-32-544')
    })
    Check ($tokenAcl.AreAccessRulesProtected -and $ownerRead.Count -eq 1 -and $unexpectedReaders.Count -eq 0) 'local_token: ACL ให้เจ้าของอ่านและจำกัดบัญชี'
    Check ((Get-Content -Raw -LiteralPath $tokenPath) -cmatch '^[a-f0-9]{64}$') 'service เขียน local_token สำเร็จ'
    $aclWork = Acl (Join-Path $dataDir 'updater')
    Check ($aclWork -notmatch 'SynaptaConnect:' ) 'พื้นที่ตัวติดตั้ง (updater): agent เข้าไม่ได้'
    $aclStatus = Acl (Join-Path $dataDir 'updater-status')
    Check ($aclStatus -match 'SynaptaConnect:\(OI\)\(CI\)\(RX\)') 'updater-status: agent อ่านได้อย่างเดียว'
    $aclInstall = Acl $installDir
    Check ($aclInstall -notmatch 'NT SERVICE\\SynaptaConnect') 'Program Files: agent เขียนทับโปรแกรมตัวเองไม่ได้'
    $rules = Get-NetFirewallRule -DisplayName 'Synapta Connect*' -ErrorAction SilentlyContinue
    $badRules = @($rules | Where-Object { [string]$_.Profile -ne 'Domain, Private' })
    Check (@($rules).Count -eq 2 -and $badRules.Count -eq 0) 'firewall 2 กฎ เฉพาะ private/domain'
    Check ([bool](Get-ItemProperty 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Run' -Name SynaptaConnectHelper -ErrorAction SilentlyContinue)) 'ลงทะเบียน helper ตอนล็อกอิน'
    $sddl = (& sc.exe sdshow SynaptaConnectUpdater | Out-String)
    Check ($sddl -match 'RPLCRC;;;S-1-5-80-' -and $sddl -notmatch 'S-1-5-80-[0-9-]+\)\(A;;[^)]*(WP|DT|SD|WD)') 'agent เริ่มตัวติดตั้งได้อย่างเดียว (SDDL)'

    $status = Api 'GET' '/api/v1/local/status'
    Check ($status.agent.mode -eq 'service' -and $status.agent.privilege -eq 'standard') 'panel API: โหมด service สิทธิ์ standard' "($($status.agent.account))"
    Check ($status.screen_input.enabled -eq $false) 'จับภาพ/input ปิดเป็นค่าเริ่มต้น'
    $exchange = Api 'POST' '/api/v1/local/panel-session'
    Check ($exchange.code.Length -eq 32) 'ขอรหัสเปิด panel ได้'
    $page = Invoke-WebRequest 'http://127.0.0.1:52402/panel/' -UseBasicParsing
    Check ($page.StatusCode -eq 200 -and $page.Headers['Content-Security-Policy'] -match "default-src 'none'") 'หน้า panel ถูกเสิร์ฟพร้อม CSP'

    # ---------- อัปเดตสำเร็จ ----------
    Write-Host '== อัปเดต 0.0.1 -> 0.0.2 =='
    $hash030 = Sha $installedExe
    Publish (Join-Path $Work 'rel\good')
    [void](Api 'POST' '/api/v1/local/update/check')
    $avail = WaitFor 'พบรุ่นใหม่' { $s = Api 'GET' '/api/v1/local/update'; if ($s.state -eq 'update_available') { $s } } 60
    Check ($avail.available.version -eq '0.0.2') 'ตรวจพบ 0.0.2'
    [void](Api 'POST' '/api/v1/local/update/apply' @{ version = '0.0.2' })
    $done = WaitFor 'อัปเดตเสร็จ' { $s = Api 'GET' '/api/v1/local/update'; if ($s.current_version -eq '0.0.2' -and $s.state -ne 'applying') { $s } } 240
    Check ($done -and $done.state -eq 'up_to_date') 'อัปเดตสำเร็จ รุ่นใหม่ทำงาน' "(state=$($done.state))"
    Check ((Sha $installedExe) -eq (Sha $exe031)) 'ไฟล์ที่ติดตั้งคือ v0.0.2'
    Check ((SvcState 'SynaptaConnect') -eq 'RUNNING') 'service ยัง RUNNING หลังอัปเดต'
    Check ((Test-Path (Join-Path $dataDir 'updater\backup\0.0.1\synapta-connect.exe')) -and (Sha (Join-Path $dataDir 'updater\backup\0.0.1\synapta-connect.exe')) -eq $hash030) 'เก็บตัวติดตั้งรุ่นเดิมไว้'
    Check ((Get-Content -Raw (Join-Path $dataDir 'settings.json') | ConvertFrom-Json).owner -eq $owner) 'ค่าตั้ง (เจ้าของ) คงอยู่'
    Check (($done.history | Where-Object { $_.outcome -eq 'installed' }) -ne $null) 'ประวัติมี installed'

    # ---------- อัปเดตล้ม -> กู้ ----------
    Write-Host '== อัปเดต 0.0.3 (ใช้ไม่ได้) -> กู้คืน =='
    $hash031 = Sha $installedExe
    Publish (Join-Path $Work 'rel\bad')
    Start-Sleep -Seconds 16   # ข้ามเพดานตรวจด้วยมือ 15 วินาที
    [void](Api 'POST' '/api/v1/local/update/check')
    [void](WaitFor 'พบ 0.0.3' { $s = Api 'GET' '/api/v1/local/update'; if ($s.available.version -eq '0.0.3') { $s } } 60)
    [void](Api 'POST' '/api/v1/local/update/apply' @{ version = '0.0.3' })
    $rb = WaitFor 'กู้คืน' { $s = Api 'GET' '/api/v1/local/update'; if ($s.state -eq 'rolled_back') { $s } } 240
    Check ($rb -and @('service_start_failed', 'health_check_failed') -contains $rb.last_error.code) 'ล้ม -> กู้ และ panel เห็นสาเหตุ' "(code=$($rb.last_error.code))"
    Check ($rb.can_retry -eq $true) 'ลองใหม่ได้'
    Check ((Sha $installedExe) -eq $hash031) 'ไฟล์โปรแกรมกลับเป็นรุ่นเดิมทุกไบต์'
    Check ((Invoke-RestMethod 'http://127.0.0.1:52402/api/v1/local/health').version -eq '0.0.2') 'agent รุ่นเดิมทำงานต่อ'
    Check ((SvcState 'SynaptaConnect') -eq 'RUNNING') 'service RUNNING หลังกู้'

    # ---------- ลายเซ็นปลอม ----------
    Write-Host '== release ที่เซ็นด้วยกุญแจอื่น =='
    Publish (Join-Path $Work 'rel\forged')
    Start-Sleep -Seconds 16
    [void](Api 'POST' '/api/v1/local/update/check')
    $forged = WaitFor 'ปฏิเสธลายเซ็นปลอม' { $s = Api 'GET' '/api/v1/local/update'; if ($s.last_error.code -eq 'signature_invalid') { $s } } 60
    Check ($forged -ne $null -and $forged.available.version -ne '0.0.4') 'ลายเซ็นปลอมถูกปฏิเสธ ไม่เสนอเป็นรุ่นใหม่'
    Check ((Sha $installedExe) -eq $hash031) 'ไม่มีอะไรถูกติดตั้ง'

    # ---------- ขัดจังหวะ -> กู้ตอนตัวติดตั้งตื่น ----------
    Write-Host '== จำลองไฟดับกลางการสลับไฟล์ =='
    & sc.exe stop SynaptaConnect | Out-Null
    [void](WaitFor 'agent หยุด' { (SvcState 'SynaptaConnect') -eq 'STOPPED' } 60)
    $journal = '{"from":"0.0.2","to":"0.0.9","state":"swapping"}'
    Set-Content -NoNewline -Encoding ASCII (Join-Path $dataDir 'updater\journal.json') $journal
    Move-Item -Force $installedExe "$installedExe.old"
    Copy-Item "$env:SystemRoot\System32\whoami.exe" $installedExe
    & sc.exe start SynaptaConnectUpdater | Out-Null
    $healed = WaitFor 'กู้หลังขัดจังหวะ' { (Sha $installedExe) -eq $hash031 -and (Invoke-RestMethod 'http://127.0.0.1:52402/api/v1/local/health' -TimeoutSec 3).version -eq '0.0.2' } 120
    Check ([bool]$healed) 'ตัวติดตั้งกู้รุ่นเดิมหลังถูกขัดจังหวะ และ agent กลับมาทำงาน'
    $res = Get-Content -Raw (Join-Path $dataDir 'updater-status\result.json') | ConvertFrom-Json
    Check ($res.error.code -eq 'interrupted_rolled_back') 'ผลที่บันทึก: interrupted_rolled_back'
    Check (-not (Test-Path (Join-Path $dataDir 'updater\journal.json'))) 'journal ถูกเคลียร์'

    # ---------- หยุด/เริ่มด้วยคำสั่ง ----------
    Write-Host '== จัดการ service =='
    Check ((Run $installedExe @('service', 'stop')).Code -eq 0 -and (SvcState 'SynaptaConnect') -eq 'STOPPED') 'service stop'
    Check ((Run $installedExe @('service', 'start')).Code -eq 0) 'service start'
    [void](WaitFor 'agent กลับมา' { Invoke-RestMethod 'http://127.0.0.1:52402/api/v1/local/health' -TimeoutSec 3 } 60)
    $st = (Run $installedExe @('service', 'status', '--json'))
    Check ($st.Code -eq 0 -and ($st.Out | ConvertFrom-Json).agent.state -eq 'Running') 'service status --json'
}
catch {
    Check $false 'สคริปต์หยุดก่อนจบ' $_.Exception.Message
}
finally {
    Remove-Item Env:\SYNAPTA_RELEASE_PUBKEY -ErrorAction SilentlyContinue
    if ($installed) {
        Write-Host '== ถอนการติดตั้ง =='
        $un = Run $installedExe @('uninstall', '--json')
        Check ($un.Code -eq 0) 'uninstall จากไฟล์ที่ติดตั้งสำเร็จ (exit 0)' $un.Out
        if ($un.Code -ne 0) {
            $cleanup = Run $exe030 @('uninstall', '--json')
            Check ($cleanup.Code -eq 0) 'ถอนด้วยไฟล์ทดสอบต้นฉบับหลังถอนครั้งแรกไม่สำเร็จ' $cleanup.Out
        }
        Check ((SvcState 'SynaptaConnect') -eq 'ABSENT' -and (SvcState 'SynaptaConnectUpdater') -eq 'ABSENT') 'service ทั้งสองถูกลบ'
        Check (-not (Get-NetFirewallRule -DisplayName 'Synapta Connect*' -ErrorAction SilentlyContinue)) 'กฎ firewall ถูกลบ'
        Check (-not (Get-ItemProperty 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Run' -Name SynaptaConnectHelper -ErrorAction SilentlyContinue)) 'Run key ถูกลบ'
        Check (-not (Test-Path $dataDir)) 'ข้อมูลถูกลบ'
        $gone = WaitFor 'โฟลเดอร์โปรแกรมถูกลบ' { -not (Test-Path $installDir) } 30
        Check ([bool]$gone) 'โฟลเดอร์โปรแกรมถูกลบ (หลังโปรแกรมถอนปิดตัว)'
        Check (-not (Get-NetTCPConnection -LocalPort 52401, 52402 -State Listen -ErrorAction SilentlyContinue)) 'พอร์ตถูกปล่อย'
    }
    $results | Set-Content -Encoding UTF8 (Join-Path $Work 'report.txt')
    $fails = @($results | Where-Object { $_ -like '[[]FAIL]*' }).Count
    Write-Host ("== สรุป: ผ่าน {0} / ล้ม {1} — รายงาน {2} ==" -f ($results.Count - $fails), $fails, (Join-Path $Work 'report.txt'))
    if (-not $KeepWork -and $fails -eq 0) {
        foreach ($item in Get-ChildItem -LiteralPath $Work -Force | Where-Object Name -ne 'report.txt') {
            if ($item.DirectoryName -eq $Work -or $item.Parent.FullName -eq $Work) {
                Remove-Item -LiteralPath $item.FullName -Recurse -Force -ErrorAction SilentlyContinue
            }
        }
    }
    if ($fails -gt 0) { exit 1 }
}
