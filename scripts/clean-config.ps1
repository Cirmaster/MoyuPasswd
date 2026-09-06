# 摸鱼密码 - 配置清理脚本
# 用于清除所有本地配置、数据库和凭据，恢复到初始状态

Write-Output "========================================="
Write-Output "  摸鱼密码 - 配置清理工具"
Write-Output "========================================="
Write-Output ""

# 1. 关闭应用和编译进程
Write-Output "[1/3] 关闭相关进程..."
Get-Process -Name "app", "cargo", "rustc" -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Seconds 1
Write-Output "  已终止所有相关进程"

# 2. 删除本地文件
Write-Output ""
Write-Output "[2/3] 删除本地文件..."
$appDataPath = "$env:APPDATA\com.moyu.passwd"
if (Test-Path $appDataPath) {
    Get-ChildItem -Path $appDataPath -File -ErrorAction SilentlyContinue | ForEach-Object {
        [System.IO.File]::Delete($_.FullName)
        Write-Output "  已删除: $($_.Name)"
    }
    # 如果目录为空，删除目录本身
    if ((Get-ChildItem -Path $appDataPath -ErrorAction SilentlyContinue).Count -eq 0) {
        Remove-Item -Path $appDataPath -Force -ErrorAction SilentlyContinue
        Write-Output "  已删除空目录"
    }
} else {
    Write-Output "  目录不存在，跳过"
}

# 3. 删除 Credential Manager 凭据
Write-Output ""
Write-Output "[3/3] 删除 Credential Manager 凭据..."
$credNames = @(
    "com.moyu.passwd.meta",
    "com.moyu.passwd"
)
foreach ($name in $credNames) {
    $result = cmdkey /delete:$name 2>&1
    if ($result -match "successfully") {
        Write-Output "  已删除凭据: $name"
    } else {
        Write-Output "  凭据不存在: $name"
    }
}

# 完成
Write-Output ""
Write-Output "========================================="
Write-Output "  清理完成！"
Write-Output "  现在可以重新运行应用了。"
Write-Output "========================================="
