"""Burst load test: cold start, then bursts per cooler config. Run as root: python3 -I run.py <plan.json> <outdir>."""
import json, os, re, subprocess, sys, time
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from harness import sample, rd, NCPU, AIOCONF

def apply(pump, fan, path):
    open(path, "w").write(f"pump = {pump}\nfan = {fan}\n")
    subprocess.run([AIOCONF, "apply", path], check=True, stdout=subprocess.DEVNULL)

LOG = []
def record(phase, cfg, s):
    s.update(phase=phase, cfg=cfg); LOG.append(s)

def idle(cfg, phase, secs, until=None, dt=0.5):
    end = time.monotonic() + secs
    while time.monotonic() < end:
        s = sample(); record(phase, cfg, s)
        if until and until(s): return
        time.sleep(dt)

def load(cfg, phase, secs, dt=0.25):
    p = subprocess.Popen(["stress-ng", "--cpu", str(NCPU), "--cpu-method", "matrixprod", "--timeout", f"{secs}s",
                          "--metrics-brief"], stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
    while p.poll() is None:
        record(phase, cfg, sample()); time.sleep(dt)
    out = p.stdout.read()
    m = re.search(r"cpu\s+(\d+)\s+[\d.]+\s+[\d.]+\s+[\d.]+\s+([\d.]+)", out)
    return float(m.group(2)) if m else None   # bogo ops/s (real time)

def summarize(cfg, phase):
    rows = [r for r in LOG if r["cfg"] == cfg and r["phase"] == phase]
    a, b = rows[0], rows[-1]
    dt = b["t"] - a["t"]
    return {
        "secs": round(dt, 1), "pkg_max": max(r["pkg"] for r in rows), "core_max": max(r["core_max"] for r in rows),
        "pkg_mean": round(sum(r["pkg"] for r in rows) / len(rows), 1),
        "core_thr_ms_per_core_s": round((b["core_thr_ms"] - a["core_thr_ms"]) / NCPU / dt, 1),
        "pkg_thr_ms": b["pkg_thr_ms"] - a["pkg_thr_ms"],
        "mhz_mean": round(sum(r["mhz"] for r in rows) / len(rows)),
        "watts": round((b["energy_uj"] - a["energy_uj"]) / 1e6 / dt, 1),
        "liquid_start": a["liquid"], "liquid_end": b["liquid"], "liquid_max": max(r["liquid"] for r in rows),
        "pump_rpm_max": max(r["pump_rpm"] for r in rows), "fan_rpm_max": max(r["fan_rpm"] for r in rows),
        "fan_rpm_mean": round(sum(r["fan_rpm"] for r in rows) / len(rows)),
    }

def main():
    plan = json.load(open(sys.argv[1])); out = sys.argv[2]; os.makedirs(out, exist_ok=True)
    base = sample()["liquid"]
    results = {"baseline_liquid": base, "configs": {}}
    for cfg in plan["configs"]:
        name = cfg["name"]
        # Cold start: full cooling until the liquid is back near baseline and the package is idle-cool.
        apply("100", "100", f"{out}/full.conf")
        idle(name, "cooldown", plan["cooldown_max"],
             until=lambda s: s["liquid"] <= base + plan["liquid_margin"] and s["pkg"] < 40 and s["t"] - LOG[0]["t"] > 0)
        apply(cfg["pump"], cfg["fan"], f"{out}/{name}.conf")
        idle(name, "settle", plan["settle"])
        ops = []
        for i in range(plan["bursts"]):
            ops.append(load(name, "burst", plan["burst_secs"]))
            idle(name, "burst_gap", plan["gap_secs"])
        res = {"pump": cfg["pump"], "fan": cfg["fan"], "burst": summarize(name, "burst"),
               "burst_ops_per_s": round(sum(o for o in ops if o) / len(ops))}
        if plan.get("sustained"):
            res["sustained_ops_per_s"] = round(load(name, "sustained", plan["sustained"]))
            res["sustained"] = summarize(name, "sustained")
        results["configs"][name] = res
        print(json.dumps({name: res}), flush=True)
        json.dump(results, open(f"{out}/results.json", "w"), indent=1)
        json.dump(LOG, open(f"{out}/samples.json", "w"))
    apply("100", "100", f"{out}/full.conf")   # leave the cooler on full until a curve is chosen

main()
