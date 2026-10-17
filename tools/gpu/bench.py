"""Ollama generation speed on the GPU, with a determinism check. python3 -I bench.py <label> [runs]"""
import hashlib, json, statistics, subprocess, sys, threading, time, urllib.request

MODEL = "qwen3:8b"
URL = "http://127.0.0.1:11434"
PROMPT = ("Explain, step by step and in detail, how a liquid cooler moves heat from a CPU to the air: "
          "the cold plate, the pump, the coolant, the radiator and the fans. Then compare it with an air cooler.")
OPTS = {"temperature": 0, "seed": 42, "num_ctx": 4096, "num_predict": 512, "top_k": 1}

def post(path, body):
    req = urllib.request.Request(URL + path, data=json.dumps(body).encode(), headers={"Content-Type": "application/json"})
    with urllib.request.urlopen(req, timeout=600) as r:
        return json.loads(r.read())

def gpu_sampler(stop, rows):
    p = subprocess.Popen(["nvidia-smi", "--query-gpu=clocks.mem,clocks.gr,fan.speed,temperature.gpu,utilization.gpu,pstate,clocks_event_reasons.active",
                          "--format=csv,noheader,nounits", "-lms", "250"], stdout=subprocess.PIPE, text=True)
    while not stop.is_set():
        line = p.stdout.readline()
        if line:
            f = [x.strip() for x in line.split(",")]
            rows.append([float(f[0]), float(f[1]), float(f[2]), float(f[3]), float(f[4]), f[5], f[6]])
    p.terminate()

def run():
    return post("/api/generate", {"model": MODEL, "prompt": PROMPT, "stream": False, "think": False, "options": OPTS, "keep_alive": "10m"})

label = sys.argv[1]; runs = int(sys.argv[2]) if len(sys.argv) > 2 else 5
run()  # warm-up and load
ps = post("/api/ps", {}) if False else json.loads(urllib.request.urlopen(URL + "/api/ps").read())
m = next(x for x in ps["models"] if x["name"].startswith(MODEL))
on_gpu = m["size_vram"] / m["size"] * 100
speeds, hashes, prompt_speeds, rows = [], set(), [], []
stop = threading.Event(); th = threading.Thread(target=gpu_sampler, args=(stop, rows), daemon=True); th.start()
for _ in range(runs):
    r = run()
    speeds.append(r["eval_count"] / r["eval_duration"] * 1e9)
    if r.get("prompt_eval_duration"):
        prompt_speeds.append(r["prompt_eval_count"] / r["prompt_eval_duration"] * 1e9)
    hashes.add(hashlib.sha256(r["response"].encode()).hexdigest()[:12])
stop.set(); th.join(timeout=3)
busy = [x for x in rows if x[4] > 50] or rows
out = {"label": label, "model_on_gpu_pct": round(on_gpu, 1), "tok_s_mean": round(statistics.mean(speeds), 2),
       "tok_s_sd": round(statistics.pstdev(speeds), 2), "tok_s_runs": [round(s, 2) for s in speeds],
       "tokens": r["eval_count"], "output_hashes": sorted(hashes),
       "mem_mhz": round(statistics.median(x[0] for x in busy)), "gr_mhz": round(statistics.median(x[1] for x in busy)),
       "fan_pct_max": max(x[2] for x in busy), "temp_max": max(x[3] for x in busy),
       "pstates": sorted({x[5] for x in busy}), "limiters": sorted({x[6] for x in busy})}
print(json.dumps(out), flush=True)
