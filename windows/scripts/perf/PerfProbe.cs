// Win32 helpers for the CPU and drawing profile (win-perf-drive.ps1).
// Compiled by Windows PowerShell's Add-Type, so C# 5: no string
// interpolation, no tuples, no expression-bodied members.
//
// What it does: starts the app suspended inside a job object that limits
// its cores (affinity) and its CPU rate (a hard cap, as a share of the
// whole machine), so the throttle holds from the first instruction; reads
// the app's CPU per thread, the machine's busy time and DWM's composed
// frame count; and times the app's UI thread with WM_NULL round trips on a
// background thread, which needs no change to the app.
using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Runtime.InteropServices;
using System.Threading;

public static class PerfProbe {
    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    struct STARTUPINFO {
        public int cb; public string lpReserved, lpDesktop, lpTitle;
        public int dwX, dwY, dwXSize, dwYSize, dwXCountChars, dwYCountChars, dwFillAttribute, dwFlags;
        public short wShowWindow, cbReserved2; public IntPtr lpReserved2, hStdInput, hStdOutput, hStdError;
    }
    [StructLayout(LayoutKind.Sequential)]
    struct PROCESS_INFORMATION { public IntPtr hProcess, hThread; public int dwProcessId, dwThreadId; }
    [StructLayout(LayoutKind.Sequential)]
    struct JOBOBJECT_BASIC_LIMIT_INFORMATION {
        public long PerProcessUserTimeLimit, PerJobUserTimeLimit; public uint LimitFlags;
        public UIntPtr MinimumWorkingSetSize, MaximumWorkingSetSize; public uint ActiveProcessLimit;
        public UIntPtr Affinity; public uint PriorityClass, SchedulingClass;
    }
    [StructLayout(LayoutKind.Sequential)]
    struct IO_COUNTERS { public ulong a, b, c, d, e, f; }
    [StructLayout(LayoutKind.Sequential)]
    struct JOBOBJECT_EXTENDED_LIMIT_INFORMATION {
        public JOBOBJECT_BASIC_LIMIT_INFORMATION Basic; public IO_COUNTERS Io;
        public UIntPtr ProcessMemoryLimit, JobMemoryLimit, PeakProcessMemoryUsed, PeakJobMemoryUsed;
    }
    [StructLayout(LayoutKind.Sequential)]
    struct JOBOBJECT_CPU_RATE_CONTROL_INFORMATION { public uint ControlFlags; public uint CpuRate; }
    [StructLayout(LayoutKind.Sequential)]
    struct FILETIME { public uint Low, High; }

