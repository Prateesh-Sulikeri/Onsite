"""Onsite's speech-to-text worker: Whisper large-v3 on the GPU, kept loaded.

Started by Onsite (src-tauri/src/voice/whisper.rs) with VoiceStudio's bundled Python, which
already has faster-whisper and the CUDA libraries. VoiceStudio's own Whisper endpoint reloads
the model on every call (~38 s) because its voice model fills the GPU; a model that stays
loaded transcribes a 10 s answer in ~1.2 s.

Protocol, one JSON object per line:
  stdout first: {"ready": true, "model": ..., "load_ms": ...} or {"ready": false, "error": ...}
  stdin:  {"id": 1, "wav": "C:\\...\\seg.wav", "prompt": "Technical interview. Kafka, ..."}
  stdout: {"id": 1, "text": "...", "ms": 1234} or {"id": 1, "error": "..."}
Audio is read from the file Onsite wrote; nothing in a request is ever executed.
"""

import json
import os
import site
import sys
import time


def say(obj):
    sys.stdout.write(json.dumps(obj) + "\n")
    sys.stdout.flush()


def add_cuda_dlls():
    # VoiceStudio puts these on the DLL path itself at runtime; a plain interpreter doesn't.
    for sp in site.getsitepackages():
        for d in ("cudnn8_compat/nvidia/cudnn/bin", "torch/lib", "nvidia/cublas/bin", "nvidia/cudnn/bin"):
            d = os.path.join(sp, d)
            if os.path.isdir(d):
                os.add_dll_directory(d)
                os.environ["PATH"] = d + os.pathsep + os.environ.get("PATH", "")


def main():
    model_dir = sys.argv[1]
    add_cuda_dlls()
    started = time.time()
    try:
        import numpy as np
        from faster_whisper import WhisperModel

        model = WhisperModel(model_dir, device="cuda", compute_type="int8_float16")
        # The first decode initialises CUDA kernels (~2 s); do it before the first answer.
        list(model.transcribe(np.zeros(16000, dtype=np.float32), language="en", beam_size=1)[0])
    except Exception as e:  # noqa: BLE001 - any failure means "use the fallback"
        say({"ready": False, "error": f"{type(e).__name__}: {e}"})
        return
    say({"ready": True, "model": os.path.basename(model_dir.rstrip("\\/")), "load_ms": int((time.time() - started) * 1000)})

    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        req_id = None
        try:
            req = json.loads(line)
            req_id = req["id"]
            t = time.time()
            segments, _ = model.transcribe(
                req["wav"],
                language="en",
                initial_prompt=req.get("prompt") or None,
                beam_size=1,
                condition_on_previous_text=False,
                vad_filter=False,
            )
            text = " ".join(s.text.strip() for s in segments).strip()
            say({"id": req_id, "text": text, "ms": int((time.time() - t) * 1000)})
        except Exception as e:  # noqa: BLE001
            say({"id": req_id, "error": f"{type(e).__name__}: {e}"})


if __name__ == "__main__":
    main()
