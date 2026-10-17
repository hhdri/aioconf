#!/usr/bin/env python3
"""Render the measurement charts in docs/ from docs/data/*.json.

Standard library only. Each SVG carries light and dark styles and follows the
viewer's color scheme. Run from the repository root:  python3 docs/make_charts.py
"""
import json
import os

HERE = os.path.dirname(os.path.abspath(__file__))
W, H = 720, 420
PAD_L, PAD_R, PAD_T, PAD_B = 64, 132, 92, 56

STYLE = """
  .surface { fill: #fcfcfb; }
  .title { fill: #0b0b0b; font: 600 17px system-ui, -apple-system, "Segoe UI", sans-serif; }
  .sub { fill: #52514e; font: 13px system-ui, -apple-system, "Segoe UI", sans-serif; }
  .tick { fill: #898781; font: 12px system-ui, -apple-system, "Segoe UI", sans-serif; font-variant-numeric: tabular-nums; }
  .axis-label { fill: #52514e; font: 12px system-ui, -apple-system, "Segoe UI", sans-serif; }
  .label { fill: #0b0b0b; font: 600 12px system-ui, -apple-system, "Segoe UI", sans-serif; }
  .label-muted { fill: #52514e; font: 12px system-ui, -apple-system, "Segoe UI", sans-serif; }
  .grid { stroke: #e1e0d9; stroke-width: 1; }
  .base { stroke: #c3c2b7; stroke-width: 1; }
  .ref { stroke: #898781; stroke-width: 1; }
  .s1-line { stroke: #2a78d6; fill: none; }
  .s2-line { stroke: #eb6834; fill: none; }
  .s3-line { stroke: #1baf7a; fill: none; }
  .crit-line { stroke: #d03b3b; fill: none; }
  .crit-dot { fill: #d03b3b; }
  .s1-dot { fill: #2a78d6; }
  .s2-dot { fill: #eb6834; }
  .ring { stroke: #fcfcfb; stroke-width: 2; }
  @media (prefers-color-scheme: dark) {
    .surface { fill: #1a1a19; }
    .title, .label { fill: #f0efec; }
    .sub, .axis-label, .label-muted { fill: #c3c2b7; }
    .grid { stroke: #2c2c2a; }
    .base { stroke: #383835; }
    .s1-line { stroke: #3987e5; }
    .s2-line { stroke: #d95926; }
    .s3-line { stroke: #199e70; }
    .s1-dot { fill: #3987e5; }
    .s2-dot { fill: #d95926; }
    .ring { stroke: #1a1a19; }
  }
"""


def esc(s):
    return str(s).replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;")


