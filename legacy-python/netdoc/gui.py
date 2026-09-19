"""Tkinter front end.

The monitor thread pushes snapshots into a queue; Tk drains it on a timer.
Nothing touches widgets from a worker thread.
"""

import queue
import sys
import threading
import time
import tkinter as tk
from tkinter import filedialog, messagebox, ttk

from . import autostart, bandwidth, diagnose, monitor, notify, optimize, probes
from .settings import S
from .storage import Store

# palette
BG = "#14161a"
BG2 = "#1c1f26"
BG3 = "#242832"
FG = "#e6e8ed"
FG_DIM = "#8b93a3"
ACCENT = "#4da3ff"
GREEN = "#3ddc84"
YELLOW = "#ffc44d"
RED = "#ff5f5f"
GRID = "#2b303b"

SEV_COLOR = {diagnose.CRIT: RED, diagnose.WARN: YELLOW,
             diagnose.INFO: ACCENT, diagnose.GOOD: GREEN}

STATUS_COLOR = {
    monitor.OK: GREEN,
    monitor.DEGRADED: YELLOW,
    monitor.DNS_FAIL: YELLOW,
    monitor.ISP_DOWN: RED,
    monitor.LAN_DOWN: RED,
    monitor.ADAPTER_DOWN: RED,
}

SERIES_COLOURS = ["#7bd88f", ACCENT, "#b07bff", "#ffa94d", "#4dd0e1", "#ff8fab"]


def series_defs():
    """(key, label, colour) per monitored target, including user-added ones."""
    out = []
    for i, t in enumerate(S.targets()):
        label = {"gateway": "Router", "dns_isp": "DNS operatora"}.get(t["key"])
        if not label:
            label = t["host"] or t["label"]
        out.append((t["key"], label, SERIES_COLOURS[i % len(SERIES_COLOURS)]))
    return out


class LiveChart(tk.Canvas):
    """Scrolling latency plot. Gaps mean lost packets and are marked in red."""

    def __init__(self, master, **kw):
        super().__init__(master, bg=BG2, highlightthickness=0, **kw)
        self.defs = series_defs()
        self.data = {k: [] for k, _, _ in self.defs}
        self.visible = {k: True for k, _, _ in self.defs}
        self.bind("<Configure>", lambda e: self.redraw())

    def set_defs(self, defs):
        self.defs = defs
        for key, _, _ in defs:
            self.visible.setdefault(key, True)
        self.redraw()

    def set_data(self, data):
        self.data = data
        self.redraw()

    def toggle(self, key):
        self.visible[key] = not self.visible.get(key, True)
        self.redraw()

    def redraw(self):
        self.delete("all")
        w = self.winfo_width()
        h = self.winfo_height()
        if w < 50 or h < 40:
            return
        pad_l, pad_r, pad_t, pad_b = 46, 10, 12, 26
        plot_w = w - pad_l - pad_r
        plot_h = h - pad_t - pad_b

        vals = [v for k, pts in self.data.items() if self.visible.get(k)
                for _, v in pts if v is not None]
        top = max(60.0, (max(vals) * 1.25) if vals else 60.0)
        top = min(top, 1000.0)

        # horizontal grid + scale
        for frac in (0, 0.25, 0.5, 0.75, 1.0):
            y = pad_t + plot_h * frac
            self.create_line(pad_l, y, w - pad_r, y, fill=GRID)
            self.create_text(pad_l - 8, y, text="%d" % round(top * (1 - frac)),
                             fill=FG_DIM, anchor="e", font=("Segoe UI", 8))
        self.create_text(pad_l - 8, pad_t - 4, text="ms", fill=FG_DIM,
                         anchor="e", font=("Segoe UI", 8))

        n = S.history_points

        def x_of(i):
            return pad_l + plot_w * (i / max(1, n - 1))

        def y_of(v):
            return pad_t + plot_h * (1 - min(v, top) / top)

        for key, label, colour in self.defs:
            if not self.visible.get(key):
                continue
            pts = self.data.get(key, [])
            if not pts:
                continue
            offset = n - len(pts)
            run = []

            def flush(run, colour=colour):
                # create_line needs two points; a lone sample is drawn as a dot.
                if len(run) >= 4:
                    self.create_line(run, fill=colour, width=1.6, smooth=False)
                elif len(run) == 2:
                    x, y = run
                    self.create_oval(x - 1.5, y - 1.5, x + 1.5, y + 1.5,
                                     fill=colour, outline="")

            for i, (_, v) in enumerate(pts):
                xi = x_of(i + offset)
                if v is None:
                    # Packet loss: flush the current line and mark the gap.
                    flush(run)
                    run = []
                    self.create_line(xi, pad_t, xi, pad_t + plot_h, fill=RED, width=1.4)
                else:
                    run.extend((xi, y_of(v)))
            flush(run)

        self.create_line(pad_l, pad_t + plot_h, w - pad_r, pad_t + plot_h, fill=GRID)
        self._draw_time_axis(pad_l, pad_t, plot_w, plot_h, n)

    def _draw_time_axis(self, pad_l, pad_t, plot_w, plot_h, n):
        """Label the x axis with seconds back from now."""
        span_s = n * S.probe_interval_s
        y = pad_t + plot_h + 6
        for frac in (0.0, 0.25, 0.5, 0.75, 1.0):
            x = pad_l + plot_w * frac
            back = span_s * (1 - frac)
            if back < 1:
                text = "teraz"
            elif back < 90:
                text = "-%ds" % round(back)
            else:
                text = "-%.0f min" % (back / 60)
            anchor = "w" if frac == 0.0 else ("e" if frac == 1.0 else "n")
            self.create_text(x, y, text=text, fill=FG_DIM, anchor=anchor,
                             font=("Segoe UI", 8))


