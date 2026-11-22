# Clicks through the installer's dialogs with UI Automation, screenshotting each one.
# Usage: gui-test.ps1 -Msi <path> -Out <dir>. Takes the Just me path so no UAC prompt appears.
param([Parameter(Mandatory)][string]$Msi, [Parameter(Mandatory)][string]$Out)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes, System.Drawing, System.Windows.Forms
$A = [System.Windows.Automation.AutomationElement]
$Scope = [System.Windows.Automation.TreeScope]
New-Item -ItemType Directory -Force $Out | Out-Null

function Setup-Window {
  $A::RootElement.FindFirst($Scope::Children,
    (New-Object System.Windows.Automation.PropertyCondition($A::NameProperty, 'blinkengraphs Setup')))
}
$CT = [System.Windows.Automation.ControlType]
# Win32 labels carry & mnemonics; compare without them. A label and its control can share a name, so match the type.
function Find($win, [string]$like, $type = $CT::Button) {
  foreach ($e in $win.FindAll($Scope::Descendants, [System.Windows.Automation.Condition]::TrueCondition)) {
    if ($e.Current.ControlType -eq $type -and ($e.Current.Name -replace '&', '') -like $like) { return $e }
  }
}
function Enabled($e) { $e -and $e.Current.IsEnabled }
# Windows Installer dialogs don't always act on UI Automation patterns, so fall back to the keyboard.
function Press($e, [string]$keys) {
  $e.SetFocus(); Start-Sleep -Milliseconds 200
  [System.Windows.Forms.SendKeys]::SendWait($keys)
}
function Click($e) {
  Write-Host "click '$($e.Current.Name)'"
  $p = $null
  if ($e.TryGetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern, [ref]$p)) { $p.Invoke() }
  else { Press $e '{ENTER}' }
}
function Toggle-To($e, [bool]$on) {
  $p = $null
  if (-not $e.TryGetCurrentPattern([System.Windows.Automation.TogglePattern]::Pattern, [ref]$p)) { return }
  if (($p.Current.ToggleState -eq 'On') -eq $on) { return }
  Write-Host "toggle '$($e.Current.Name)' to $on"
  $p.Toggle(); Start-Sleep -Milliseconds 300
  if (($p.Current.ToggleState -eq 'On') -ne $on) { Press $e ' ' }
}
function Dump($win) {
  foreach ($e in $win.FindAll($Scope::Descendants, [System.Windows.Automation.Condition]::TrueCondition)) {
    Write-Host ("  {0,-12} enabled={1,-5} '{2}'" -f $e.Current.ControlType.ProgrammaticName.Replace('ControlType.', ''), $e.Current.IsEnabled, $e.Current.Name)
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
    $n++
    $label = if ($finish) { 'finish' } elseif (Find $win 'Install just for you*' $CT::RadioButton) { 'scope' }
             elseif (Find $win 'I accept the terms*' $CT::CheckBox) { 'welcome' } elseif (Enabled (Find $win 'Install')) { 'ready' }
             else { 'progress' }
    Shot $win ('installer-{0:d2}-{1}' -f $n, $label)
    Dump $win
    $last = $names; $same = 0
  } elseif (++$same -gt 40) {
    throw "stuck on '$label': its actions had no effect"
  }
  if ($finish) {
    $launch = Find $win 'Launch blinkengraphs' $CT::CheckBox
    if ($launch) { Toggle-To $launch $false }
    Click $finish; break
  }
  $eula = Find $win 'I accept the terms*' $CT::CheckBox
  if ($eula) { Toggle-To $eula $true }
  $justMe = Find $win 'Install just for you*' $CT::RadioButton
  $sel = $null
  if ($justMe -and $justMe.TryGetCurrentPattern([System.Windows.Automation.SelectionItemPattern]::Pattern, [ref]$sel)) { $sel.Select() }
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
