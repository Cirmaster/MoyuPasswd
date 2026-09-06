# 终止残留的 cargo/rustc 进程
# 当遇到 "另一个程序正在使用此文件" 错误时运行

Write-Output "正在终止 cargo/rustc 进程..."
Get-Process -Name "cargo", "rustc" -ErrorAction SilentlyContinue | Stop-Process -Force
Write-Output "已终止，可以重新编译了。"
