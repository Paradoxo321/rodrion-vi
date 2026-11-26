import ctypes
import ctypes.wintypes as w
import os
import time
import tkinter as tk

u = ctypes.windll.user32
k = ctypes.windll.kernel32
psapi = ctypes.windll.psapi
pdh = ctypes.windll.pdh

PROCESS_QUERY_INFORMATION = 0x0400
PROCESS_VM_READ = 0x0010
GWL_EXSTYLE = -20
WS_EX_LAYERED = 0x00080000
WS_EX_TRANSPARENT = 0x00000020
WS_EX_TOPMOST = 0x00000008
WS_EX_TOOLWINDOW = 0x00000080
PDH_FMT_LARGE = 0x00000400


class PROCESS_MEMORY_COUNTERS_EX(ctypes.Structure):
    _fields_ = [
        ("cb", w.DWORD),
        ("PageFaultCount", w.DWORD),
        ("PeakWorkingSetSize", ctypes.c_size_t),
        ("WorkingSetSize", ctypes.c_size_t),
        ("QuotaPeakPagedPoolUsage", ctypes.c_size_t),
        ("QuotaPagedPoolUsage", ctypes.c_size_t),
        ("QuotaPeakNonPagedPoolUsage", ctypes.c_size_t),
        ("QuotaNonPagedPoolUsage", ctypes.c_size_t),
        ("PagefileUsage", ctypes.c_size_t),
        ("PeakPagefileUsage", ctypes.c_size_t),
        ("PrivateUsage", ctypes.c_size_t),
    ]


class MEMORYSTATUSEX(ctypes.Structure):
    _fields_ = [
        ("dwLength", w.DWORD),
        ("dwMemoryLoad", w.DWORD),
        ("ullTotalPhys", ctypes.c_uint64),
        ("ullAvailPhys", ctypes.c_uint64),
        ("ullTotalPageFile", ctypes.c_uint64),
        ("ullAvailPageFile", ctypes.c_uint64),
        ("ullTotalVirtual", ctypes.c_uint64),
        ("ullAvailVirtual", ctypes.c_uint64),
        ("ullAvailExtendedVirtual", ctypes.c_uint64),
    ]


class FILETIME(ctypes.Structure):
    _fields_ = [("dwLowDateTime", w.DWORD), ("dwHighDateTime", w.DWORD)]


class PDH_FMT_COUNTERVALUE_ITEM_W(ctypes.Structure):
    _fields_ = [
        ("szName", w.LPWSTR),
        ("CStatus", w.DWORD),
        ("_pad", w.DWORD),
        ("largeValue", ctypes.c_int64),
    ]


class GUID(ctypes.Structure):
    _fields_ = [("Data1", w.DWORD), ("Data2", w.WORD), ("Data3", w.WORD), ("Data4", ctypes.c_ubyte * 8)]


class DXGI_ADAPTER_DESC1(ctypes.Structure):
    _fields_ = [
        ("Description", w.WCHAR * 128),
        ("VendorId", w.UINT), ("DeviceId", w.UINT), ("SubSysId", w.UINT), ("Revision", w.UINT),
        ("DedicatedVideoMemory", ctypes.c_size_t),
        ("DedicatedSystemMemory", ctypes.c_size_t),
        ("SharedSystemMemory", ctypes.c_size_t),
        ("AdapterLuid", ctypes.c_int64),
        ("Flags", w.UINT),
    ]


def detect_gpu_caps():
    """Returns (gpu_name, dedicated_vram_mb, shared_gpu_mb) via DXGI."""
    try:
        dxgi = ctypes.windll.dxgi
        iid = GUID(0x1bc6ea02, 0xef36, 0x464f, (ctypes.c_ubyte * 8)(0xbf, 0x0c, 0x21, 0xca, 0x39, 0xe5, 0x16, 0x8a))
        factory = ctypes.c_void_p()
        if dxgi.CreateDXGIFactory1(ctypes.byref(iid), ctypes.byref(factory)) != 0:
            return ("GPU", 8192.0, 8192.0)
        vtbl_f = ctypes.cast(ctypes.cast(factory, ctypes.POINTER(ctypes.c_void_p))[0], ctypes.POINTER(ctypes.c_void_p))
        enum_adapters = ctypes.WINFUNCTYPE(ctypes.c_long, ctypes.c_void_p, w.UINT, ctypes.POINTER(ctypes.c_void_p))(vtbl_f[12])
        adapter = ctypes.c_void_p()
        if enum_adapters(factory, 0, ctypes.byref(adapter)) != 0:
            return ("GPU", 8192.0, 8192.0)
        vtbl_a = ctypes.cast(ctypes.cast(adapter, ctypes.POINTER(ctypes.c_void_p))[0], ctypes.POINTER(ctypes.c_void_p))
        get_desc = ctypes.WINFUNCTYPE(ctypes.c_long, ctypes.c_void_p, ctypes.POINTER(DXGI_ADAPTER_DESC1))(vtbl_a[10])
        desc = DXGI_ADAPTER_DESC1()
        get_desc(adapter, ctypes.byref(desc))
        name = desc.Description.replace("AMD Radeon ", "").strip() or "GPU"
        ded_mb = desc.DedicatedVideoMemory / (1024 * 1024)
        shr_mb = desc.SharedSystemMemory / (1024 * 1024)
        return (name, ded_mb or 8192.0, shr_mb or 8192.0)
    except Exception:
        return ("GPU", 8192.0, 8192.0)


