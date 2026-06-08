---
language:
- en
license: other
license_name: civitai-various
license_link: https://civitai.com/content/tos
tags:
- image
- generated-image
- ai-art
- civitai
- stable-diffusion
- flux
- daily-snapshot
annotations_creators:
- found
language_creators:
- other
multilinguality:
- monolingual
size_categories:
- 100K<n<1M
source_datasets:
- original
task_categories:
- other
pretty_name: Civistash — Daily Top CivitAI Images
configs:
- config_name: default
  data_files: "*.tar.gz"
  default: true
---

# Dataset Card for Civistash

## Dataset Summary

**Civistash** is a daily snapshot of the top images and videos from
[CivitAI](https://civitai.com), the largest community platform for
AI-generated media. Each day it fetches the most-reacted-to content on the
platform, downloads the media files, and stores them alongside full metadata
sidecars in timestamped daily WebDataset shards.

The dataset is designed as a **rolling archive** — each `.tar.gz` shard is a
self-contained WebDataset partition of one day's popular content, making it
easy for downstream projects to consume a specific date range without
processing the entire repository.

## Format

This is a **WebDataset** — every `.tar.gz` shard contains paired files
sharing the same stem (the CivitAI image ID). A sample is the tuple of
all files with a given stem:

```
2026-06-08.tar.gz
├── 12345678.png        # media file
├── 12345678.json       # full CivitAI metadata
├── 98765432.jpg        # media file
├── 98765432.json       # full CivitAI metadata
├── 55555555.mp4        # video file
└── 55555555.json       # full CivitAI metadata
```

The dataset viewer groups files by stem and decodes them per extension:

| Extension | Decoded as |
|---|---|
| `.jpg`, `.png`, `.webp` | `Image` (preview) |
| `.mp4` | `Video` |
| `.json` | `Json` (full sidecar, see schema below) |

One row in the viewer = one image/video. A shard is one day.

### Sidecar JSON schema

Each `<id>.json` sidecar contains the full CivitAI API response for that
item: model info, generation parameters, base model, dimensions, creator
username, tags, stats — plus a `_civistash` provenance block with the
download timestamp, source URL, on-disk path, and archive date.

```json
{
  "id": 12345678,
  "url": "https://image.civitai.com/…",
  "type": "image",
  "nsfw": "None",
  "width": 1024,
  "height": 1536,
  "hash": "abc123...",
  "meta": {
    "prompt": "a beautiful landscape...",
    "negativePrompt": "blurry, low quality...",
    "cfgScale": 7,
    "sampler": "Euler a",
    "seed": 1234567890,
    "steps": 20
  },
  "modelVersionId": 98765,
  "modelId": 5432,
  "username": "some_creator",
  "createdAt": "2026-06-08T10:30:00.000Z",
  "stats": {
    "reactionCount": 1420,
    "commentCount": 89,
    "cryCount": 3,
    "likeCount": 1420
  },
  "tags": [
    { "id": 1, "name": "landscape" },
    { "id": 2, "name": "digital painting" }
  ],
  "_civistash": {
    "downloaded_at": "2026-06-08T14:30:00Z",
    "source_url": "https://image.civitai.com/…",
    "stored_as": "2026-06-08/12345678.png",
    "archive_date": "2026-06-08"
  }
}
```

## Supported Tasks

- **Text-to-image research** — study real-world prompt patterns, CFG scale
  distributions, sampler preferences, and step counts from a large community
  of AI image generators.
- **Aesthetic analysis** — correlate community engagement (reactions,
  comments) with generation parameters and model choice.
- **Trend analysis** — track which models, styles, and tags dominate the
  CivitAI platform over time.
- **Dataset augmentation** — use metadata (prompts, tags) as weak captions
  for image-captioning or CLIP-style training.
- **Model benchmarking** — compare outputs of different base models and
  fine-tunes against community-voted favorites.

## Languages

Metadata and prompts are primarily in **English**. Tags use a controlled
vocabulary from the CivitAI platform. Some prompts may contain fragments of
other languages (Japanese, Chinese, etc.) when creators use multilingual
descriptions.

## Dataset Structure

### Data Splits

There is no train/validation/test split — this is a raw archive. One shard
per day, named `YYYY-MM-DD.tar.gz`.

| Split | Description |
|-------|-------------|
| `default` (train) | Every image from every daily shard, ordered by date |

### Data Fields

The sidecar JSON is exposed as the `json` column. The media file is exposed
as `jpg` / `png` / `webp` / `mp4` depending on its type. Other fields:

| Sidecar key | Type | Description |
|---|---|---|
| `id` | integer | CivitAI image ID |
| `url` | string | Direct media URL on CivitAI CDN |
| `type` | string | `image` or `video` |
| `nsfw` | string | NSFW level: `None`, `Soft`, `Mature`, `X` |
| `width` | integer | Image width in pixels |
| `height` | integer | Image height in pixels |
| `hash` | string | Perceptual hash |
| `meta` | object | Generation parameters (prompt, negative prompt, CFG scale, sampler, seed, steps, etc.) |
| `modelVersionId` | integer | Specific model version used |
| `modelId` | integer | Base model ID |
| `username` | string | CivitAI creator username |
| `createdAt` | datetime | When the image was posted |
| `stats` | object | Reaction count, comment count, cry count, like count |
| `tags` | array | Tag objects with `id` and `name` |
| `_civistash.downloaded_at` | datetime | Civitash fetch timestamp |
| `_civistash.source_url` | string | Same as `url`, kept for provenance |
| `_civistash.stored_as` | string | Local on-disk path (relative to stash root) |
| `_civistash.archive_date` | string | YYYY-MM-DD — which daily shard this lives in |

## Dataset Creation

### Curation Rationale

CivitAI is the largest public repository of AI-generated images, with
millions of uploads and an active community voting system. However, it has
no official bulk-export or historical snapshot API. This dataset fills that
gap by providing a scheduled, reproducible archive of the platform's most
popular daily content.

### Source Data

All data originates from the **CivitAI public API** (`GET /api/v1/images`).
Media files are downloaded from the CivitAI CDN. No scraping of the website
HTML is performed.

#### Collection Process

1. Query the API for the top images of the current period (day, week, month,
   or all-time), sorted by most reactions.
2. Skip any images already present in the local archive (deduplication by ID
   across all date partitions).
3. Download each image sequentially with retry backoff (1s/2s/4s) on rate
   limits and transport errors.
4. Write a JSON sidecar containing the full API response plus a `_civistash`
   provenance block.
5. Bundle the day's partition into a `.tar.gz` WebDataset shard (file pairs
   at the tarball root, grouped by CivitAI image ID) and upload to this
   Hugging Face dataset repository.

