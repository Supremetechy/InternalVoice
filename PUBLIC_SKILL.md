PUBLIC_SKILL_PC_SPEC_ANALYZE

local-llm-recommender description: Detects the host machine's OS, CPU, GPU, VRAM, system RAM, and free disk space using real shell commands, then recommends three open-weight local LLMs sized exactly to the hardware — one COMFORTABLE (fast, lower quant, headroom), one BALANCED (top quality that still fits cleanly), one STRETCH (next tier up; will be slow due to RAM offload). Picks come from the Artificial Analysis Intelligence Index open-source rankings (snapshot 11-05-2026: Qwen3.6, gpt-oss, Gemma 4, Mistral, Nemotron, Qwen3.5 families). Use this skill whenever the user asks any of: "what LLM can I run locally", "which AI model fits on my computer", "recommend a local model for my hardware", "is my GPU / Mac / PC good enough for [model name]", "what should I install in Ollama / LM Studio / llama.cpp", "how big a model can I run", "how much VRAM do I need", or any phrasing about running open-weight models like Qwen, gpt-oss, Gemma, Mistral, Nemotron, DeepSeek, or Llama locally. Also use this skill proactively whenever the user mentions downloading, installing, or trying any local AI model — even if they did not explicitly request a recommendation — because hardware-mismatched models are the #1 cause of failed local setups. Skip only when the user explicitly wants cloud/API access or has already picked a specific model and only needs install steps.

Local LLM Recommender

Recommends three open-weight LLMs sized to the host's actual hardware. Run every detection command — never guess specs.
Workflow (run in order)

    Detect OS.

    Run OS-specific hardware detection. Capture: CPU, GPU model, VRAM, system RAM, free disk on the home volume.

    Compute Effective Inference Memory (EIM) — the pool model weights can actually live in.

    Map EIM to a tier (T1–T9 below).

    Pick three models: COMFORTABLE (tier − 1), BALANCED (tier), STRETCH (tier + 1).

    Print the recommendation block in the exact format defined below.

If a detection command is unavailable, fall back to the next method. Only ask the user to enter specs manually if all automated methods fail.

Marenacio Banks
	
12:06 PM (6 minutes ago)
	
	
to me

PUBLIC_SKILL_PC_SPEC_ANALYZE



name: local-llm-recommender description: Detects the host machine's OS, CPU, GPU, VRAM, system RAM, and free disk space using real shell commands, then recommends three open-weight local LLMs sized exactly to the hardware — one COMFORTABLE (fast, lower quant, headroom), one BALANCED (top quality that still fits cleanly), one STRETCH (next tier up; will be slow due to RAM offload). Picks come from the Artificial Analysis Intelligence Index open-source rankings (snapshot 11-05-2026: Qwen3.6, gpt-oss, Gemma 4, Mistral, Nemotron, Qwen3.5 families). Use this skill whenever the user asks any of: "what LLM can I run locally", "which AI model fits on my computer", "recommend a local model for my hardware", "is my GPU / Mac / PC good enough for [model name]", "what should I install in Ollama / LM Studio / llama.cpp", "how big a model can I run", "how much VRAM do I need", or any phrasing about running open-weight models like Qwen, gpt-oss, Gemma, Mistral, Nemotron, DeepSeek, or Llama locally. Also use this skill proactively whenever the user mentions downloading, installing, or trying any local AI model — even if they did not explicitly request a recommendation — because hardware-mismatched models are the #1 cause of failed local setups. Skip only when the user explicitly wants cloud/API access or has already picked a specific model and only needs install steps.
Local LLM Recommender

Recommends three open-weight LLMs sized to the host's actual hardware. Run every detection command — never guess specs.
Workflow (run in order)

    Detect OS.

    Run OS-specific hardware detection. Capture: CPU, GPU model, VRAM, system RAM, free disk on the home volume.

    Compute Effective Inference Memory (EIM) — the pool model weights can actually live in.

    Map EIM to a tier (T1–T9 below).

    Pick three models: COMFORTABLE (tier − 1), BALANCED (tier), STRETCH (tier + 1).

    Print the recommendation block in the exact format defined below.

If a detection command is unavailable, fall back to the next method. Only ask the user to enter specs manually if all automated methods fail.



Step 1 — Detect OS