class GpuVramReader:
    """Zero-subprocess Windows PDH GPU memory reader (<0.1% CPU)."""
    def __init__(self):
        self.ok = False
        self.q = w.HANDLE()
        self.c_ded = w.HANDLE()
        self.c_shr = w.HANDLE()
        self.c_proc_ded = w.HANDLE()
        self.c_proc_shr = w.HANDLE()
        try:
            if pdh.PdhOpenQueryW(None, 0, ctypes.byref(self.q)) == 0:
                r1 = pdh.PdhAddEnglishCounterW(self.q, r"\GPU Adapter Memory(*)\Dedicated Usage", 0, ctypes.byref(self.c_ded))
                r2 = pdh.PdhAddEnglishCounterW(self.q, r"\GPU Adapter Memory(*)\Shared Usage", 0, ctypes.byref(self.c_shr))
                r3 = pdh.PdhAddEnglishCounterW(self.q, r"\GPU Process Memory(*)\Dedicated Usage", 0, ctypes.byref(self.c_proc_ded))
                r4 = pdh.PdhAddEnglishCounterW(self.q, r"\GPU Process Memory(*)\Shared Usage", 0, ctypes.byref(self.c_proc_shr))
                self.ok = (r1 == 0 and r2 == 0 and r3 == 0 and r4 == 0)
        except Exception:
            self.ok = False

    def _array(self, counter):
        buf_sz = w.DWORD(0)
        cnt = w.DWORD(0)
        pdh.PdhGetFormattedCounterArrayW(counter, PDH_FMT_LARGE, ctypes.byref(buf_sz), ctypes.byref(cnt), None)
        if buf_sz.value == 0:
            return []
        buf = ctypes.create_string_buffer(buf_sz.value)
        if pdh.PdhGetFormattedCounterArrayW(counter, PDH_FMT_LARGE, ctypes.byref(buf_sz), ctypes.byref(cnt), buf) != 0:
            return []
        arr = ctypes.cast(buf, ctypes.POINTER(PDH_FMT_COUNTERVALUE_ITEM_W))
        return [(arr[i].szName or "", arr[i].largeValue) for i in range(cnt.value)]

    def sample(self, game_pid):
        """Returns (total_vram_mb, total_shared_mb, game_vram_mb, game_shared_mb)."""
        if not self.ok:
            return (0.0, 0.0, 0.0, 0.0)
        try:
            if pdh.PdhCollectQueryData(self.q) != 0:
                return (0.0, 0.0, 0.0, 0.0)
            tot_ded = sum(max(0, v) for _, v in self._array(self.c_ded)) / (1024 * 1024)
            tot_shr = sum(max(0, v) for _, v in self._array(self.c_shr)) / (1024 * 1024)
            g_ded = 0.0
            g_shr = 0.0
            if game_pid:
                tag = f"pid_{game_pid}_"
                g_ded = sum(max(0, v) for n, v in self._array(self.c_proc_ded) if n.startswith(tag)) / (1024 * 1024)
                g_shr = sum(max(0, v) for n, v in self._array(self.c_proc_shr) if n.startswith(tag)) / (1024 * 1024)
            return (tot_ded, tot_shr, g_ded, g_shr)
        except Exception:
            return (0.0, 0.0, 0.0, 0.0)


def ft_to_int(ft):
    return (ft.dwHighDateTime << 32) | ft.dwLowDateTime


