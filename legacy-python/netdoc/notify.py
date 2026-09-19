"""Toast notifications, drawn with Tk.

Windows' own toast API needs either a registered AppUserModelID or an external
package. A borderless Toplevel in the corner of the screen costs nothing, works
when the main window is minimised, and cannot fail to install.
"""

import tkinter as tk

BG = "#1c1f26"
FG = "#e6e8ed"
FG_DIM = "#8b93a3"

WIDTH = 340
MARGIN = 16
GAP = 8

_open = []          # stack of live toasts, newest last


def show(master, title, body="", colour="#ff5f5f", ms=9000, on_click=None):
    """Pop a toast above the tray. Must be called from the Tk thread."""
    try:
        win = tk.Toplevel(master)
    except tk.TclError:
        return None
    win.overrideredirect(True)
    win.attributes("-topmost", True)
    try:
        win.attributes("-alpha", 0.0)
    except tk.TclError:
        pass
    win.configure(bg=colour)

    inner = tk.Frame(win, bg=BG, padx=14, pady=12)
    inner.pack(fill="both", expand=True, padx=(4, 1), pady=1)

    tk.Label(inner, text=title, bg=BG, fg=FG, font=("Segoe UI", 10, "bold"),
             wraplength=WIDTH - 40, justify="left", anchor="w").pack(fill="x")
    if body:
        tk.Label(inner, text=body, bg=BG, fg=FG_DIM, font=("Segoe UI", 9),
                 wraplength=WIDTH - 40, justify="left", anchor="w").pack(fill="x", pady=(4, 0))

    win.update_idletasks()
    h = win.winfo_reqheight()
    sw = win.winfo_screenwidth()
    sh = win.winfo_screenheight()

    # Stack upward from the bottom-right corner.
    offset = sum(w.winfo_reqheight() + GAP for w in _open if w.winfo_exists())
    x = sw - WIDTH - MARGIN
    y = sh - h - MARGIN - 48 - offset
    win.geometry("%dx%d+%d+%d" % (WIDTH, h, x, max(MARGIN, y)))

    _open.append(win)

    def close(_event=None):
        if win in _open:
            _open.remove(win)
        try:
            win.destroy()
        except tk.TclError:
            pass

    def clicked(_event=None):
        if on_click:
            try:
                on_click()
            except Exception:
                pass
        close()

    for widget in (win, inner, *inner.winfo_children()):
        widget.bind("<Button-1>", clicked)

    def fade_in(step=0):
        if step > 10 or not win.winfo_exists():
            return
        try:
            win.attributes("-alpha", step / 10 * 0.96)
        except tk.TclError:
            return
        win.after(18, lambda: fade_in(step + 1))

    fade_in()
    win.after(ms, close)
    return win


def clear_all():
    for win in list(_open):
        try:
            win.destroy()
        except tk.TclError:
            pass
    _open.clear()
