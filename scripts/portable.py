"""What the proof scripts need from the OS, the same on macOS, Linux and Windows (BL168.8).

* `ui_socket_path()` / `connect_ui(path, timeout)`: the editor window's local socket. Python on Windows has
  no `socket.AF_UNIX` (the editor listens on a native Windows AF_UNIX socket, `uds_windows`), so there the
  socket is opened through Winsock with `ctypes` and handed back as an ordinary `socket.socket`.
* `ffmpeg()` / `ffprobe()`: the executables, found the way the editor finds them (`TESSEL_FFMPEG` /
  `TESSEL_FFPROBE`, then `PATH`, then the WinGet Gyan package), never assumed to be on `PATH`.
* `kill_tree(process)` / `kill_group(pid)`: ends a process and what it started (on Windows the WebView2
  helpers too); `kill_group` is `killpg(SIGKILL)` on Unix.
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
    """`killpg(pid, SIGKILL)` on Unix; on Windows the whole process tree (`taskkill /T /F`)."""
    if not WINDOWS:
        import signal
        try:
            os.killpg(pid, signal.SIGKILL)
        except ProcessLookupError:
            if not quiet:
                raise
        return
    done = subprocess.run(['taskkill', '/T', '/F', '/PID', str(pid)], capture_output=True, timeout=30)
    if done.returncode != 0 and not quiet:
        raise OSError(f"taskkill /T /F /PID {pid}: {done.stderr.decode('oem', errors='replace').strip()[:120]}")


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
            handle.seek(0)
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
    handle.seek(0)
    msvcrt.locking(handle.fileno(), msvcrt.LK_UNLCK, 1)


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
if ($window -eq [IntPtr]::Zero) { Write-Output 'no main window for this PID'; exit 0 }
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
