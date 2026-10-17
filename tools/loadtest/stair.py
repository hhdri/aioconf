"""Continuous full load with stepped cooler settings; each step held until liquid temp settles.
Run as root: python3 -I stair.py <plan.json> <outdir>"""
import json, os, re, subprocess, sys, threading, time
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from harness import sample, NCPU, AIOCONF

plan = json.load(open(sys.argv[1])); out = sys.argv[2]; os.makedirs(out, exist_ok=True)
CHUNKS, LOG, stop = [], [], threading.Event()

def loader():
    while not stop.is_set():
        t0 = time.monotonic()
        r = subprocess.run(["stress-ng", "--cpu", str(NCPU), "--cpu-method", "matrixprod", "--timeout", "15s",
                            "--metrics-brief"], capture_output=True, text=True)
        m = re.search(r"cpu\s+(\d+)\s+([\d.]+)\s+[\d.]+\s+[\d.]+\s+([\d.]+)", r.stdout + r.stderr)
        if m: CHUNKS.append({"t0": t0, "t1": time.monotonic(), "ops_s": float(m.group(3))})

def setcool(pump, fan):
    for ch, v in (("pump", pump), ("fan", fan)):
        subprocess.run([AIOCONF, "set", ch, *str(v).split()], check=True, stdout=subprocess.DEVNULL)

def window(rows, secs):
    t_end = rows[-1]["t"]; w = [r for r in rows if r["t"] >= t_end - secs]
    a, b = w[0], w[-1]; dt = b["t"] - a["t"]
    ch = [c for c in CHUNKS if c["t0"] >= a["t"] - 1 and c["t1"] <= b["t"] + 1]
    return {
        "liquid": b["liquid"], "pkg_mean": round(sum(r["pkg"] for r in w) / len(w), 1), "pkg_max": max(r["pkg"] for r in w),
        "core_max": max(r["core_max"] for r in w),
        "throttle_pct": round((b["core_thr_ms"] - a["core_thr_ms"]) / NCPU / (dt * 1000) * 100, 2),
        "mhz": round(sum(r["mhz"] for r in w) / len(w)), "watts": round((b["energy_uj"] - a["energy_uj"]) / 1e6 / dt, 1),
        "ops_s": round(sum(c["ops_s"] for c in ch) / len(ch)) if ch else None,
        "pump_rpm": round(sum(r["pump_rpm"] for r in w) / len(w)), "fan_rpm": round(sum(r["fan_rpm"] for r in w) / len(w)),
    }

results = []
th = threading.Thread(target=loader, daemon=True); th.start()
try:
    for step in plan["steps"]:
        setcool(step["pump"], step["fan"])
        t0 = time.monotonic(); rows = []
        while True:
            s = sample(); s["step"] = f'{step["pump"]}/{step["fan"]}'; rows.append(s); LOG.append(s)
            el = time.monotonic() - t0
            if el >= step["min"]:
                old = [r for r in rows if r["t"] <= s["t"] - 90]
                slope = (s["liquid"] - old[-1]["liquid"]) / 1.5 if old else 9   # °C per minute over last 90 s
                if abs(slope) < plan["settle_slope"] or el >= step["max"]:
                    break
            time.sleep(0.5)
        res = {"pump": step["pump"], "fan": step["fan"], "held_s": round(el), "settled": el < step["max"], **window(rows, 60)}
        results.append(res); print(json.dumps(res), flush=True)
        json.dump(results, open(f"{out}/results.json", "w"), indent=1); json.dump(LOG, open(f"{out}/samples.json", "w"))
finally:
    stop.set(); th.join(timeout=30)
    setcool(100, 100)
