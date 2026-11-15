"""What the proof scripts need from the OS, the same on macOS, Linux and Windows (BL168.8).

* `ui_socket_path()` / `connect_ui(path, timeout)`: the editor window's local socket. Python on Windows has
  no `socket.AF_UNIX` (the editor listens on a native Windows AF_UNIX socket, `uds_windows`), so there the
  socket is opened through Winsock with `ctypes` and handed back as an ordinary `socket.socket`.
* `ffmpeg()` / `ffprobe()`: the executables, found the way the editor finds them (`TESSEL_FFMPEG` /
  `TESSEL_FFPROBE`, then `PATH`, then the WinGet Gyan package), never assumed to be on `PATH`.
* `kill_tree(process)` / `kill_group(pid)`: ends a process and what it started (on Windows the WebView2
  helpers too); `kill_group` is `killpg(SIGKILL)` on Unix.
* `program(path, *args)`: the command line that starts `path`. Windows cannot start a `#!` script (the
  proofs' stand-in editors), so there one runs under its interpreter, and is still the process started.
* `alive(pid)` / `process_tree(pid)`: whether a process runs, and the processes it started, read without
  sending anything (on Windows `os.kill(pid, 0)` is not a probe: it sends CTRL+C to the console).
* `TEMP` / `temp_path(name)`: `/tmp` on Unix, exactly as the scripts always used it (short socket paths on
  macOS), and the system's temporary folder on Windows, where there is no `/tmp`.
* `executable(path)`: `path`, or `path.exe` on Windows when only that exists.
* `rss_kib(pid)` / `processes()`: resident memory of a process and the process table, without `ps`.
* `capture_window(pid, path)`: the OS capture of a process's window, `PrintWindow` on Windows (no click).
* `lock_file(handle, blocking)` / `unlock_file(handle)`: an exclusive whole-file lock, `flock` on Unix and
  `msvcrt.locking` on Windows (no `fcntl` there).
* Importing it on Windows re-runs the script in Python's UTF-8 mode (see below).
"""
import glob
import os
import shutil
import socket
import subprocess
import sys
import tempfile
import time

WINDOWS = os.name == 'nt'
if WINDOWS and not sys.flags.utf8_mode:
    # The editor speaks UTF-8 (JSON replies, fixture files, titles like "Ação"), but Python on Windows
    # decodes text with the ANSI code page (cp1252) unless it runs in UTF-8 mode, and a mode cannot be
    # switched once the interpreter started. So the script that imported this runs again, the same
    # command line, in UTF-8 mode, and its exit status is this one's.
    sys.exit(subprocess.run([sys.executable, '-X', 'utf8', *sys.orig_argv[1:]]).returncode)
TEMP = tempfile.gettempdir() if WINDOWS else '/tmp'


def temp_path(name):
    """`/tmp/<name>` on Unix (unchanged), `<system temp>\\<name>` on Windows."""
    return os.path.join(TEMP, name)


def executable(path):
    """`path` made absolute, with `.exe` on Windows when only that file exists (the scripts name
    `target/debug/tessel-editor`; a relative name with `/` does not start a process there)."""
    path = os.path.abspath(str(path))
    if WINDOWS and not os.path.exists(path) and os.path.exists(path + '.exe'):
        return path + '.exe'
    return path


def rss_kib(pid):
    """Resident memory of `pid` in KiB (`ps -o rss=` on Unix; the working set on Windows), or `None`."""
    if not WINDOWS:
        out = subprocess.run(['ps', '-o', 'rss=', '-p', str(pid)], capture_output=True, text=True).stdout.strip()
        return int(out) if out.isdigit() else None
    for row in processes():
        if row['pid'] == pid:
            return row['rss_kib']
    return None