def line_chart(path, title, subtitle, x_label, y_label, x_range, y_range, x_ticks, y_ticks, series,
               refs=(), labels=(), x_fmt=str, y_fmt=str, end_labels=True):
    """series: [{"name", "cls", "points": [(x, y), ...], "markers": bool}]
    refs: horizontal reference lines [(y, text)]. labels: [(series_index, point_index, text, dx, dy)]."""
    x0, x1 = x_range
    y0, y1 = y_range
    pw, ph = W - PAD_L - PAD_R, H - PAD_T - PAD_B
    X = lambda x: PAD_L + (x - x0) / (x1 - x0) * pw
    Y = lambda y: PAD_T + ph - (y - y0) / (y1 - y0) * ph
    o = [f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {W} {H}" width="{W}" height="{H}" role="img" '
         f'aria-label="{esc(title)}. {esc(subtitle)}">', f"<style>{STYLE}</style>",
         f'<rect class="surface" width="{W}" height="{H}" rx="8"/>',
         f'<text class="title" x="{PAD_L}" y="30">{esc(title)}</text>',
         f'<text class="sub" x="{PAD_L}" y="50">{esc(subtitle)}</text>']
    # Legend for two or more series, in one row under the subtitle.
    if len(series) > 1:
        lx = PAD_L
        for s in series:
            o.append(f'<line class="{s["cls"]}-line" x1="{lx}" y1="70" x2="{lx + 18}" y2="70" stroke-width="2" stroke-linecap="round"/>')
            o.append(f'<text class="label-muted" x="{lx + 24}" y="74">{esc(s["name"])}</text>')
            lx += 24 + 8 * len(s["name"]) + 28
    for t in y_ticks:
        o.append(f'<line class="grid" x1="{PAD_L}" y1="{Y(t):.1f}" x2="{PAD_L + pw}" y2="{Y(t):.1f}"/>')
        o.append(f'<text class="tick" x="{PAD_L - 8}" y="{Y(t) + 4:.1f}" text-anchor="end">{esc(y_fmt(t))}</text>')
    o.append(f'<line class="base" x1="{PAD_L}" y1="{PAD_T + ph}" x2="{PAD_L + pw}" y2="{PAD_T + ph}"/>')
    for t in x_ticks:
        o.append(f'<text class="tick" x="{X(t):.1f}" y="{PAD_T + ph + 18}" text-anchor="middle">{esc(x_fmt(t))}</text>')
    o.append(f'<text class="axis-label" x="{PAD_L + pw / 2}" y="{H - 14}" text-anchor="middle">{esc(x_label)}</text>')
    o.append(f'<text class="axis-label" transform="translate(16 {PAD_T + ph / 2}) rotate(-90)" text-anchor="middle">{esc(y_label)}</text>')
    for y, text in refs:
        o.append(f'<line class="ref" x1="{PAD_L}" y1="{Y(y):.1f}" x2="{PAD_L + pw}" y2="{Y(y):.1f}"/>')
        o.append(f'<text class="label-muted" x="{PAD_L + pw + 8}" y="{Y(y) + 4:.1f}">{esc(text)}</text>')
    for s in series:
        pts = sorted(s["points"])
        d = " ".join(f"{'M' if i == 0 else 'L'}{X(x):.1f} {Y(y):.1f}" for i, (x, y) in enumerate(pts))
        o.append(f'<path class="{s["cls"]}-line" d="{d}" stroke-width="2" stroke-linejoin="round" stroke-linecap="round"/>')
        if s.get("markers", True):
            for x, y in pts:
                o.append(f'<circle class="{s["cls"]}-dot ring" cx="{X(x):.1f}" cy="{Y(y):.1f}" r="5" style="paint-order: stroke"/>')
        if len(series) > 1 and end_labels:
            x, y = pts[-1]
            o.append(f'<text class="label-muted" x="{X(x) + 10:.1f}" y="{Y(y) + 4:.1f}">{esc(s["name"])}</text>')
    for si, pi, text, dx, dy in labels:
        x, y = sorted(series[si]["points"])[pi]
        o.append(f'<text class="label" x="{X(x) + dx:.1f}" y="{Y(y) + dy:.1f}" text-anchor="middle">{esc(text)}</text>')
    o.append("</svg>")
    with open(os.path.join(HERE, path), "w") as f:
        f.write("\n".join(o) + "\n")
    print("wrote", path)


def data(name):
    with open(os.path.join(HERE, "data", name)) as f:
        return json.load(f)


def power_chart():
    d = data("power-scaling.json")
    ref = next(r for r in d["runs"] if r["limit_w"] == 253)["ops_s"]
    pts = [(r["watts"], r["ops_s"] / ref * 100) for r in d["runs"]]
    marks = {125: "125 W: 80%", 180: "180 W: 92%", 253: "253 W: 100%"}
    labels = []
    for i, r in enumerate(d["runs"]):
        if r["limit_w"] in marks:
            labels.append((0, i, marks[r["limit_w"]], 0, -14))
        if r["limit_w"] == 4095:
            labels.append((0, i, f"No limit, {r['watts']:.0f} W: {r['ops_s'] / ref * 100:.0f}%", -20, -14))
    line_chart("power-scaling.svg", "All-core throughput vs package power",
               "i9-14900K, 24 threads, stress-ng matrixprod, 2-minute runs, 100% = Intel's 253 W limit",
               "Package power (W)", "Throughput (% of 253 W)", (80, 330), (50, 110),
               [100, 150, 200, 250, 300], [50, 60, 70, 80, 90, 100, 110],
               [{"name": "Throughput", "cls": "s1", "points": pts}], labels=labels, y_fmt=lambda v: f"{v}%")


def efficiency_chart():
    d = data("power-scaling.json")
    ref = next(r for r in d["runs"] if r["limit_w"] == 253)
    ref_eff = ref["ops_s"] / ref["watts"]
    pts = [(r["watts"], r["ops_s"] / r["watts"] / ref_eff * 100) for r in d["runs"]]
    labels = [(0, 0, f"{pts[0][1]:.0f}%", 0, -14), (0, len(pts) - 1, f"{pts[-1][1]:.0f}%", 0, -14)]
    line_chart("efficiency.svg", "Work per joule vs package power",
               "Same runs. 100% = the efficiency at 253 W. Lower limits do more work per watt.",
               "Package power (W)", "Work per joule (% of 253 W)", (80, 330), (60, 180),
               [100, 150, 200, 250, 300], [60, 80, 100, 120, 140, 160, 180],
               [{"name": "Efficiency", "cls": "s1", "points": pts}], labels=labels, y_fmt=lambda v: f"{v}%")


