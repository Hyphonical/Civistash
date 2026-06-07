# Configuration reference

Every option is available as a CLI flag. Two of them also read environment
variables. Run `civistash --help` for a live summary.

---

## All flags

### `--daemon`

| | |
|---|---|
| Type | `bool` |
| Default | `false` |
| Env | none |

When `true`, the process runs continuously: fetch, download, optionally
bundle and upload, then sleep for the cooldown duration before repeating.

**Constraint**: `--daemon` cannot be combined with `--period AllTime`. There
is no sensible cooldown for AllTime — run a one-shot instead.

Graceful shutdown: `SIGTERM` or `Ctrl+C` finishes the current download and
exits cleanly. Two `SIGTERM` signals in quick succession force an immediate
exit.

### `--period`

| | |
|---|---|
| Type | `Period` enum |
| Default | `Day` |
| Values | `Day`, `Week`, `Month`, `AllTime` |

The CivitAI time window for "popular" images. In daemon mode this also
determines the cooldown between cycles:

| Period | Cooldown |
|---|---|
| `Day` | 24 hours (86,400 s) |
| `Week` | 7 days (604,800 s) |
| `Month` | 30 days (2,592,000 s) |
| `AllTime` | *(not supported in daemon mode)* |

### `--sort`

| | |
|---|---|
| Type | `SortOrder` enum |
| Default | `MostReactions` |
| Values | `MostReactions`, `MostComments`, `Newest`, `Oldest` |

Determines the ranking of images returned by the CivitAI API.

### `--limit`

| | |
|---|---|
| Type | `u32` |
| Default | `100` |

Number of images to fetch per cycle. Values above 200 trigger cursor-based
pagination; each page fetches 200 items until the limit is reached.

Set to `0` to fetch nothing — useful when you only want to bundle and upload
existing data.

### `--nsfw-level`

| | |
|---|---|
| Type | comma-separated `NsfwLevel` enum(s) |
| Default | (empty — no NSFW filter sent) |

If set, the values are OR'd into a `browsingLevel` bitmask sent to
the CivitAI API. Values:

| Value | Bitmask position |
|---|---|
| `none` | 1 |
| `soft` | 2 |
| `mature` | 4 |
| `x` | 8 |

Examples:
- `--nsfw-level none` → `browsingLevel=1` (SFW only)
- `--nsfw-level mature,x` → `browsingLevel=12` (mature + explicit)

When omitted entirely, no `browsingLevel` parameter is sent. The API
defaults to whatever your CivitAI account's browsing preferences allow.

### `--all-types`

| | |
|---|---|
| Type | `bool` |
| Default | `false` |

By default only images (`type=image`) are downloaded. When `true`, all media
types (image, video, audio, etc.) are kept. Filtered items are counted in the
cycle summary but not downloaded.

The file extension is derived from the image URL, not the API type field.

### `--bundle`

| | |
|---|---|
| Type | `bool` |
| Default | `false` |

After the cycle completes, tars and gzips the date partition
(`stash/YYYY-MM-DD/`) into `stash/YYYY-MM-DD.tar.gz`. The archive root
directory is the date string, so extracting produces `YYYY-MM-DD/` containing
all images and sidecars.

If the partition directory does not exist (first run, no images downloaded),
an effectively-empty tarball is still created.

### `--upload-hf`

| | |
|---|---|
| Type | `String` (repo ID) |
| Default | (none) |
| Env | requires `HUGGINGFACE_TOKEN` |

Uploads the tarball to a Hugging Face dataset repository. Uses the
[`hf-hub`](https://crates.io/crates/hf-hub) crate. The repo is created if
it doesn't already exist.

Requires `--bundle` (the upload targets the tarball). The env var
`HUGGINGFACE_TOKEN` must be set to a Hugging Face write-access token.

Format: `owner/repo-name` (e.g. `my-org/civistash-daily`).

### `--delete-after`

| | |
|---|---|
| Type | `bool` |
| Default | `false` |

Deletes local files after a successful Hugging Face upload. Requires both
`--bundle` and `--upload-hf` — it has no effect without them.

On a successful upload:
1. The tarball is deleted (HF has the bytes).
2. Date partitions older than 2 days are removed. This preserves `today`
   and `yesterday`, covering the rolling 24-hour window CivitAI uses for
   `period=Day`.

If the upload fails, nothing is deleted — the next cycle will retry with
the same partition.

### `--output-dir`

| | |
|---|---|
| Type | `PathBuf` |
| Default | `stash` |

Base directory for all downloads, sidecars, bundles, and tarballs. Created
automatically on first use.

### `--ca-token`

| | |
|---|---|
| Type | `Option<String>` |
| Default | (none) |
| Env | `CIVITAI_TOKEN` |

CivitAI API key, sent as a `Bearer` token on requests to
`/api/v1/images`. Optional — the API works without authentication for
public content — but you are more likely to hit rate limits without one.

Obtain from: CivitAI → Account Settings → API Keys.

### `--hf-token`

| | |
|---|---|
| Type | `Option<String>` |
| Default | (none) |
| Env | `HUGGINGFACE_TOKEN` |

Hugging Face access token with write permission. Required when `--upload-hf`
is set. Ignored otherwise.

Obtain from: huggingface.co → Settings → Access Tokokens.

### `--log-level`

| | |
|---|---|
| Type | `String` |
| Default | `info` |

Log verbosity for `civistash::*` messages. Third-party crate logs default to
`warn`. Values: `trace`, `debug`, `info`, `warn`, `error`.

The `RUST_LOG` env var overrides this flag entirely:
```bash
RUST_LOG=civistash=debug,hf_hub=info,hf_hub::api=warn
```

---

## Flag compatibility matrix

| Combination | Allowed? |
|---|---|
| `--daemon` + `--period AllTime` | **No** — no sensible cooldown |
| `--upload-hf` without `--bundle` | **No** — upload targets the tarball |
| `--delete-after` without `--upload-hf` | Allowed but no-op |
| `--delete-after` without `--bundle` | Allowed but no-op |
| `--daemon` + `--upload-hf` + `--delete-after` | **Yes** — fully automated pipeline |
| `--period Day` + `--daemon` | Yes (24h cooldown) |
| `--period AllTime` + one-shot | Yes |

---

## NSFW behavior

- No `--nsfw-level` flag: no `browsingLevel` parameter sent. API returns
  whatever your account preferences allow.
- `--nsfw-level none`: `browsingLevel=1`, returns SFW content only.
- Multiple values are OR'd: `--nsfw-level none,soft` → `browsingLevel=3`,
  returns both SFW and soft images.
- The `Image.nsfw` field and the more granular `Image.nsfw_level` are
  preserved in the sidecar JSON but not used for filtering (the API
  already filtered).

---

## Tracing setup

Logging uses `tracing-subscriber` with the `env-filter` feature. The
effective filter is resolved as:

1. If `RUST_LOG` is set in the environment, use it as-is.
2. Otherwise, apply `civistash={--log-level},warn`.

All third-party crates (reqwest, hf-hub, tokio, etc.) default to `warn`
unless explicitly raised via `RUST_LOG`.

Output goes to stderr without timestamps (each cycle prints its duration
in the summary).
