$ErrorActionPreference = 'Stop'

$exe = 'E:\Projects\Desktop-Apps\Tauri\MCDownload\target\debug\mcdl-app.exe'
$log = 'E:\Projects\Desktop-Apps\Tauri\MCDownload\app\.shots\tauri-run.log'
$shot = 'E:\Projects\Desktop-Apps\Tauri\MCDownload\app\.shots\40-desktop-real.png'

$p = Start-Process -FilePath $exe -PassThru -RedirectStandardError $log -RedirectStandardOutput "$log.out"
Start-Sleep -Seconds 10

if ($p.HasExited) {
    Write-Output "EXITED code=$($p.ExitCode)"
} else {
    $proc = Get-Process -Id $p.Id
    Write-Output "RUNNING pid=$($p.Id) title='$($proc.MainWindowTitle)'"

    # 截整个主屏，确认窗口真的画出来了
    Add-Type -AssemblyName System.Windows.Forms, System.Drawing
    $bounds = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds
    $bmp = New-Object System.Drawing.Bitmap $bounds.Width, $bounds.Height
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.CopyFromScreen($bounds.Location, [System.Drawing.Point]::Empty, $bounds.Size)
    $bmp.Save($shot, [System.Drawing.Imaging.ImageFormat]::Png)
    $g.Dispose(); $bmp.Dispose()
    Write-Output "SHOT $shot"

    Stop-Process -Id $p.Id -Force
    Write-Output "KILLED"
}

if (Test-Path $log) {
    $err = Get-Content $log -Raw
    if ($err) { Write-Output "--- stderr ---"; Write-Output $err }
}
