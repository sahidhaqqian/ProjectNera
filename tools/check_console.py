"""Diagnostic helper: inspect a live target without changing its console/window."""
import ctypes, json, sys
kernel = ctypes.WinDLL('kernel32', use_last_error=True)
kernel.FreeConsole()
attached = bool(kernel.AttachConsole(int(sys.argv[1])))
error = ctypes.get_last_error() if not attached else 0
if attached:
    kernel.FreeConsole()
print(json.dumps({'pid':int(sys.argv[1]), 'has_console':attached, 'win32_error':error}))