def fan_chart():
    d = data("fan-staircase.json")
    steps = d["steps"]
    liquid = [(s["fan"], s["liquid"]) for s in steps]
    pkg = [(s["fan"], s["pkg_mean"]) for s in steps]
    line_chart("fan-staircase.svg", "Sustained 253 W: radiator fan duty vs temperature",
               "Pump at 100%, each step held until the liquid settled. Fans bottom out near 735 rpm below 35%.",
               "Radiator fan duty (%)", "Temperature (°C)", (20, 105), (20, 105),
               [25, 35, 50, 70, 100], [20, 40, 60, 80, 100],
               [{"name": "Package", "cls": "s2", "points": pkg}, {"name": "Liquid", "cls": "s1", "points": liquid}],
               refs=[(100, "Throttles at 100 °C")], x_fmt=lambda v: f"{v}%")


def pump_chart():
    d = data("pump.json")
    burst = [(r["pump_rpm"], r["rise"]) for r in d["burst"]]
    sustained = [(r["pump_rpm"], r["offset"]) for r in d["sustained"]]
    line_chart("pump.svg", "Pump speed vs how far the CPU runs above the liquid",
               "253 W all-core. Bursts are 20 s from idle; sustained is after the liquid settled.",
               "Pump speed (rpm)", "Package above liquid (°C)", (800, 3300), (40, 70),
               [1000, 1500, 2000, 2500, 3000], [40, 45, 50, 55, 60, 65, 70],
               [{"name": "20 s burst", "cls": "s2", "points": burst},
                {"name": "Sustained", "cls": "s1", "points": sustained}],
               y_fmt=lambda v: f"{v}")


def gpu_chart():
    d = data("gpu-memory.json")
    ok = [(s["mem_mhz"], s["tok_s"]) for s in d["steps"] if s["output_correct"]]
    bad = [(s["mem_mhz"], s["tok_s"]) for s in d["steps"] if not s["output_correct"]]
    stock = d["steps"][0]["tok_s"]
    labels = [(0, 0, f"Stock {ok[0][1]:.1f}", 0, 22)]
    i3000 = next(i for i, p in enumerate(sorted(ok)) if p[0] == 9751)
    labels.append((0, i3000, f"+3000: {ok[i3000][1]:.1f} (+{(ok[i3000][1] / stock - 1) * 100:.1f}%)", 30, 28))
    labels.append((1, 0, "Wrong answers", -6, -16))
    line_chart("gpu-memory.svg", "RTX 4060: memory clock vs Ollama generation speed",
               "qwen3:8b fully on the GPU, 8 runs per step. Every answer checked against the stock answer.",
               "Memory clock in compute mode (MHz)", "Tokens per second", (8100, 10400), (44, 62),
               [8250, 8750, 9250, 9750, 10250], [44, 48, 52, 56, 60],
               [{"name": "Correct output", "cls": "s1", "points": ok}, {"name": "Corrupted output", "cls": "crit", "points": bad}],
               labels=labels, end_labels=False)


def curve_chart():
    d = data("curves.json")
    temps = list(range(20, 60))
    series = []
    for name, cls in (("Pump", "s1"), ("Fan", "s2")):
        table = d[name.lower()]
        series.append({"name": name, "cls": cls, "markers": False, "points": list(zip(temps, table))})
    line_chart("curves.svg", "Kraken curves in /etc/aioconf.conf",
               "Duty the firmware runs at each liquid temperature. 59 °C forces 100% as a failsafe.",
               "Liquid temperature (°C)", "Duty (%)", (20, 60), (0, 100),
               [20, 25, 30, 35, 40, 45, 50, 55, 60], [0, 20, 40, 60, 80, 100],
               series, x_fmt=lambda v: f"{v}", y_fmt=lambda v: f"{v}%", end_labels=False)


