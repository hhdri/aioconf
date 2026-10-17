# Load-test scripts

The scripts that produced the data in `docs/data`. They need `stress-ng`,
Python 3, root, an installed `aioconf` (or `AIOCONF=/path/to/aioconf`), and
the `coretemp` and `kraken2023` hwmon drivers. Each one leaves the cooler at
full speed or on the config it validated, and restores the CPU power limits
it changes.

| Script | What it measures |
|---|---|
| `harness.py` | shared sensor sampling: package and core temperatures, throttle counters, clocks, RAPL energy, Kraken readings |
| `stair.py` | continuous full load with stepped pump and fan settings, each held until the liquid settles |
| `run.py` | short bursts from a cooled-down start, per cooler setting |
| `turbo.py` | single-thread clocks per core type, then all-core throughput per power limit |
| `threads.py` | clocks per busy-thread count under one power limit |
| `validate.py` | a config end to end: idle, bursts, 10 minutes of load, recovery |

Plans are small JSON files. For example, a fan staircase:

```json
{"settle_slope": 0.15,
 "steps": [{"pump": 100, "fan": 100, "min": 300, "max": 540},
           {"pump": 100, "fan": 50,  "min": 240, "max": 480}]}
```

```sh
sudo python3 -I tools/loadtest/stair.py plan.json out/
sudo python3 -I tools/loadtest/turbo.py out/ 125,180,253
sudo python3 -I tools/loadtest/validate.py /etc/aioconf.conf 180 out/
```

The CPU core numbers in `turbo.py` and `threads.py` match an i9-14900K with
Hyper-Threading off, where cpu0-7 are P-cores and cpu4 and cpu5 reach 6.0 GHz.
Adjust them for other CPUs.

## GPU memory

`tools/gpu/bench.py LABEL [RUNS]` measures Ollama generation speed with a
deterministic prompt and prints a hash of each answer. Run it once at stock to
get the reference hash. `REF=<hash> tools/gpu/longrun.sh OFFSET MINUTES` then
holds a memory offset (through `aioconf-nvidia`) and stops at the first answer
that differs from the reference. The model is `qwen3:8b`; edit `MODEL` in
`bench.py` for another one that fits fully in video memory.
