# Clicks through the installer's dialogs with UI Automation, screenshotting each one.
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
# Win32 labels carry & mnemonics; compare without them.
function Find($win, [string]$like) {
  foreach ($e in $win.FindAll($Scope::Descendants, [System.Windows.Automation.Condition]::TrueCondition)) {
    if (($e.Current.Name -replace '&', '') -like $like) { return $e }
  }
}
function Enabled($e) { $e -and $e.Current.IsEnabled }
function Click($e) { $e.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern).Invoke() }
function Toggle-To($e, [bool]$on) {
  $p = $e.GetCurrentPattern([System.Windows.Automation.TogglePattern]::Pattern)
  if (($p.Current.ToggleState -eq 'On') -ne $on) { $p.Toggle() }
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
foreach ($_ in 1..60) { $win = Setup-Window; if ($win) { break }; Start-Sleep 1 }
if (-not $win) { throw 'installer window never appeared (no interactive desktop?)' }

$n = 0; $last = ''
foreach ($_ in 1..120) {
  Start-Sleep -Milliseconds 800
  $win = Setup-Window
  if (-not $win) { break }
  $names = ($win.FindAll($Scope::Descendants, [System.Windows.Automation.Condition]::TrueCondition) |
    ForEach-Object { $_.Current.Name }) -join '|'
  $finish = Find $win 'Finish'
  if ($names -ne $last) {
    $n++
    $label = if ($finish) { 'finish' } elseif (Find $win 'Install just for you*') { 'scope' }
             elseif (Find $win 'I accept the terms*') { 'welcome' } elseif (Enabled (Find $win 'Install')) { 'ready' }
             else { 'progress' }
    Shot $win ('installer-{0:d2}-{1}' -f $n, $label)
    $last = $names
  }
  if ($finish) {
    $launch = Find $win 'Launch blinkengraphs'
    if ($launch) { Toggle-To $launch $false }
    Click $finish; break
  }
  $eula = Find $win 'I accept the terms*'
  if ($eula) { Toggle-To $eula $true }
  $justMe = Find $win 'Install just for you*'
  if ($justMe) { $justMe.GetCurrentPattern([System.Windows.Automation.SelectionItemPattern]::Pattern).Select() }
  foreach ($b in 'Advanced', 'Next', 'Install') {
    $e = Find $win $b
    if (Enabled $e) { Click $e; break }
  }
}
$proc.WaitForExit(120000) | Out-Null
if ($proc.ExitCode -ne 0) { Get-Content gui.log -Tail 60; throw "msiexec exited $($proc.ExitCode)" }
$exe = "$env:LOCALAPPDATA\Programs\blinkengraphs\blinkengraphs.exe"
if (-not (Test-Path $exe)) { throw "GUI install did not create $exe" }
Write-Host "GUI install OK: $exe"
