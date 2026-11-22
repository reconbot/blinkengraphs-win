# Clicks through the installer's dialogs (found with UI Automation, clicked with button messages), screenshotting each one.
# Usage: gui-test.ps1 -Msi <path> -Out <dir>. Takes the Just me path so no UAC prompt appears.
param([Parameter(Mandatory)][string]$Msi, [Parameter(Mandatory)][string]$Out)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes, System.Drawing
$A = [System.Windows.Automation.AutomationElement]
$Scope = [System.Windows.Automation.TreeScope]
New-Item -ItemType Directory -Force $Out | Out-Null

function Setup-Window {
  $A::RootElement.FindFirst($Scope::Children,
    (New-Object System.Windows.Automation.PropertyCondition($A::NameProperty, 'blinkengraphs Setup')))
}
Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class Win32 {
  [DllImport("user32.dll")] public static extern IntPtr SendMessage(IntPtr h, int msg, IntPtr w, IntPtr l);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
}
'@
# Windows Installer's dialog controls reach UI Automation only as Panes with no patterns,
# but each is a real Win32 button, so drive them with button messages by handle.
$BM_GETCHECK = 0x00F0; $BM_CLICK = 0x00F5
function Find($win, [string]$like) {
  foreach ($e in $win.FindAll($Scope::Descendants, [System.Windows.Automation.Condition]::TrueCondition)) {
    if (($e.Current.Name -replace '&', '') -like $like) { return $e }
  }
}
function Enabled($e) { $e -and $e.Current.IsEnabled }
function Send($e, [int]$msg) {
  [Win32]::SetForegroundWindow([IntPtr]$win.Current.NativeWindowHandle) | Out-Null
  [Win32]::SendMessage([IntPtr]$e.Current.NativeWindowHandle, $msg, [IntPtr]::Zero, [IntPtr]::Zero)
}
function Click($e) { Write-Host "click '$($e.Current.Name)'"; Send $e $BM_CLICK | Out-Null }
function Toggle-To($e, [bool]$on) {
  if (((Send $e $BM_GETCHECK).ToInt64() -eq 1) -ne $on) { Write-Host "toggle '$($e.Current.Name)' to $on"; Send $e $BM_CLICK | Out-Null }
}
function Dump($win) {
  foreach ($e in $win.FindAll($Scope::Descendants, [System.Windows.Automation.Condition]::TrueCondition)) {
    Write-Host ("  hwnd={0,-8} enabled={1,-5} '{2}'" -f $e.Current.NativeWindowHandle, $e.Current.IsEnabled, ($e.Current.Name -split "`n")[0])
  }
}
function Shot($win, [string]$name) {
  $r = $win.Current.BoundingRectangle
  $bmp = New-Object System.Drawing.Bitmap ([int]$r.Width), ([int]$r.Height)
  [System.Drawing.Graphics]::FromImage($bmp).CopyFromScreen([int]$r.X, [int]$r.Y, 0, 0, $bmp.Size)
  $bmp.Save((Join-Path $Out "$name.png"))
  Write-Host "screenshot $name"
}

$proc = Start-Process msiexec -ArgumentList "/i `"$Msi`" /l*v gui.log" -PassThru
$win = $null
foreach ($i in 1..60) { $win = Setup-Window; if ($win) { break }; Start-Sleep 1 }
if (-not $win) { throw 'installer window never appeared (no interactive desktop?)' }

$n = 0; $last = ''; $same = 0; $label = ''
foreach ($i in 1..120) {
  Start-Sleep -Milliseconds 800
  $win = Setup-Window
  if (-not $win) { break }
  $names = ($win.FindAll($Scope::Descendants, [System.Windows.Automation.Condition]::TrueCondition) |
    ForEach-Object { $_.Current.Name }) -join '|'
  $finish = Find $win 'Finish'
  if ($names -ne $last) {
    # Between dialogs the window can report an empty rectangle; catch it on the next pass.
    if ($win.Current.BoundingRectangle.Width -lt 1) { continue }
    $n++
    $label = if ($finish) { 'finish' } elseif (Find $win 'Install just for you*') { 'scope' }
             elseif (Find $win 'I accept the terms*') { 'welcome' } elseif (Enabled (Find $win 'Install')) { 'ready' }
             else { 'progress' }
    Shot $win ('installer-{0:d2}-{1}' -f $n, $label)
    Dump $win
    $last = $names; $same = 0
  } elseif (++$same -gt 40) {
    throw "stuck on '$label': its actions had no effect"
  }
  if ($finish) {
    $launch = Find $win 'Launch blinkengraphs'
    if ($launch) { Toggle-To $launch $false }
    Click $finish; break
  }
  $eula = Find $win 'I accept the terms*'
  if ($eula) { Toggle-To $eula $true }
  $justMe = Find $win 'Install just for you*'
  if ($justMe -and (Send $justMe $BM_GETCHECK).ToInt64() -ne 1) { Click $justMe }
  foreach ($b in 'Advanced', 'Next', 'Install') {
    $e = Find $win $b
    if (Enabled $e) { Click $e; break }
  }
}
if (-not $proc.WaitForExit(120000)) { throw 'installer still running after Finish' }
if ($proc.ExitCode -ne 0) { Get-Content gui.log -Tail 60; throw "msiexec exited $($proc.ExitCode)" }
$exe = "$env:LOCALAPPDATA\Programs\blinkengraphs\blinkengraphs.exe"
if (-not (Test-Path $exe)) { throw "GUI install did not create $exe" }
Write-Host "GUI install OK: $exe"