def processes():
    """Every process as `{pid, ppid, rss_kib, name, command}` (`ps -axo` on Unix, CIM on Windows)."""
    if not WINDOWS:
        out = subprocess.run(['ps', '-axo', 'pid=,ppid=,rss=,comm='], capture_output=True, text=True).stdout
        rows = []
        for line in out.splitlines():
            parts = line.split(None, 3)
            if len(parts) == 4 and parts[0].isdigit():
                rows.append({'pid': int(parts[0]), 'ppid': int(parts[1]), 'rss_kib': int(parts[2]),
                             'name': os.path.basename(parts[3]), 'command': parts[3]})
        return rows
    script = ('Get-CimInstance Win32_Process | Select-Object ProcessId,ParentProcessId,WorkingSetSize,Name,CommandLine'
              ' | ConvertTo-Json -Compress')
    out = subprocess.run(['powershell.exe', '-NoProfile', '-NonInteractive', '-Command', script],
                         capture_output=True, text=True, encoding='utf-8', errors='replace').stdout
    import json
    listed = json.loads(out or '[]')
    return [{'pid': row['ProcessId'], 'ppid': row['ParentProcessId'], 'rss_kib': int(row['WorkingSetSize'] or 0) // 1024,
             'name': row['Name'] or '', 'command': row['CommandLine'] or row['Name'] or ''}
            for row in (listed if isinstance(listed, list) else [listed])]


def kill_group(pid, quiet=True):
    """`killpg(pid, SIGKILL)` on Unix; on Windows the whole process tree (`taskkill /T /F`).

    `taskkill /T` finds the descendants through the leader, so once the leader is gone it finds nothing:
    then what is left of the tree (`process_tree`, which still sees the orphans) is ended by PID."""
    if not WINDOWS:
        import signal
        try:
            os.killpg(pid, signal.SIGKILL)
        except ProcessLookupError:
            if not quiet:
                raise
        return
    done = subprocess.run(['taskkill', '/T', '/F', '/PID', str(pid)], capture_output=True, timeout=30)
    if done.returncode == 0:
        return
    left = process_tree(pid)
    if left:
        done = subprocess.run(['taskkill', '/F', *(word for member in left for word in ('/PID', str(member)))],
                              capture_output=True, timeout=30)
    if done.returncode != 0 and not quiet:
        raise OSError(f"taskkill /T /F /PID {pid}: {done.stderr.decode('oem', errors='replace').strip()[:120]}")


def program(path, *args):
    """The command line that starts the program at `path` with `args`: `[path, *args]`.

    On Unix that is all (the kernel reads a script's `#!`), and on Windows too for a real executable.
    Windows cannot start a script: there a file whose `#!` names Python runs under this Python (in
    UTF-8 mode, like the editor; the shebang's own options kept) and one whose `#!` names `sh`/`bash`
    under Git's `sh`. The process started is that interpreter running the script, so its `pid`,
    `terminate()` and `kill()` reach the script itself, not a wrapper around it."""
    if not WINDOWS:
        return [path, *args]
    if os.path.dirname(str(path)):
        # A path (not a bare name looked up on PATH): `target/debug/tessel-editor` is `...\tessel-editor.exe` there.
        path = executable(path)
    command = [path, *args]
    interpreter = _shebang(path)
    if not interpreter:
        return command
    name = os.path.basename(interpreter[0]).lower()
    if name.endswith('.exe'):
        name = name[:-4]
    if name.startswith('python'):
        # A venv's `python.exe` is a launcher that starts the base interpreter as a SECOND process; the
        # stand-ins only need the standard library, so the base interpreter runs them directly.
        python = getattr(sys, '_base_executable', None) or sys.executable
        return [python, '-X', 'utf8', *interpreter[1:], *command]
    if name in ('sh', 'bash'):
        return [git_sh(), *interpreter[1:], *command]
    return command