def find_game_window():
    found = [None]

    def cb(hwnd, _):
        if not u.IsWindowVisible(hwnd):
            return True
        cls = ctypes.create_unicode_buffer(128)
        u.GetClassNameW(hwnd, cls, 128)
        title = ctypes.create_unicode_buffer(256)
        u.GetWindowTextW(hwnd, title, 256)
        if cls.value == "sgaWindowClass" or "VIctory" in title.value or "KLBF" in title.value:
            r = w.RECT()
            u.GetWindowRect(hwnd, ctypes.byref(r))
            if (r.right - r.left) >= 200 and (r.bottom - r.top) >= 200:
                pid = w.DWORD()
                u.GetWindowThreadProcessId(hwnd, ctypes.byref(pid))
                found[0] = (hwnd, pid.value, r)
                return False
        return True

    u.EnumWindows(ctypes.WINFUNCTYPE(w.BOOL, w.HWND, w.LPARAM)(cb), 0)
    return found[0]


class VictoryHUD:
    def __init__(self):
        self.gpu_name, self.vram_tot_mb, self.gpu_shr_tot_mb = detect_gpu_caps()
        self.vram_reader = GpuVramReader()

        self.root = tk.Tk()
        self.root.overrideredirect(True)
        self.root.attributes("-topmost", True)
        self.root.attributes("-alpha", 0.85)
        self.root.configure(bg="#0c1018")
        self.root.geometry("+52+36")

        frame = tk.Frame(self.root, bg="#0c1018", highlightbackground="#3b4d6b", highlightthickness=1, padx=8, pady=5)
        frame.pack()

        font = ("Consolas", 9, "bold")
        self.lbl_title = tk.Label(frame, text=f"VICTORY HUD ({self.gpu_name})   0s", fg="#8ecae6", bg="#0c1018", font=font, anchor="w")
        self.lbl_cpu = tk.Label(frame, text="CPU:   Jogo  0.0% | PC  0.0%", fg="#e0e6ed", bg="#0c1018", font=font, anchor="w")
        self.lbl_vram_pc = tk.Label(frame, text=f"VRAM GPU:    0/{self.vram_tot_mb:.0f} MB (0.0%)", fg="#72efdd", bg="#0c1018", font=font, anchor="w")
        self.lbl_vram_game = tk.Label(frame, text="VRAM Jogo:   0 MB | Shared: 0 MB", fg="#72efdd", bg="#0c1018", font=font, anchor="w")
        self.lbl_priv = tk.Label(frame, text="RAM Jogo:      0.0 MB (WS 0)", fg="#a8e6cf", bg="#0c1018", font=font, anchor="w")
        self.lbl_commit = tk.Label(frame, text="Commit Jogo:   0.0 MB", fg="#ffd166", bg="#0c1018", font=font, anchor="w")
        self.lbl_pc_ram = tk.Label(frame, text="PC RAM:       0/0 MB (0.0%)", fg="#ff6b6b", bg="#0c1018", font=font, anchor="w")
        self.lbl_pc_com = tk.Label(frame, text="PC Commit:    0/0 MB (0.0%)", fg="#ff6b6b", bg="#0c1018", font=font, anchor="w")
        for lbl in (
            self.lbl_title,
            self.lbl_cpu,
            self.lbl_vram_pc,
            self.lbl_vram_game,
            self.lbl_priv,
            self.lbl_commit,
            self.lbl_pc_ram,
            self.lbl_pc_com,
        ):
            lbl.pack(fill="x")

        self.root.update_idletasks()
        hwnd = u.GetParent(self.root.winfo_id()) or self.root.winfo_id()
        ex = u.GetWindowLongW(hwnd, GWL_EXSTYLE)
        u.SetWindowLongW(hwnd, GWL_EXSTYLE, ex | WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOPMOST | WS_EX_TOOLWINDOW)

        self.start_t = time.time()
        self.prev_sys = None
        self.prev_proc = None
        self.prev_pid = None
        self.seen_game = False
        self.miss_ticks = 0
        self.cpu_count = os.cpu_count() or 1
        self.root.withdraw()
        self.tick()

    def tick(self):
        info = find_game_window()
        if info:
            self.seen_game = True
            self.miss_ticks = 0
            if self.root.state() == "withdrawn":
                self.root.deiconify()
        else:
            self.miss_ticks += 1
            if self.seen_game and self.miss_ticks >= 3:
                self.root.destroy()
                return
            if not self.seen_game and self.miss_ticks >= 120:
                self.root.destroy()
                return
            self.root.after(1000, self.tick)
            return
        ms = MEMORYSTATUSEX()
        ms.dwLength = ctypes.sizeof(ms)
        k.GlobalMemoryStatusEx(ctypes.byref(ms))

        tot_ram = ms.ullTotalPhys / (1024 * 1024)
        used_ram = (ms.ullTotalPhys - ms.ullAvailPhys) / (1024 * 1024)
        ram_pct = (used_ram / tot_ram * 100.0) if tot_ram else 0.0

        tot_com = ms.ullTotalPageFile / (1024 * 1024)
        used_com = (ms.ullTotalPageFile - ms.ullAvailPageFile) / (1024 * 1024)
        com_pct = (used_com / tot_com * 100.0) if tot_com else 0.0

        idle, kern, user = FILETIME(), FILETIME(), FILETIME()
        k.GetSystemTimes(ctypes.byref(idle), ctypes.byref(kern), ctypes.byref(user))
        sys_idle, sys_tot = ft_to_int(idle), ft_to_int(kern) + ft_to_int(user)
        pc_cpu = 0.0
        if self.prev_sys:
            di = sys_idle - self.prev_sys[0]
            dt = sys_tot - self.prev_sys[1]
            if dt > 0:
                pc_cpu = max(0.0, min(100.0, (1.0 - di / dt) * 100.0))
        self.prev_sys = (sys_idle, sys_tot)

        game_cpu = 0.0
        ws_mb = priv_mb = com_mb = 0.0
        game_pid = None
        elapsed = int(time.time() - self.start_t)

        if info:
            hwnd, pid, rect = info
            game_pid = pid
            if pid != self.prev_pid:
                self.prev_pid = pid
                self.start_t = time.time()
                self.prev_proc = None
                elapsed = 0
            self.root.geometry(f"+{rect.left + 12}+{rect.top + 38}")

            hproc = k.OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, False, pid)
            if hproc:
                try:
                    pmc = PROCESS_MEMORY_COUNTERS_EX()
                    pmc.cb = ctypes.sizeof(pmc)
                    if psapi.GetProcessMemoryInfo(hproc, ctypes.byref(pmc), pmc.cb):
                        ws_mb = pmc.WorkingSetSize / (1024 * 1024)
                        com_mb = pmc.PrivateUsage / (1024 * 1024)
                        priv_mb = min(ws_mb, com_mb)
                    ct, et, kt, ut = FILETIME(), FILETIME(), FILETIME(), FILETIME()
                    if k.GetProcessTimes(hproc, ctypes.byref(ct), ctypes.byref(et), ctypes.byref(kt), ctypes.byref(ut)):
                        pt = ft_to_int(kt) + ft_to_int(ut)
                        now = time.perf_counter()
                        if self.prev_proc:
                            dp = pt - self.prev_proc[0]
                            dw = (now - self.prev_proc[1]) * 1e7
                            if dw > 0:
                                game_cpu = max(0.0, min(100.0, (dp / (dw * self.cpu_count)) * 100.0))
                        self.prev_proc = (pt, now)
                finally:
                    k.CloseHandle(hproc)

        tot_vram, tot_shr, g_vram, g_shr = self.vram_reader.sample(game_pid)
        vram_pct = (tot_vram / self.vram_tot_mb * 100.0) if self.vram_tot_mb else 0.0
        vram_col = "#ff6b6b" if vram_pct >= 88 else ("#ffd166" if vram_pct >= 72 else "#72efdd")
        shr_col = "#ff6b6b" if g_shr >= 5500 else ("#ffd166" if g_shr >= 3500 else "#72efdd")

        self.lbl_title.config(text=f"VICTORY HUD ({self.gpu_name})  {elapsed}s")
        self.lbl_cpu.config(text=f"CPU:   Jogo {game_cpu:4.1f}% | PC {pc_cpu:4.1f}%")
        self.lbl_vram_pc.config(text=f"VRAM GPU: {tot_vram:5.0f}/{self.vram_tot_mb:.0f} MB ({vram_pct:4.1f}%)", fg=vram_col)
        self.lbl_vram_game.config(text=f"VRAM Jogo:{g_vram:5.0f} MB | Shrd:{g_shr:5.0f} MB", fg=shr_col)
        self.lbl_priv.config(text=f"RAM Jogo: {priv_mb:6.0f} MB (WS {ws_mb:.0f})")
        self.lbl_commit.config(text=f"Commit Jogo: {com_mb:6.0f} MB")
        ram_col = "#ff6b6b" if ram_pct >= 85 else ("#ffd166" if ram_pct >= 70 else "#a8e6cf")
        com_col = "#ff6b6b" if com_pct >= 85 else ("#ffd166" if com_pct >= 70 else "#a8e6cf")
        self.lbl_pc_ram.config(text=f"PC RAM:   {used_ram:5.0f}/{tot_ram:.0f} MB ({ram_pct:4.1f}%)", fg=ram_col)
        self.lbl_pc_com.config(text=f"PC Commit:{used_com:5.0f}/{tot_com:.0f} MB ({com_pct:4.1f}%)", fg=com_col)

        self.root.after(1000, self.tick)

    def run(self):
        self.root.mainloop()


if __name__ == "__main__":
    VictoryHUD().run()
