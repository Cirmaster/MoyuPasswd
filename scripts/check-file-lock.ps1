# 检查文件被谁占用
# 用法: .\check-file-lock.ps1 <文件路径>
# 示例: .\check-file-lock.ps1 "C:\xxx\target\release\build\xxx\build-script-build.exe"

param(
    [Parameter(Mandatory = $true, Position = 0)]
    [string]$FilePath
)

if (-not (Test-Path $FilePath)) {
    Write-Host "文件不存在: $FilePath" -ForegroundColor Red
    exit 1
}

Write-Host "检查文件: $FilePath" -ForegroundColor Cyan
Write-Host ("=" * 80)

# 方法1: 使用 handle.exe（Sysinternals 工具，最准确）
$handleExe = Get-Command handle.exe -ErrorAction SilentlyContinue
if ($handleExe) {
    Write-Host "`n[handle.exe] 正在搜索..." -ForegroundColor Yellow
    & handle.exe $FilePath 2>&1
    exit 0
}

# 方法2: 使用 openfiles（需要管理员权限）
Write-Host "`n[openfiles] 正在搜索（可能需要管理员权限）..." -ForegroundColor Yellow
$rawPath = $FilePath.Replace('\', '\\')
try {
    $output = openfiles /query /fo CSV 2>&1 | Where-Object { $_ -like "*$FilePath*" }
    if ($output) {
        $output | ForEach-Object { Write-Host $_ -ForegroundColor Green }
    } else {
        Write-Host "openfiles 未找到占用进程（可能需要管理员权限运行此脚本）" -ForegroundColor DarkYellow
    }
} catch {
    Write-Host "openfiles 执行失败" -ForegroundColor DarkYellow
}

# 方法3: 尝试独占打开文件来确认是否被锁
Write-Host "`n[文件锁测试]" -ForegroundColor Yellow
try {
    $stream = [System.IO.File]::Open($FilePath, 'Open', 'ReadWrite', 'None')
    Write-Host "当前文件未被锁定" -ForegroundColor Green
    $stream.Close()
} catch [System.IO.IOException] {
    Write-Host "文件正在被锁定: $($_.Exception.Message)" -ForegroundColor Red
}

# 方法4: 查找所有可能相关的进程
Write-Host "`n[相关进程扫描]" -ForegroundColor Yellow

# 搜索名称匹配的进程
$fileName = [System.IO.Path]::GetFileNameWithoutExtension($FilePath)
$matched = Get-Process | Where-Object { $_.ProcessName -like "*$fileName*" }
if ($matched) {
    Write-Host "找到同名进程:" -ForegroundColor Green
    $matched | Select-Object Id, ProcessName, Path, StartTime | Format-Table -AutoSize
}

# 搜索路径包含 target 目录的进程
$targetDir = Split-Path $FilePath
$relatedProcs = Get-Process | Where-Object {
    try { $_.Path -like "*target*" } catch { $false }
}
if ($relatedProcs) {
    Write-Host "路径包含 target 的进程:" -ForegroundColor Green
    $relatedProcs | Select-Object Id, ProcessName, Path | Format-Table -AutoSize
}

# 方法5: 检查 Windows Defender 实时保护
Write-Host "[Windows Defender 检查]" -ForegroundColor Yellow
$defender = Get-Process -Name "MsMpEng" -ErrorAction SilentlyContinue
if ($defender) {
    Write-Host "MsMpEng.exe (Windows Defender) 正在运行 - PID: $($defender.Id)" -ForegroundColor Red
    Write-Host "这很可能是占用文件的罪魁祸首！" -ForegroundColor Red
    Write-Host "建议将 target 目录加入 Defender 排除项:" -ForegroundColor Yellow
    Write-Host '  Add-MpPreference -ExclusionPath "你的target目录路径"' -ForegroundColor White
} else {
    Write-Host "MsMpEng.exe 未运行" -ForegroundColor Green
}

$security = Get-Process -Name "SecurityHealthService" -ErrorAction SilentlyContinue
if ($security) {
    Write-Host "SecurityHealthService 正在运行 - PID: $($security.Id)" -ForegroundColor Yellow
}