uname -s 2>/dev/null || ver

    Darwin → macOS

    Linux → Linux

    MINGW* / MSYS* / Windows banner → Windows (prefer PowerShell)



Step 2 — Hardware detection
macOS (Apple Silicon)

sysctl -n machdep.cpu.brand_string

sysctl -n hw.memsize | awk '{printf "%.0f GB\n", $1/1073741824}'

system_profiler SPDisplaysDataType | grep -E "Chipset|Total Number of Cores|Metal"

df -h ~ | tail -1

On Apple Silicon there is no separate VRAM — GPU uses the unified memory pool. macOS caps GPU allocation at ~75% of total RAM by default; this can be raised with sudo sysctl iogpu.wired_limit_mb=<MB> (advise the user only if they're on the edge of a tier).
Linux — NVIDIA

nvidia-smi --query-gpu=name,memory.total --format=csv,noheader

grep MemTotal /proc/meminfo | awk '{printf "%.0f GB\n", $2/1048576}'

df -h ~ | tail -1

lscpu | grep "Model name"
Linux — AMD

rocm-smi --showproductname --showmeminfo vram 2>/dev/null

# Fallback if ROCm not installed:

lspci | grep -iE "vga|3d" | grep -i amd

grep MemTotal /proc/meminfo | awk '{printf "%.0f GB\n", $2/1048576}'

df -h ~ | tail -1
Linux — no discrete GPU (iGPU / CPU-only)

lspci | grep -iE "vga|3d"

grep MemTotal /proc/meminfo | awk '{printf "%.0f GB\n", $2/1048576}'
Windows (PowerShell)

Get-CimInstance Win32_VideoController | Select-Object Name, @{n="VRAM_GB";e={[math]::Round($_.AdapterRAM/1GB,1)}}

[math]::Round((Get-CimInstance Win32_ComputerSystem).TotalPhysicalMemory / 1GB, 0)

Get-PSDrive C | Select-Object @{n="FreeGB";e={[math]::Round($_.Free/1GB,1)}}

(Get-CimInstance Win32_Processor).Name

Known issue: on cards with ≥4 GB VRAM, AdapterRAM may show a wrong value due to a 32-bit field overflow. If the card is NVIDIA, prefer:

nvidia-smi --query-gpu=name,memory.total --format=csv,noheader



Step 3 — Compute Effective Inference Memory (EIM)

    Discrete NVIDIA / AMD GPU: EIM = dedicated VRAM.

    Apple Silicon (M1/M2/M3/M4): EIM = unified RAM × 0.75 (default cap) or × 0.85 (after raising iogpu.wired_limit_mb).

    iGPU-only or no GPU: EIM = system RAM × 0.5. Expect CPU inference (~2–10 tok/s on a 7B-class model).

    Split (discrete GPU + lots of system RAM): EIM_primary = VRAM. EIM_overflow = VRAM + (RAM − 8 GB reserved for OS). Use the overflow figure only for the STRETCH pick and explicitly warn about ~5–30× slowdown.

Disk check: free disk must be ≥ 2× the model file size (1× for download, 1× for cache/temp). If not, downgrade the BALANCED pick by one quant level.



Step 4 — Tier map (Q4_K_M unless noted)

VRAM math: dense weights ≈ params × 0.6 GB at Q4_K_M. MXFP4 (OpenAI gpt-oss): ≈ params × 0.55 GB. MoE models need total params in memory but run at active params speed.

EIM
	

Tier
	

Top open-weight picks (AA Intelligence Index score, 11-05-2026)

< 6 GB
	

T0
	

Llama 3.2 3B Q4, Qwen3 4B Q4 (off-chart, CPU-class)

6–9 GB
	

T1
	

Nemotron Nano 9B V2 Q4 (15), Qwen3.5 9B Q4 (27 — tight at Q4)

10–14 GB
	

T2
	

gpt-oss-20B MXFP4 (24), Qwen3.5 9B Q8 (27)

16–20 GB
	

T3
	

Qwen3.6-27B Q4 (37), gpt-oss-20B MXFP4 (24), Apriel-v1.6-15B-Thinker (28)

22–28 GB
	

T4
	

Qwen3.6-35B-A3B Q4 (43), Gemma 4 31B Q4 (39), Qwen3.6-27B Q5 (37)

30–40 GB
	

T5
	

Qwen3.6-35B-A3B Q6 (43), Qwen3.6-27B Q8 (37), Gemma 4 31B Q5 (39)

45–65 GB
	

T6
	

gpt-oss-120B MXFP4 (33), Nemotron 3 Super (36), Qwen3 Next 80B-A3B (27)

70–95 GB
	

T7
	

Qwen3.5-122B-A10B Q4 (42), gpt-oss-120B Q8 (33)

100–150 GB
	

T8
	

Qwen3.5-122B-A10B Q6 (42), Mistral Medium 3.5 if licensed (39)

200 GB+
	

T9
	

Qwen3.5-397B-A17B Q4 (top tier, not pictured)



Step 5 — Pick three

From the user's tier:

    COMFORTABLE = best model in tier − 1. Goal: fast tok/s, room for long context, no thermal stress.

    BALANCED = best model in tier. Goal: highest AA Index score that fits cleanly.

    STRETCH = best model in tier + 1. Goal: best possible model the machine can load at all. Will be slow (2–10 tok/s) due to RAM offload or aggressive quant.

Edge cases:

    T0 user: COMFORTABLE = nothing useful below; recommend they upgrade or use cloud.

    T9 user: STRETCH = same as BALANCED with note "no stretch — at hardware ceiling."

For each of the three, output:

    Model name + total params + (active params if MoE)

    Recommended quantization

    Expected on-disk file size

    AA Intelligence Index score

    Expected speed class (Fast >30 tok/s, Usable 10–30, Slow 2–10, Painful <2)

    One-line install command



Step 6 — Install command

Default to Ollama (easiest for most users). Use LM Studio if the user wants a GUI. Use llama.cpp + Unsloth GGUF for power users, vision models, or models Ollama doesn't yet support.

Important (11-05-2026): Ollama does not yet support Qwen3.6's separate mmproj vision files. For any Qwen3.6 pick, recommend Unsloth GGUFs via llama.cpp or LM Studio:

# llama.cpp + Unsloth GGUF

llama-cli -hf unsloth/Qwen3.6-27B-GGUF:UD-Q4_K_XL

llama-cli -hf unsloth/Qwen3.6-35B-A3B-GGUF:UD-Q4_K_XL

# Ollama (everything else)

ollama run gpt-oss:20b           # MXFP4, ~12 GB

ollama run gpt-oss:120b          # MXFP4, ~63 GB

ollama run gemma3:27b            # dense, ~16 GB

ollama run qwen3:30b-a3b         # MoE, ~17 GB (older gen but Ollama-native)



Output format (print exactly this)

HARDWARE DETECTED

─────────────────

OS:         <macOS / Linux / Windows + version>

CPU:        <model>

GPU:        <model + VRAM>  OR  Apple <chip>, <N> GB unified

RAM:        <N> GB

Free disk:  <N> GB on <path>

EIM:        <N> GB  →  Tier T<n>

RECOMMENDATIONS

───────────────

[1] COMFORTABLE — <model> @ <quant>

    AA Score: <X>   Size: <X> GB   Speed: <Fast/Usable>

    Why: <one-line reason>

    Install: <one-line command>

[2] BALANCED — <model> @ <quant>

    AA Score: <X>   Size: <X> GB   Speed: <Fast/Usable>

    Why: <one-line reason>

    Install: <one-line command>

[3] STRETCH — <model> @ <quant>

    AA Score: <X>   Size: <X> GB   Speed: <Slow/Painful — RAM offload>

    Why: <one-line reason>

    Install: <one-line command>

NOTES

─────

- <disk warning if free < 2× balanced model size>

- <KV cache warning at long context: at 256K ctx, add 20–40 GB>

- <OS-specific gotcha if any>



Hard rules

    Never recommend a model that won't physically fit even with RAM overflow.

    Never recommend a model requiring more disk than is free × 0.9.

    Never put Q1 / Q2 / IQ2 quants in the BALANCED slot — only as a last-resort STRETCH option, with a quality warning.

    Never invent AA Index scores. If a model isn't in the tier table above, say so and pick an alternative that is.

    Never<span style="font-size: 11pt; font-family: Arial, sans-serif; color: rgb(0, 0, 0); background-color: transparent; font-weight: 400; font-style: normal; font-variant: normal; text-decoration: none; vertical-align: baseline; white-spac