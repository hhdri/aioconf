"""Validate a config: cold start, idle, bursts, sustained load, recovery. Run as root:
python3 -I validate.py <config> <watts> <outdir>"""
import glob, json, os, re, subprocess, sys, threading, time
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from harness import sample, rd, NCPU, AIOCONF

cfg, watts, out = sys.argv[1], int(sys.argv[2]), sys.argv[3]; os.makedirs(out, exist_ok=True)
NCT = next(os.path.dirname(f) for f in glob.glob("/sys/class/hwmon/hwmon*/name") if open(f).read().startswith("nct6"))
Z = "/sys/class/powercap/intel-rapl:0"
PL = [f"{Z}/constraint_0_power_limit_uw", f"{Z}/constraint_1_power_limit_uw"]
orig = [rd(p) for p in PL]
LOG, CHUNKS, t_start = [], [], time.monotonic()

def snap(phase):
    s = sample(); s.update(phase=phase, t_rel=round(s["t"] - t_start, 2), case_rpm=rd(f"{NCT}/fan1_input"),
                           case_duty=round(rd(f"{NCT}/pwm1") * 100 / 255), cputin=rd(f"{NCT}/temp2_input") / 1000)
    LOG.append(s); return s

def idle(phase, secs, until=None):
    end = time.monotonic() + secs
    while time.monotonic() < end:
        s = snap(phase)
        if until and until(s): return
        time.sleep(0.5)

def load(phase, secs, chunk=None):
    stop = time.monotonic() + secs
    def run():
        while time.monotonic() < stop - 1:
            n = min(chunk or secs, max(1, int(stop - time.monotonic())))
            t0 = time.monotonic()
            r = subprocess.run(["stress-ng", "--cpu", str(NCPU), "--cpu-method", "matrixprod", "--timeout", f"{n}s",
                                "--metrics-brief"], capture_output=True, text=True)
            m = re.search(r"cpu\s+(\d+)\s+[\d.]+\s+[\d.]+\s+[\d.]+\s+([\d.]+)", r.stdout + r.stderr)
            if m: CHUNKS.append({"phase": phase, "t0": t0, "t1": time.monotonic(), "ops_s": float(m.group(2))})
    th = threading.Thread(target=run); th.start()
    while th.is_alive():
        snap(phase); time.sleep(0.5)

try:
    for p in PL: open(p, "w").write(str(watts * 1_000_000))
    base = sample()["liquid"]
    subprocess.run([AIOCONF, "full"], check=True, stdout=subprocess.DEVNULL)
    idle("cooldown", 300, until=lambda s: s["liquid"] <= 25.0 and s["pkg"] < 40 and s["t_rel"] > 30)
    subprocess.run([AIOCONF, "apply", cfg], check=True, stdout=subprocess.DEVNULL)
    idle("idle", 90)
    for i in range(5):
        load(f"burst{i + 1}", 20); idle(f"gap{i + 1}", 40)
    load("sustained", 600, chunk=30)
    idle("recovery", 150)
finally:
    for p, v in zip(PL, orig): open(p, "w").write(str(v))

def phase(name): return [r for r in LOG if r["phase"] == name]
def stats(rows):
    a, b = rows[0], rows[-1]; dt = b["t"] - a["t"]
    return {"liquid_end": b["liquid"], "pkg_max": max(r["pkg"] for r in rows), "pkg_mean": round(sum(r["pkg"] for r in rows) / len(rows), 1),
            "throttle_pct": round((b["core_thr_ms"] - a["core_thr_ms"]) / NCPU / (dt * 1000) * 100, 2),
            "watts": round((b["energy_uj"] - a["energy_uj"]) / 1e6 / dt, 1),
            **{k: round(sum(r[k] for r in rows) / len(rows)) for k in ("pump_rpm", "fan_rpm", "case_rpm", "pump_duty", "fan_duty", "case_duty")},
            "case_rpm_max": max(r["case_rpm"] for r in rows), "cputin_max": max(r["cputin"] for r in rows)}
summary = {"config": open(cfg).read(), "limit_w": watts, "idle": stats(phase("idle")[-60:]),
           "bursts": [dict(stats(phase(f"burst{i}")), liquid_start=phase(f"burst{i}")[0]["liquid"]) for i in range(1, 6)]}
sus = phase("sustained"); tail = [r for r in sus if r["t"] >= sus[-1]["t"] - 180]
summary["sustained_last_3min"] = stats(tail)
ch = [c for c in CHUNKS if c["phase"] == "sustained"]
summary["sustained_ops_s_by_chunk"] = [round(c["ops_s"]) for c in ch]
summary["sustained_ops_s_last_3min"] = round(sum(c["ops_s"] for c in ch[-6:]) / len(ch[-6:]))
summary["recovery_end"] = stats(phase("recovery")[-20:])
json.dump(summary, open(f"{out}/summary.json", "w"), indent=1)
json.dump([{k: r[k] for k in ("t_rel", "phase", "pkg", "liquid", "pump_rpm", "fan_rpm", "case_rpm", "cputin", "mhz")} for r in LOG],
          open(f"{out}/timeline.json", "w"))
print(json.dumps({k: v for k, v in summary.items() if k != "config"}, indent=1))
