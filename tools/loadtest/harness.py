"""Shared sensor sampling."""
import glob, json, os, re, subprocess, sys, time

AIOCONF = os.environ.get("AIOCONF", "aioconf")
NCPU = os.cpu_count()

def hwmon(name):
    for d in glob.glob("/sys/class/hwmon/hwmon*"):
        if open(d + "/name").read().strip() == name:
            return d
    raise SystemExit(f"no hwmon {name}")

CORE, KRAKEN = hwmon("coretemp"), hwmon("kraken2023")
PKG = next(f[:-6] + "_input" for f in glob.glob(CORE + "/temp*_label") if open(f).read().startswith("Package"))
CORE_TEMPS = [f for f in glob.glob(CORE + "/temp*_input") if f != PKG]
CORE_THR = glob.glob("/sys/devices/system/cpu/cpu*/thermal_throttle/core_throttle_total_time_ms")
PKG_THR = "/sys/devices/system/cpu/cpu0/thermal_throttle/package_throttle_total_time_ms"
FREQS = glob.glob("/sys/devices/system/cpu/cpu*/cpufreq/scaling_cur_freq")
RAPL = "/sys/class/powercap/intel-rapl:0/energy_uj"

def rd(p): return int(open(p).read())

def sample():
    return {
        "t": time.monotonic(), "pkg": rd(PKG) / 1000, "core_max": max(rd(f) for f in CORE_TEMPS) / 1000,
        "core_thr_ms": sum(rd(f) for f in CORE_THR), "pkg_thr_ms": rd(PKG_THR),
        "mhz": sum(rd(f) for f in FREQS) / len(FREQS) / 1000, "energy_uj": rd(RAPL),
        "liquid": rd(KRAKEN + "/temp1_input") / 1000, "pump_rpm": rd(KRAKEN + "/fan1_input"),
        "fan_rpm": rd(KRAKEN + "/fan2_input"),
        "pump_duty": round(rd(KRAKEN + "/pwm1") * 100 / 255), "fan_duty": round(rd(KRAKEN + "/pwm2") * 100 / 255),
    }