def _shebang(path):
    """The interpreter and its options from the `#!` line of the file at `path`, or `None` (no such
    file, a binary such as a PE `MZ` executable, no `#!`). `#!/usr/bin/env NAME` names `NAME`."""
    try:
        with open(path, 'rb') as handle:
            head = handle.read(512)
    except OSError:
        return None
    if not head.startswith(b'#!'):
        return None
    line = head[2:].split(b'\n', 1)[0].decode('utf-8', errors='replace').strip()
    if os.path.isfile(line):
        return [line]           # an interpreter path with spaces (`C:\Program Files\...\python.exe`)
    words = line.split()
    if words and os.path.basename(words[0]).lower() in ('env', 'env.exe'):
        words = words[1:]
        while words and words[0].startswith('-'):
            words = words[1:]   # `env -S python3 -u`: the options of `env`, not of the program
    return words or None


def suspend(pid):
    """Stops every thread of process `pid` until `resume(pid)`: `SIGSTOP` on Unix; on Windows, which has no such signal,
    `NtSuspendProcess` (the process is frozen the same way: it neither runs nor answers)."""
    _suspend_or_resume(pid, 'NtSuspendProcess', 'SIGSTOP')


def resume(pid):
    """Lets a process stopped by `suspend` run again: `SIGCONT` on Unix, `NtResumeProcess` on Windows."""
    _suspend_or_resume(pid, 'NtResumeProcess', 'SIGCONT')


def _suspend_or_resume(pid, call, signal_name):
    if not WINDOWS:
        import signal
        os.kill(pid, getattr(signal, signal_name))
        return
    import ctypes
    from ctypes import wintypes
    kernel32 = ctypes.WinDLL('kernel32', use_last_error=True)
    kernel32.OpenProcess.restype = wintypes.HANDLE
    kernel32.OpenProcess.argtypes = (wintypes.DWORD, wintypes.BOOL, wintypes.DWORD)
    process_suspend_resume = 0x0800
    handle = kernel32.OpenProcess(process_suspend_resume, False, pid)
    if not handle:
        raise OSError(ctypes.get_last_error(), f'OpenProcess({pid}) for {call} failed')
    try:
        function = getattr(ctypes.WinDLL('ntdll'), call)
        function.restype = ctypes.c_long
        function.argtypes = (wintypes.HANDLE,)
        status = function(handle)
        if status != 0:
            raise OSError(f'{call}({pid}) failed: NTSTATUS {status & 0xFFFFFFFF:#010x}')
    finally:
        kernel32.CloseHandle(wintypes.HANDLE(handle))


def alive(pid):
    """Whether the process `pid` runs. Unix: `kill(pid, 0)`. Windows: a wait of zero on its handle;
    there `os.kill(pid, 0)` is NOT a probe, it sends CTRL+C (`CTRL_C_EVENT` is 0) to the console."""
    if not WINDOWS:
        try:
            os.kill(pid, 0)
        except ProcessLookupError:
            return False
        except PermissionError:
            return True
        return True
    kernel32, wintypes = _kernel32()
    handle = kernel32.OpenProcess(0x00100000 | 0x1000, False, pid)  # SYNCHRONIZE | QUERY_LIMITED_INFORMATION
    if not handle:
        return False
    try:
        return kernel32.WaitForSingleObject(handle, 0) == 0x102  # WAIT_TIMEOUT: still running
    finally:
        kernel32.CloseHandle(handle)


def process_tree(pid):
    """`pid` while it runs and every running process it started, directly or not, as sorted PIDs. Sends
    nothing to any process.

    Unix: by parent id from `processes()` (an orphan is re-parented to init and leaves the tree).
    Windows: by parent id from a Toolhelp snapshot. Windows does not re-parent, so the children of a
    leader that already ended are still found while its PID is not reused (a `Popen` that has not been
    collected keeps it). A process counts as a child only if it started after its parent (or, when the
    parent's start is unknown, after the nearest known ancestor's), so a recycled PID never adopts
    another program's processes. A grandchild whose own parent already ended is not found."""
    parents = _windows_parents() if WINDOWS else {row['pid']: row['ppid'] for row in processes()}
    started = {}
    found = {pid} if pid in parents else set()
    frontier = [(pid, _started(pid, started))]
    while frontier:
        parent, bound = frontier.pop()
        for child, of in parents.items():
            if of != parent or child == pid or child in found:
                continue
            born = _started(child, started)
            if bound is not None and born is not None and born < bound:
                continue    # older than its "parent": the child of an earlier owner of that PID
            found.add(child)
            frontier.append((child, born if born is not None else bound))
    return sorted(found)


