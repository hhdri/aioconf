"""Turbo headroom: single-thread clocks per core type, then all-core throughput vs power limit.
Run as root: python3 -I turbo.py <outdir>"""
import json, os, re, subprocess, sys, time
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from harness import sample, rd, NCPU, AIOCONF, PKG

out = sys.argv[1]; os.makedirs(out, exist_ok=True)
LEVELS = [int(x) for x in sys.argv[2].split(",")] if len(sys.argv) > 2 else [125, 180, 253, 300, 4095]
Z = "/sys/class/powercap/intel-rapl:0"
PL = {0: f"{Z}/constraint_0_power_limit_uw", 1: f"{Z}/constraint_1_power_limit_uw"}
orig = {k: rd(p) for k, p in PL.items()}

def set_pl(watts):
    for p in PL.values():
        open(p, "w").write(str(int(watts * 1e6)))
    return {k: rd(p) / 1e6 for k, p in PL.items()}

def stress(args, secs):
    r = subprocess.run(["stress-ng", *args, "--cpu-method", "matrixprod", "--timeout", f"{secs}s", "--metrics-brief"],
                       capture_output=True, text=True)
    m = re.search(r"cpu\s+(\d+)\s+[\d.]+\s+[\d.]+\s+[\d.]+\s+([\d.]+)", r.stdout + r.stderr)
    return float(m.group(2)) if m else None

def run_sampled(cmd_args, secs, freq_cpus):
    import threading
    res = {}
    th = threading.Thread(target=lambda: res.update(ops=stress(cmd_args, secs))); th.start()
    rows = []
    while th.is_alive():
        s = sample()
        s["f"] = [rd(f"/sys/devices/system/cpu/cpu{c}/cpufreq/scaling_cur_freq") / 1000 for c in freq_cpus]
        rows.append(s); time.sleep(0.25)
    th.join()
    return res.get("ops"), rows

results = {"single": [], "allcore": []}
subprocess.run([AIOCONF, "full"], check=True, stdout=subprocess.DEVNULL)
try:
    # Single thread on a 6.0 GHz P-core, a 5.7 GHz P-core, and an E-core.
    for cpu in (() if len(sys.argv) > 2 else (4, 0, 8)):
        time.sleep(15)
        ops, rows = run_sampled(["--cpu", "1", "--taskset", str(cpu)], 30, [cpu])
        tail = rows[len(rows) // 3:]
        f = [r["f"][0] for r in tail]
        r = {"cpu": cpu, "max_mhz": rd(f"/sys/devices/system/cpu/cpu{cpu}/cpufreq/cpuinfo_max_freq") // 1000,
             "mean_mhz": round(sum(f) / len(f)), "min_mhz": round(min(f)), "peak_mhz": round(max(f)),
             "pkg_max": max(x["pkg"] for x in tail), "ops_s": ops,
             "watts": round((tail[-1]["energy_uj"] - tail[0]["energy_uj"]) / 1e6 / (tail[-1]["t"] - tail[0]["t"]), 1)}
        results["single"].append(r); print(json.dumps(r), flush=True)

    # All-core throughput at each package power limit.
    for watts in LEVELS:
        applied = set_pl(watts)
        time.sleep(20)
        ops, rows = run_sampled(["--cpu", str(NCPU)], 120, list(range(NCPU)))
        tail = [x for x in rows if x["t"] >= rows[-1]["t"] - 60]
        a, b = tail[0], tail[-1]; dt = b["t"] - a["t"]
        pf = [sum(x["f"][:8]) / 8 for x in tail]; ef = [sum(x["f"][8:]) / 16 for x in tail]
        r = {"limit_w": watts, "applied": applied, "ops_s": ops,
             "watts": round((b["energy_uj"] - a["energy_uj"]) / 1e6 / dt, 1),
             "p_mhz": round(sum(pf) / len(pf)), "e_mhz": round(sum(ef) / len(ef)),
             "pkg_mean": round(sum(x["pkg"] for x in tail) / len(tail), 1), "pkg_max": max(x["pkg"] for x in tail),
             "throttle_pct": round((b["core_thr_ms"] - a["core_thr_ms"]) / NCPU / (dt * 1000) * 100, 2),
             "liquid": b["liquid"]}
        results["allcore"].append(r); print(json.dumps(r), flush=True)
        json.dump(results, open(f"{out}/results.json", "w"), indent=1)
finally:
    for k, p in PL.items():
        open(p, "w").write(str(orig[k]))
    print("restored power limits:", {k: rd(p) / 1e6 for k, p in PL.items()}, flush=True)