class App(tk.Tk):
    def __init__(self):
        super().__init__()
        self.title("NetDoctor — diagnostyka i optymalizacja sieci")
        self.geometry("1080x720")
        self.minsize(900, 600)
        self.configure(bg=BG)

        self.store = Store()
        self.queue = queue.Queue()
        self.monitor = monitor.Monitor(self.store, on_sample=self.queue.put)
        self.findings = []
        self.tweaks = []
        self._scanning = False
        self._bloat_running = False
        self._last_status = monitor.OK
        self._outage_started = None
        self._active_keys = None
        self.bloat_result = None

        self._setup_style()
        self._build()
        self.protocol("WM_DELETE_WINDOW", self.on_close)

        self.monitor.start()
        self.after(200, self._pump)
        self.after(1000, self._refresh_summary)

        if "--minimised" in sys.argv or S.start_minimised:
            self.iconify()

    # -- chrome -----------------------------------------------------------
    def _setup_style(self):
        st = ttk.Style(self)
        try:
            st.theme_use("clam")
        except tk.TclError:
            pass
        st.configure(".", background=BG, foreground=FG, fieldbackground=BG3,
                     font=("Segoe UI", 10))
        st.configure("TNotebook", background=BG, borderwidth=0)
        st.configure("TNotebook.Tab", background=BG2, foreground=FG_DIM,
                     padding=(18, 9), borderwidth=0)
        st.map("TNotebook.Tab", background=[("selected", BG3)],
               foreground=[("selected", FG)])
        st.configure("TFrame", background=BG)
        st.configure("Card.TFrame", background=BG2)
        st.configure("TLabel", background=BG, foreground=FG)
        st.configure("Card.TLabel", background=BG2, foreground=FG)
        st.configure("Dim.TLabel", background=BG, foreground=FG_DIM)
        st.configure("CardDim.TLabel", background=BG2, foreground=FG_DIM)
        st.configure("H1.TLabel", background=BG, foreground=FG, font=("Segoe UI", 15, "bold"))
        st.configure("TButton", background=BG3, foreground=FG, borderwidth=0, padding=(12, 7))
        st.map("TButton", background=[("active", "#323846"), ("disabled", BG2)],
               foreground=[("disabled", FG_DIM)])
        st.configure("Accent.TButton", background=ACCENT, foreground="#08121e")
        st.map("Accent.TButton", background=[("active", "#6fb6ff")])
        st.configure("Treeview", background=BG2, fieldbackground=BG2, foreground=FG,
                     borderwidth=0, rowheight=26)
        st.configure("Treeview.Heading", background=BG3, foreground=FG_DIM, borderwidth=0)
        st.map("Treeview", background=[("selected", "#2f3a4d")])
        st.configure("TProgressbar", background=ACCENT, troughcolor=BG3, borderwidth=0)

    def _build(self):
        header = ttk.Frame(self, padding=(16, 12, 16, 8))
        header.pack(fill="x")

        self.lamp = tk.Canvas(header, width=16, height=16, bg=BG, highlightthickness=0)
        self.lamp.pack(side="left", padx=(0, 10))
        self._lamp_id = self.lamp.create_oval(2, 2, 14, 14, fill=FG_DIM, outline="")

        box = ttk.Frame(header)
        box.pack(side="left", fill="x", expand=True)
        self.lbl_status = ttk.Label(box, text="Uruchamiam monitor…", style="H1.TLabel")
        self.lbl_status.pack(anchor="w")
        self.lbl_conn = ttk.Label(box, text="", style="Dim.TLabel")
        self.lbl_conn.pack(anchor="w")

        right = ttk.Frame(header)
        right.pack(side="right")
        self.lbl_admin = ttk.Label(right, text="", style="Dim.TLabel")
        self.lbl_admin.pack(anchor="e")
        if not probes.is_admin():
            self.lbl_admin.configure(text="tryb zwykły — zmiany wymagają administratora")
            ttk.Button(right, text="Uruchom jako administrator",
                       command=self.elevate).pack(anchor="e", pady=(4, 0))
        else:
            self.lbl_admin.configure(text="administrator")

        nb = ttk.Notebook(self)
        nb.pack(fill="both", expand=True, padx=12, pady=(4, 12))
        self.tab_live = ttk.Frame(nb, padding=12)
        self.tab_diag = ttk.Frame(nb, padding=12)
        self.tab_opt = ttk.Frame(nb, padding=12)
        self.tab_hist = ttk.Frame(nb, padding=12)
        self.tab_bloat = ttk.Frame(nb, padding=12)
        self.tab_set = ttk.Frame(nb, padding=12)
        nb.add(self.tab_live, text="Na żywo")
        nb.add(self.tab_diag, text="Diagnoza")
        nb.add(self.tab_bloat, text="Test obciążenia")
        nb.add(self.tab_opt, text="Optymalizacja")
        nb.add(self.tab_hist, text="Historia zerwań")
        nb.add(self.tab_set, text="Ustawienia")
        nb.bind("<<NotebookTabChanged>>", self._on_tab)

        self._build_live()
        self._build_diag()
        self._build_bloat()
        self._build_opt()
        self._build_hist()
        self._build_settings()

    # -- live tab ---------------------------------------------------------
    def _build_live(self):
        t = self.tab_live
        self.chart = LiveChart(t, height=300)
        self.chart.pack(fill="both", expand=True)

        self.legend = ttk.Frame(t)
        self.legend.pack(fill="x", pady=(8, 12))
        self._rebuild_legend()

        self.cards = {}
        grid = ttk.Frame(t)
        grid.pack(fill="x")
        for i, (key, label) in enumerate([
            ("cloudflare", "Ping (1.1.1.1)"), ("jitter", "Jitter"),
            ("loss", "Utrata pakietów"), ("gateway", "Ping do routera"),
            ("dns", "Czas DNS"), ("uptime", "Bez zerwań"),
        ]):
            card = ttk.Frame(grid, style="Card.TFrame", padding=12)
            card.grid(row=0, column=i, sticky="ew", padx=(0 if i == 0 else 8, 0))
            grid.columnconfigure(i, weight=1)
            ttk.Label(card, text=label, style="CardDim.TLabel",
                      font=("Segoe UI", 9)).pack(anchor="w")
            val = ttk.Label(card, text="—", style="Card.TLabel", font=("Segoe UI", 18, "bold"))
            val.pack(anchor="w")
            sub = ttk.Label(card, text="", style="CardDim.TLabel", font=("Segoe UI", 8))
            sub.pack(anchor="w")
            self.cards[key] = (val, sub)

        bar = ttk.Frame(t)
        bar.pack(fill="x", pady=(12, 0))
        self.btn_pause = ttk.Button(bar, text="Wstrzymaj monitor", command=self.toggle_pause)
        self.btn_pause.pack(side="left")
        ttk.Button(bar, text="Traceroute do 1.1.1.1",
                   command=self.run_traceroute).pack(side="left", padx=8)
        ttk.Button(bar, text="Zapisz raport…", command=self.export_report).pack(side="right")

        self.note = tk.Text(t, height=3, bg=BG2, fg=FG_DIM, bd=0, wrap="word",
                            font=("Consolas", 9), padx=10, pady=8)
        self.note.pack(fill="x", pady=(12, 0))
        self.note.configure(state="disabled")

    def _rebuild_legend(self):
        """Legend entries follow the configured targets; click one to hide it."""
        for child in self.legend.winfo_children():
            child.destroy()
        for key, label, colour in self.chart.defs:
            f = ttk.Frame(self.legend)
            f.pack(side="left", padx=(0, 18))
            c = tk.Canvas(f, width=12, height=12, bg=BG, highlightthickness=0)
            c.create_rectangle(2, 5, 12, 8, fill=colour, outline="")
            c.pack(side="left", padx=(0, 6))
            lb = ttk.Label(f, text=label, style="Dim.TLabel")
            lb.pack(side="left")
            for widget in (c, lb):
                widget.bind("<Button-1>", lambda e, k=key: self.chart.toggle(k))
        ttk.Label(self.legend, text="czerwona kreska = zgubiony pakiet",
                  style="Dim.TLabel").pack(side="right")

    # -- diagnostics tab --------------------------------------------------
    def _build_diag(self):
        t = self.tab_diag
        bar = ttk.Frame(t)
        bar.pack(fill="x")
        self.btn_scan = ttk.Button(bar, text="Uruchom pełny skan", style="Accent.TButton",
                                   command=self.run_scan)
        self.btn_scan.pack(side="left")
        self.lbl_scan = ttk.Label(bar, text="Skan trwa około 40 sekund.", style="Dim.TLabel")
        self.lbl_scan.pack(side="left", padx=12)
        self.prog = ttk.Progressbar(t, mode="determinate", maximum=1.0)
        self.prog.pack(fill="x", pady=(10, 12))

        self.lbl_verdict = ttk.Label(t, text="", style="H1.TLabel", wraplength=1000)
        self.lbl_verdict.pack(anchor="w", pady=(0, 10))

        panes = ttk.Frame(t)
        panes.pack(fill="both", expand=True)
        self.tree_find = ttk.Treeview(panes, columns=("sev", "title"), show="headings", height=14)
        self.tree_find.heading("sev", text="Waga")
        self.tree_find.heading("title", text="Wynik")
        self.tree_find.column("sev", width=110, anchor="w", stretch=False)
        self.tree_find.column("title", width=600, anchor="w")
        self.tree_find.pack(side="left", fill="both", expand=True)
        for sev, colour in SEV_COLOR.items():
            self.tree_find.tag_configure("sev%d" % sev, foreground=colour)
        self.tree_find.bind("<<TreeviewSelect>>", self._show_finding)

        side = ttk.Frame(panes, style="Card.TFrame", padding=12, width=330)
        side.pack(side="right", fill="y", padx=(12, 0))
        side.pack_propagate(False)
        self.det_title = ttk.Label(side, text="Wybierz pozycję z listy", style="Card.TLabel",
                                   font=("Segoe UI", 11, "bold"), wraplength=300)
        self.det_title.pack(anchor="w")
        self.det_body = tk.Text(side, bg=BG2, fg=FG_DIM, bd=0, wrap="word",
                                font=("Segoe UI", 9), height=18)
        self.det_body.pack(fill="both", expand=True, pady=10)
        self.det_body.configure(state="disabled")
        self.btn_fix = ttk.Button(side, text="Napraw to", style="Accent.TButton",
                                  command=self._fix_selected, state="disabled")
        self.btn_fix.pack(fill="x")

    # -- bufferbloat tab --------------------------------------------------
    def _build_bloat(self):
        t = self.tab_bloat
        ttk.Label(t, text="Ping pod obciążeniem", style="H1.TLabel").pack(anchor="w")
        ttk.Label(
            t, style="Dim.TLabel", wraplength=980, justify="left",
            text="Ping na spoczynku niewiele mówi. Liczy się, co się z nim dzieje, gdy ktoś "
                 "w domu pobiera plik albo aktualizuje grę. Test wysyca łącze i mierzy wzrost "
                 "opóźnienia.\n\nPobiera kilkadziesiąt MB z serwera Cloudflare i trwa około "
                 "25 sekund. Na łączu limitowanym transferem lepiej go pominąć.",
        ).pack(anchor="w", pady=(6, 12))

        bar = ttk.Frame(t)
        bar.pack(fill="x")
        self.btn_bloat = ttk.Button(bar, text="Uruchom test", style="Accent.TButton",
                                    command=self.run_bloat)
        self.btn_bloat.pack(side="left")
        self.lbl_bloat = ttk.Label(bar, text="", style="Dim.TLabel")
        self.lbl_bloat.pack(side="left", padx=12)
        self.prog_bloat = ttk.Progressbar(t, mode="determinate", maximum=1.0)
        self.prog_bloat.pack(fill="x", pady=(10, 14))

        cards = ttk.Frame(t)
        cards.pack(fill="x")
        self.bloat_cards = {}
        for i, (key, label) in enumerate([
            ("idle", "Ping na spoczynku"), ("loaded", "Ping pod obciążeniem"),
            ("bump", "Wzrost"), ("mbps", "Przepustowość"), ("grade", "Ocena"),
        ]):
            card = ttk.Frame(cards, style="Card.TFrame", padding=12)
            card.grid(row=0, column=i, sticky="ew", padx=(0 if i == 0 else 8, 0))
            cards.columnconfigure(i, weight=1)
            ttk.Label(card, text=label, style="CardDim.TLabel",
                      font=("Segoe UI", 9)).pack(anchor="w")
            val = ttk.Label(card, text="—", style="Card.TLabel", font=("Segoe UI", 18, "bold"))
            val.pack(anchor="w")
            self.bloat_cards[key] = val

        self.bloat_text = tk.Text(t, bg=BG2, fg=FG_DIM, bd=0, wrap="word",
                                  font=("Segoe UI", 10), padx=14, pady=12)
        self.bloat_text.pack(fill="both", expand=True, pady=(14, 0))
        self._set_text(self.bloat_text,
                       "Wynik pojawi się tutaj razem z tym, co z nim zrobić.")

    def run_bloat(self):
        if self._bloat_running:
            return
        if not messagebox.askyesno(
                "Uruchomić test?",
                "Test pobierze kilkadziesiąt megabajtów i na ~25 sekund wysyci łącze.\n\n"
                "W tym czasie internet w całym domu będzie wolniejszy. Kontynuować?"):
            return
        self._bloat_running = True
        self.btn_bloat.configure(state="disabled")
        # Pausing the monitor keeps its pings out of the measurement.
        was_paused = self.monitor.paused
        self.monitor.paused = True

        def progress(text, frac):
            self.after(0, lambda: (self.lbl_bloat.configure(text=text),
                                   self.prog_bloat.configure(value=frac)))

        def work():
            try:
                res = bandwidth.run(progress=progress)
            except Exception as e:
                res = bandwidth.BloatResult(error=str(e))
            self.after(0, lambda: self._bloat_done(res, was_paused))

        threading.Thread(target=work, daemon=True).start()

    def _bloat_done(self, res, was_paused):
        self._bloat_running = False
        self.bloat_result = res
        self.monitor.paused = was_paused
        self.btn_bloat.configure(state="normal")
        self.lbl_bloat.configure(text="Test zakończony %s." % time.strftime("%H:%M"))
        self.prog_bloat.configure(value=1.0)

        def put(key, text, colour=FG):
            self.bloat_cards[key].configure(text=text, foreground=colour)

        if res.idle_avg is not None:
            put("idle", "%.0f ms" % res.idle_avg)
        if res.loaded_avg is not None:
            c = GREEN if (res.bump_ms or 0) < 60 else (
                YELLOW if (res.bump_ms or 0) < 150 else RED)
            put("loaded", "%.0f ms" % res.loaded_avg, c)
            put("bump", "+%.0f ms" % res.bump_ms, c)
        if res.mbps:
            put("mbps", "%.0f Mb/s" % res.mbps)
        grade_colour = {"A": GREEN, "B": GREEN, "C": YELLOW, "D": RED, "F": RED}
        put("grade", res.grade, grade_colour.get(res.grade, FG_DIM))

        body = []
        if res.verdict:
            body.append(res.verdict)
        if res.error:
            body.append(res.error)
        if res.loaded_max is not None:
            body.append("Najgorszy pomiar pod obciążeniem: %.0f ms, strata pakietów %.0f%%."
                        % (res.loaded_max, res.loaded_loss))
        body.append("")
        body.append(bandwidth.advice(res))
        self._set_text(self.bloat_text, "\n".join(body))

    # -- optimisation tab -------------------------------------------------
    def _build_opt(self):
        t = self.tab_opt
        ttk.Label(t, text="Każda zmiana zapisuje poprzedni stan i da się ją cofnąć.",
                  style="Dim.TLabel").pack(anchor="w")
        bar = ttk.Frame(t)
        bar.pack(fill="x", pady=(8, 10))
        ttk.Button(bar, text="Odśwież stan", command=self.refresh_tweaks).pack(side="left")
        ttk.Button(bar, text="Zastosuj wszystkie bezpieczne",
                   style="Accent.TButton", command=self.apply_safe).pack(side="left", padx=8)

        wrap = ttk.Frame(t)
        wrap.pack(fill="both", expand=True)
        self.tree_tweak = ttk.Treeview(
            wrap, columns=("name", "state", "risk"), show="headings", height=10)
        for col, txt, w in (("name", "Zmiana", 330), ("state", "Stan obecny", 330),
                            ("risk", "Ryzyko", 90)):
            self.tree_tweak.heading(col, text=txt)
            self.tree_tweak.column(col, width=w, anchor="w")
        self.tree_tweak.pack(fill="both", expand=True)
        self.tree_tweak.tag_configure("todo", foreground=YELLOW)
        self.tree_tweak.tag_configure("done", foreground=GREEN)
        self.tree_tweak.tag_configure("na", foreground=FG_DIM)
        self.tree_tweak.bind("<<TreeviewSelect>>", self._show_tweak)

        info = ttk.Frame(t, style="Card.TFrame", padding=12)
        info.pack(fill="x", pady=(12, 0))
        self.tw_title = ttk.Label(info, text="Wybierz zmianę z listy", style="Card.TLabel",
                                  font=("Segoe UI", 11, "bold"))
        self.tw_title.pack(anchor="w")
        self.tw_body = tk.Text(info, bg=BG2, fg=FG_DIM, bd=0, wrap="word",
                               font=("Segoe UI", 9), height=5)
        self.tw_body.pack(fill="x", pady=8)
        self.tw_body.configure(state="disabled")
        btns = ttk.Frame(info, style="Card.TFrame")
        btns.pack(anchor="w")
        self.btn_apply = ttk.Button(btns, text="Zastosuj", style="Accent.TButton",
                                    command=self.apply_selected, state="disabled")
        self.btn_apply.pack(side="left")
        self.btn_revert = ttk.Button(btns, text="Cofnij", command=self.revert_selected,
                                     state="disabled")
        self.btn_revert.pack(side="left", padx=8)

    # -- history tab ------------------------------------------------------
    def _build_hist(self):
        t = self.tab_hist
        self.lbl_hist = ttk.Label(t, text="", style="H1.TLabel", wraplength=1000)
        self.lbl_hist.pack(anchor="w", pady=(0, 10))
        ttk.Button(t, text="Odśwież", command=self.refresh_history).pack(anchor="w")
        self.tree_hist = ttk.Treeview(
            t, columns=("start", "dur", "kind", "detail"), show="headings")
        for col, txt, w in (("start", "Początek", 150), ("dur", "Czas trwania", 110),
                            ("kind", "Rodzaj", 190), ("detail", "Szczegóły", 520)):
            self.tree_hist.heading(col, text=txt)
            self.tree_hist.column(col, width=w, anchor="w")
        self.tree_hist.pack(fill="both", expand=True, pady=10)
        self.tree_hist.tag_configure("bad", foreground=RED)
        self.tree_hist.tag_configure("warn", foreground=YELLOW)

    # -- settings tab -----------------------------------------------------
    def _build_settings(self):
        t = self.tab_set
        self.set_vars = {}

        cols = ttk.Frame(t)
        cols.pack(fill="both", expand=True)
        left = ttk.Frame(cols, style="Card.TFrame", padding=14)
        left.pack(side="left", fill="both", expand=True)
        right = ttk.Frame(cols, style="Card.TFrame", padding=14)
        right.pack(side="right", fill="both", expand=True, padx=(12, 0))

        ttk.Label(left, text="Pomiary", style="Card.TLabel",
                  font=("Segoe UI", 12, "bold")).pack(anchor="w", pady=(0, 10))
        self._num_field(left, "probe_interval_s", "Odstęp między pomiarami (s)",
                        "Niżej = dokładniej, ale więcej ruchu i obciążenia.")
        self._num_field(left, "ping_timeout_ms", "Limit czasu pinga (ms)")
        self._num_field(left, "outage_after_fails", "Nieudanych pomiarów przed alarmem",
                        "Zabezpieczenie przed zgłaszaniem pojedynczych zgubionych pakietów.")
        self._num_field(left, "history_points", "Szerokość wykresu (liczba próbek)")
        self._num_field(left, "keep_days", "Ile dni trzymać historię")

        ttk.Label(left, text="Dodatkowe cele pingowania", style="Card.TLabel",
                  font=("Segoe UI", 11, "bold")).pack(anchor="w", pady=(14, 4))
        ttk.Label(left, text="Po jednym adresie w wierszu — np. serwer gry, na którym grasz.",
                  style="CardDim.TLabel", wraplength=380, justify="left").pack(anchor="w")
        self.txt_targets = tk.Text(left, height=4, bg=BG3, fg=FG, bd=0, insertbackground=FG,
                                   font=("Consolas", 9), padx=8, pady=6)
        self.txt_targets.pack(fill="x", pady=(6, 0))
        self.txt_targets.insert("end", "\n".join(S.extra_targets))

        ttk.Label(right, text="Progi ocen", style="Card.TLabel",
                  font=("Segoe UI", 12, "bold")).pack(anchor="w", pady=(0, 10))
        self._num_field(right, "ping_ok", "Ping jeszcze dobry do (ms)")
        self._num_field(right, "ping_bad", "Ping zły powyżej (ms)")
        self._num_field(right, "jitter_good", "Jitter dobry do (ms)")
        self._num_field(right, "jitter_ok", "Jitter akceptowalny do (ms)")
        self._num_field(right, "loss_ok", "Dopuszczalna strata pakietów (%)")

        ttk.Label(right, text="Zachowanie", style="Card.TLabel",
                  font=("Segoe UI", 12, "bold")).pack(anchor="w", pady=(16, 8))
        self.var_notify = tk.BooleanVar(value=S.notify_on_outage)
        self.var_minimised = tk.BooleanVar(value=S.start_minimised)
        self.var_autostart = tk.BooleanVar(value=autostart.is_enabled())
        for var, text in (
            (self.var_notify, "Pokazuj powiadomienie przy zerwaniu"),
            (self.var_minimised, "Uruchamiaj zminimalizowany"),
            (self.var_autostart, "Startuj razem z Windows"),
        ):
            cb = tk.Checkbutton(right, text=text, variable=var, bg=BG2, fg=FG,
                                selectcolor=BG3, activebackground=BG2, activeforeground=FG,
                                highlightthickness=0, bd=0, font=("Segoe UI", 10),
                                anchor="w")
            cb.pack(fill="x", pady=2)
        ttk.Label(right, style="CardDim.TLabel", wraplength=380, justify="left",
                  text="Autostart ma sens tylko razem z monitorowaniem w tle — bez niego "
                       "aplikacja nie zapisze zerwania, które nastąpi przy wyłączonym oknie."
                  ).pack(anchor="w", pady=(6, 0))

        bar = ttk.Frame(t)
        bar.pack(fill="x", pady=(14, 0))
        ttk.Button(bar, text="Zapisz ustawienia", style="Accent.TButton",
                   command=self.save_settings).pack(side="left")
        ttk.Button(bar, text="Przywróć domyślne", command=self.reset_settings).pack(
            side="left", padx=8)
        self.lbl_set = ttk.Label(bar, text="", style="Dim.TLabel")
        self.lbl_set.pack(side="left", padx=12)

    def _num_field(self, parent, key, label, hint=""):
        row = ttk.Frame(parent, style="Card.TFrame")
        row.pack(fill="x", pady=3)
        ttk.Label(row, text=label, style="Card.TLabel", width=34,
                  anchor="w").pack(side="left")
        var = tk.StringVar(value=str(getattr(S, key)))
        entry = tk.Entry(row, textvariable=var, bg=BG3, fg=FG, bd=0, width=8,
                         insertbackground=FG, justify="right", font=("Consolas", 10))
        entry.pack(side="right", ipady=3, ipadx=4)
        self.set_vars[key] = var
        if hint:
            ttk.Label(parent, text=hint, style="CardDim.TLabel", wraplength=380,
                      justify="left", font=("Segoe UI", 8)).pack(anchor="w", pady=(0, 4))

    def save_settings(self):
        values = {}
        for key, var in self.set_vars.items():
            raw = var.get().strip().replace(",", ".")
            try:
                values[key] = float(raw) if "." in raw else int(raw)
            except ValueError:
                messagebox.showerror("Zła wartość",
                                     "„%s” nie jest liczbą (pole: %s)." % (raw, key))
                return
        if values["probe_interval_s"] < 0.3:
            messagebox.showerror("Za mały odstęp",
                                 "Odstęp poniżej 0.3 s obciąża sieć bardziej niż mierzy.")
            return
        if values["history_points"] < 30:
            values["history_points"] = 30

        targets = [ln.strip() for ln in self.txt_targets.get("1.0", "end").splitlines()
                   if ln.strip()]
        values["extra_targets"] = targets
        values["notify_on_outage"] = self.var_notify.get()
        values["start_minimised"] = self.var_minimised.get()
        S.update(**values)
        self._active_keys = None

        ok, msg = autostart.toggle(self.var_autostart.get())
        if not ok:
            messagebox.showerror("Autostart", msg)
            self.var_autostart.set(autostart.is_enabled())

        # Targets may have changed, so the monitor's buffers and the chart
        # legend both have to follow.
        self.monitor.rebuild_targets()
        self.chart.set_defs(series_defs())
        self._rebuild_legend()
        self.lbl_set.configure(text="Zapisano %s." % time.strftime("%H:%M"), foreground=GREEN)

    def reset_settings(self):
        if not messagebox.askyesno("Przywrócić domyślne?",
                                   "Wszystkie ustawienia wrócą do wartości fabrycznych."):
            return
        S.reset()
        for key, var in self.set_vars.items():
            var.set(str(getattr(S, key)))
        self.txt_targets.delete("1.0", "end")
        self.var_notify.set(S.notify_on_outage)
        self.var_minimised.set(S.start_minimised)
        self._active_keys = None
        self.monitor.rebuild_targets()
        self.chart.set_defs(series_defs())
        self._rebuild_legend()
        self.lbl_set.configure(text="Przywrócono domyślne.", foreground=FG_DIM)

    # -- event pump -------------------------------------------------------
    def _pump(self):
        snap = None
        try:
            while True:
                snap = self.queue.get_nowait()
        except queue.Empty:
            pass
        if snap:
            # A drawing glitch must never kill the refresh loop.
            try:
                self._apply_snapshot(snap)
            except Exception:
                pass
        self.after(300, self._pump)

    def _apply_snapshot(self, snap):
        colour = STATUS_COLOR.get(snap.status, FG_DIM)
        self.lamp.itemconfigure(self._lamp_id, fill=colour)
        self.lbl_status.configure(text=monitor.STATUS_LABEL.get(snap.status, snap.status))

        n = snap.net
        if n.adapter_type == "Wi-Fi":
            conn = "%s · %s · sygnał %s%% · kanał %s (%s) · %s Mb/s · brama %s" % (
                n.adapter, n.ssid or "?", n.signal_pct if n.signal_pct is not None else "?",
                n.channel or "?", n.radio or "?", n.rx_rate or "?", n.gateway or "?")
        else:
            conn = "%s · %s · brama %s · DNS %s" % (
                n.adapter or "?", n.link_speed or "?", n.gateway or "?",
                ", ".join(n.dns_servers) or "?")
        self.lbl_conn.configure(text=conn)

        # Some targets resolve to nothing (e.g. the ISP DNS is the router
        # itself), so the legend follows what is actually being pinged.
        active = set(snap.results)
        if active and active != getattr(self, "_active_keys", None):
            self._active_keys = active
            self.chart.set_defs([d for d in series_defs() if d[0] in active])
            self._rebuild_legend()

        self.chart.set_data({k: self.monitor.series(k) for k, _, _ in self.chart.defs})

        note = snap.note
        if snap.roamed:
            note = (note + "  " if note else "") + "Karta przeskoczyła na inny nadajnik (roaming)."
        self._set_text(self.note, note or "Bez uwag. Monitor zapisuje wszystko do historii.")

        self._maybe_notify(snap)

    def _maybe_notify(self, snap):
        """Toast when the verdict changes, so a dropout is visible when minimised."""
        prev, now = self._last_status, snap.status
        if now == prev:
            return
        self._last_status = now

        if not S.notify_on_outage:
            return
        if now != monitor.OK:
            self._outage_started = snap.ts
            notify.show(self, monitor.STATUS_LABEL.get(now, now),
                        snap.note or "Kliknij, żeby otworzyć NetDoctor.",
                        colour=STATUS_COLOR.get(now, RED), on_click=self._restore)
        elif prev != monitor.OK:
            secs = (snap.ts - self._outage_started) if self._outage_started else 0
            notify.show(self, "Połączenie wróciło",
                        "Przerwa trwała %.0f s. Szczegóły w zakładce Historia zerwań." % secs,
                        colour=GREEN, ms=6000, on_click=self._restore)
            self._outage_started = None

    def _restore(self):
        self.deiconify()
        self.lift()
        self.focus_force()

    def _refresh_summary(self):
        try:
            self._update_cards()
        except Exception:
            pass
        self.after(2000, self._refresh_summary)

    def _update_cards(self):
        _, loss, avg, mn, mx, jitter = self.store.stats("cloudflare", 300)
        gw = self.store.stats("gateway", 300)

        def put(key, text, sub="", colour=FG):
            val, s = self.cards[key]
            val.configure(text=text, foreground=colour)
            s.configure(text=sub)

        if avg is None:
            put("cloudflare", "—", "brak danych")
        else:
            c = GREEN if avg < S.ping_ok else (YELLOW if avg < S.ping_bad else RED)
            put("cloudflare", "%.0f ms" % avg, "min %.0f / max %.0f (5 min)" % (mn, mx), c)
        if jitter is None:
            put("jitter", "—")
        else:
            c = GREEN if jitter < S.jitter_good else (
                YELLOW if jitter < S.jitter_ok else RED)
            put("jitter", "%.1f ms" % jitter, "wahania między pakietami", c)
        c = GREEN if loss < S.loss_good else (YELLOW if loss < S.loss_ok else RED)
        put("loss", "%.1f%%" % loss, "ostatnie 5 minut", c)
        if gw[2] is None:
            put("gateway", "—", "router nie odpowiada", RED)
        else:
            c = GREEN if gw[2] < 10 else YELLOW
            put("gateway", "%.0f ms" % gw[2], "strata %.1f%%" % gw[1], c)

        dns = self.monitor.last.dns_ms
        if self.monitor.last.dns_error:
            put("dns", "błąd", self.monitor.last.dns_error[:40], RED)
        elif dns:
            c = GREEN if dns < 60 else (YELLOW if dns < 150 else RED)
            put("dns", "%.0f ms" % dns, "rozwiązywanie nazw", c)

        events = self.store.events_since(24 * 3600)
        if events:
            last = max(e[1] for e in events)
            mins = (time.time() - last) / 60
            put("uptime", "%.0f min" % mins, "%d zerwań w 24 h" % len(events),
                YELLOW if len(events) < 3 else RED)
        else:
            put("uptime", "24 h+", "brak zerwań w historii", GREEN)

    # -- actions ----------------------------------------------------------
    def toggle_pause(self):
        self.monitor.paused = not self.monitor.paused
        self.btn_pause.configure(
            text="Wznów monitor" if self.monitor.paused else "Wstrzymaj monitor")

    def run_traceroute(self):
        win = tk.Toplevel(self)
        win.title("Traceroute do 1.1.1.1")
        win.configure(bg=BG)
        win.geometry("640x420")
        txt = tk.Text(win, bg=BG2, fg=FG, bd=0, font=("Consolas", 9), padx=10, pady=10)
        txt.pack(fill="both", expand=True, padx=10, pady=10)
        txt.insert("end", "Trwa śledzenie trasy…\n")

        def work():
            hops = probes.traceroute()
            lines = ["%2d  %-16s %s" % h for h in hops]
            lines.append("")
            lines.append("Hop 1 to Twój router. Jeśli opóźnienie skacze dopiero dalej,")
            lines.append("problem jest po stronie operatora, nie u Ciebie.")
            self.after(0, lambda: (txt.delete("1.0", "end"), txt.insert("end", "\n".join(lines))))

        threading.Thread(target=work, daemon=True).start()

    def run_scan(self):
        if self._scanning:
            return
        self._scanning = True
        self.btn_scan.configure(state="disabled")
        self.prog.configure(value=0)

        def progress(text, frac):
            self.after(0, lambda: (self.lbl_scan.configure(text=text),
                                   self.prog.configure(value=frac)))

        def work():
            try:
                res = diagnose.scan(net=self.monitor.net, store=self.store, progress=progress)
            except Exception as e:
                res = [diagnose.Finding("err", "Skan nie powiódł się", diagnose.WARN, str(e))]
            self.after(0, lambda: self._scan_done(res))

        threading.Thread(target=work, daemon=True).start()

    def _scan_done(self, findings):
        self._scanning = False
        self.findings = findings
        self.btn_scan.configure(state="normal")
        self.lbl_scan.configure(text="Skan zakończony %s." % time.strftime("%H:%M"))
        self.lbl_verdict.configure(text=diagnose.summarize(findings),
                                   foreground=RED if any(f.severity == diagnose.CRIT
                                                         for f in findings) else FG)
        self.tree_find.delete(*self.tree_find.get_children())
        for i, f in enumerate(findings):
            self.tree_find.insert("", "end", iid=str(i),
                                  values=(diagnose.SEVERITY_LABEL[f.severity], f.title),
                                  tags=("sev%d" % f.severity,))

    def _show_finding(self, _event=None):
        sel = self.tree_find.selection()
        if not sel:
            return
        f = self.findings[int(sel[0])]
        self.det_title.configure(text=f.title, foreground=SEV_COLOR[f.severity])
        body = f.detail or ""
        if f.advice:
            body += ("\n\n" if body else "") + f.advice
        self._set_text(self.det_body, body)
        if f.tweak_id:
            self.btn_fix.configure(state="normal")
            self._fix_target = f.tweak_id
        else:
            self.btn_fix.configure(state="disabled")
            self._fix_target = None

    def _fix_selected(self):
        tid = getattr(self, "_fix_target", None)
        if not tid:
            return
        tweak = next((t for t in optimize.all_tweaks(self.monitor.net) if t.id == tid), None)
        if tweak:
            self._do_apply(tweak)

    def refresh_tweaks(self):
        self.tweaks = optimize.all_tweaks(self.monitor.net)
        self.tree_tweak.delete(*self.tree_tweak.get_children())
        for i, t in enumerate(self.tweaks):
            txt, optimal, _ = t.status()
            tag = "done" if optimal else ("todo" if optimal is False else "na")
            self.tree_tweak.insert("", "end", iid=str(i),
                                   values=(t.title, txt, t.risk), tags=(tag,))

    def _show_tweak(self, _event=None):
        sel = self.tree_tweak.selection()
        if not sel:
            return
        t = self.tweaks[int(sel[0])]
        self.tw_title.configure(text=t.title)
        body = "Co robi: %s\n\nDlaczego: %s\n\nRyzyko: %s%s" % (
            t.what, t.why, t.risk,
            "" if t.reversible else "  (tej akcji nie da się cofnąć)")
        self._set_text(self.tw_body, body)
        self.btn_apply.configure(state="normal")
        self.btn_revert.configure(
            state="normal" if (t.reversible and optimize.saved_snapshot(t.id)) else "disabled")

    def _selected_tweak(self):
        sel = self.tree_tweak.selection()
        return self.tweaks[int(sel[0])] if sel else None

    def apply_selected(self):
        t = self._selected_tweak()
        if t:
            self._do_apply(t)

    def _do_apply(self, tweak):
        if not probes.is_admin() and tweak.needs_admin:
            if messagebox.askyesno(
                    "Wymagany administrator",
                    "Ta zmiana wymaga uprawnień administratora.\n\n"
                    "Uruchomić NetDoctor ponownie jako administrator?"):
                self.elevate()
            return
        if not messagebox.askyesno(
                "Potwierdź zmianę",
                "%s\n\n%s\n\nRyzyko: %s\n\nStan sprzed zmiany zostanie zapisany%s."
                % (tweak.title, tweak.what, tweak.risk,
                   "" if tweak.reversible else " (ale tej akcji nie da się cofnąć)")):
            return
        ok, msg = optimize.apply_tweak(tweak, self.store)
        (messagebox.showinfo if ok else messagebox.showerror)(
            "Zastosowano" if ok else "Nie udało się", msg)
        self.refresh_tweaks()

    def revert_selected(self):
        t = self._selected_tweak()
        if not t:
            return
        ok, msg = optimize.revert_tweak(t, self.store)
        (messagebox.showinfo if ok else messagebox.showerror)(
            "Cofnięto" if ok else "Nie udało się", msg)
        self.refresh_tweaks()

    def apply_safe(self):
        """Apply every low-risk tweak that is not already in place."""
        if not probes.is_admin():
            if messagebox.askyesno("Wymagany administrator",
                                   "Zmiany wymagają administratora. Uruchomić ponownie?"):
                self.elevate()
            return
        self.refresh_tweaks()
        todo = [t for t in self.tweaks
                if t.risk == optimize.RISK_LOW and t.reversible and t.status()[1] is False]
        if not todo:
            messagebox.showinfo("Nic do zrobienia",
                                "Wszystkie bezpieczne ustawienia są już poprawne.")
            return
        names = "\n".join("• " + t.title for t in todo)
        if not messagebox.askyesno("Zastosować?", "Zostaną wprowadzone:\n\n%s\n\n"
                                                  "Każdą da się cofnąć osobno." % names):
            return
        lines = []
        for t in todo:
            ok, msg = optimize.apply_tweak(t, self.store)
            lines.append("%s %s — %s" % ("OK" if ok else "BŁĄD", t.title, msg))
        messagebox.showinfo("Wynik", "\n".join(lines))
        self.refresh_tweaks()

    def refresh_history(self):
        rows = self.store.recent_events(200)
        self.tree_hist.delete(*self.tree_hist.get_children())
        for _id, ts_start, ts_end, kind, scope, detail, _ctx in rows:
            dur = ("%.0f s" % (ts_end - ts_start)) if ts_end else "trwa"
            self.tree_hist.insert(
                "", "end",
                values=(time.strftime("%d.%m %H:%M:%S", time.localtime(ts_start)),
                        dur, monitor.STATUS_LABEL.get(kind, kind), detail or ""),
                tags=("bad" if scope in ("lan", "adapter", "isp") else "warn",))
        day = self.store.events_since(24 * 3600)
        if not day:
            self.lbl_hist.configure(text="Brak zerwań w ostatniej dobie.", foreground=GREEN)
        else:
            by = {}
            for e in day:
                by[e[4]] = by.get(e[4], 0) + 1
            worst = max(by, key=by.get)
            where = {"lan": "między komputerem a routerem", "adapter": "na karcie sieciowej",
                     "isp": "po stronie operatora", "dns": "w DNS",
                     "internet": "jako spadek jakości"}.get(worst, worst)
            self.lbl_hist.configure(
                text="%d zerwań w ostatniej dobie, najczęściej %s." % (len(day), where),
                foreground=RED)

    def export_report(self):
        path = filedialog.asksaveasfilename(
            defaultextension=".txt", initialfile="netdoctor-raport.txt",
            filetypes=[("Plik tekstowy", "*.txt")])
        if not path:
            return
        n = self.monitor.net
        lines = [
            "NetDoctor — raport z %s" % time.strftime("%Y-%m-%d %H:%M"),
            "=" * 70, "",
            "POŁĄCZENIE",
            "  karta      : %s (%s)" % (n.adapter, n.adapter_type),
            "  sterownik  : %s" % n.adapter_desc,
            "  brama      : %s" % n.gateway,
            "  DNS        : %s" % ", ".join(n.dns_servers),
        ]
        if n.adapter_type == "Wi-Fi":
            lines += ["  SSID       : %s (BSSID %s)" % (n.ssid, n.bssid),
                      "  sygnał     : %s%%, kanał %s, %s" % (n.signal_pct, n.channel, n.radio),
                      "  prędkości  : odbiór %s / wysyłanie %s Mb/s" % (n.rx_rate, n.tx_rate)]
        lines += ["", "POMIARY (ostatnia godzina)"]
        for key, label, _ in series_defs():
            cnt, loss, avg, mn, mx, jit = self.store.stats(key, 3600)
            if cnt:
                lines.append("  %-12s próbek %4d, strata %5.1f%%, avg %6.1f ms, "
                             "min %5.0f, max %6.0f, jitter %5.1f"
                             % (label, cnt, loss, avg or 0, mn or 0, mx or 0, jit or 0))
        lines += ["", "ZERWANIA (ostatnia doba)"]
        day = self.store.events_since(24 * 3600)
        if not day:
            lines.append("  brak")
        for _id, s, e, kind, scope, detail in day:
            lines.append("  %s  %-6s  %-12s  %s" % (
                time.strftime("%d.%m %H:%M:%S", time.localtime(s)),
                ("%.0fs" % (e - s)) if e else "trwa", scope, detail or ""))
        if self.findings:
            lines += ["", "DIAGNOZA", "  " + diagnose.summarize(self.findings), ""]
            for f in self.findings:
                lines.append("  [%s] %s" % (diagnose.SEVERITY_LABEL[f.severity], f.title))
                if f.detail:
                    lines.append("      %s" % f.detail)
                if f.advice:
                    lines.append("      -> %s" % f.advice)
        b = self.bloat_result
        if b is not None and b.idle_avg is not None:
            lines += ["", "PING POD OBCIĄŻENIEM (test bufferbloat)",
                      "  spoczynek  : %.0f ms" % b.idle_avg,
                      "  obciążenie : %s" % ("brak odpowiedzi" if b.loaded_avg is None
                                             else "%.0f ms (max %.0f, strata %.0f%%)"
                                                  % (b.loaded_avg, b.loaded_max, b.loaded_loss)),
                      "  wzrost     : %s" % ("-" if b.bump_ms is None else "%.0f ms" % b.bump_ms),
                      "  przepust.  : %s" % ("-" if not b.mbps else "%.0f Mb/s" % b.mbps),
                      "  ocena      : %s — %s" % (b.grade, b.verdict)]
        lines += ["", "ZMIANY USTAWIEŃ"]
        log = self.store.tweak_log(50)
        if not log:
            lines.append("  brak")
        for ts, tid, action, result in log:
            lines.append("  %s  %-16s %-7s %s" % (
                time.strftime("%d.%m %H:%M", time.localtime(ts)), tid, action, result))

        with open(path, "w", encoding="utf-8") as f:
            f.write("\n".join(lines))
        messagebox.showinfo("Zapisano", "Raport zapisany:\n%s" % path)

    def elevate(self):
        import ctypes
        import sys
        try:
            ctypes.windll.shell32.ShellExecuteW(
                None, "runas", sys.executable,
                '"%s"' % " ".join(sys.argv) if len(sys.argv) > 1 else '"%s"' % sys.argv[0],
                None, 1)
            self.on_close()
        except Exception as e:
            messagebox.showerror("Nie udało się", str(e))

    # -- misc -------------------------------------------------------------
    def _on_tab(self, event):
        tab = event.widget.tab(event.widget.index("current"), "text")
        if tab == "Optymalizacja" and not self.tweaks:
            self.refresh_tweaks()
        elif tab == "Historia zerwań":
            self.refresh_history()

    @staticmethod
    def _set_text(widget, text):
        widget.configure(state="normal")
        widget.delete("1.0", "end")
        widget.insert("end", text)
        widget.configure(state="disabled")

    def on_close(self):
        try:
            self.monitor.stop()
            self.store.close()
        finally:
            self.destroy()


def main():
    App().mainloop()