def _started(pid, cache):
    """Windows: when `pid` started (FILETIME ticks), or `None` when it cannot be opened. Unix: `None`."""
    if not WINDOWS:
        return None
    if pid not in cache:
        import ctypes
        kernel32, wintypes = _kernel32()
        handle = kernel32.OpenProcess(0x1000, False, pid)  # PROCESS_QUERY_LIMITED_INFORMATION
        cache[pid] = None
        if handle:
            try:
                times = [wintypes.FILETIME() for _ in range(4)]
                if kernel32.GetProcessTimes(handle, *(ctypes.byref(item) for item in times)):
                    cache[pid] = (times[0].dwHighDateTime << 32) | times[0].dwLowDateTime
            finally:
                kernel32.CloseHandle(handle)
    return cache[pid]


_KERNEL32 = []


def _kernel32():
    """This module's own `kernel32` (not `ctypes.windll`'s, whose prototypes other code may set)."""
    import ctypes
    from ctypes import wintypes
    if _KERNEL32:
        return _KERNEL32[0], wintypes
    kernel32 = ctypes.WinDLL('kernel32', use_last_error=True)
    kernel32.OpenProcess.restype = wintypes.HANDLE
    kernel32.OpenProcess.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
    kernel32.WaitForSingleObject.restype = wintypes.DWORD
    kernel32.WaitForSingleObject.argtypes = [wintypes.HANDLE, wintypes.DWORD]
    kernel32.GetProcessTimes.restype = wintypes.BOOL
    kernel32.GetProcessTimes.argtypes = [wintypes.HANDLE] + [ctypes.POINTER(wintypes.FILETIME)] * 4
    kernel32.CloseHandle.argtypes = [wintypes.HANDLE]
    kernel32.CreateToolhelp32Snapshot.restype = wintypes.HANDLE
    kernel32.CreateToolhelp32Snapshot.argtypes = [wintypes.DWORD, wintypes.DWORD]
    kernel32.Process32FirstW.argtypes = [wintypes.HANDLE, ctypes.c_void_p]
    kernel32.Process32NextW.argtypes = [wintypes.HANDLE, ctypes.c_void_p]
    _KERNEL32.append(kernel32)
    return kernel32, wintypes


def _windows_parents():
    """`{pid: parent pid}` of every running process (a Toolhelp snapshot: fast, no PowerShell)."""
    import ctypes
    kernel32, wintypes = _kernel32()

    class ProcessEntry(ctypes.Structure):
        _fields_ = [('dwSize', wintypes.DWORD), ('cntUsage', wintypes.DWORD), ('th32ProcessID', wintypes.DWORD),
                    ('th32DefaultHeapID', ctypes.c_size_t), ('th32ModuleID', wintypes.DWORD),
                    ('cntThreads', wintypes.DWORD), ('th32ParentProcessID', wintypes.DWORD),
                    ('pcPriClassBase', ctypes.c_long), ('dwFlags', wintypes.DWORD),
                    ('szExeFile', ctypes.c_wchar * 260)]

    snapshot = kernel32.CreateToolhelp32Snapshot(0x2, 0)  # TH32CS_SNAPPROCESS
    if snapshot is None or snapshot == ctypes.c_void_p(-1).value:
        raise OSError(ctypes.get_last_error(), 'CreateToolhelp32Snapshot failed')
    parents = {}
    try:
        entry = ProcessEntry()
        entry.dwSize = ctypes.sizeof(entry)
        more = kernel32.Process32FirstW(snapshot, ctypes.byref(entry))
        while more:
            parents[entry.th32ProcessID] = entry.th32ParentProcessID
            more = kernel32.Process32NextW(snapshot, ctypes.byref(entry))
    finally:
        kernel32.CloseHandle(snapshot)
    return parents


