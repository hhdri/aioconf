"""Clocks vs busy-thread count under a fixed package power limit. Run as root: python3 -I threads.py <watts> <outdir>"""
import json, os, re, subprocess, sys, threading, time
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from harness import sample, rd

watts, out = int(sys.argv[1]), sys.argv[2]; os.makedirs(out, exist_ok=True)
Z = "/sys/class/powercap/intel-rapl:0"
PL = [f"{Z}/constraint_0_power_limit_uw", f"{Z}/constraint_1_power_limit_uw"]
orig = [rd(p) for p in PL]
# P-cores are cpu0-7 (cpu4 and cpu5 reach 6.0 GHz), E-cores cpu8-23.
PLANS = [("1 P-core (6.0 GHz core)", [4]), ("2 P-cores (both 6.0 GHz cores)", [4, 5]), ("4 P-cores", [2, 3, 4, 5]),
         ("8 P-cores", list(range(8))), ("8 P + 8 E", list(range(16))), ("All 24 cores", list(range(24)))]
freq = lambda c: rd(f"/sys/devices/system/cpu/cpu{c}/cpufreq/scaling_cur_freq") / 1000
results = []
try:
    for p in PL:
        open(p, "w").write(str(watts * 1_000_000))
    for name, cpus in PLANS:
        time.sleep(15)
        res = {}
        cmd = ["stress-ng", "--cpu", str(len(cpus)), "--taskset", ",".join(map(str, cpus)), "--cpu-method", "matrixprod",
               "--timeout", "40s", "--metrics-brief"]
        th = threading.Thread(target=lambda: res.update(out=subprocess.run(cmd, capture_output=True, text=True)))
        th.start(); rows = []
        while th.is_alive():
            s = sample(); s["f"] = {c: freq(c) for c in cpus}; rows.append(s); time.sleep(0.25)
        th.join()
        tail = [r for r in rows if r["t"] >= rows[-1]["t"] - 25]
        a, b = tail[0], tail[-1]
        pc = [c for c in cpus if c < 8]; ec = [c for c in cpus if c >= 8]
        mean = lambda cs: round(sum(r["f"][c] for r in tail for c in cs) / (len(cs) * len(tail))) if cs else None
        m = re.search(r"cpu\s+(\d+)\s+[\d.]+\s+[\d.]+\s+[\d.]+\s+([\d.]+)", res["out"].stdout + res["out"].stderr)
        r = {"load": name, "threads": len(cpus), "p_mhz": mean(pc), "e_mhz": mean(ec),
             "fav_mhz": mean([c for c in cpus if c in (4, 5)]),
             "watts": round((b["energy_uj"] - a["energy_uj"]) / 1e6 / (b["t"] - a["t"]), 1),
             "pkg_max": max(x["pkg"] for x in tail), "ops_s": float(m.group(2)) if m else None}
        results.append(r); print(json.dumps(r), flush=True)
        json.dump({"limit_w": watts, "runs": results}, open(f"{out}/results.json", "w"), indent=1)
finally:
    for p, v in zip(PL, orig):
        open(p, "w").write(str(v))
    print("restored power limits:", [rd(p) / 1e6 for p in PL], flush=True)
