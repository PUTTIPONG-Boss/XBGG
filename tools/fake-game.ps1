# A plain window that stands in for a game in the foreground during tests.
# Start it with a normal window style (a hidden start would also hide the form)
# — it hides its own console window.
param([int]$Seconds = 60)
Add-Type -Namespace FakeGame -Name Con -MemberDefinition @'
[DllImport("kernel32.dll")] public static extern IntPtr GetConsoleWindow();
[DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int c);
'@
[FakeGame.Con]::ShowWindow([FakeGame.Con]::GetConsoleWindow(), 0) | Out-Null
Add-Type -AssemblyName System.Windows.Forms
$f = New-Object System.Windows.Forms.Form
$f.Text = 'XBGG Fake Game'
$f.Width = 640; $f.Height = 360
$f.TopMost = $true
$t = New-Object System.Windows.Forms.Timer
$t.Interval = $Seconds * 1000
$t.add_Tick({ $f.Close() })
$t.Start()
$f.add_Shown({ $f.Activate() })
[void]$f.ShowDialog()