def lock_file(handle, blocking=True):
    """Exclusive lock of the open file `handle`; with `blocking=False` a held lock raises
    `BlockingIOError` at once (as `flock(LOCK_NB)` does)."""
    if not WINDOWS:
        import fcntl
        fcntl.flock(handle, fcntl.LOCK_EX | (0 if blocking else fcntl.LOCK_NB))
        return
    import msvcrt
    while True:
        try:
            os.lseek(handle.fileno(), _LOCK_OFFSET, os.SEEK_SET)
            msvcrt.locking(handle.fileno(), msvcrt.LK_NBLCK, 1)
            return
        except OSError as error:
            if not blocking:
                raise BlockingIOError(str(error)) from error
            time.sleep(0.05)


def unlock_file(handle):
    if not WINDOWS:
        import fcntl
        fcntl.flock(handle, fcntl.LOCK_UN)
        return
    import msvcrt
    os.lseek(handle.fileno(), _LOCK_OFFSET, os.SEEK_SET)
    msvcrt.locking(handle.fileno(), msvcrt.LK_UNLCK, 1)


# Windows locks are mandatory: a lock on the first byte made every READ of a held lock file fail (`PermissionError`). The
# byte locked is far past the end instead (as SQLite does), so the file's content stays readable while it is held. Every
# script locks through `lock_file`, so they all agree on this byte (it must fit the CRT's 32-bit `_locking` offset).
_LOCK_OFFSET = 0x7FFFFFFE


def ui_socket_path():
    """The window's socket: `TESSEL_UI_SOCKET`, else the per-user default
    (crates/desktop/src/automation/mod.rs)."""
    override = os.environ.get('TESSEL_UI_SOCKET')
    if override:
        return override
    if WINDOWS:
        return os.path.join(os.environ['LOCALAPPDATA'], 'tessel-editor', 'ui.sock')
    return os.path.expanduser('~/.cache/tessel-editor/ui.sock')


def window_socket(home):
    """The socket of a window a proof starts with its own `home`. On Unix `HOME` alone isolates it
    (`~/.cache/tessel-editor/ui.sock` under that home). On Windows the default is under
    `%LOCALAPPDATA%`, which also holds the WinGet FFmpeg, so the window gets `TESSEL_UI_SOCKET`
    instead (see `window_env`), in a folder short enough for an AF_UNIX path (< 108 bytes)."""
    if not WINDOWS:
        return os.path.join(home, '.cache/tessel-editor/ui.sock')
    path = os.path.join(home, 'ui.sock')
    if len(os.fsencode(path)) >= 100:
        path = os.path.join(tempfile.mkdtemp(prefix='tessel-ui-'), 'ui.sock')
    return path


def window_env(socket_path):
    """What a window started by a proof needs in its environment, besides its `HOME`, to listen on
    `socket_path` (from `window_socket`): nothing on Unix, `TESSEL_UI_SOCKET` on Windows."""
    return {'TESSEL_UI_SOCKET': socket_path} if WINDOWS else {}


def connect_ui(path, timeout):
    """A stream socket connected to the editor at `path`, with `timeout` (seconds) on every call."""
    if not WINDOWS:
        conn = socket.socket(socket.AF_UNIX)
        try:
            conn.settimeout(timeout)
            conn.connect(path)
        except OSError:
            conn.close()
            raise
        return conn
    conn = socket.socket(fileno=_winsock_unix_connect(path))
    conn.settimeout(timeout)
    return conn


