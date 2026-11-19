<#
.SYNOPSIS
  Shows CPU time per thread over an interval, with thread names and the
  module + offset each thread started in.
#>
param([int]$ProcessId, [int]$Seconds = 10)
if (-not ('XbggThreads.Native' -as [type])) {
Add-Type -TypeDefinition @'
using System; using System.Runtime.InteropServices;
namespace XbggThreads {
public static class Native {
  [DllImport("kernel32.dll")] public static extern IntPtr OpenThread(uint access, bool inherit, uint id);
  [DllImport("kernel32.dll")] public static extern bool CloseHandle(IntPtr h);
  [DllImport("kernel32.dll")] public static extern int GetThreadDescription(IntPtr h, out IntPtr desc);
  [DllImport("kernel32.dll")] public static extern IntPtr LocalFree(IntPtr p);
  [DllImport("ntdll.dll")] public static extern int NtQueryInformationThread(IntPtr h, int cls, out IntPtr info, int len, IntPtr ret);
  public static string Name(uint tid) {
    IntPtr h = OpenThread(0x1000, false, tid);
    if (h == IntPtr.Zero) return "?";
    IntPtr p; string s = "";
    if (GetThreadDescription(h, out p) >= 0 && p != IntPtr.Zero) { s = Marshal.PtrToStringUni(p); LocalFree(p); }
    CloseHandle(h); return s;
  }
  public static long StartAddress(uint tid) {
    IntPtr h = OpenThread(0x0040, false, tid); // THREAD_QUERY_INFORMATION
    if (h == IntPtr.Zero) return 0;
    IntPtr addr; int st = NtQueryInformationThread(h, 9, out addr, IntPtr.Size, IntPtr.Zero); // ThreadQuerySetWin32StartAddress
    CloseHandle(h); return st == 0 ? addr.ToInt64() : 0;
  }
}
}
'@
}
$p = Get-Process -Id $ProcessId
$mods = $p.Modules | ForEach-Object { [pscustomobject]@{ Name = $_.ModuleName; Base = $_.BaseAddress.ToInt64(); End = $_.BaseAddress.ToInt64() + $_.ModuleMemorySize } }
function Where-Addr([long]$a) {
    $m = $mods | Where-Object { $a -ge $_.Base -and $a -lt $_.End } | Select-Object -First 1
    if ($m) { '{0}+0x{1:x}' -f $m.Name, ($a - $m.Base) } else { '0x{0:x}' -f $a }
}
$before = @{}; foreach ($t in $p.Threads) { $before[$t.Id] = $t.TotalProcessorTime.TotalMilliseconds }
Start-Sleep -Seconds $Seconds
$p.Refresh()
$p.Threads | ForEach-Object {
    $prev = if ($before.ContainsKey($_.Id)) { $before[$_.Id] } else { 0 }
    [pscustomobject]@{
        Tid   = $_.Id
        Name  = [XbggThreads.Native]::Name([uint32]$_.Id)
        Start = Where-Addr ([XbggThreads.Native]::StartAddress([uint32]$_.Id))
        CpuMs = [math]::Round($_.TotalProcessorTime.TotalMilliseconds - $prev)
        Prio  = $_.CurrentPriority
    }
} | Sort-Object CpuMs -Descending
