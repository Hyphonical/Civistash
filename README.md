# Civistash

Archiver for popular [CivitAI](https://civitai.com) images with full metadata sidecars.
Fetches the top images from the `/api/v1/images` endpoint, downloads each one
alongside a complete JSON metadata file, and optionally bundles the results into
a `.tar.gz` for upload to Hugging Face.

---

## Quick start

### Native (Rust)

Requires Rust **1.89.0** or later.

```bash
git clone https://github.com/Hyphonical/civistash.git
cd civistash
cargo build --release
```

Run a single cycle — fetch the top 100 images of the day:

```bash
export CIVITAI_TOKEN="eyJ…"
./target/release/civistash --limit 100 --period Day
```

The `CIVITAI_TOKEN` is a CivitAI API key from your account settings.
It is **optional** — omitting it still works for public content — but you may
hit rate limits faster without one.

### Docker

```bash
echo "CIVITAI_TOKEN=eyJ…" > .env
docker compose up -d
```

This runs the daemon daily with all NSFW levels, bundles each day, and
persists everything in `./stash`. See [docs/deployment.md](docs/deployment.md)
for the full Docker and systemd setup.

---

## What it does

```
CivitAI API                    Civistash                         Hugging Face
    │                              │                                  │
    │  GET /api/v1/images          │                                  │
    │  (Most Reactions, Day, N)    │                                  │
    ├─────────────────────────────>│                                  │
    │                              │                                  │
    │  JSON page (up to 200 items) │                                  │
    │<─────────────────────────────│                                  │
    │                              │                                  │
    │                              │  Download each image             │
    │  GET {image.url}             │  to stash/2026-06-07/{id}.{ext}  │
    │<─────────────────────────────│                                  │
    │                              │                                  │
    │                              │  Write metadata sidecar          │
    │                              │  stash/2026-06-07/{id}.json      │
    │                              │                                  │
    │                              │  --bundle: tar+gzip partition    │
    │                              │  stash/2026-06-07.tar.gz         │
    │                              │                                  │
    │                              │  --upload-hf: push .tar.gz       │
    │                              │  to Hugging Face dataset repo    │
    │                              ├─────────────────────────────────>│
    │                              │                                  │
    │                              │  --delete-after: clean local     │
    │                              │  (keeps 2 days as rolling window)│
```

1. **Fetch** – hits `GET /api/v1/images` with your chosen period, sort order,
   NSFW level, and count. Pages via `cursor` when `--limit` > 200.
2. **Filter** – by default keeps only `type=image` (skip video/audio/other).
   Pass `--all-types` to keep everything.
3. **Deduplicate** – skips any image whose ID already exists anywhere under
   the output directory (across all date partitions).
4. **Download** – streams each image to a `.partial` file, renames to
   `{id}.{ext}` on success, cleans up on failure. Retries with 1s/2s/4s
   backoff on 429s, 5xx, and transport errors.
5. **Sidecar** – writes `{id}.json` containing the full API response
   flattened into a single object, plus a `_civistash` block with the
   download path, source URL, and archive date.
6. **Bundle** (optional) – tars and gzips the date partition into
   `stash/YYYY-MM-DD.tar.gz` with the date as the archive root directory.
7. **Upload** (optional) – pushes the tarball to a Hugging Face dataset repo.
8. **Clean** (optional) – deletes the tarball (HF already has it) and
   removes date partitions older than 2 days, preserving the 24-hour rolling
   window CivitAI uses for `period=Day`.

---

## Output structure

```
stash/
├── 2026-06-07/
│   ├── 12345678.png          # downloaded image
│   ├── 12345678.json          # metadata sidecar
│   ├── 98765432.jpg
│   └── 98765432.json
├── 2026-06-07.tar.gz          # --bundle output
├── 2026-06-08/
│   ├── 23456789.png
│   └── 23456789.json
└── 2026-06-08.tar.gz
```

Each sidecar JSON contains the full CivitAI API response — model info, stats,
tags, base model, creator username, dimensions — plus a `_civistash` block:

```json
{
  "id": 12345678,
  "url": "https://image.civitai.com/…",
  "width": 1024,
  "height": 1536,
  "meta": { … },
  "_civistash": {
    "downloaded_at": "2026-06-07T14:30:00Z",
    "source_url": "https://image.civitai.com/…",
    "stored_as": "stash/2026-06-07/12345678.png",
    "archive_date": "2026-06-07"
  }
}
```

---

## CLI flags

Run `civistash --help` for the complete reference. Key flags:

| Flag | Default | Description |
|---|---|---|
| `--daemon` | `false` | Run continuously, sleeping between cycles |
| `--period` | `Day` | `Day`, `Week`, `Month`, `AllTime` |
| `--sort` | `MostReactions` | `MostReactions`, `MostComments`, `Newest`, `Oldest` |
| `--limit` | `100` | Images per cycle (cursor-paginated past 200) |
| `--nsfw-level` | (none) | Comma-separated: `none`, `soft`, `mature`, `x` |
| `--all-types` | `false` | Also download video/audio (default: images only) |
| `--bundle` | `false` | Create `.tar.gz` after each cycle |
| `--upload-hf` | (none) | Hugging Face repo ID (e.g. `my-org/my-dataset`) |
| `--delete-after` | `false` | Delete local files after successful HF upload |
| `--output-dir` | `stash` | Where to store downloads and bundles |
| `--log-level` | `info` | `trace`, `debug`, `info`, `warn`, `error` |

### Environment variables

| Variable | Maps to |
|---|---|
| `CIVITAI_TOKEN` | `--ca-token` (optional API key) |
| `HUGGINGFACE_TOKEN` | `--hf-token` (required for `--upload-hf`) |
| `RUST_LOG` | Overrides `--log-level` (format: `civistash=debug,hf_hub=info`) |

---

## Modes

### One-shot

Run a single fetch-download-bundle-upload cycle and exit.

```bash
civistash --period Day --limit 200 --bundle
```

### Daemon

Sleep for the period-appropriate cooldown between cycles, forever. Graceful
shutdown on `SIGTERM` or `Ctrl+C` — the current download finishes before
the process exits.

```bash
civistash --daemon --period Day --limit 200 --bundle
```

Cooldown durations:
- `Day` → 24 hours
- `Week` → 7 days
- `Month` → 30 days
- `AllTime` → **(not allowed with `--daemon`)**

### Upload only

If you already have tarballs, you can run a no-fetch cycle that still iterates
the download phase (nothing to download → 0 images), then bundles and uploads
the current date partition:

```bash
civistash --period Day --limit 0 --bundle --upload-hf my-org/dataset
```

---

## About

- **Language**: Rust (edition 2024, MSRV 1.89.0)
- **Async runtime**: Tokio (single-threaded, connection-pooled reqwest client)
- **License**: MIT
- **Author**: [Hyphonical](https://github.com/Hyphonical)