def _winsock_unix_connect(path):
    import ctypes

    af_unix = 1
    encoded = os.fsencode(path)
    if len(encoded) >= 108:
        raise OSError(f'AF_UNIX path too long ({len(encoded)} bytes): {path}')

    class SockaddrUn(ctypes.Structure):
        _fields_ = [('sun_family', ctypes.c_ushort), ('sun_path', ctypes.c_char * 108)]

    ws2 = ctypes.WinDLL('ws2_32', use_last_error=True)
    ws2.socket.restype = ctypes.c_size_t
    ws2.socket.argtypes = [ctypes.c_int, ctypes.c_int, ctypes.c_int]
    ws2.connect.restype = ctypes.c_int
    ws2.connect.argtypes = [ctypes.c_size_t, ctypes.c_void_p, ctypes.c_int]
    ws2.closesocket.argtypes = [ctypes.c_size_t]
    invalid = ctypes.c_size_t(-1).value
    # Importing `socket` already started Winsock (WSAStartup) for this process.
    handle = ws2.socket(af_unix, socket.SOCK_STREAM, 0)
    if handle == invalid:
        raise OSError(ctypes.get_last_error(), 'socket(AF_UNIX) failed')
    address = SockaddrUn(af_unix, encoded)
    if ws2.connect(handle, ctypes.byref(address), ctypes.sizeof(address)) != 0:
        error = ctypes.get_last_error()
        ws2.closesocket(handle)
        # WSAECONNREFUSED / WSAENOENT-like errors read as "nobody listens there yet".
        raise ConnectionRefusedError(error, f'connect({path}) failed: Winsock error {error}')
    return handle


def _tool(name):
    configured = os.environ.get(f'TESSEL_{name.upper()}')
    if configured:
        return configured
    found = shutil.which(name)
    if found:
        return found
    if WINDOWS:
        local = os.environ.get('LOCALAPPDATA', '')
        pattern = os.path.join(local, 'Microsoft', 'WinGet', 'Packages', 'Gyan.FFmpeg*', '*', 'bin', f'{name}.exe')
        candidates = sorted(glob.glob(pattern))
        if candidates:
            return candidates[-1]
    raise FileNotFoundError(f'{name} not found: set TESSEL_{name.upper()} or put it on PATH (docs/media-setup.md)')


def ffmpeg():
    return _tool('ffmpeg')


def ffprobe():
    return _tool('ffprobe')


def kill_tree(process):
    """Ends `process` (a `subprocess.Popen`) at once, with everything it started on Windows."""
    if process.poll() is not None and not WINDOWS:
        return
    if WINDOWS:
        subprocess.run(['taskkill', '/T', '/F', '/PID', str(process.pid)], capture_output=True, timeout=30)
    if process.poll() is None:
        process.kill()
    process.wait(timeout=30)


EXTENDED = '\\\\?\\'


def long_path(path):
    """`path` in the extended-length form on Windows (`\\\\?\\C:\\...`), past MAX_PATH; unchanged elsewhere."""
    path = os.path.abspath(path)
    if WINDOWS and not path.startswith(EXTENDED):
        return EXTENDED + path
    return path


def fresh_dir(path, attempts=50):
    """Empties and recreates `path`. On Windows a folder a just-ended process used can stay "delete
    pending" for a moment, and creating it again at once fails: try again for a few seconds."""
    # A project folder nests caches deeper than MAX_PATH (260) once its own path is long; Python on
    # Windows only reaches those through the extended-length form.
    target = long_path(path)
    for _ in range(attempts):
        shutil.rmtree(target, ignore_errors=True)
        if not os.path.exists(target):
            break
        time.sleep(0.2)
    os.makedirs(path)


def zip_test_command(package):
    """An external tool that reads every member of a zip (CRC included): `unzip -tq`, or the
    Windows `tar` (bsdtar) extracting to nothing when there is no `unzip`."""
    unzip = shutil.which('unzip')
    if unzip:
        return [unzip, '-tq', package]
    return ['tar', '-xOf', package]