    [DllImport("kernel32.dll", SetLastError = true, CharSet = CharSet.Unicode)]
    static extern bool CreateProcess(string app, string cmd, IntPtr pa, IntPtr ta, bool inherit, uint flags, IntPtr env, string dir, ref STARTUPINFO si, out PROCESS_INFORMATION pi);
    [DllImport("kernel32.dll", SetLastError = true, CharSet = CharSet.Unicode)] static extern IntPtr CreateJobObject(IntPtr sa, string name);
    [DllImport("kernel32.dll", SetLastError = true)] static extern bool SetInformationJobObject(IntPtr job, int cls, ref JOBOBJECT_EXTENDED_LIMIT_INFORMATION info, int size);
    [DllImport("kernel32.dll", SetLastError = true)] static extern bool SetInformationJobObject(IntPtr job, int cls, ref JOBOBJECT_CPU_RATE_CONTROL_INFORMATION info, int size);
    [DllImport("kernel32.dll", SetLastError = true)] static extern bool AssignProcessToJobObject(IntPtr job, IntPtr process);
    [DllImport("kernel32.dll", SetLastError = true)] static extern uint ResumeThread(IntPtr thread);
    [DllImport("kernel32.dll", SetLastError = true)] static extern bool CloseHandle(IntPtr h);
    [DllImport("kernel32.dll")] static extern bool GetSystemTimes(out FILETIME idle, out FILETIME kernel, out FILETIME user);
    [DllImport("user32.dll", SetLastError = true)] static extern IntPtr SendMessageTimeout(IntPtr h, uint msg, IntPtr w, IntPtr l, uint flags, uint timeout, out IntPtr result);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("dwmapi.dll")] static extern int DwmGetCompositionTimingInfo(IntPtr h, IntPtr info);
    [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr after, int x, int y, int cx, int cy, uint flags);

    static IntPtr _job = IntPtr.Zero;

    /// <summary>
    /// Starts <paramref name="exe"/> suspended, puts it in a new job limited
    /// to <paramref name="affinity"/> (0: every core) and a hard cap of
    /// <paramref name="ratePercent"/> of the whole machine (0: none), then
    /// lets it run. The process inherits this process's environment. The job
    /// kills the app when the job handle closes (EndJob, or this process
    /// exiting), so no throttled process outlives the run.
    /// </summary>
    public static int StartThrottled(string exe, ulong affinity, double ratePercent) {
        EndJob();
        var si = new STARTUPINFO(); si.cb = Marshal.SizeOf(typeof(STARTUPINFO));
        PROCESS_INFORMATION pi;
        const uint CREATE_SUSPENDED = 0x4, CREATE_BREAKAWAY_FROM_JOB = 0x01000000;
        string dir = System.IO.Path.GetDirectoryName(exe);
        if (!CreateProcess(exe, "\"" + exe + "\"", IntPtr.Zero, IntPtr.Zero, false, CREATE_SUSPENDED | CREATE_BREAKAWAY_FROM_JOB, IntPtr.Zero, dir, ref si, out pi)) {
            // Breakaway is refused when this process's own job forbids it; try without.
            if (!CreateProcess(exe, "\"" + exe + "\"", IntPtr.Zero, IntPtr.Zero, false, CREATE_SUSPENDED, IntPtr.Zero, dir, ref si, out pi))
                throw new Exception("CreateProcess failed " + Marshal.GetLastWin32Error());
        }
        _job = CreateJobObject(IntPtr.Zero, null);
        if (_job == IntPtr.Zero) throw new Exception("CreateJobObject failed " + Marshal.GetLastWin32Error());
        var ext = new JOBOBJECT_EXTENDED_LIMIT_INFORMATION();
        const uint LIMIT_AFFINITY = 0x10, LIMIT_KILL_ON_JOB_CLOSE = 0x2000;
        ext.Basic.LimitFlags = LIMIT_KILL_ON_JOB_CLOSE;
        if (affinity != 0) { ext.Basic.LimitFlags |= LIMIT_AFFINITY; ext.Basic.Affinity = new UIntPtr(affinity); }
        if (!SetInformationJobObject(_job, 9, ref ext, Marshal.SizeOf(typeof(JOBOBJECT_EXTENDED_LIMIT_INFORMATION))))
            throw new Exception("job limits failed " + Marshal.GetLastWin32Error());
        if (ratePercent > 0) {
            // CpuRate is in hundredths of a percent of all the machine's cores.
            var rate = new JOBOBJECT_CPU_RATE_CONTROL_INFORMATION();
            rate.ControlFlags = 0x1 | 0x4; // ENABLE | HARD_CAP
            rate.CpuRate = (uint)Math.Round(ratePercent * 100);
            if (!SetInformationJobObject(_job, 15, ref rate, Marshal.SizeOf(typeof(JOBOBJECT_CPU_RATE_CONTROL_INFORMATION))))
                throw new Exception("job cpu rate failed " + Marshal.GetLastWin32Error());
        }
        if (!AssignProcessToJobObject(_job, pi.hProcess)) throw new Exception("AssignProcessToJobObject failed " + Marshal.GetLastWin32Error());
        ResumeThread(pi.hThread);
        CloseHandle(pi.hThread); CloseHandle(pi.hProcess);
        return pi.dwProcessId;
    }

    /// <summary>Closes the job, which ends every process still in it.</summary>
    public static void EndJob() {
        if (_job != IntPtr.Zero) { CloseHandle(_job); _job = IntPtr.Zero; }
    }

    /// <summary>Pins this process (the harness) to <paramref name="mask"/>, away from the app's cores.</summary>
    public static void PinSelf(ulong mask) {
        if (mask != 0) Process.GetCurrentProcess().ProcessorAffinity = new IntPtr((long)mask);
    }

    static ulong Ticks(FILETIME f) { return ((ulong)f.High << 32) | f.Low; }

    /// <summary>The machine's busy time so far, all cores, in milliseconds.</summary>
    public static double MachineBusyMs() {
        FILETIME idle, kernel, user;
        GetSystemTimes(out idle, out kernel, out user);
        // Kernel time includes idle time.
        return (Ticks(kernel) + Ticks(user) - Ticks(idle)) / 10000.0;
    }

    /// <summary>A process's CPU so far in ms, or -1 when it cannot be read.</summary>
    public static double ProcessMs(int pid) {
        try { using (var p = Process.GetProcessById(pid)) return p.TotalProcessorTime.TotalMilliseconds; }
        catch { return -1; }
    }

    /// <summary>CPU so far of every process with this name, in ms (-1 when none could be read).</summary>
    public static double NamedMs(string name) {
        double sum = 0; bool any = false;
        foreach (var p in Process.GetProcessesByName(name)) {
            try { sum += p.TotalProcessorTime.TotalMilliseconds; any = true; } catch { }
            p.Dispose();
        }
        return any ? sum : -1;
    }

    /// <summary>Each thread's CPU so far, in ms, keyed by thread id.</summary>
    public static Dictionary<int, double> ThreadMs(int pid) {
        var map = new Dictionary<int, double>();
        try {
            using (var p = Process.GetProcessById(pid)) {
                foreach (ProcessThread t in p.Threads) {
                    try { map[t.Id] = t.TotalProcessorTime.TotalMilliseconds; } catch { }
                }
            }
        } catch { }
        return map;
    }

    /// <summary>"workingSetMB privateMB" of the process.</summary>
    public static string Memory(int pid) {
        try {
            using (var p = Process.GetProcessById(pid)) {
                p.Refresh();
                return string.Format("{0:0} {1:0}", p.WorkingSet64 / 1048576.0, p.PrivateMemorySize64 / 1048576.0);
            }
        } catch { return "- -"; }
    }

    public static int UiThread(IntPtr hwnd) { uint pid; return (int)GetWindowThreadProcessId(hwnd, out pid); }

    [DllImport("kernel32.dll")] static extern IntPtr OpenThread(uint access, bool inherit, int tid);
    [DllImport("ntdll.dll")] static extern int NtQueryInformationThread(IntPtr thread, int cls, out IntPtr info, int size, IntPtr ret);
    static Dictionary<int, string> _startModule = new Dictionary<int, string>();
    static int _modulesPid;
    static List<KeyValuePair<string, long[]>> _modules;

    /// <summary>
    /// The module a thread started in (its Win32 start address), e.g.
    /// d3d10warp for WARP's rasteriser threads or ntdll for the thread
    /// pool; remembered per thread id.
    /// </summary>
    public static string ThreadModule(int pid, int tid) {
        string m;
        if (_startModule.TryGetValue(tid, out m)) return m;
        if (_modules == null || _modulesPid != pid) {
            _modules = new List<KeyValuePair<string, long[]>>(); _modulesPid = pid;
            try {
                using (var p = Process.GetProcessById(pid)) {
                    foreach (ProcessModule mod in p.Modules) {
                        long b = mod.BaseAddress.ToInt64();
                        _modules.Add(new KeyValuePair<string, long[]>(System.IO.Path.GetFileNameWithoutExtension(mod.ModuleName).ToLowerInvariant(), new[] { b, b + mod.ModuleMemorySize }));
                    }
                }
            } catch { }
        }
        m = "?";
        IntPtr h = OpenThread(0x0040, false, tid); // THREAD_QUERY_INFORMATION
        if (h != IntPtr.Zero) {
            IntPtr start;
            if (NtQueryInformationThread(h, 9, out start, IntPtr.Size, IntPtr.Zero) == 0) {
                long a = start.ToInt64();
                foreach (var kv in _modules) if (a >= kv.Value[0] && a < kv.Value[1]) { m = kv.Key; break; }
                if (m == "?") { _modules = null; }
            }
            CloseHandle(h);
        }
        if (m != "?") _startModule[tid] = m;
        return m;
    }

    /// <summary>The first bytes of DWM_TIMING_INFO for each size tried, for checking the layout.</summary>
    public static string DwmDebug() {
        var sb = new System.Text.StringBuilder();
        IntPtr buf = Marshal.AllocHGlobal(512);
        try {
            foreach (int size in new[] { 292, 296, 304, 312, 320 }) {
                for (int i = 0; i < 512; i++) Marshal.WriteByte(buf, i, 0);
                Marshal.WriteInt32(buf, 0, size);
                int hr = DwmGetCompositionTimingInfo(IntPtr.Zero, buf);
                sb.Append("size " + size + " hr=0x" + hr.ToString("X") + ":");
                if (hr == 0) for (int o = 0; o < 96; o += 4) sb.Append(" " + Marshal.ReadInt32(buf, o));
                sb.Append(" | ");
            }
        } finally { Marshal.FreeHGlobal(buf); }
        return sb.ToString();
    }

    static int _timingSize = 0;

    /// <summary>
    /// Frames DWM has composed so far (DWM_TIMING_INFO.cFrame), or -1. The
    /// struct is packed to 1 byte (dwmapi.h includes pshpack1.h); the size
    /// that the call accepts is remembered.
    /// </summary>
    public static long DwmFrames() {
        IntPtr buf = Marshal.AllocHGlobal(512);
        try {
            int[] sizes = _timingSize != 0 ? new[] { _timingSize } : new[] { 292, 304, 312 };
            foreach (int size in sizes) {
                for (int i = 0; i < 512; i++) Marshal.WriteByte(buf, i, 0);
                Marshal.WriteInt32(buf, 0, size);
                if (DwmGetCompositionTimingInfo(IntPtr.Zero, buf) == 0) {
                    _timingSize = size;
                    // cFrame: after cbSize(4) rateRefresh(8) qpcRefreshPeriod(8) rateCompose(8) qpcVBlank(8) cRefresh(8) cDXRefresh(4) qpcCompose(8), packed.
                    return Marshal.ReadInt64(buf, size == 292 ? 56 : 64);
                }
            }
            return -1;
        } finally { Marshal.FreeHGlobal(buf); }
    }

    // ---- UI-thread responsiveness ------------------------------------

    static Thread _pinger;
    static volatile bool _ping;
    static List<double> _rtt = new List<double>();
    static readonly object _gate = new object();
    static IntPtr _target;

    /// <summary>
    /// Sends WM_NULL to <paramref name="hwnd"/> every <paramref name="everyMs"/>
    /// on a background thread and records how long each takes to come back.
    /// A sent message is handled only when the UI thread next pumps, so the
    /// round trip is how long a click or a key would wait to be seen.
    /// </summary>
    public static void StartPing(IntPtr hwnd, int everyMs) {
        StopPing();
        _target = hwnd; _ping = true;
        lock (_gate) _rtt = new List<double>();
        _pinger = new Thread(delegate () {
            var sw = new Stopwatch();
            while (_ping) {
                IntPtr r;
                sw.Restart();
                SendMessageTimeout(_target, 0, IntPtr.Zero, IntPtr.Zero, 0, 10000, out r);
                double ms = sw.Elapsed.TotalMilliseconds;
                lock (_gate) _rtt.Add(ms);
                Thread.Sleep(everyMs);
            }
        });
        _pinger.IsBackground = true;
        _pinger.Start();
    }

    public static void StopPing() {
        _ping = false;
        if (_pinger != null) { _pinger.Join(15000); _pinger = null; }
    }

    /// <summary>"n p50 p95 max over50" of the round trips since the last call, and clears them.</summary>
    public static string TakePing() {
        List<double> got;
        lock (_gate) { got = _rtt; _rtt = new List<double>(); }
        if (got.Count == 0) return "0 - - - -";
        got.Sort();
        int over = 0; foreach (double d in got) if (d > 50) over++;
        return string.Format("{0} {1:0.0} {2:0.0} {3:0.0} {4}", got.Count, got[got.Count / 2], got[Math.Min(got.Count - 1, (int)(got.Count * 0.95))], got[got.Count - 1], over);
    }
}
