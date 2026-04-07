# ADR-2604070200: Bazzite Standalone Inference Configuration

## Status
Accepted

## Context
hex requires local LLM inference for autonomous swarm code generation. Testing on Bazzite (immutable Fedora Linux) with AMD Ryzen AI MAX+ 395 + Radeon 8060S iGPU + 128GB unified memory revealed that the default Ollama setup underperforms, and a tuned llama.cpp Vulkan configuration is significantly faster.

## Hardware
- **CPU**: AMD Ryzen AI MAX+ 395 (16 cores, 32 threads)
- **GPU**: Radeon 8060S iGPU (RDNA 3.5, gfx1105, 40 CUs, 2900 MHz)
- **Memory**: 128GB unified RAM (shared CPU/GPU)
- **OS**: Bazzite (immutable Fedora, rpm-ostree)

## Benchmarks

### Ollama (Vulkan backend)

| Model | Context | Prompt tok/s | Generate tok/s |
|-------|---------|-------------|----------------|
| qwen3:4b Q4_K_M | 128K | ~200 | 49.2 |
| qwen3.5:9b Q4_K_M | 128K | ~200 | 27.3 |
| qwen3.5:27b Q4_K_M | 128K | ~100 | 9.7 |

Key finding: 27b is **compute bound** (same 9.7 tok/s at 4K or 128K context). Memory is not the bottleneck — 128GB unified RAM fits any model + KV cache.

### llama.cpp (Vulkan, direct)

| Model | Context | Prompt tok/s | Generate tok/s |
|-------|---------|-------------|----------------|
| qwen3:4b Q4_K_M | 512 | **1522** | **63.3** |
| qwen3:8b Q4_K_M | 512 | **882** | **34.3** |
| qwen3:8b Q4_K_M | 128K (q4 KV) | **325** | **32.8** |

**llama.cpp is 25-30% faster on generation and 4-7x faster on prompt processing** vs Ollama.

### ROCm Status
- ROCm does not detect the Strix Halo iGPU (`total_vram: 0B`)
- Vulkan works via `OLLAMA_VULKAN=true` or `cmake -DGGML_VULKAN=ON`
- Cooperative matrix multiply detected (`KHR_coopmat`) but not yet leveraged

## Decision

### Production llama-server configuration
```bash
distrobox enter llama-build -- /tmp/llama.cpp/build/bin/llama-server \
  -m <model.gguf> \
  -ngl 99 -t 16 \
  --host 0.0.0.0 --port 8088 \
  -c 131072 \
  -fa on \
  --cache-type-k q4_0 --cache-type-v q4_0 \
  --batch-size 2048 --ubatch-size 512 \
  --parallel 4 \
  --cache-reuse 256
```

### Optimizations applied
| Optimization | Flag | Effect |
|-------------|------|--------|
| Flash attention | `-fa on` | Faster attention computation |
| q4 KV cache | `--cache-type-k q4_0 --cache-type-v q4_0` | 4x less KV memory |
| Large batch | `--batch-size 2048` | Better prompt throughput |
| 4 parallel slots | `--parallel 4` | Concurrent swarm workers |
| Prefix caching | `--cache-reuse 256` | Shared system prompts across requests |
| Full GPU offload | `-ngl 99` | All layers on Vulkan GPU |

### hex integration
```bash
hex inference add openai-compat http://localhost:8088 --model qwen3-8b --id llama-cpp
hex dev start "feature" --provider openai-compat --model qwen3-8b --dir . --auto
```

### Model tiering for RL engine
| Tier | Model | tok/s | Use case |
|------|-------|-------|----------|
| Fast | qwen3:4b | 63 | Lint checks, simple fixes |
| Standard | qwen3:8b | 34 | ADR, workplan, code gen, review |
| Quality | qwen3.5:27b | 10 | Complex multi-file gen, escalation |

### Ollama configuration (fallback)
```ini
# /etc/systemd/system/ollama.service.d/override.conf
[Service]
Environment=OLLAMA_MODELS=/home/gary/.ollama/models
Environment=HOME=/home/gary
Environment=HSA_OVERRIDE_GFX_VERSION=11.0.0
Environment=OLLAMA_VULKAN=true
Environment=OLLAMA_FLASH_ATTENTION=true
Environment=OLLAMA_KV_CACHE_TYPE=q8_0
User=gary
```

## Consequences
- llama-server is the primary inference engine on Bazzite (30% faster than Ollama)
- Ollama kept as fallback for Qwen3.5 models (not yet supported in llama.cpp)
- 4 parallel slots enable concurrent swarm worker inference
- 128K context fits entire project structure in working memory
- Compute-bound at ~33 tok/s for 8b model on 40 CU Vulkan GPU