CAPTURE_TIMEOUT = 60  # seconds a PowerShell capture may take
_WINDOWS_CAPTURE = r'''
$ErrorActionPreference = 'Stop'
Add-Type -ReferencedAssemblies System.Drawing @"
using System; using System.Drawing; using System.Runtime.InteropServices;
public static class TesselCapture {
  [StructLayout(LayoutKind.Sequential)] public struct Rect { public int Left, Top, Right, Bottom; }
  [DllImport("user32.dll")] static extern bool SetProcessDPIAware();
  [DllImport("user32.dll")] static extern bool GetWindowRect(IntPtr window, out Rect rect);
  [DllImport("user32.dll")] static extern bool PrintWindow(IntPtr window, IntPtr dc, uint flags);
  public static string Save(IntPtr window, string path) {
    SetProcessDPIAware();
    Rect r; if (!GetWindowRect(window, out r)) return "GetWindowRect failed";
    int w = r.Right - r.Left, h = r.Bottom - r.Top; if (w <= 0 || h <= 0) return "empty window";
    using (var bitmap = new Bitmap(w, h)) {
      using (var g = Graphics.FromImage(bitmap)) {
        IntPtr dc = g.GetHdc();
        bool ok = PrintWindow(window, dc, 2); // PW_RENDERFULLCONTENT: composed content (WebView2) too
        g.ReleaseHdc(dc);
        if (!ok) return "PrintWindow failed";
      }
      bitmap.Save(path, System.Drawing.Imaging.ImageFormat.Png);
    }
    return "";
  }
}
"@
$window = (Get-Process -Id $env:TESSEL_CAPTURE_PID).MainWindowHandle
if ($window -eq [IntPtr]::Zero) { Write-Output 'no on-screen window for this PID'; exit 0 }
Write-Output ([TesselCapture]::Save($window, $env:TESSEL_CAPTURE_PATH))
'''


def capture_window(pid, path):
    """`PrintWindow` of `pid`'s main window into `path` (PNG) on Windows; `None` and a reason when it
    fails or elsewhere (macOS proofs use `screencapture -l`). It reads pixels only: no click."""
    if not WINDOWS:
        return None, 'capture_window is the Windows capture; use screencapture on macOS'
    env = {**os.environ, 'TESSEL_CAPTURE_PID': str(pid), 'TESSEL_CAPTURE_PATH': str(path)}
    try:
        run = subprocess.run(['powershell.exe', '-NoProfile', '-NonInteractive', '-Command', _WINDOWS_CAPTURE],
                             env=env, capture_output=True, text=True, timeout=CAPTURE_TIMEOUT)
    except (subprocess.TimeoutExpired, OSError) as error:
        return None, f'PrintWindow did not run: {error}'
    reason = (run.stdout.strip() or run.stderr.strip())[:160]
    if run.returncode != 0 or reason or not os.path.exists(path):
        return None, f'PrintWindow capture failed ({run.returncode}): {reason}'
    return str(path), ''


def link(source, target):
    """`target` pointing at `source`: a symlink on Unix. Windows refuses symlinks without a privilege
    (developer mode or administrator), so there a folder becomes a junction (`mklink /J`, which Node
    resolves to its real path like a symlink) and a file a hard link, or a copy across volumes."""
    if not WINDOWS:
        os.symlink(source, target)
        return
    if os.path.isdir(source):
        done = subprocess.run(['cmd', '/c', 'mklink', '/J', target, source], capture_output=True)
        if done.returncode != 0:
            raise OSError(f"mklink /J {target} {source}: {done.stdout.decode('oem', errors='replace').strip()}")
        return
    try:
        os.link(source, target)
    except OSError:
        shutil.copy2(source, target)


def git_sh():
    """Git's `sh` on Windows: the one on `PATH` under Git Bash, else the installed one. Never the
    `bash.exe` of `System32`, which is WSL's and does not see this checkout's toolchain."""
    found = shutil.which('sh')
    if found:
        return found
    return os.path.join(os.environ.get('ProgramFiles', r'C:\Program Files'), 'Git', 'bin', 'sh.exe')


def script_command(path, *args):
    """A repository shell script (`scripts/<name>`) as a command line: Windows cannot run it
    directly, so Git's `sh` does there."""
    command = [str(path), *(str(arg) for arg in args)]
    return [git_sh(), *command] if WINDOWS else command


def loadavg():
    """`os.getloadavg()` on Unix; `None` on Windows, which has no load average (reported as unknown,
    never as a number)."""
    return os.getloadavg() if hasattr(os, 'getloadavg') else None