### Annotations

The metadata fields (`meta`, `tags`, `stats`, `modelVersionId`, etc.) are
provided directly by the CivitAI API and are **not** annotated by the
Civistash tool. The `_civistash` provenance block is the only addition.

### Personal and Sensitive Information

CivitAI usernames are public by design. No private user data or
authentication tokens are included in the archive. Media flagged as NSFW may
be present depending on the archive configuration — the `nsfw` field in each
sidecar allows downstream consumers to filter content.

## Considerations for Using the Data

### Biases

The dataset reflects the popularity bias of the CivitAI platform: only the
most-reacted-to images are included, which skews toward content that engages
the platform's userbase. Model representation is biased toward popular base
models (Stable Diffusion variants, Flux, etc.). This is not a random sample
of AI-generated content — it is explicitly a **popularity-ranked snapshot**.

### Licensing

The media files and metadata in this dataset are sourced from **CivitAI**
and are subject to the [CivitAI Terms of Service](https://civitai.com/content/tos).
Individual images may carry additional licenses set by their creators.
Consumers of this dataset are responsible for complying with all applicable
licenses and terms.

## How to self-host / run the archiver

This dataset is produced by **Civistash**, an open-source Rust CLI tool.
You can run your own instance to archive different periods, sort orders,
NSFW levels, or upload to your own Hugging Face repo.

**Source code:** [github.com/Hyphonical/Civistash](https://github.com/Hyphonical/Civistash)

```bash
# One-shot: fetch and bundle today's top 200 images
civistash --period Day --limit 200 --bundle

# Daemon: run continuously with daily cycles, auto-upload to HF
civistash --daemon --period Day --limit 1000 --bundle --upload-hf your-org/your-dataset

# Docker
echo "CIVITAI_TOKEN=eyJ…" > .env
echo "HUGGINGFACE_TOKEN=hf_…" >> .env
docker compose up -d
```

Full documentation is available in the [project README](https://github.com/Hyphonical/Civistash).

## Additional Information

### Dataset Curators

This dataset is maintained by [Hyphonical](https://github.com/Hyphonical)
using the automated Civistash archiver.

### Licensing Information

Media and metadata sourced from [CivitAI](https://civitai.com). Refer to
the [CivitAI Terms of Service](https://civitai.com/content/tos) and
individual content licenses for usage terms.

The Civistash tool itself is licensed under the [MIT License](https://github.com/Hyphonical/Civistash/blob/main/LICENSE).

### Citation

If you use this dataset in research, please cite:

```bibtex
@misc{civistash2026,
  author = {Hyphonical},
  title = {Civistash: Daily Top CivitAI Images Archive},
  year = {2026},
  publisher = {Hugging Face},
  howpublished = {\url{https://huggingface.co/datasets/Hyphonical/Civistash}},
  note = {Archived with the Civistash tool: \url{https://github.com/Hyphonical/Civistash}}
}
```

### Contributions

Archiving is fully automated. For issues or feature requests, please open an
issue on the [GitHub repository](https://github.com/Hyphonical/Civistash).