def timeline_chart():
    """Two panels on one time axis: temperatures, then fan speeds."""
    d = data("validation-180w.json")
    rows = d["timeline"]
    # Skip the first seconds, while the cooler leaves the full-speed cooldown.
    rows = [r for r in rows if r["t"] >= rows[0]["t"] + 15]
    t0 = rows[0]["t"]
    T = lambda r: (r["t"] - t0) / 60
    tmax = T(rows[-1])
    h, pl, pr = 640, 64, 150
    pw = W - pl - pr
    panels = [(96, 330, (20, 105), [20, 40, 60, 80, 100], "Temperature (°C)",
               [("Package", "s2", "pkg"), ("Liquid", "s1", "liquid")], [(100, "Throttles at 100 °C")]),
              (380, 590, (0, 2500), [0, 500, 1000, 1500, 2000, 2500], "Fan speed (rpm)",
               [("Pump", "s1", "pump_rpm"), ("Radiator fans", "s2", "fan_rpm")], [])]
    X = lambda m: pl + m / tmax * pw
    o = [f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {W} {h}" width="{W}" height="{h}" role="img" '
         f'aria-label="Validation run of the example config at 180 W">', f"<style>{STYLE}</style>",
         f'<rect class="surface" width="{W}" height="{h}" rx="8"/>',
         f'<text class="title" x="{pl}" y="30">Validation: the example config at a 180 W limit</text>',
         f'<text class="sub" x="{pl}" y="50">Kraken curves. Idle, five 20 s bursts, 10 minutes of full load, recovery. No throttling at any point.</text>']
    # Phase boundaries as hairlines with labels.
    marks, seen = [], set()
    for r in rows:
        ph = r["phase"].rstrip("0123456789")
        if ph not in seen:
            marks.append((T(r), ph)); seen.add(ph)
    names = {"idle": "Idle", "burst": "Bursts", "sustained": "Sustained full load", "recovery": "Recovery"}
    for m, ph in marks:
        if ph in names:
            o.append(f'<line class="grid" x1="{X(m):.1f}" y1="78" x2="{X(m):.1f}" y2="590"/>')
            o.append(f'<text class="label-muted" x="{X(m) + 4:.1f}" y="88">{names[ph]}</text>')
    for top, bottom, (y0, y1), ticks, ylabel, series, refs in panels:
        Y = lambda v: bottom - (v - y0) / (y1 - y0) * (bottom - top)
        for t in ticks:
            o.append(f'<line class="grid" x1="{pl}" y1="{Y(t):.1f}" x2="{pl + pw}" y2="{Y(t):.1f}"/>')
            o.append(f'<text class="tick" x="{pl - 8}" y="{Y(t) + 4:.1f}" text-anchor="end">{t}</text>')
        o.append(f'<line class="base" x1="{pl}" y1="{bottom}" x2="{pl + pw}" y2="{bottom}"/>')
        o.append(f'<text class="axis-label" transform="translate(16 {(top + bottom) / 2}) rotate(-90)" text-anchor="middle">{ylabel}</text>')
        for y, text in refs:
            o.append(f'<line class="ref" x1="{pl}" y1="{Y(y):.1f}" x2="{pl + pw}" y2="{Y(y):.1f}"/>')
            o.append(f'<text class="label-muted" x="{pl + pw + 8}" y="{Y(y) + 4:.1f}">{text}</text>')
        ends = []
        for name, cls, key in series:
            pts = " ".join(f"{'M' if i == 0 else 'L'}{X(T(r)):.1f} {Y(r[key]):.1f}" for i, r in enumerate(rows))
            o.append(f'<path class="{cls}-line" d="{pts}" stroke-width="2" stroke-linejoin="round"/>')
            ends.append([Y(rows[-1][key]), name])
        # Direct end labels, nudged apart only enough not to overlap.
        ends.sort()
        for i in range(1, len(ends)):
            ends[i][0] = max(ends[i][0], ends[i - 1][0] + 15)
        for y, name in ends:
            o.append(f'<text class="label-muted" x="{pl + pw + 8}" y="{y + 4:.1f}">{name}</text>')
    for m in range(0, int(tmax) + 1, 2):
        o.append(f'<text class="tick" x="{X(m):.1f}" y="608" text-anchor="middle">{m}</text>')
    o.append(f'<text class="axis-label" x="{pl + pw / 2}" y="630" text-anchor="middle">Minutes</text>')
    o.append("</svg>")
    with open(os.path.join(HERE, "validation.svg"), "w") as f:
        f.write("\n".join(o) + "\n")
    print("wrote validation.svg")


if __name__ == "__main__":
    for chart in (power_chart, efficiency_chart, fan_chart, pump_chart):
        chart()
    if os.path.exists(os.path.join(HERE, "data", "curves.json")):
        curve_chart()
    if os.path.exists(os.path.join(HERE, "data", "validation-180w.json")):
        timeline_chart()
    if os.path.exists(os.path.join(HERE, "data", "gpu-memory.json")):
        gpu_chart()
